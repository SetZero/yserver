// RECORD ground-truth probe (issue #180): requests, errors, GetContext ranges
// and the EnableContext stream. The recorder and the recorded extra clients
// are raw sockets (their byte order is selectable and their setup bytes are
// known); the control client is xcb. Output is decoded with XIDs named after
// their connection and times dropped, so Xvfb and yserver runs diff directly.
//   gcc -o /tmp/record-probe tools/record-probe.c -lxcb -lxcb-record -lxcb-xtest
//   DISPLAY=:N /tmp/record-probe basic|ranges|errors|free|recdie|ownerdie|repeat [l|B] [ehdr]
//   DISPLAY=:N /tmp/record-probe listen l SECS   (physical keys; see vng-scenarios/record.sh)
// RECORD_PROBE_HEX=1 also dumps every packet the raw recorder reads.
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <xcb/record.h>
#include <xcb/xcb.h>
#include <xcb/xtest.h>

typedef struct {
    const char *name;
    int fd, big;
    unsigned base, mask;
    unsigned char *setup; /* the setup reply bytes this client received */
    int setup_len;
    unsigned seq;
} Raw;

static int major, first_error;
static unsigned root;
static Raw *named[8];
static int nnamed;
static unsigned ctl_base, ctl_mask;

static void put16(Raw *r, unsigned char *p, unsigned v) {
    if (r->big) { p[0] = v >> 8; p[1] = v; } else { p[0] = v; p[1] = v >> 8; }
}
static void put32(Raw *r, unsigned char *p, unsigned v) {
    if (r->big) { p[0] = v >> 24; p[1] = v >> 16; p[2] = v >> 8; p[3] = v; }
    else { p[0] = v; p[1] = v >> 8; p[2] = v >> 16; p[3] = v >> 24; }
}
static unsigned get16o(int big, const unsigned char *p) { return big ? (p[0] << 8 | p[1]) : (p[1] << 8 | p[0]); }
static unsigned get32o(int big, const unsigned char *p) {
    return big ? ((unsigned)p[0] << 24 | p[1] << 16 | p[2] << 8 | p[3])
               : ((unsigned)p[3] << 24 | p[2] << 16 | p[1] << 8 | p[0]);
}

static const char *nm(unsigned v) {
    static char buf[8][48];
    static int k;
    char *b = buf[k++ & 7];
    if (v == 0) return "0";
    if (v <= 3) { sprintf(b, "spec%u", v); return b; }
    if (v == root) return "root";
    if ((v & ~ctl_mask) == ctl_base) { v & ctl_mask ? sprintf(b, "ctl+%x", v & ctl_mask) : sprintf(b, "ctl"); return b; }
    for (int i = 0; i < nnamed; i++) {
        Raw *r = named[i];
        if ((v & ~r->mask) == r->base) {
            v & r->mask ? sprintf(b, "%s+%x", r->name, v & r->mask) : sprintf(b, "%s", r->name);
            return b;
        }
    }
    sprintf(b, "0x%x", v);
    return b;
}

static void rd(Raw *r, void *b, int n) {
    int o = 0;
    while (o < n) {
        int k = read(r->fd, (char *)b + o, n - o);
        if (k <= 0) { printf("%s: read failed\n", r->name); exit(1); }
        o += k;
    }
}

static Raw *raw_connect(const char *name, int big) {
    const char *d = getenv("DISPLAY");
    struct sockaddr_un a = {AF_UNIX};
    snprintf(a.sun_path, sizeof a.sun_path, "/tmp/.X11-unix/X%d", atoi(strchr(d, ':') + 1));
    Raw *r = calloc(1, sizeof *r);
    r->name = name;
    r->big = big;
    r->fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (connect(r->fd, (void *)&a, sizeof a)) { perror("connect"); exit(1); }
    unsigned char s[12] = {big ? 'B' : 'l'};
    put16(r, s + 2, 11);
    if (write(r->fd, s, 12) != 12) exit(1);
    unsigned char h[8];
    rd(r, h, 8);
    int len = 8 + 4 * get16o(big, h + 6);
    r->setup = malloc(len);
    memcpy(r->setup, h, 8);
    rd(r, r->setup + 8, len - 8);
    r->setup_len = len;
    r->base = get32o(big, r->setup + 12);
    r->mask = get32o(big, r->setup + 16);
    int vlen = get16o(big, r->setup + 24), nfmt = r->setup[29];
    root = get32o(big, r->setup + 8 + 32 + ((vlen + 3) & ~3) + 8 * nfmt);
    named[nnamed++] = r;
    return r;
}

