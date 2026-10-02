/* xfce4-settings-manager showing a settings dialog through XEmbed, under
 * the compositor: two clients share one frame's redirect backing.
 *
 *   ./xembed-probe redirect | direct
 *
 * Client A owns the frame F (756x534), its client window C at (5,29)
 * 746x500 with a button bar in C's rows 464..500, and a GtkSocket S, a
 * child of C at (8,8) 730x531 clipped by its bounding shape to the 450
 * rows of the viewport. Client B owns the plug P, created inside S at
 * (0,0) 730x531 with depth 24, as xfce4-screensaver-preferences
 * --socket-id does. A scrolls by moving S and shifting its shape (the
 * order of the measured xfce4-settings-manager trace); both repaint
 * from their own pixmaps on Expose. Each step logs F along column x=400
 * and the button-bar row y=509, the Expose events each client got, and
 * whether F's DAMAGE covers every pixel that changed.
 *
 *   cc -O1 -o xembed-probe xembed-scroll-probe.c -lxcb -lxcb-composite \
 *      -lxcb-damage -lxcb-shape -lxcb-xfixes
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <xcb/composite.h>
#include <xcb/damage.h>
#include <xcb/shape.h>
#include <xcb/xcb.h>
#include <xcb/xfixes.h>

#define FRAME 0x202020u
#define GRAY 0x3b3b3eu
#define YELLOW 0xffff00u
#define BLUE 0x0000ffu
#define RED 0xff0000u

#define FW 756
#define FH 534

static xcb_connection_t *ca, *cb;
static xcb_screen_t *s;
static xcb_visualid_t vis32;
static xcb_colormap_t cmap32;
static xcb_window_t f, cw, sock, plug;
static xcb_pixmap_t c_pix, p_pix;
static xcb_gcontext_t c_gc, p_gc;
static xcb_damage_damage_t damage;
static xcb_xfixes_region_t parts;
static uint32_t prev[FW * FH];
static int prev_valid, redirect;

static xcb_connection_t *connect_retry(void)
{
    xcb_connection_t *c;
    /* Xorg resets when the previous run's last client leaves. */
    for (int tries = 0; (c = xcb_connect(NULL, NULL)) && xcb_connection_has_error(c) && tries < 50;
         tries++) {
        xcb_disconnect(c);
        usleep(100000);
    }
    return c;
}

static void sync_conn(xcb_connection_t *c)
{
    free(xcb_get_input_focus_reply(c, xcb_get_input_focus(c), NULL));
}

static const char *colour(uint32_t px)
{
    static char hex[16];
    switch (px) {
    case FRAME: return "frame";
    case GRAY: return "gray";
    case YELLOW: return "yellow";
    case BLUE: return "blue";
    case RED: return "red";
    }
    snprintf(hex, sizeof hex, "%06x", px);
    return hex;
}

static void runs(const uint32_t *img, int x0, int y0, int dx, int dy, int n)
{
    uint32_t run = 0;
    int from = -1, last = 0;
    for (int i = 0; i <= n; i++) {
        int at = dx ? x0 + i * dx : y0 + i * dy;
        uint32_t px = i < n ? img[(y0 + i * dy) * FW + x0 + i * dx] & 0xffffff : ~0u;
        if (from >= 0 && px != run) {
            printf(" %d-%d %s", from, last, colour(run));
            from = -1;
        }
        if (from < 0) {
            from = at;
            run = px;
        }
        last = at;
    }
}

/* Both clients repaint what they are sent Expose for from their pixmaps,
 * GTK's double buffer; ClipByChildren GCs. The events are logged when
 * `log` is set. */
static void handle_exposes(int log)
{
    sync_conn(ca);
    sync_conn(cb);
    usleep(100000);
    for (int pass = 0; pass < 2; pass++) {
        xcb_connection_t *c = pass ? cb : ca;
        xcb_generic_event_t *e;
        while ((e = xcb_poll_for_event(c))) {
            if ((e->response_type & 0x7f) == XCB_EXPOSE) {
                xcb_expose_event_t *x = (xcb_expose_event_t *)e;
                const char *who = x->window == cw ? "C" : x->window == plug ? "P" : NULL;
                if (who && log)
                    printf("  Expose %s %d,%d %ux%u count %u\n", who, x->x, x->y, x->width,
                           x->height, x->count);
                if (who)
                    xcb_copy_area(c, pass ? p_pix : c_pix, x->window, pass ? p_gc : c_gc, x->x,
                                  x->y, x->x, x->y, x->width, x->height);
            }
            free(e);
        }
        sync_conn(c);
    }
}

