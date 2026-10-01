/* A compositor's fullscreen round (muffin, mutter, picom): RedirectSubwindows
 * (root, Manual), unredirect a fullscreen window, redirect it again when it
 * leaves fullscreen, and keep compositing it from its named pixmap. Another
 * client listens for SubstructureNotify on the root and checks QueryTree.
 *
 *   ./probe <width> <height>
 *
 * After the re-redirect the compositor copies the window's named pixmap into
 * the COW, then the window paints again without a recomposite: a Manual
 * redirect keeps that paint off the screen. READY-0 announces it with the
 * expected colour (the composited one) in expect-0, and the probe waits up to
 * a minute for DONE-0 (the host's scanout dump). probe.log carries no ids, so
 * Xorg and yserver runs diff directly.
 *
 *   cc -O1 -o probe composite-reredirect-probe.c -lxcb -lxcb-composite
 */
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>
#include <xcb/composite.h>
#include <xcb/xcb.h>

#define FIRST 0x3060c0u
#define SECOND 0xc08020u
#define THIRD 0x20a040u

static xcb_connection_t *c, *l;
static xcb_window_t root, cow, win;
static uint16_t sw, sh;

static void check(xcb_connection_t *conn, xcb_void_cookie_t ck, const char *what)
{
    xcb_generic_error_t *e = xcb_request_check(conn, ck);
    printf("%s: %s", what, e ? "error" : "ok");
    if (e)
        printf(" code=%u", e->error_code);
    printf("\n");
    fflush(stdout);
    free(e);
}

static const char *name(xcb_window_t w)
{
    if (w == root)
        return "root";
    if (cow && w == cow)
        return "COW";
    if (w == win)
        return "W";
    return "other";
}

static void query_tree(const char *when)
{
    xcb_query_tree_reply_t *t = xcb_query_tree_reply(l, xcb_query_tree(l, root), NULL);
    if (!t) {
        printf("QueryTree(root) %s: error\n", when);
        return;
    }
    xcb_window_t *kids = xcb_query_tree_children(t);
    int listed = 0;
    for (int i = 0; i < xcb_query_tree_children_length(t); i++)
        listed |= cow && kids[i] == cow;
    printf("QueryTree(root) %s: COW %s\n", when, listed ? "listed" : "not listed");
    fflush(stdout);
    free(t);
}

static void geometry(int16_t x, int16_t y, uint16_t w, uint16_t h, uint16_t bw)
{
    if (x == 0 && y == 0 && w == sw && h == sh)
        printf(" geometry=screen");
    else
        printf(" geometry=%dx%d+%d+%d", w, h, x, y);
    printf(" border=%u", bw);
}

/* Every SubstructureNotify event the listener got, in order. */
static void log_events(void)
{
    xcb_generic_event_t *ev;
    while ((ev = xcb_poll_for_event(l))) {
        switch (ev->response_type & 0x7f) {
        case XCB_CREATE_NOTIFY: {
            xcb_create_notify_event_t *e = (void *)ev;
            printf("event CreateNotify parent=%s window=%s", name(e->parent), name(e->window));
            geometry(e->x, e->y, e->width, e->height, e->border_width);
            printf(" override=%u\n", e->override_redirect);
            break;
        }
        case XCB_MAP_NOTIFY: {
            xcb_map_notify_event_t *e = (void *)ev;
            printf("event MapNotify event=%s window=%s override=%u\n", name(e->event),
                   name(e->window), e->override_redirect);
            break;
        }
        case XCB_UNMAP_NOTIFY: {
            xcb_unmap_notify_event_t *e = (void *)ev;
            printf("event UnmapNotify event=%s window=%s from_configure=%u\n", name(e->event),
                   name(e->window), e->from_configure);
            break;
        }
        case XCB_DESTROY_NOTIFY: {
            xcb_destroy_notify_event_t *e = (void *)ev;
            printf("event DestroyNotify event=%s window=%s\n", name(e->event), name(e->window));
            break;
        }
        case XCB_CONFIGURE_NOTIFY: {
            xcb_configure_notify_event_t *e = (void *)ev;
            printf("event ConfigureNotify event=%s window=%s", name(e->event), name(e->window));
            geometry(e->x, e->y, e->width, e->height, e->border_width);
            printf("\n");
            break;
        }
        case XCB_MAPPING_NOTIFY: /* sent to every client, not SubstructureNotify */
            break;
        case 0:
            printf("event error code=%u\n", ((xcb_generic_error_t *)ev)->error_code);
            break;
        default:
            printf("event type=%u\n", ev->response_type & 0x7f);
        }
        free(ev);
    }
    fflush(stdout);
}