static void send_req(Raw *r, unsigned char *req, int len) {
    put16(r, req + 2, len / 4);
    if (write(r->fd, req, len) != len) { printf("%s: write failed\n", r->name); exit(1); }
    r->seq++;
}

/* A request with a 32-bit first field (Get/Enable/Disable/FreeContext). */
static void req_ctx(Raw *r, int minor, unsigned ctx) {
    unsigned char q[8] = {major, minor};
    put32(r, q + 4, ctx);
    send_req(r, q, 8);
}

static unsigned char *next_pkt(Raw *r, int timeout, int *len) {
    for (;;) {
        struct pollfd p = {r->fd, POLLIN, 0};
        if (poll(&p, 1, timeout) <= 0) return NULL;
        unsigned char h[32];
        rd(r, h, 32);
        int extra = h[0] == 1 ? 4 * get32o(r->big, h + 4) : 0;
        unsigned char *b = malloc(32 + extra);
        memcpy(b, h, 32);
        rd(r, b + 32, extra);
        *len = 32 + extra;
        if (getenv("RECORD_PROBE_HEX")) {
            printf("%s hex:", r->name);
            for (int i = 0; i < *len && i < 256; i++) printf(i % 4 ? "%02x" : " %02x", b[i]);
            printf("\n");
        }
        if (h[0] == 0 || h[0] == 1) return b;
        if (h[0] != 34) printf("%s: event type=%d\n", r->name, h[0]); /* MappingNotify: Xorg XTEST slave switch */
        free(b);
    }
}

static const char *errname(int code) {
    static char b[32];
    static const char *core[] = {"?", "BadRequest", "BadValue", "BadWindow", "BadPixmap", "BadAtom",
                                 "BadCursor", "BadFont", "BadMatch", "BadDrawable", "BadAccess",
                                 "BadAlloc", "BadColor", "BadGC", "BadIDChoice", "BadName", "BadLength",
                                 "BadImplementation"};
    if (code == first_error) return "BadContext";
    if (code < 18) return core[code];
    sprintf(b, "error%d", code);
    return b;
}

static void print_error(const char *tag, int big, const unsigned char *e) {
    int code = e[1];
    /* Xorg leaves a stale errorValue for errors that do not set one. */
    int valued = code == 2 || code == 14 || code == first_error;
    printf("%s: %s value=%s minor=%u major=%s seq=%u\n", tag, errname(code),
           valued ? nm(get32o(big, e + 4)) : "-", get16o(big, e + 8),
           e[10] == major ? "RECORD" : "other", get16o(big, e + 2));
}

static void print_ranges(int big, const unsigned char *p, int n) {
    for (int i = 0; i < n; i++, p += 24) {
        printf("    range req=%u-%u rep=%u-%u extreq=%u-%u/%u-%u extrep=%u-%u/%u-%u dlv=%u-%u dev=%u-%u "
               "err=%u-%u started=%u died=%u\n",
               p[0], p[1], p[2], p[3], p[4], p[5], get16o(big, p + 6), get16o(big, p + 8), p[10], p[11],
               get16o(big, p + 12), get16o(big, p + 14), p[16], p[17], p[18], p[19], p[20], p[21], p[22],
               p[23]);
    }
}

static void print_get_context(const char *tag, int big, const unsigned char *b) {
    unsigned n = get32o(big, b + 12);
    printf("%s: GetContext enabled=%u ehdr=%u nclients=%u len=%u seq=%u\n", tag, b[1], b[8], n,
           get32o(big, b + 4), get16o(big, b + 2));
    const unsigned char *p = b + 32;
    for (unsigned i = 0; i < n; i++) {
        unsigned nr = get32o(big, p + 4);
        printf("  client=%s nranges=%u\n", nm(get32o(big, p)), nr);
        print_ranges(big, p + 8, nr);
        p += 8 + 24 * nr;
    }
}

/* Print one reply or error the recorder read; returns its category (or
 * 100 + error code). */
static const char *cats[] = {"FromServer", "FromClient", "ClientStarted", "ClientDied", "StartOfData",
                             "EndOfData"};
