// SYNC ground-truth probe: Await / AwaitFence suspension, CounterNotify, errors,
// SERVERTIME waits. Client A drives counters and fences, client B awaits and then
// sends GetInputFocus; "blocked" means B's reply had not arrived after A's round
// trip. Run against Xvfb and yserver and compare (XIDs and times differ).
//   gcc -o /tmp/sync-await-probe tools/sync-await-probe.c -lxcb -lxcb-sync
//   DISPLAY=:N /tmp/sync-await-probe
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <time.h>
#include <xcb/xcb.h>
#include <xcb/sync.h>
#include <xcb/xcbext.h>

static uint8_t s_ev, s_err;
static uint32_t servertime;

static xcb_connection_t *conn(xcb_screen_t **scr) {
    xcb_connection_t *c = xcb_connect(NULL, NULL);
    if (xcb_connection_has_error(c)) { fprintf(stderr, "connect\n"); exit(1); }
    const xcb_query_extension_reply_t *q = xcb_get_extension_data(c, &xcb_sync_id);
    s_ev = q->first_event; s_err = q->first_error;
    free(xcb_sync_initialize_reply(c, xcb_sync_initialize(c, 3, 1), NULL));
    if (scr) *scr = xcb_setup_roots_iterator(xcb_get_setup(c)).data;
    return c;
}

static void sync_rt(xcb_connection_t *c) { free(xcb_get_input_focus_reply(c, xcb_get_input_focus(c), NULL)); }

static void show_err(const char *what, xcb_generic_error_t *e) {
    if (!e) { printf("%-44s OK\n", what); return; }
    if (e->error_code >= s_err && e->error_code < s_err + 3)
        printf("%-44s ERR sync+%d value=0x%x minor=%d\n", what, e->error_code - s_err, e->resource_id, e->minor_code);
    else
        printf("%-44s ERR %d value=0x%x minor=%d\n", what, e->error_code, e->resource_id, e->minor_code);
    free(e);
}

// Print everything B has received so far (events, errors) and whether its pending reply arrived.
static void peek(xcb_connection_t *b, const char *tag, unsigned int *pending, int *have) {
    xcb_generic_event_t *ev;
    if (pending && !*have) {
        void *r = NULL; xcb_generic_error_t *e = NULL;
        if (xcb_poll_for_reply(b, *pending, &r, &e)) {
            if (r) { *have = 1; printf("  [%s] B: pending reply arrived\n", tag); free(r); }
            if (e) { show_err("  B pending error", e); *have = 1; }
        } else printf("  [%s] B: pending reply NOT arrived (blocked)\n", tag);
    }
    while ((ev = xcb_poll_for_event(b))) {
        int t = ev->response_type & 0x7f;
        if (t == 0) { show_err("  B async error", (xcb_generic_error_t *)ev); continue; }
        if (t == s_ev) {
            xcb_sync_counter_notify_event_t *n = (void *)ev;
            long long wv = ((long long)n->wait_value.hi << 32) | n->wait_value.lo;
            long long cv = ((long long)n->counter_value.hi << 32) | n->counter_value.lo;
            printf("  [%s] B: CounterNotify kind=%d counter=0x%x wait=%lld value=%lld count=%u destroyed=%u\n",
                   tag, n->kind, n->counter, wv, cv, n->count, n->destroyed);
        } else printf("  [%s] B: event type %d\n", tag, t);
        free(ev);
    }
}

static xcb_sync_int64_t i64(long long v) { xcb_sync_int64_t r = { (int32_t)(v >> 32), (uint32_t)v }; return r; }

static xcb_sync_waitcondition_t wc(xcb_sync_counter_t c, int vt, long long wv, int tt, long long th) {
    xcb_sync_waitcondition_t w;
    w.trigger.counter = c; w.trigger.wait_type = vt; w.trigger.wait_value = i64(wv); w.trigger.test_type = tt;
    w.event_threshold = i64(th);
    return w;
}

// After B sends await (+ a GetInputFocus), A does something; report B's state.
static void scenario(const char *name, xcb_connection_t *a, xcb_connection_t *unused,
                     int nconds, xcb_sync_waitcondition_t *conds,
                     void (*act)(xcb_connection_t *, void *), void *arg, int nacts) {
    (void)unused;
    xcb_connection_t *b = conn(NULL);
    printf("== %s\n", name);
    xcb_sync_await(b, nconds, conds);
    unsigned int cookie = xcb_get_input_focus(b).sequence;
    xcb_flush(b);
    sync_rt(a); usleep(50000); sync_rt(a);
    int have = 0;
    peek(b, "after await", &cookie, &have);
    for (int i = 0; i < nacts && !have; i++) {
        act(a, arg);
        sync_rt(a); usleep(50000);
        char tag[32]; snprintf(tag, sizeof tag, "after act %d", i + 1);
        peek(b, tag, &cookie, &have);
    }
    if (!have) { printf("  (still blocked; giving up)\n"); }
    xcb_disconnect(b);
    sync_rt(a);
}