static void report(const char *when)
{
    sync_conn(cb);
    sync_conn(ca);
    xcb_drawable_t d = s->root;
    xcb_pixmap_t p = XCB_NONE;
    if (!redirect)
        usleep(150000); /* the root reads the last composited frame */
    else {
        p = xcb_generate_id(ca);
        xcb_composite_name_window_pixmap(ca, f, p);
        d = p;
    }
    xcb_get_image_reply_t *r = xcb_get_image_reply(
        ca, xcb_get_image(ca, XCB_IMAGE_FORMAT_Z_PIXMAP, d, 0, 0, FW, FH, ~0u), NULL);
    if (p)
        xcb_free_pixmap(ca, p);
    printf("%s\n", when);
    if (!r) {
        printf("  GetImage failed\n");
        return;
    }
    const uint32_t *img = (const uint32_t *)xcb_get_image_data(r);
    printf("  x=400:");
    runs(img, 400, 0, 0, 3, FH / 3);
    printf("\n  y=509:");
    runs(img, 0, 509, 4, 0, FW / 4);
    printf("\n");

    xcb_damage_subtract(ca, damage, XCB_NONE, parts);
    xcb_xfixes_fetch_region_reply_t *dr =
        xcb_xfixes_fetch_region_reply(ca, xcb_xfixes_fetch_region(ca, parts), NULL);
    if (dr && prev_valid) {
        xcb_rectangle_t *rects = xcb_xfixes_fetch_region_rectangles(dr);
        int n = xcb_xfixes_fetch_region_rectangles_length(dr), missed = 0;
        for (int py = 0; py < FH; py++)
            for (int px = 0; px < FW; px++) {
                if ((img[py * FW + px] & 0xffffff) == (prev[py * FW + px] & 0xffffff))
                    continue;
                int in = 0;
                for (int i = 0; i < n && !in; i++)
                    in = px >= rects[i].x && px < rects[i].x + rects[i].width &&
                         py >= rects[i].y && py < rects[i].y + rects[i].height;
                missed += !in;
            }
        if (missed)
            printf("  damage misses %d changed pixels\n", missed);
        else
            printf("  damage covers every change\n");
    }
    free(dr);
    memcpy(prev, img, sizeof prev);
    prev_valid = 1;
    free(r);
    fflush(stdout);
}

static void find_argb(void)
{
    for (xcb_depth_iterator_t d = xcb_screen_allowed_depths_iterator(s); d.rem;
         xcb_depth_next(&d))
        if (d.data->depth == 32) {
            vis32 = xcb_depth_visuals_iterator(d.data).data->visual_id;
            break;
        }
    cmap32 = xcb_generate_id(ca);
    xcb_create_colormap(ca, XCB_COLORMAP_ALLOC_NONE, cmap32, s->root, vis32);
}

static xcb_window_t window32(xcb_window_t parent, int x, int y, int w, int h, uint32_t bg)
{
    xcb_window_t id = xcb_generate_id(ca);
    uint32_t vals[4] = {0xff000000u | bg, 0, XCB_EVENT_MASK_EXPOSURE, cmap32};
    xcb_create_window(ca, 32, id, parent, x, y, w, h, 0, XCB_WINDOW_CLASS_INPUT_OUTPUT, vis32,
                      XCB_CW_BACK_PIXEL | XCB_CW_BORDER_PIXEL | XCB_CW_EVENT_MASK |
                          XCB_CW_COLORMAP,
                      vals);
    return id;
}

static void fill(xcb_connection_t *c, xcb_drawable_t d, uint32_t px, int x, int y, int w, int h)
{
    xcb_gcontext_t g = xcb_generate_id(c);
    uint32_t v = 0xff000000u | px;
    xcb_create_gc(c, g, d, XCB_GC_FOREGROUND, &v);
    xcb_rectangle_t r = {x, y, w, h};
    xcb_poly_fill_rectangle(c, d, g, 1, &r);
    xcb_free_gc(c, g);
}

static void viewport_shape(int top)
{
    xcb_rectangle_t r = {0, top, 730, 450};
    xcb_shape_rectangles(ca, XCB_SHAPE_SO_SET, XCB_SHAPE_SK_BOUNDING, XCB_CLIP_ORDERING_YX_BANDED,
                         sock, 0, 0, 1, &r);
}