static int print_pkt(Raw *r, const char *tag, unsigned char *b, int len) {
    int big = r->big;
    if (b[0] == 0) { print_error(tag, big, b); return 100 + b[1]; }
    if (b[1] > 5) { printf("%s: reply data=%u len=%d seq=%u\n", tag, b[1], len, get16o(big, b + 2)); return -2; }
    int cat = b[1], ehdr = b[8], swapped = b[9];
    char head[256];
    snprintf(head, sizeof head, "%s: %s seq=%u ehdr=%u swapped=%u idbase=%s recseq=%u", tag, cats[cat],
             get16o(big, b + 2), ehdr, swapped, nm(get32o(big, b + 12)), get32o(big, b + 20));
    const unsigned char *p = b + 32, *end = b + len;
    if (cat == 0) {
        while (p < end) {
            int t = 0;
            if (ehdr & 1) { t = 1; p += 4; }
            printf("%s%s ev type=%u detail=%u seqfield=%u root=%s event=%s child=%s rx=%d ry=%d ex=%d ey=%d "
                   "state=0x%x same=%u\n",
                   head, t ? " +time" : "", p[0], p[1], get16o(big, p + 2), nm(get32o(big, p + 8)),
                   nm(get32o(big, p + 12)), nm(get32o(big, p + 16)), (short)get16o(big, p + 20),
                   (short)get16o(big, p + 22), (short)get16o(big, p + 24), (short)get16o(big, p + 26),
                   get16o(big, p + 28), p[30]);
            p += 32;
        }
    } else if (cat == 2) {
        int cbig = big ^ swapped;
        unsigned cbase = get32o(cbig, p + 12);
        int match = 0;
        for (int i = 0; i < nnamed; i++)
            if (named[i]->base == cbase)
                match = named[i]->setup_len == end - p && !memcmp(named[i]->setup, p, end - p);
        printf("%s setup success=%u major=%u len=%d base=%s same-bytes-as-client=%s\n", head, p[0],
               get16o(cbig, p + 2), (int)(end - p), nm(cbase), match ? "yes" : "no");
    } else {
        printf("%s len=%u", head, get32o(big, b + 4));
        if (cat == 3 && (ehdr & 4)) printf(" seqheader=%u", get32o(big, p));
        printf("\n");
    }
    return cat;
}

/* Read the recorder's stream until EndOfData / an error / a quiet period. */
static void drain(Raw *r, int until_end) {
    int len;
    unsigned char *b;
    while ((b = next_pkt(r, until_end ? 2000 : 300, &len))) {
        int c = print_pkt(r, r->name, b, len);
        free(b);
        if (until_end && c == 5) return;
    }
    if (until_end) printf("%s: no EndOfData\n", r->name);
}

static void expect(Raw *r, const char *tag) {
    int len;
    unsigned char *b = next_pkt(r, 500, &len);
    if (!b) { printf("%s: nothing\n", tag); return; }
    print_pkt(r, tag, b, len);
    free(b);
}

/* GetContext from the raw recorder, printed. */
static void raw_get(Raw *r, unsigned ctx, const char *tag) {
    req_ctx(r, 4, ctx);
    int len;
    unsigned char *b = next_pkt(r, 500, &len);
    if (!b) { printf("%s: nothing\n", tag); return; }
    if (b[0] == 0) print_error(tag, r->big, b);
    else print_get_context(tag, r->big, b);
    free(b);
}

/* Any reply/error the raw client got within a short wait. */
static void raw_result(Raw *r, const char *tag) {
    int len;
    unsigned char *b = next_pkt(r, 300, &len);
    if (!b) { printf("%s: ok (no error)\n", tag); return; }
    if (b[0] == 0) print_error(tag, r->big, b);
    else printf("%s: reply len=%d seq=%u\n", tag, len, get16o(r->big, b + 2));
    free(b);
}

static void qversion(Raw *r) {
    unsigned char qv[8] = {major, 0};
    put16(r, qv + 4, 1);
    put16(r, qv + 6, 13);
    send_req(r, qv, 8);
    int len;
    unsigned char *b = next_pkt(r, 500, &len);
    printf("QueryVersion: %u.%u seq=%u\n", get16o(r->big, b + 8), get16o(r->big, b + 10), get16o(r->big, b + 2));
    free(b);
}