/* Name W's pixmap and report its centre pixel; returns the pixmap or 0. */
static xcb_pixmap_t name_pixmap(const char *what)
{
    xcb_pixmap_t p = xcb_generate_id(c);
    xcb_generic_error_t *e =
        xcb_request_check(c, xcb_composite_name_window_pixmap_checked(c, win, p));
    printf("NameWindowPixmap(W) %s: %s", what, e ? "error" : "ok");
    if (e) {
        printf(" code=%u\n", e->error_code);
        free(e);
        fflush(stdout);
        return 0;
    }
    xcb_get_image_reply_t *img = xcb_get_image_reply(
        c, xcb_get_image(c, XCB_IMAGE_FORMAT_Z_PIXMAP, p, sw / 2, sh / 2, 1, 1, ~0u), NULL);
    if (img && xcb_get_image_data_length(img) >= 4) {
        uint8_t *d = xcb_get_image_data(img);
        printf(" centre=%06x", (unsigned)(d[2] << 16 | d[1] << 8 | d[0]));
    } else {
        printf(" centre=GetImage error");
    }
    printf("\n");
    fflush(stdout);
    free(img);
    return p;
}

static void fill(xcb_drawable_t d, uint32_t colour)
{
    xcb_gcontext_t gc = xcb_generate_id(c);
    xcb_create_gc(c, gc, d, XCB_GC_FOREGROUND, &colour);
    xcb_rectangle_t r = {0, 0, sw, sh};
    xcb_poly_fill_rectangle(c, d, gc, 1, &r);
    xcb_free_gc(c, gc);
}

static void touch(const char *path, const char *body)
{
    FILE *f = fopen(path, "w");
    if (f) {
        fputs(body, f);
        fclose(f);
    }
}

static void pause_ms(int ms)
{
    struct timespec ts = {ms / 1000, (ms % 1000) * 1000000L};
    nanosleep(&ts, NULL);
}

static void sync_both(void)
{
    free(xcb_get_input_focus_reply(c, xcb_get_input_focus(c), NULL));
    free(xcb_get_input_focus_reply(l, xcb_get_input_focus(l), NULL));
}