/* As xfce4-settings-manager scrolls: move the socket, then shift its
 * shape so it stays on the viewport (C's rows 8..458). */
static void scroll(int y, const char *when)
{
    uint32_t v = (uint32_t)y;
    xcb_configure_window(ca, sock, XCB_CONFIG_WINDOW_Y, &v);
    viewport_shape(8 - y);
    char line[96];
    snprintf(line, sizeof line, "%s, before Expose", when);
    report(line);
    handle_exposes(1);
    snprintf(line, sizeof line, "%s, Expose handled", when);
    report(line);
}

int main(int argc, char **argv)
{
    redirect = argc > 1 && !strcmp(argv[1], "redirect");
    ca = connect_retry();
    if (xcb_connection_has_error(ca)) {
        printf("cannot connect\n");
        return 1;
    }
    cb = xcb_connect(NULL, NULL);
    s = xcb_setup_roots_iterator(xcb_get_setup(ca)).data;
    free(xcb_composite_query_version_reply(ca, xcb_composite_query_version(ca, 0, 4), NULL));
    free(xcb_damage_query_version_reply(ca, xcb_damage_query_version(ca, 1, 1), NULL));
    free(xcb_xfixes_query_version_reply(ca, xcb_xfixes_query_version(ca, 5, 0), NULL));
    if (redirect)
        xcb_composite_redirect_subwindows(ca, s->root, XCB_COMPOSITE_REDIRECT_MANUAL);
    find_argb();

    /* A: frame, client window with its button bar, socket. */
    f = window32(s->root, 0, 0, FW, FH, FRAME);
    cw = window32(f, 5, 29, 746, 500, GRAY);
    sock = window32(cw, 8, 8, 730, 531, 0);
    viewport_shape(0);
    c_pix = xcb_generate_id(ca);
    xcb_create_pixmap(ca, 32, c_pix, cw, 746, 500);
    fill(ca, c_pix, GRAY, 0, 0, 746, 500);
    fill(ca, c_pix, YELLOW, 8, 464, 730, 36);
    c_gc = xcb_generate_id(ca);
    xcb_create_gc(ca, c_gc, c_pix, 0, NULL);
    xcb_map_window(ca, sock);
    xcb_map_window(ca, cw);
    xcb_map_window(ca, f);
    sync_conn(ca);

    /* B: the plug, depth 24, made inside the socket; its rows past the
     * viewport (450..531) are red. */
    plug = xcb_generate_id(cb);
    uint32_t pv[2] = {0, XCB_EVENT_MASK_EXPOSURE};
    xcb_create_window(cb, 24, plug, sock, 0, 0, 730, 531, 0, XCB_WINDOW_CLASS_INPUT_OUTPUT,
                      s->root_visual, XCB_CW_BACK_PIXEL | XCB_CW_EVENT_MASK, pv);
    p_pix = xcb_generate_id(cb);
    xcb_create_pixmap(cb, 24, p_pix, plug, 730, 531);
    fill(cb, p_pix, BLUE, 0, 0, 730, 450);
    fill(cb, p_pix, RED, 0, 450, 730, 81);
    p_gc = xcb_generate_id(cb);
    xcb_create_gc(cb, p_gc, p_pix, 0, NULL);
    xcb_map_window(cb, plug);
    sync_conn(cb);
    usleep(300000);
    /* GTK paints each window whole on map; the map's own exposures are
     * not what this probe is about. */
    handle_exposes(0);
    xcb_copy_area(ca, c_pix, cw, c_gc, 0, 0, 0, 0, 746, 500);
    xcb_copy_area(cb, p_pix, plug, p_gc, 0, 0, 0, 0, 730, 531);

    damage = xcb_generate_id(ca);
    xcb_damage_create(ca, damage, f, XCB_DAMAGE_REPORT_LEVEL_NON_EMPTY);
    parts = xcb_generate_id(ca);
    xcb_xfixes_create_region(ca, parts, 0, NULL);
    report("mapped, viewport at the top");
    scroll(-73, "scrolled down 81");
    scroll(8, "scrolled back to the top");

    FILE *done = fopen("XEMBED-DONE", "w");
    if (done)
        fclose(done);
    xcb_disconnect(cb);
    xcb_disconnect(ca);
    return 0;
}
