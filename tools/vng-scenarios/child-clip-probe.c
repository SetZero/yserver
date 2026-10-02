/* Drawing into a window that is NOT a top-level, inside a redirected
 * top-level: a GTK client window inside its xfwm4 frame under the
 * compositor, which shares the frame's one redirect backing with the
 * frame's title bar and border windows.
 *
 *   ./probe redirect | direct
 *
 * F is the frame (depth 32, 200x150) with three children: the title bar
 * T (0,0 200x20), the client C (5,20 190x100) and the button bar B
 * (0,125 200x25). C has a child V (100,10 80x120) that reaches past C's
 * bottom, as GTK's scrolled bin window does. In `redirect` the probe is
 * its own compositor (RedirectSubwindows(root, Manual)) and reads F's
 * backing (NameWindowPixmap); in `direct` it reads F on screen, through
 * the root window (F sits at the root's origin).
 *
 * Each stage resets every window to its background, draws into C or V
 * with rects that reach past C's bounds or moves V, and logs F along
 * columns x=50 (C only) and x=145 (through V) and rows y=10 (T), y=25,
 * y=60 and y=137 (B), run-length encoded in F's coordinates, plus the
 * GraphicsExpose and NoExpose events a CopyArea produced. Nothing
 * outside C may change.
 *
 *   cc -O1 -o probe child-clip-probe.c -lxcb -lxcb-composite -lxcb-render -lxcb-shm
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ipc.h>
#include <sys/shm.h>
#include <unistd.h>
#include <xcb/composite.h>
#include <xcb/render.h>
#include <xcb/shm.h>
#include <xcb/xcb.h>

#define FRAME 0x202020u
#define GRAY 0x3b3b3eu
#define BLUE 0x0000ffu
#define GREEN 0x00ff00u
#define MAGENTA 0xff00ffu
#define RED 0xff0000u
#define ORANGE 0xff8000u

#define FW 200
#define FH 150

static xcb_connection_t *c;
static xcb_screen_t *s;
static xcb_visualid_t vis32;
static xcb_colormap_t cmap32;
static xcb_render_pictformat_t fmt32;
static xcb_window_t f, t, cw, b, v;
static int redirect;

static void sync_server(void)
{
    free(xcb_get_input_focus_reply(c, xcb_get_input_focus(c), NULL));
}

static const char *colour(uint32_t px)
{
    static char hex[16];
    switch (px) {
    case FRAME: return "frame";
    case GRAY: return "gray";
    case BLUE: return "blue";
    case GREEN: return "green";
    case MAGENTA: return "magenta";
    case RED: return "red";
    case ORANGE: return "orange";
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

static void events(void)
{
    xcb_generic_event_t *e;
    while ((e = xcb_poll_for_event(c))) {
        switch (e->response_type & 0x7f) {
        case XCB_GRAPHICS_EXPOSURE: {
            xcb_graphics_exposure_event_t *g = (xcb_graphics_exposure_event_t *)e;
            printf("  GraphicsExpose %d,%d %ux%u count %u\n", g->x, g->y, g->width, g->height,
                   g->count);
            break;
        }
        case XCB_NO_EXPOSURE:
            printf("  NoExpose\n");
            break;
        }
        free(e);
    }
}

static void report(const char *when)
{
    sync_server();
    xcb_drawable_t d = s->root;
    xcb_pixmap_t p = XCB_NONE;
    if (!redirect)
        usleep(150000); /* the root reads the last composited frame */
    if (redirect) {
        p = xcb_generate_id(c);
        xcb_composite_name_window_pixmap(c, f, p);
        d = p;
    }
    xcb_get_image_reply_t *r = xcb_get_image_reply(
        c, xcb_get_image(c, XCB_IMAGE_FORMAT_Z_PIXMAP, d, 0, 0, FW, FH, ~0u), NULL);
    if (p)
        xcb_free_pixmap(c, p);
    printf("%s\n", when);
    if (!r) {
        printf("  GetImage failed\n");
        return;
    }
    const uint32_t *img = (const uint32_t *)xcb_get_image_data(r);
    printf("  x=50: ");
    runs(img, 50, 0, 0, 5, FH / 5);
    printf("\n  x=145:");
    runs(img, 145, 0, 0, 5, FH / 5);
    printf("\n  y=10: ");
    runs(img, 0, 10, 5, 0, FW / 5);
    printf("\n  y=25: ");
    runs(img, 0, 25, 5, 0, FW / 5);
    printf("\n  y=60: ");
    runs(img, 0, 60, 5, 0, FW / 5);
    printf("\n  y=137:");
    runs(img, 0, 137, 5, 0, FW / 5);
    printf("\n");
    free(r);
    events();
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
    cmap32 = xcb_generate_id(c);
    xcb_create_colormap(c, XCB_COLORMAP_ALLOC_NONE, cmap32, s->root, vis32);
    xcb_render_query_pict_formats_reply_t *pf =
        xcb_render_query_pict_formats_reply(c, xcb_render_query_pict_formats(c), NULL);
    for (xcb_render_pictscreen_iterator_t ps = xcb_render_query_pict_formats_screens_iterator(pf);
         ps.rem; xcb_render_pictscreen_next(&ps))
        for (xcb_render_pictdepth_iterator_t pd = xcb_render_pictscreen_depths_iterator(ps.data);
             pd.rem; xcb_render_pictdepth_next(&pd))
            for (xcb_render_pictvisual_iterator_t pv =
                     xcb_render_pictdepth_visuals_iterator(pd.data);
                 pv.rem; xcb_render_pictvisual_next(&pv))
                if (pv.data->visual == vis32)
                    fmt32 = pv.data->format;
    free(pf);
}