/* Create/RegisterClients with `n` specs and `nr` ranges. */
static void reg(Raw *r, int minor, unsigned ctx, int ehdr, const unsigned *specs, int n,
                const unsigned char (*ranges)[24], int nr) {
    int len = 20 + 4 * n + 24 * nr;
    unsigned char *q = calloc(1, len);
    q[0] = major;
    q[1] = minor;
    put32(r, q + 4, ctx);
    q[8] = ehdr;
    put32(r, q + 12, n);
    put32(r, q + 16, nr);
    for (int i = 0; i < n; i++) put32(r, q + 20 + 4 * i, specs[i]);
    for (int i = 0; i < nr; i++) {
        unsigned char *p = q + 20 + 4 * n + 24 * i;
        memcpy(p, ranges[i], 24);
        /* the four minor-opcode fields are CARD16 in the client's order */
        for (int o = 6; o <= 14; o += 2) {
            if (o == 10) continue;
            unsigned v = ranges[i][o] | ranges[i][o + 1] << 8;
            put16(r, p + o, v);
        }
    }
    send_req(r, q, len);
    free(q);
}

static unsigned char devrange[24];

static void setup_ext(Raw *r) {
    unsigned char q[16] = {98, 0, 0, 0};
    put16(r, q + 4, 6);
    memcpy(q + 8, "RECORD", 6);
    send_req(r, q, 16);
    unsigned char rep[32];
    rd(r, rep, 32);
    major = rep[9];
    first_error = rep[11];
    printf("QueryExtension RECORD present=%u events=%u\n", rep[8], rep[10]);
    devrange[18] = 2;
    devrange[19] = 6;
    devrange[22] = 1;
    devrange[23] = 1;
}

static xcb_connection_t *ctl_connect(void) {
    xcb_connection_t *c = xcb_connect(NULL, NULL);
    if (xcb_connection_has_error(c)) { printf("ctl connect failed\n"); exit(1); }
    ctl_base = xcb_get_setup(c)->resource_id_base;
    ctl_mask = xcb_get_setup(c)->resource_id_mask;
    return c;
}

static void ctl_sync(xcb_connection_t *c) { free(xcb_get_input_focus_reply(c, xcb_get_input_focus(c), NULL)); }

static void ctl_err(xcb_connection_t *c, xcb_void_cookie_t ck, const char *tag) {
    xcb_generic_error_t *e = xcb_request_check(c, ck);
    if (!e) { printf("%s: ok (no error)\n", tag); return; }
    print_error(tag, 0, (unsigned char *)e);
    free(e);
}

static void ctl_get(xcb_connection_t *c, unsigned ctx, const char *tag) {
    xcb_generic_error_t *e = NULL;
    xcb_record_get_context_reply_t *g = xcb_record_get_context_reply(c, xcb_record_get_context(c, ctx), &e);
    if (e) { print_error(tag, 0, (unsigned char *)e); free(e); return; }
    print_get_context(tag, 0, (unsigned char *)g);
    free(g);
}

static void fake_input(xcb_connection_t *c) {
    xcb_test_fake_input(c, 6, 0, 0, root, 100, 50, 0);
    xcb_test_fake_input(c, 2, 38, 0, 0, 0, 0, 0);
    xcb_test_fake_input(c, 3, 38, 0, 0, 0, 0, 0);
    xcb_test_fake_input(c, 4, 1, 0, 0, 0, 0, 0);
    xcb_test_fake_input(c, 5, 1, 0, 0, 0, 0, 0);
    ctl_sync(c);
}