int main(int argc, char **argv)
{
    if (argc < 3)
        return 2;
    sw = (uint16_t)atoi(argv[1]);
    sh = (uint16_t)atoi(argv[2]);
    c = xcb_connect(NULL, NULL);
    l = xcb_connect(NULL, NULL);
    if (xcb_connection_has_error(c) || xcb_connection_has_error(l))
        return 1;
    xcb_screen_t *s = xcb_setup_roots_iterator(xcb_get_setup(c)).data;
    root = s->root;
    free(xcb_composite_query_version_reply(c, xcb_composite_query_version(c, 0, 4), NULL));
    free(xcb_composite_query_version_reply(l, xcb_composite_query_version(l, 0, 4), NULL));
    uint32_t sub = XCB_EVENT_MASK_SUBSTRUCTURE_NOTIFY;
    check(l, xcb_change_window_attributes_checked(l, root, XCB_CW_EVENT_MASK, &sub),
          "listener selects SubstructureNotify on root");

    check(c, xcb_composite_redirect_subwindows_checked(c, root, XCB_COMPOSITE_REDIRECT_MANUAL),
          "RedirectSubwindows(root, Manual)");
    check(l, xcb_composite_redirect_subwindows_checked(l, root, XCB_COMPOSITE_REDIRECT_MANUAL),
          "other client RedirectSubwindows(root, Manual)");
    xcb_composite_get_overlay_window_reply_t *ow =
        xcb_composite_get_overlay_window_reply(c, xcb_composite_get_overlay_window(c, root), NULL);
    if (!ow)
        return 1;
    cow = ow->overlay_win;
    free(ow);
    printf("GetOverlayWindow: ok\n");
    sync_both();
    query_tree("with the COW");

    win = xcb_generate_id(c);
    uint32_t wv[] = {FIRST, 1};
    check(c,
          xcb_create_window_checked(c, s->root_depth, win, root, 0, 0, sw, sh, 0,
                                    XCB_WINDOW_CLASS_INPUT_OUTPUT, s->root_visual,
                                    XCB_CW_BACK_PIXEL | XCB_CW_OVERRIDE_REDIRECT, wv),
          "create W (override-redirect, fullscreen)");
    check(c, xcb_map_window_checked(c, win), "map W");
    fill(win, FIRST);
    name_pixmap("redirected by root");

    /* Entering fullscreen. */
    check(c, xcb_composite_unredirect_window_checked(c, win, XCB_COMPOSITE_REDIRECT_MANUAL),
          "UnredirectWindow(W, Manual)");
    check(c, xcb_composite_unredirect_window_checked(c, win, XCB_COMPOSITE_REDIRECT_MANUAL),
          "UnredirectWindow(W, Manual) again");
    name_pixmap("unredirected");

    /* Leaving fullscreen. */
    check(c, xcb_composite_redirect_window_checked(c, win, XCB_COMPOSITE_REDIRECT_MANUAL),
          "RedirectWindow(W, Manual)");
    check(l, xcb_composite_redirect_window_checked(l, win, XCB_COMPOSITE_REDIRECT_MANUAL),
          "other client RedirectWindow(W, Manual)");
    check(l, xcb_composite_redirect_window_checked(l, win, XCB_COMPOSITE_REDIRECT_AUTOMATIC),
          "other client RedirectWindow(W, Automatic)");
    check(l, xcb_composite_unredirect_window_checked(l, win, XCB_COMPOSITE_REDIRECT_AUTOMATIC),
          "other client UnredirectWindow(W, Automatic)");
    name_pixmap("redirected again");
    fill(win, SECOND);
    xcb_pixmap_t p = name_pixmap("after W painted");

    /* Composite: the named pixmap into the COW, as the compositor's frame. */
    if (p) {
        xcb_gcontext_t gc = xcb_generate_id(c);
        xcb_create_gc(c, gc, cow, 0, NULL);
        xcb_copy_area(c, p, cow, gc, 0, 0, 0, 0, sw, sh);
        xcb_free_gc(c, gc);
    }
    fill(win, THIRD);
    name_pixmap("after W painted again");
    sync_both();
    char body[16];
    snprintf(body, sizeof body, "%06x\n", SECOND);
    touch("expect-0", body);
    touch("READY-0", "");
    for (int t = 0; t < 600 && access("DONE-0", F_OK) != 0; t++)
        pause_ms(100);

    check(c, xcb_composite_unredirect_subwindows_checked(c, root, XCB_COMPOSITE_REDIRECT_MANUAL),
          "UnredirectSubwindows(root, Manual)");
    name_pixmap("after UnredirectSubwindows(root)");
    check(c, xcb_composite_unredirect_window_checked(c, win, XCB_COMPOSITE_REDIRECT_MANUAL),
          "UnredirectWindow(W, Manual) after UnredirectSubwindows(root)");

    check(l, xcb_unmap_subwindows_checked(l, root), "UnmapSubwindows(root)");
    xcb_get_window_attributes_reply_t *a =
        xcb_get_window_attributes_reply(c, xcb_get_window_attributes(c, cow), NULL);
    printf("COW map_state after UnmapSubwindows(root): %s\n",
           !a ? "error" : a->map_state == XCB_MAP_STATE_VIEWABLE ? "viewable" : "not viewable");
    free(a);

    check(c, xcb_composite_release_overlay_window_checked(c, root), "ReleaseOverlayWindow");
    sync_both();
    query_tree("after release");
    pause_ms(200);
    sync_both();
    log_events();
    printf("done\n");
    fflush(stdout);
    touch("PROBE-DONE", "");
    xcb_disconnect(l);
    xcb_disconnect(c);
    return 0;
}