static xcb_window_t window(xcb_window_t parent, int x, int y, int w, int h, uint32_t bg)
{
    xcb_window_t id = xcb_generate_id(c);
    uint32_t vals[3] = {0xff000000u | bg, 0, cmap32};
    xcb_create_window(c, 32, id, parent, x, y, w, h, 0, XCB_WINDOW_CLASS_INPUT_OUTPUT, vis32,
                      XCB_CW_BACK_PIXEL | XCB_CW_BORDER_PIXEL | XCB_CW_COLORMAP, vals);
    xcb_map_window(c, id);
    return id;
}

static void reset(void)
{
    uint32_t y = 10;
    xcb_configure_window(c, v, XCB_CONFIG_WINDOW_Y, &y);
    xcb_window_t all[] = {f, t, cw, b, v};
    for (unsigned i = 0; i < sizeof all / sizeof all[0]; i++)
        xcb_clear_area(c, 0, all[i], 0, 0, 0, 0);
    sync_server();
    events();
}

static xcb_gcontext_t gc(xcb_drawable_t d, uint32_t fg, uint32_t mode)
{
    xcb_gcontext_t id = xcb_generate_id(c);
    uint32_t vals[2] = {0xff000000u | fg, mode};
    xcb_create_gc(c, id, d, XCB_GC_FOREGROUND | XCB_GC_SUBWINDOW_MODE, vals);
    return id;
}

static void fill(xcb_window_t w, uint32_t mode, int x, int y, int ww, int hh)
{
    xcb_gcontext_t g = gc(w, ORANGE, mode);
    xcb_rectangle_t r = {x, y, ww, hh};
    xcb_poly_fill_rectangle(c, w, g, 1, &r);
    xcb_free_gc(c, g);
}

static uint32_t *orange_image(int w, int h)
{
    uint32_t *px = malloc((size_t)w * h * 4);
    for (int i = 0; i < w * h; i++)
        px[i] = 0xff000000u | ORANGE;
    return px;
}

static xcb_render_picture_t picture(xcb_drawable_t d, uint32_t mode)
{
    xcb_render_picture_t p = xcb_generate_id(c);
    xcb_render_create_picture(c, p, d, fmt32, XCB_RENDER_CP_SUBWINDOW_MODE, &mode);
    return p;
}

static const xcb_render_color_t orange_rc = {0xffff, 0x8080, 0, 0xffff};

static void gc_clip_fill(uint32_t mode, int x, int y, int w, int h)
{
    xcb_gcontext_t g = gc(cw, ORANGE, mode);
    xcb_rectangle_t clip = {x, y, w, h};
    xcb_set_clip_rectangles(c, XCB_CLIP_ORDERING_UNSORTED, g, 0, 0, 1, &clip);
    xcb_rectangle_t r = {-50, -50, 400, 400};
    xcb_poly_fill_rectangle(c, cw, g, 1, &r);
    xcb_free_gc(c, g);
}