static int scenario_basic(int big, int ehdr) {
    Raw *rec = raw_connect("rec", big);
    setup_ext(rec);
    qversion(rec);
    unsigned ctx = rec->base + 1, all = 3;
    reg(rec, 1, ctx, ehdr, &all, 1, &devrange, 1);
    raw_get(rec, ctx, "GetContext(created)");
    xcb_connection_t *ctl = ctl_connect();
    ctl_sync(ctl);
    raw_get(rec, ctx, "GetContext(ctl joined)");
    req_ctx(rec, 5, ctx);
    expect(rec, "Enable");
    ctl_get(ctl, ctx, "ctl GetContext(enabled)");
    {
        xcb_generic_error_t *e = NULL;
        free(xcb_record_enable_context_reply(ctl, xcb_record_enable_context(ctl, ctx), &e));
        if (e) { print_error("ctl Enable(already enabled)", 0, (unsigned char *)e); free(e); }
        else printf("ctl Enable(already enabled): no error\n");
    }
    xcb_record_client_spec_t spec = rec->base;
    xcb_record_range_t none = {0};
    ctl_err(ctl, xcb_record_register_clients_checked(ctl, ctx, 0, 1, 1, &spec, &none),
            "ctl RegisterClients(recorder)");
    fake_input(ctl);
    Raw *c3 = raw_connect("c3", 0);
    unsigned char gif[4] = {43, 0};
    send_req(c3, gif, 4);
    unsigned char rep[32];
    rd(c3, rep, 32);
    close(c3->fd);
    usleep(200000);
    ctl_sync(ctl);
    xcb_record_disable_context(ctl, ctx);
    ctl_sync(ctl);
    drain(rec, 1);
    raw_get(rec, ctx, "GetContext(disabled)");
    req_ctx(rec, 6, ctx);
    raw_result(rec, "Disable(already disabled)");
    req_ctx(rec, 7, ctx);
    raw_get(rec, ctx, "GetContext(freed)");
    req_ctx(rec, 6, ctx);
    raw_result(rec, "Disable(freed)");
    unsigned char bad[4] = {major, 9};
    send_req(rec, bad, 4);
    raw_result(rec, "minor 9");
    xcb_disconnect(ctl);
    return 0;
}

static void R(unsigned char *r, int o, unsigned a, unsigned b) { r[o] = a; r[o + 1] = b; }
static void R16(unsigned char *r, int o, unsigned a, unsigned b) { r[o] = a; r[o + 1] = a >> 8; r[o + 2] = b; r[o + 3] = b >> 8; }

static int scenario_ranges(int big) {
    Raw *rec = raw_connect("rec", big);
    setup_ext(rec);
    xcb_connection_t *ctl = ctl_connect();
    ctl_sync(ctl);
    unsigned char rr[3][24] = {{0}};
    R(rr[0], 0, 10, 20); R(rr[0], 2, 5, 5); R(rr[0], 4, 150, 150); R16(rr[0], 6, 3, 7);
    R(rr[0], 18, 2, 3); R(rr[0], 20, 1, 4); rr[0][22] = 1;
    R(rr[1], 0, 21, 30); R(rr[1], 4, 150, 150); R16(rr[1], 6, 8, 9); R(rr[1], 16, 12, 14);
    R(rr[1], 18, 5, 6); rr[1][23] = 1;
    R(rr[2], 0, 100, 200); R(rr[2], 2, 1, 2); R(rr[2], 10, 140, 141); R16(rr[2], 12, 1, 2);
    R(rr[2], 4, 160, 170);
    unsigned ctx = rec->base + 1;
    unsigned specs[] = {ctl_base, ctl_base, 2, rec->base + 1};
    reg(rec, 1, ctx, 0, specs, 3, rr, 3);
    raw_get(rec, ctx, "GetContext(3 ranges, dup client)");
    /* context XID as a client spec names its owner (rec) */
    unsigned char dev4[1][24] = {{0}};
    R(dev4[0], 18, 4, 4);
    reg(rec, 2, ctx, 1, &specs[3], 1, dev4, 1);
    raw_get(rec, ctx, "GetContext(+rcap by ctx xid)");
    reg(rec, 2, ctx, 5, NULL, 0, dev4, 1);
    raw_get(rec, ctx, "GetContext(register 0 clients, ehdr 5)");
    unsigned fut = 2;
    unsigned char uq[16] = {major, 3};
    put32(rec, uq + 4, ctx);
    put32(rec, uq + 8, 1);
    put32(rec, uq + 12, fut);
    send_req(rec, uq, 16);
    raw_get(rec, ctx, "GetContext(unregister future)");
    put32(rec, uq + 12, 1);
    send_req(rec, uq, 16);
    raw_get(rec, ctx, "GetContext(unregister current)");
    /* AllClients + a later spec: the expansion replaces the whole list */
    unsigned mix[] = {ctl_base, 3, rec->base};
    reg(rec, 2, ctx, 0, mix, 3, &devrange, 1);
    raw_get(rec, ctx, "GetContext(register ctl,all,rec)");
    /* CurrentClients with only... the recorder excluded? not enabled: both */
    unsigned cur = 1;
    reg(rec, 1, rec->base + 2, 0, &cur, 1, dev4, 1);
    raw_get(rec, rec->base + 2, "GetContext(create current)");
    xcb_disconnect(ctl);
    usleep(100000);
    raw_get(rec, rec->base + 2, "GetContext(after ctl left)");
    return 0;
}

