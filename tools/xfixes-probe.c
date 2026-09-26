// XFIXES ground-truth probe: version negotiation + request gate, Hide/ShowCursor,
// CursorNotify, ChangeCursor(ByName), ExpandRegion. Run against Xvfb and yserver
// and compare (serials, atoms and XIDs differ between servers).
//   gcc -o /tmp/xfixes-probe tools/xfixes-probe.c -lxcb -lxcb-xfixes
//   DISPLAY=:N /tmp/xfixes-probe
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <xcb/xcb.h>
#include <xcb/xfixes.h>

static uint8_t xf_ev, xf_err, xf_op;

static void err(const char *what, xcb_generic_error_t *e) {
    if (!e) { printf("%-40s OK\n", what); return; }
    int code = e->error_code;
    if (code >= xf_err && code < xf_err + 2)
        printf("%-40s ERR xfixes+%d value=0x%x major=%d minor=%d\n", what, code - xf_err, e->resource_id, e->major_code, e->minor_code);
    else
        printf("%-40s ERR %d value=0x%x major=%d minor=%d\n", what, code, e->resource_id, e->major_code, e->minor_code);
    free(e);
}

static void drain(xcb_connection_t *c, const char *tag) {
    xcb_generic_event_t *ev;

    free(xcb_get_input_focus_reply(c, xcb_get_input_focus(c), NULL));
    while ((ev = xcb_poll_for_event(c))) {
        int t = ev->response_type & 0x7f;
        if (t == 0) { err("  async-error", (xcb_generic_error_t *)ev); continue; }
        if (t == xf_ev + 1) {
            xcb_xfixes_cursor_notify_event_t *n = (void *)ev;
            printf("  [%s] CursorNotify subtype=%d window=0x%x serial=%u name=%u\n", tag, n->subtype, n->window, n->cursor_serial, n->name);
        } else {
            printf("  [%s] event type %d\n", tag, t);
        }
        free(ev);
    }

}

static xcb_connection_t *conn(xcb_screen_t **scr) {
    xcb_connection_t *c = xcb_connect(NULL, NULL);
    if (xcb_connection_has_error(c)) { fprintf(stderr, "connect\n"); exit(1); }
    const xcb_query_extension_reply_t *q = xcb_get_extension_data(c, &xcb_xfixes_id);
    xf_ev = q->first_event; xf_err = q->first_error; xf_op = q->major_opcode;
    *scr = xcb_setup_roots_iterator(xcb_get_setup(c)).data;
    return c;
}

static void qv(xcb_connection_t *c, uint32_t maj, uint32_t min) {
    xcb_xfixes_query_version_reply_t *r = xcb_xfixes_query_version_reply(c, xcb_xfixes_query_version(c, maj, min), NULL);
    printf("QueryVersion %u.%u -> %u.%u\n", maj, min, r->major_version, r->minor_version);
    free(r);
}

static uint32_t cursor_serial(xcb_connection_t *c) {
    xcb_xfixes_get_cursor_image_reply_t *r = xcb_xfixes_get_cursor_image_reply(c, xcb_xfixes_get_cursor_image(c), NULL);
    uint32_t s = r ? r->cursor_serial : 0;
    free(r);
    return s;
}