int main(int argc, char **argv)
{
    redirect = argc > 1 && !strcmp(argv[1], "redirect");
    /* Xorg resets when the previous run's last client leaves. */
    for (int tries = 0; (c = xcb_connect(NULL, NULL)) && xcb_connection_has_error(c) && tries < 50;
         tries++) {
        xcb_disconnect(c);
        usleep(100000);
    }
    if (xcb_connection_has_error(c)) {
        printf("cannot connect\n");
        return 1;
    }
    s = xcb_setup_roots_iterator(xcb_get_setup(c)).data;
    free(xcb_composite_query_version_reply(c, xcb_composite_query_version(c, 0, 4), NULL));
    free(xcb_render_query_version_reply(c, xcb_render_query_version(c, 0, 11), NULL));
    if (redirect)
        xcb_composite_redirect_subwindows(c, s->root, XCB_COMPOSITE_REDIRECT_MANUAL);
    find_argb();

    f = window(s->root, 0, 0, FW, FH, FRAME);
    t = window(f, 0, 0, 200, 20, MAGENTA);
    cw = window(f, 5, 20, 190, 100, GRAY);
    b = window(f, 0, 125, 200, 25, GREEN);
    v = window(cw, 100, 10, 80, 120, BLUE);
    sync_server();
    usleep(300000);
    reset();
    report("reset");

    /* V scrolled the way GTK moves its bin window: a pure move. */
    {
        xcb_gcontext_t red = gc(v, RED, XCB_SUBWINDOW_MODE_CLIP_BY_CHILDREN);
        xcb_rectangle_t band = {0, 60, 80, 20};
        xcb_poly_fill_rectangle(c, v, red, 1, &band);
        uint32_t y = (uint32_t)-40;
        xcb_configure_window(c, v, XCB_CONFIG_WINDOW_Y, &y);
        report("V: red band at 60, moved from y=10 to y=-40");
        reset();
        xcb_poly_fill_rectangle(c, v, red, 1, &band);
        y = 40;
        xcb_configure_window(c, v, XCB_CONFIG_WINDOW_Y, &y);
        report("V: red band at 60, moved from y=10 to y=40");
        reset();
        xcb_free_gc(c, red);
    }

    fill(cw, XCB_SUBWINDOW_MODE_CLIP_BY_CHILDREN, -50, -50, 400, 400);
    report("C: PolyFillRectangle -50,-50 400x400 ClipByChildren");
    reset();
    fill(cw, XCB_SUBWINDOW_MODE_INCLUDE_INFERIORS, -50, -50, 400, 400);
    report("C: PolyFillRectangle -50,-50 400x400 IncludeInferiors");
    reset();
    fill(v, XCB_SUBWINDOW_MODE_CLIP_BY_CHILDREN, -50, -50, 400, 400);
    report("V: PolyFillRectangle -50,-50 400x400");
    reset();
    gc_clip_fill(XCB_SUBWINDOW_MODE_CLIP_BY_CHILDREN, 20, 50, 40, 200);
    report("C: PolyFillRectangle -50,-50 400x400, clip 20,50 40x200");
    reset();

    {
        uint32_t *px = orange_image(230, 140);
        xcb_gcontext_t g = gc(cw, ORANGE, XCB_SUBWINDOW_MODE_CLIP_BY_CHILDREN);
        xcb_put_image(c, XCB_IMAGE_FORMAT_Z_PIXMAP, cw, g, 230, 140, -20, -20, 0, 32,
                      230 * 140 * 4, (const uint8_t *)px);
        report("C: PutImage -20,-20 230x140");
        reset();

        int id = shmget(IPC_PRIVATE, 230 * 140 * 4, IPC_CREAT | 0600);
        void *mem = id >= 0 ? shmat(id, NULL, 0) : (void *)-1;
        if (mem != (void *)-1) {
            memcpy(mem, px, 230 * 140 * 4);
            xcb_shm_seg_t seg = xcb_generate_id(c);
            xcb_shm_attach(c, seg, (uint32_t)id, 0);
            xcb_shm_put_image(c, cw, g, 230, 140, 0, 0, 230, 140, -20, -20, 32,
                              XCB_IMAGE_FORMAT_Z_PIXMAP, 0, seg, 0);
            report("C: ShmPutImage -20,-20 230x140");
            xcb_shm_detach(c, seg);
            sync_server();
            shmdt(mem);
            shmctl(id, IPC_RMID, NULL);
        } else {
            printf("C: ShmPutImage skipped, no shm segment\n");
        }
        reset();
        xcb_free_gc(c, g);
        free(px);
    }

    {
        xcb_render_picture_t dst = picture(cw, XCB_SUBWINDOW_MODE_CLIP_BY_CHILDREN);
        xcb_rectangle_t r = {-50, -50, 400, 400};
        xcb_render_fill_rectangles(c, XCB_RENDER_PICT_OP_SRC, dst, orange_rc, 1, &r);
        report("C: RENDER FillRectangles -50,-50 400x400");
        reset();

        xcb_rectangle_t clip = {0, 0, 40, 200};
        xcb_render_set_picture_clip_rectangles(c, dst, 20, 50, 1, &clip);
        xcb_render_fill_rectangles(c, XCB_RENDER_PICT_OP_SRC, dst, orange_rc, 1, &r);
        report("C: RENDER FillRectangles -50,-50 400x400, clip 0,0 40x200 at 20,50");
        reset();
        xcb_render_free_picture(c, dst);

        xcb_pixmap_t pm = xcb_generate_id(c);
        xcb_create_pixmap(c, 32, pm, f, 300, 300);
        xcb_render_picture_t src = picture(pm, XCB_SUBWINDOW_MODE_CLIP_BY_CHILDREN);
        xcb_rectangle_t all = {0, 0, 300, 300};
        xcb_render_fill_rectangles(c, XCB_RENDER_PICT_OP_SRC, src, orange_rc, 1, &all);
        dst = picture(cw, XCB_SUBWINDOW_MODE_CLIP_BY_CHILDREN);
        xcb_render_composite(c, XCB_RENDER_PICT_OP_SRC, src, XCB_NONE, dst, 0, 0, 0, 0, -30, -30,
                             300, 300);
        report("C: RENDER Composite -30,-30 300x300");
        reset();
        xcb_rectangle_t clip2 = {0, 0, 40, 40};
        xcb_render_set_picture_clip_rectangles(c, dst, 20, 70, 1, &clip2);
        xcb_render_composite(c, XCB_RENDER_PICT_OP_SRC, src, XCB_NONE, dst, 0, 0, 0, 0, -30, -30,
                             300, 300);
        report("C: RENDER Composite -30,-30 300x300, clip 0,0 40x40 at 20,70");
        reset();
        xcb_render_free_picture(c, dst);
        xcb_render_free_picture(c, src);
        xcb_free_pixmap(c, pm);
    }

    /* Scrolls: a red band in C's left half, copied within it. */
    {
        xcb_gcontext_t red = gc(cw, RED, XCB_SUBWINDOW_MODE_CLIP_BY_CHILDREN);
        xcb_gcontext_t cp = gc(cw, RED, XCB_SUBWINDOW_MODE_CLIP_BY_CHILDREN);
        xcb_rectangle_t band = {0, 0, 100, 30};
        xcb_poly_fill_rectangle(c, cw, red, 1, &band);
        xcb_copy_area(c, cw, cw, cp, 0, 0, 0, 40, 100, 200);
        report("C: CopyArea 0,0 -> 0,40 100x200 (scroll down)");
        reset();
        band.y = 70;
        xcb_poly_fill_rectangle(c, cw, red, 1, &band);
        xcb_copy_area(c, cw, cw, cp, 0, 50, 0, 0, 100, 200);
        report("C: CopyArea 0,50 -> 0,0 100x200 (scroll up)");
        reset();
        band.y = 0;
        xcb_poly_fill_rectangle(c, cw, red, 1, &band);
        xcb_copy_area(c, cw, cw, cp, 0, 0, -20, -40, 100, 140);
        report("C: CopyArea 0,0 -> -20,-40 100x140");
        reset();
        xcb_free_gc(c, red);
        xcb_free_gc(c, cp);
    }

    FILE *done = fopen("PROBE-DONE", "w");
    if (done)
        fclose(done);
    xcb_disconnect(c);
    return 0;
}