static int scenario_errors(int big) {
    Raw *rec = raw_connect("rec", big);
    setup_ext(rec);
    xcb_connection_t *ctl = ctl_connect();
    xcb_window_t w = xcb_generate_id(ctl);
    xcb_create_window(ctl, 0, w, root, 0, 0, 1, 1, 0, XCB_WINDOW_CLASS_INPUT_ONLY, 0, 0, NULL);
    ctl_sync(ctl);
    unsigned ctx = rec->base + 1;
    unsigned char q[20] = {major, 1};
    send_req(rec, q, 16);
    raw_result(rec, "Create short");
    unsigned one = 3;
    reg(rec, 1, ctl_base + 1, 0, &one, 1, &devrange, 1);
    raw_result(rec, "Create foreign id");
    reg(rec, 1, ctl_base + 1, 8, &one, 1, &devrange, 1);
    raw_result(rec, "Create foreign id + ehdr 8");
    {
        unsigned char b[20] = {major, 1};
        put32(rec, b + 4, ctx);
        put32(rec, b + 12, 1000);
        send_req(rec, b, 20);
        raw_result(rec, "Create nClients=1000 no data");
    }
    {
        unsigned char b[24] = {major, 1};
        put32(rec, b + 4, ctx);
        put32(rec, b + 12, 300);
        send_req(rec, b, 24);
        raw_result(rec, "Create nClients=300 short");
    }
    {
        unsigned char b[28] = {major, 1};
        put32(rec, b + 4, ctx);
        put32(rec, b + 12, 1);
        put32(rec, b + 20, 3);
        send_req(rec, b, 28);
        raw_result(rec, "Create length mismatch");
    }
    reg(rec, 1, ctx, 8, &one, 1, &devrange, 1);
    raw_result(rec, "Create ehdr 8");
    unsigned specs[] = {4, root, ctl_base + 0x77, w, 0x1ff00000, rec->base + 5};
    const char *sn[] = {"spec 4", "spec root", "spec ctl+77", "spec ctl window", "spec 0x1ff00000",
                        "spec rec+5"};
    for (int i = 0; i < 6; i++) {
        reg(rec, 1, ctx, 0, &specs[i], 1, &devrange, 1);
        char t[64];
        snprintf(t, sizeof t, "Create %s", sn[i]);
        raw_result(rec, t);
        if (i == 3) { req_ctx(rec, 7, ctx); raw_result(rec, "Free"); }
    }
    struct { int o; unsigned a, b; int w16; const char *n; } bad[] = {
        {0, 5, 4, 0, "core req 5-4"},      {2, 9, 8, 0, "core rep 9-8"},
        {4, 100, 100, 0, "ext req 100-100"}, {4, 200, 150, 0, "ext req 200-150"},
        {6, 3, 2, 1, "ext req minor 3-2"}, {10, 128, 127, 0, "ext rep 128-127"},
        {12, 7, 1, 1, "ext rep minor 7-1"}, {16, 2, 1, 0, "delivered 2-1"},
        {16, 0, 5, 0, "delivered 0-5"},   {18, 1, 6, 0, "device 1-6"},
        {20, 9, 3, 0, "errors 9-3"},       {22, 2, 0, 0, "clientStarted 2"},
        {23, 7, 0, 0, "clientDied 7"},
    };
    for (unsigned i = 0; i < sizeof bad / sizeof bad[0]; i++) {
        unsigned char r[1][24] = {{0}};
        if (bad[i].o >= 22) r[0][bad[i].o] = bad[i].a;
        else if (bad[i].w16) R16(r[0], bad[i].o, bad[i].a, bad[i].b);
        else R(r[0], bad[i].o, bad[i].a, bad[i].b);
        reg(rec, 1, ctx, 0, &one, 1, r, 1);
        char t[64];
        snprintf(t, sizeof t, "Create range %s", bad[i].n);
        raw_result(rec, t);
    }
    reg(rec, 2, rec->base + 50, 0, &one, 1, &devrange, 1);
    raw_result(rec, "Register bogus ctx");
    reg(rec, 1, ctx, 0, &one, 1, &devrange, 1);
    raw_result(rec, "Create ok");
    reg(rec, 1, ctx, 0, &one, 1, &devrange, 1);
    raw_result(rec, "Create dup id");
    {
        unsigned char b[16] = {major, 3};
        put32(rec, b + 4, ctx);
        put32(rec, b + 8, 2);
        send_req(rec, b, 16);
        raw_result(rec, "Unregister length mismatch");
        put32(rec, b + 4, rec->base + 50);
        put32(rec, b + 8, 1);
        send_req(rec, b, 16);
        raw_result(rec, "Unregister bogus ctx");
        put32(rec, b + 4, ctx);
        put32(rec, b + 12, 4);
        send_req(rec, b, 16);
        raw_result(rec, "Unregister spec 4");
        put32(rec, b + 12, ctl_base + 0x77);
        send_req(rec, b, 16);
        raw_result(rec, "Unregister spec ctl+77");
    }
    {
        unsigned char b[12] = {major, 4};
        put32(rec, b + 4, ctx);
        send_req(rec, b, 12);
        raw_result(rec, "GetContext length 3");
        b[1] = 0;
        send_req(rec, b, 4);
        raw_result(rec, "QueryVersion length 1");
        b[1] = 5;
        send_req(rec, b, 4);
        raw_result(rec, "Enable length 1");
    }
    req_ctx(rec, 5, rec->base + 50);
    raw_result(rec, "Enable bogus");
    req_ctx(rec, 7, rec->base + 50);
    raw_result(rec, "Free bogus");
    xcb_disconnect(ctl);
    return 0;
}