struct setarg { xcb_sync_counter_t c; long long v[4]; int i; int change; };
static void do_set(xcb_connection_t *a, void *p) {
    struct setarg *s = p;
    if (s->change) xcb_sync_change_counter(a, s->c, i64(s->v[s->i]));
    else xcb_sync_set_counter(a, s->c, i64(s->v[s->i]));
    printf("  A: %s counter %lld\n", s->change ? "change" : "set", s->v[s->i]);
    s->i++;
}
static void do_destroy(xcb_connection_t *a, void *p) { xcb_sync_destroy_counter(a, *(xcb_sync_counter_t *)p); printf("  A: destroy counter\n"); }
static void do_trigger(xcb_connection_t *a, void *p) { xcb_sync_trigger_fence(a, *(xcb_sync_fence_t *)p); printf("  A: trigger fence\n"); }
static void do_destroy_fence(xcb_connection_t *a, void *p) { xcb_sync_destroy_fence(a, *(xcb_sync_fence_t *)p); printf("  A: destroy fence\n"); }
static void do_nothing(xcb_connection_t *a, void *p) { (void)a; (void)p; usleep(150000); printf("  A: waited 150ms\n"); }

int main(void) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    xcb_screen_t *scr;
    xcb_connection_t *a = conn(&scr), *b = conn(NULL);
    {
        xcb_sync_list_system_counters_reply_t *lr = xcb_sync_list_system_counters_reply(a, xcb_sync_list_system_counters(a), NULL);
        xcb_sync_systemcounter_iterator_t it = xcb_sync_list_system_counters_counters_iterator(lr);
        for (; it.rem; xcb_sync_systemcounter_next(&it)) {
            printf("system counter 0x%x '%.*s'\n", it.data->counter, xcb_sync_systemcounter_name_length(it.data), xcb_sync_systemcounter_name(it.data));
            if (strstr(xcb_sync_systemcounter_name(it.data) - 2, "SERVERTIME")) servertime = it.data->counter;
        }
        free(lr);
    }
    xcb_sync_counter_t c1 = xcb_generate_id(a), c2 = xcb_generate_id(a);
    xcb_sync_create_counter(a, c1, i64(0));
    xcb_sync_create_counter(a, c2, i64(100));
    sync_rt(a);

    xcb_sync_waitcondition_t w[2];
    struct setarg s = { c1, {3, 7}, 0, 0 };
    w[0] = wc(c1, XCB_SYNC_VALUETYPE_ABSOLUTE, 5, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 0);
    scenario("PosCmp c1>=5 thr0; set 3 then 7", a, b, 1, w, do_set, &s, 2);

    w[0] = wc(c1, XCB_SYNC_VALUETYPE_ABSOLUTE, 5, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 0);
    scenario("already satisfied (c1=7 >= 5)", a, b, 1, w, do_nothing, NULL, 1);

    struct setarg s2 = { c1, {9, 12}, 0, 0 };
    w[0] = wc(c1, XCB_SYNC_VALUETYPE_ABSOLUTE, 10, XCB_SYNC_TESTTYPE_POSITIVE_TRANSITION, 2);
    scenario("PosTrans c1 10 thr2; set 9, 12 (diff 2)", a, b, 1, w, do_set, &s2, 2);

    struct setarg s3 = { c1, {1, 20}, 0, 0 };
    w[0] = wc(c1, XCB_SYNC_VALUETYPE_ABSOLUTE, 13, XCB_SYNC_TESTTYPE_POSITIVE_TRANSITION, 2);
    scenario("PosTrans c1 13 thr2; set 1, 20 (diff 7)", a, b, 1, w, do_set, &s3, 2);

    struct setarg s4 = { c1, {1, 2}, 0, 0 };
    w[0] = wc(c1, XCB_SYNC_VALUETYPE_ABSOLUTE, 10, XCB_SYNC_TESTTYPE_POSITIVE_TRANSITION, 100);
    scenario("PosTrans c1 10 thr100 (already above, not a transition)", a, b, 1, w, do_set, &s4, 1);

    struct setarg s5 = { c1, {5, 11}, 0, 0 };
    w[0] = wc(c1, XCB_SYNC_VALUETYPE_ABSOLUTE, 10, XCB_SYNC_TESTTYPE_POSITIVE_TRANSITION, 0);
    scenario("PosTrans c1 10 (c1=2) set 5, 11", a, b, 1, w, do_set, &s5, 2);

    struct setarg s6 = { c2, {50, 40}, 0, 0 };
    w[0] = wc(c1, XCB_SYNC_VALUETYPE_ABSOLUTE, 1000, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, -2000);
    w[1] = wc(c2, XCB_SYNC_VALUETYPE_ABSOLUTE, 45, XCB_SYNC_TESTTYPE_NEGATIVE_COMPARISON, 0);
    scenario("two conds: c1>=1000 thr-2000, c2<=45 thr0; set c2 50, 40", a, b, 2, w, do_set, &s6, 2);

    struct setarg s7 = { c1, {2, 1}, 0, 1 };
    w[0] = wc(c1, XCB_SYNC_VALUETYPE_RELATIVE, 3, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 0);
    scenario("relative +3 on c1=11; change +2, +1", a, b, 1, w, do_set, &s7, 2);

    struct setarg s8 = { c1, {-5, -10}, 0, 0 };
    w[0] = wc(c1, XCB_SYNC_VALUETYPE_ABSOLUTE, -3, XCB_SYNC_TESTTYPE_NEGATIVE_TRANSITION, 0);
    scenario("NegTrans c1 -3; set -5 then -10", a, b, 1, w, do_set, &s8, 2);

    struct setarg s9 = { c1, {1}, 0, 0 };
    w[0] = wc(c1, XCB_SYNC_VALUETYPE_ABSOLUTE, 0, XCB_SYNC_TESTTYPE_POSITIVE_TRANSITION, 5);
    scenario("PosTrans c1 0 thr5 (c1=-5); set 1 (diff 1 < 5)", a, b, 1, w, do_set, &s9, 1);
    struct setarg s10 = { c1, {-5}, 0, 0 };
    do_set(a, &s10); sync_rt(a);
    w[0] = wc(c1, XCB_SYNC_VALUETYPE_ABSOLUTE, 1000, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 0);
    w[1] = wc(c2, XCB_SYNC_VALUETYPE_ABSOLUTE, 1000, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, -2000);
    scenario("destroy c1 while awaiting (c2 cond thr -2000)", a, b, 2, w, do_destroy, &c1, 1);

    // errors
    printf("== errors\n");
    show_err("Await 0 conditions", xcb_request_check(b, xcb_sync_await_checked(b, 0, w)));
    w[0] = wc(0, XCB_SYNC_VALUETYPE_ABSOLUTE, 1, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 0);
    show_err("Await counter None", xcb_request_check(b, xcb_sync_await_checked(b, 1, w)));
    w[0] = wc(0x0badbad0, XCB_SYNC_VALUETYPE_ABSOLUTE, 1, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 0);
    show_err("Await unknown counter", xcb_request_check(b, xcb_sync_await_checked(b, 1, w)));
    w[0] = wc(c2, 7, 1, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 0);
    show_err("Await bad value_type", xcb_request_check(b, xcb_sync_await_checked(b, 1, w)));
    w[0] = wc(c2, XCB_SYNC_VALUETYPE_ABSOLUTE, 1, 9, 0);
    show_err("Await bad test_type", xcb_request_check(b, xcb_sync_await_checked(b, 1, w)));
    w[0] = wc(c2, XCB_SYNC_VALUETYPE_RELATIVE, 0x7fffffffffffffffLL, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 0);
    show_err("Await relative overflow", xcb_request_check(b, xcb_sync_await_checked(b, 1, w)));
    show_err("SetCounter SERVERTIME", xcb_request_check(a, xcb_sync_set_counter_checked(a, servertime, i64(1))));
    show_err("SetCounter unknown", xcb_request_check(a, xcb_sync_set_counter_checked(a, 0x0badbad0, i64(1))));
    show_err("ChangeCounter overflow", xcb_request_check(a, xcb_sync_change_counter_checked(a, c2, i64(0x7fffffffffffffffLL))));
    show_err("DestroyCounter SERVERTIME", xcb_request_check(a, xcb_sync_destroy_counter_checked(a, servertime)));
    show_err("DestroyCounter unknown", xcb_request_check(a, xcb_sync_destroy_counter_checked(a, 0x0badbad0)));
    uint32_t nf = 0;
    show_err("AwaitFence 0", xcb_request_check(b, xcb_sync_await_fence_checked(b, 0, &nf)));
    show_err("AwaitFence None", xcb_request_check(b, xcb_sync_await_fence_checked(b, 1, &nf)));
    nf = 0x0badbad0;
    show_err("AwaitFence unknown", xcb_request_check(b, xcb_sync_await_fence_checked(b, 1, &nf)));

    // fences
    xcb_sync_fence_t f1 = xcb_generate_id(a), f2 = xcb_generate_id(a);
    xcb_sync_create_fence(a, scr->root, f1, 0);
    xcb_sync_create_fence(a, scr->root, f2, 1);
    sync_rt(a);
    printf("== AwaitFence f1 (untriggered); A triggers\n");
    xcb_sync_await_fence(b, 1, &f1);
    unsigned int cookie = xcb_get_input_focus(b).sequence; xcb_flush(b);
    sync_rt(a); usleep(50000); int have = 0;
    peek(b, "after await", &cookie, &have);
    do_trigger(a, &f1); sync_rt(a); usleep(50000);
    peek(b, "after trigger", &cookie, &have);
    printf("== AwaitFence f2 (already triggered)\n");
    xcb_sync_await_fence(b, 1, &f2);
    cookie = xcb_get_input_focus(b).sequence; xcb_flush(b);
    sync_rt(a); usleep(50000); have = 0;
    peek(b, "after await", &cookie, &have);
    xcb_sync_reset_fence(a, f1); sync_rt(a);
    printf("== AwaitFence f1 reset; A destroys it\n");
    xcb_sync_await_fence(b, 1, &f1);
    cookie = xcb_get_input_focus(b).sequence; xcb_flush(b);
    sync_rt(a); usleep(50000); have = 0;
    peek(b, "after await", &cookie, &have);
    do_destroy_fence(a, &f1); sync_rt(a); usleep(50000);
    peek(b, "after destroy", &cookie, &have);
    show_err("ResetFence untriggered", xcb_request_check(a, xcb_sync_reset_fence_checked(a, f2)));
    xcb_sync_reset_fence(a, f2); sync_rt(a);
    show_err("ResetFence already reset", xcb_request_check(a, xcb_sync_reset_fence_checked(a, f2)));

    // SERVERTIME
    xcb_sync_query_counter_reply_t *qr = xcb_sync_query_counter_reply(b, xcb_sync_query_counter(b, servertime), NULL);
    long long now = ((long long)qr->counter_value.hi << 32) | qr->counter_value.lo;
    free(qr);
    printf("== SERVERTIME await now+300 PosCmp\n");
    struct timespec t0, t1; clock_gettime(CLOCK_MONOTONIC, &t0);
    w[0] = wc(servertime, XCB_SYNC_VALUETYPE_ABSOLUTE, now + 300, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 0);
    xcb_sync_await(b, 1, w);
    free(xcb_get_input_focus_reply(b, xcb_get_input_focus(b), NULL));
    clock_gettime(CLOCK_MONOTONIC, &t1);
    long ms = (t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000;
    printf("  blocked ~%ld ms (>=250: %s)\n", ms, ms >= 250 ? "yes" : "no");
    peek(b, "servertime", NULL, NULL);
    // Relative SERVERTIME
    clock_gettime(CLOCK_MONOTONIC, &t0);
    w[0] = wc(servertime, XCB_SYNC_VALUETYPE_RELATIVE, 200, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 1000000);
    xcb_sync_await(b, 1, w);
    free(xcb_get_input_focus_reply(b, xcb_get_input_focus(b), NULL));
    clock_gettime(CLOCK_MONOTONIC, &t1);
    ms = (t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000;
    printf("  relative +200 PosCmp thr1e6 blocked ~%ld ms (>=150: %s)\n", ms, ms >= 150 ? "yes" : "no");
    peek(b, "servertime rel", NULL, NULL);

    // Awaiting client's counter owned by another client which disconnects
    xcb_connection_t *d = conn(NULL);
    xcb_sync_counter_t c3 = xcb_generate_id(d);
    xcb_sync_create_counter(d, c3, i64(0));
    sync_rt(d);
    printf("== counter owner disconnects while B awaits\n");
    w[0] = wc(c3, XCB_SYNC_VALUETYPE_ABSOLUTE, 10, XCB_SYNC_TESTTYPE_POSITIVE_COMPARISON, 0);
    xcb_sync_await(b, 1, w);
    cookie = xcb_get_input_focus(b).sequence; xcb_flush(b);
    sync_rt(a); usleep(50000); have = 0;
    peek(b, "after await", &cookie, &have);
    xcb_disconnect(d); usleep(100000); sync_rt(a);
    peek(b, "after owner disconnect", &cookie, &have);

    xcb_disconnect(b);
    xcb_disconnect(a);
    return 0;
}