int main(void) {
    xcb_screen_t *scr;
    xcb_connection_t *c;

    // --- negotiation + gate ---
    c = conn(&scr);
    err("no-QV HideCursor(29)", xcb_request_check(c, xcb_xfixes_hide_cursor_checked(c, scr->root)));
    qv(c, 1, 0);
    err("v1 CreateRegion(5)", xcb_request_check(c, xcb_xfixes_create_region_checked(c, xcb_generate_id(c), 0, NULL)));
    qv(c, 4, 0);
    qv(c, 2, 0);
    err("sticky4 HideCursor(29)", xcb_request_check(c, xcb_xfixes_hide_cursor_checked(c, scr->root)));
    err("sticky4 CreatePointerBarrier(31)", xcb_request_check(c, xcb_xfixes_delete_pointer_barrier_checked(c, 0x1234)));
    qv(c, 5, 1);
    qv(c, 3, 7);
    xcb_disconnect(c);

    // --- hide/show ---
    c = conn(&scr);
    qv(c, 5, 0);
    err("HideCursor(bad window)", xcb_request_check(c, xcb_xfixes_hide_cursor_checked(c, 0x0badbad0)));
    err("ShowCursor(bad window)", xcb_request_check(c, xcb_xfixes_show_cursor_checked(c, 0x0badbad0)));
    err("ShowCursor before hide", xcb_request_check(c, xcb_xfixes_show_cursor_checked(c, scr->root)));
    err("HideCursor 1", xcb_request_check(c, xcb_xfixes_hide_cursor_checked(c, scr->root)));
    err("HideCursor 2", xcb_request_check(c, xcb_xfixes_hide_cursor_checked(c, scr->root)));
    err("ShowCursor 1", xcb_request_check(c, xcb_xfixes_show_cursor_checked(c, scr->root)));
    err("ShowCursor 2", xcb_request_check(c, xcb_xfixes_show_cursor_checked(c, scr->root)));
    err("ShowCursor 3", xcb_request_check(c, xcb_xfixes_show_cursor_checked(c, scr->root)));
    xcb_disconnect(c);

    // --- cursor notify ---
    c = conn(&scr);
    qv(c, 5, 0);
    err("SelectCursorInput bad mask", xcb_request_check(c, xcb_xfixes_select_cursor_input_checked(c, scr->root, 2)));
    err("SelectCursorInput bad window", xcb_request_check(c, xcb_xfixes_select_cursor_input_checked(c, 0x0badbad0, 1)));
    err("SelectCursorInput root", xcb_request_check(c, xcb_xfixes_select_cursor_input_checked(c, scr->root, 1)));
    xcb_font_t f = xcb_generate_id(c);
    xcb_open_font(c, f, 6, "cursor");
    xcb_cursor_t a = xcb_generate_id(c), b = xcb_generate_id(c), d = xcb_generate_id(c);
    xcb_create_glyph_cursor(c, a, f, f, 68, 69, 0, 0, 0, 0xffff, 0xffff, 0xffff);
    xcb_create_glyph_cursor(c, b, f, f, 2, 3, 0, 0, 0, 0xffff, 0xffff, 0xffff);
    xcb_create_glyph_cursor(c, d, f, f, 150, 151, 0, 0, 0, 0xffff, 0xffff, 0xffff);
    drain(c, "after create");
    uint32_t cw = a;
    xcb_change_window_attributes(c, scr->root, XCB_CW_CURSOR, &cw);
    drain(c, "define root=A");
    uint32_t sa = cursor_serial(c);
    printf("serial(A)=%u\n", sa);
    xcb_change_window_attributes(c, scr->root, XCB_CW_CURSOR, &cw);
    drain(c, "define root=A again");
    xcb_xfixes_set_cursor_name(c, b, 5, "bname");
    cw = b;
    xcb_change_window_attributes(c, scr->root, XCB_CW_CURSOR, &cw);
    drain(c, "define root=B (named bname)");
    xcb_intern_atom_reply_t *ar = xcb_intern_atom_reply(c, xcb_intern_atom(c, 1, 5, "bname"), NULL);
    printf("atom(bname)=%u serial(B)=%u\n", ar ? ar->atom : 0, cursor_serial(c));
    free(ar);
    err("HideCursor (notify?)", xcb_request_check(c, xcb_xfixes_hide_cursor_checked(c, scr->root)));
    drain(c, "hidden");
    printf("serial while hidden=%u\n", cursor_serial(c));
    cw = a;
    xcb_change_window_attributes(c, scr->root, XCB_CW_CURSOR, &cw);
    drain(c, "define root=A while hidden");
    err("ShowCursor", xcb_request_check(c, xcb_xfixes_show_cursor_checked(c, scr->root)));
    drain(c, "shown");
    // ChangeCursor: root shows A; replace A with D
    err("ChangeCursor(src=D,dst=A)", xcb_request_check(c, xcb_xfixes_change_cursor_checked(c, d, a)));
    drain(c, "after ChangeCursor");
    printf("serial now=%u (D serial?)\n", cursor_serial(c));
    err("ChangeCursor bad src", xcb_request_check(c, xcb_xfixes_change_cursor_checked(c, 0x0badbad0, a)));
    err("ChangeCursor bad dst", xcb_request_check(c, xcb_xfixes_change_cursor_checked(c, d, 0x0badbad0)));
    // A (resource) now refers to D; define root=A => no change expected
    cw = a;
    xcb_change_window_attributes(c, scr->root, XCB_CW_CURSOR, &cw);
    drain(c, "define root=A (alias of D)");
    // ChangeCursorByName: root=B (named bname), replace name bname with A
    cw = b;
    xcb_change_window_attributes(c, scr->root, XCB_CW_CURSOR, &cw);
    drain(c, "define root=B");
    err("ChangeCursorByName(src=A,'bname')", xcb_request_check(c, xcb_xfixes_change_cursor_by_name_checked(c, a, 5, "bname")));
    drain(c, "after ChangeCursorByName");
    xcb_xfixes_get_cursor_name_reply_t *nr = xcb_xfixes_get_cursor_name_reply(c, xcb_xfixes_get_cursor_name(c, b), NULL);
    printf("GetCursorName(B) atom=%u len=%d\n", nr ? nr->atom : 0, nr ? nr->nbytes : -1);
    free(nr);
    err("ChangeCursorByName unknown name", xcb_request_check(c, xcb_xfixes_change_cursor_by_name_checked(c, a, 9, "zzznoname")));
    // Free the cursor the root displays, then define root=None: root shows default
    cw = 0;
    xcb_change_window_attributes(c, scr->root, XCB_CW_CURSOR, &cw);
    drain(c, "define root=None");
    // Child window under the pointer
    xcb_window_t w = xcb_generate_id(c);
    uint32_t vals[2] = { scr->black_pixel, b };
    xcb_create_window(c, 0, w, scr->root, 0, 0, scr->width_in_pixels, scr->height_in_pixels, 0,
                      XCB_WINDOW_CLASS_INPUT_OUTPUT, 0, XCB_CW_BACK_PIXEL | XCB_CW_CURSOR, vals);
    xcb_map_window(c, w);
    drain(c, "map fullscreen child with cursor B");
    err("SelectCursorInput child", xcb_request_check(c, xcb_xfixes_select_cursor_input_checked(c, w, 1)));
    cw = d;
    xcb_change_window_attributes(c, w, XCB_CW_CURSOR, &cw);
    drain(c, "child cursor=D (2 selections)");
    xcb_destroy_window(c, w);
    drain(c, "destroy child");
    cw = a;
    xcb_change_window_attributes(c, scr->root, XCB_CW_CURSOR, &cw);
    drain(c, "define root=A after child destroy");
    xcb_disconnect(c);

    // --- expand region ---
    c = conn(&scr);
    qv(c, 5, 0);
    xcb_xfixes_region_t src = xcb_generate_id(c), dst = xcb_generate_id(c), empty = xcb_generate_id(c);
    xcb_rectangle_t rs[2] = { {10, 10, 5, 5}, {30, 30, 5, 5} };
    xcb_xfixes_create_region(c, src, 2, rs);
    xcb_rectangle_t rd[1] = { {100, 100, 7, 7} };
    xcb_xfixes_create_region(c, dst, 1, rd);
    xcb_xfixes_create_region(c, empty, 0, NULL);
    err("ExpandRegion(src,dst,1,2,3,4)", xcb_request_check(c, xcb_xfixes_expand_region_checked(c, src, dst, 1, 2, 3, 4)));
    xcb_xfixes_fetch_region_reply_t *fr = xcb_xfixes_fetch_region_reply(c, xcb_xfixes_fetch_region(c, dst), NULL);
    xcb_rectangle_t *rr = xcb_xfixes_fetch_region_rectangles(fr);
    printf("dst extents %d,%d %ux%u n=%d:", fr->extents.x, fr->extents.y, fr->extents.width, fr->extents.height, xcb_xfixes_fetch_region_rectangles_length(fr));
    for (int i = 0; i < xcb_xfixes_fetch_region_rectangles_length(fr); i++) printf(" (%d,%d %ux%u)", rr[i].x, rr[i].y, rr[i].width, rr[i].height);
    printf("\n"); free(fr);
    err("ExpandRegion overlapping(src,dst,20,20,0,0)", xcb_request_check(c, xcb_xfixes_expand_region_checked(c, src, dst, 20, 20, 0, 0)));
    fr = xcb_xfixes_fetch_region_reply(c, xcb_xfixes_fetch_region(c, dst), NULL);
    rr = xcb_xfixes_fetch_region_rectangles(fr);
    printf("dst extents %d,%d %ux%u n=%d:", fr->extents.x, fr->extents.y, fr->extents.width, fr->extents.height, xcb_xfixes_fetch_region_rectangles_length(fr));
    for (int i = 0; i < xcb_xfixes_fetch_region_rectangles_length(fr); i++) printf(" (%d,%d %ux%u)", rr[i].x, rr[i].y, rr[i].width, rr[i].height);
    printf("\n"); free(fr);
    err("ExpandRegion(empty,dst)", xcb_request_check(c, xcb_xfixes_expand_region_checked(c, empty, dst, 1, 1, 1, 1)));
    fr = xcb_xfixes_fetch_region_reply(c, xcb_xfixes_fetch_region(c, dst), NULL);
    printf("dst after empty-source expand n=%d extents %d,%d %ux%u\n", xcb_xfixes_fetch_region_rectangles_length(fr), fr->extents.x, fr->extents.y, fr->extents.width, fr->extents.height);
    free(fr);
    err("ExpandRegion(bad src)", xcb_request_check(c, xcb_xfixes_expand_region_checked(c, 0x0badbad0, dst, 1, 1, 1, 1)));
    err("ExpandRegion(bad dst)", xcb_request_check(c, xcb_xfixes_expand_region_checked(c, src, 0x0badbad0, 1, 1, 1, 1)));
    xcb_disconnect(c);
    return 0;
}