/* FreeContext from ctl while rec records; rec's queued request runs after. */
static int scenario_free(int big) {
    Raw *rec = raw_connect("rec", big);
    setup_ext(rec);
    unsigned ctx = rec->base + 1, all = 3;
    reg(rec, 1, ctx, 0, &all, 1, &devrange, 1);
    xcb_connection_t *ctl = ctl_connect();
    ctl_sync(ctl);
    req_ctx(rec, 5, ctx);
    unsigned char gif[4] = {43, 0};
    send_req(rec, gif, 4);
    expect(rec, "Enable");
    usleep(100000);
    raw_result(rec, "queued GetInputFocus while enabled");
    xcb_record_free_context(ctl, ctx);
    ctl_sync(ctl);
    drain(rec, 1);
    raw_result(rec, "queued GetInputFocus after free");
    ctl_get(ctl, ctx, "ctl GetContext(freed)");
    xcb_disconnect(ctl);
    return 0;
}

/* The recorder disconnects while enabled. */
static int scenario_recdie(int big) {
    Raw *rec = raw_connect("rec", big);
    setup_ext(rec);
    unsigned ctx, all = 3;
    xcb_connection_t *ctl = ctl_connect();
    ctl_sync(ctl);
    /* ctl owns the context so it survives rec */
    xcb_record_range_t r = {0};
    r.device_events.first = 2;
    r.device_events.last = 6;
    r.client_died = 1;
    ctx = ctl_base + 1;
    ctl_err(ctl, xcb_record_create_context_checked(ctl, ctx, 0, 1, 1, &all, &r), "ctl Create");
    req_ctx(rec, 5, ctx);
    expect(rec, "Enable");
    ctl_get(ctl, ctx, "ctl GetContext(enabled)");
    close(rec->fd);
    usleep(200000);
    ctl_get(ctl, ctx, "ctl GetContext(recorder gone)");
    xcb_disconnect(ctl);
    return 0;
}

/* The context's creator disconnects while another client records. */
static int scenario_ownerdie(int big) {
    Raw *rec = raw_connect("rec", big);
    setup_ext(rec);
    xcb_connection_t *ctl = ctl_connect();
    unsigned all = 3, ctx = ctl_base + 1;
    xcb_record_range_t r = {0};
    r.device_events.first = 2;
    r.device_events.last = 6;
    r.client_died = 1;
    ctl_err(ctl, xcb_record_create_context_checked(ctl, ctx, 4, 1, 1, &all, &r), "ctl Create");
    req_ctx(rec, 5, ctx);
    expect(rec, "Enable");
    xcb_disconnect(ctl);
    drain(rec, 1);
    raw_get(rec, ctx, "GetContext(owner gone)");
    return 0;
}

/* Print the key elements the recorder reads, collapsing autorepeat presses
 * (their count depends on timing), until EndOfData or `timeout` ms of quiet. */
static void print_keys(Raw *rec, int timeout) {
    int len, repeats = 0;
    unsigned char *b;
    while ((b = next_pkt(rec, timeout, &len))) {
        int big = rec->big;
        if (b[0] == 1 && b[1] == 0) {
            for (unsigned char *p = b + 32; p < b + len; p += 32) {
                if (p[0] == 2 && get16o(big, p + 2) == 1) { repeats++; continue; }
                if (repeats) { printf("rec: %s repeated presses\n", repeats > 1 ? "several" : "one"); repeats = 0; }
                printf("rec: ev type=%u detail=%u seqfield=%u state=0x%x\n", p[0], p[1], get16o(big, p + 2),
                       get16o(big, p + 28));
            }
            free(b);
            continue;
        }
        int c = print_pkt(rec, "rec", b, len);
        free(b);
        if (c == 5) return;
    }
}

/* Autorepeat of an XTEST key held past the repeat delay. */
static int scenario_repeat(int big) {
    Raw *rec = raw_connect("rec", big);
    setup_ext(rec);
    unsigned ctx = rec->base + 1, all = 3;
    unsigned char keys[1][24] = {{0}};
    R(keys[0], 18, 2, 3);
    reg(rec, 1, ctx, 0, &all, 1, keys, 1);
    xcb_connection_t *ctl = ctl_connect();
    ctl_sync(ctl);
    req_ctx(rec, 5, ctx);
    expect(rec, "Enable");
    xcb_test_fake_input(ctl, 2, 38, 0, 0, 0, 0, 0);
    ctl_sync(ctl);
    usleep(900000);
    xcb_test_fake_input(ctl, 3, 38, 0, 0, 0, 0, 0);
    ctl_sync(ctl);
    usleep(100000);
    xcb_record_disable_context(ctl, ctx);
    ctl_sync(ctl);
    print_keys(rec, 2000);
    xcb_disconnect(ctl);
    return 0;
}

/* Record physical keys (pressed by the host through the QEMU monitor) for
 * `secs` seconds. */
static int scenario_listen(int big, int secs) {
    Raw *rec = raw_connect("rec", big);
    setup_ext(rec);
    unsigned ctx = rec->base + 1, all = 3;
    unsigned char keys[1][24] = {{0}};
    R(keys[0], 18, 2, 3);
    reg(rec, 1, ctx, 0, &all, 1, keys, 1);
    xcb_connection_t *ctl = ctl_connect();
    ctl_sync(ctl);
    req_ctx(rec, 5, ctx);
    expect(rec, "Enable");
    if (getenv("RECORD_PROBE_READY")) fclose(fopen(getenv("RECORD_PROBE_READY"), "w"));
    sleep(secs);
    xcb_record_disable_context(ctl, ctx);
    ctl_sync(ctl);
    print_keys(rec, 2000);
    xcb_disconnect(ctl);
    return 0;
}

int main(int argc, char **argv) {
    const char *s = argc > 1 ? argv[1] : "basic";
    int big = argc > 2 && argv[2][0] == 'B';
    int ehdr = argc > 3 ? atoi(argv[3]) : 0;
    setvbuf(stdout, NULL, _IONBF, 0);
    if (!strcmp(s, "basic")) return scenario_basic(big, ehdr);
    if (!strcmp(s, "ranges")) return scenario_ranges(big);
    if (!strcmp(s, "errors")) return scenario_errors(big);
    if (!strcmp(s, "free")) return scenario_free(big);
    if (!strcmp(s, "recdie")) return scenario_recdie(big);
    if (!strcmp(s, "ownerdie")) return scenario_ownerdie(big);
    if (!strcmp(s, "repeat")) return scenario_repeat(big);
    if (!strcmp(s, "listen")) return scenario_listen(big, ehdr ? ehdr : 20);
    fprintf(stderr, "unknown scenario %s\n", s);
    return 2;
}
