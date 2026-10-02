# Test status — latest numbers

Snapshot of the current xts5 (X Test Suite) and rendercheck (RENDER
smoke) pass rates. This file is the headline only; run-by-run history
and debugging notes live in [`xts-baseline.md`](xts-baseline.md) and
`status.md`.

## xts5 — full run #8, yserver/KMS bare-metal (bee/x86_64, 2026-10-01)

**4026 / 5987 test purposes PASS (67.2%)** — +15 vs run #7 (4011),
FAIL 802 → 790, UNRES 61 → 63. Build `713f0a9a`

Movers: Xlib4 +8 (XCirculateSubwindows/Up/Down, XDestroySubwindows,
XDestroyWindow — the CirculateWindow and destroy-order fixes of #194),
Xlib11 +8, XIproto +3, Xlib6 / Xlib12 +2, four suites +1.
Down: Xlib13 −10, XI −2. Per-suite deltas of ±1-2 are noise
(`xts-baseline.md`); larger ones are worth a look.

Xlib13 −10 is not from #194: an Xlib13-only A/B on bee gave
191 PASS on `c0bd7bfc` (before #194) and 194 on `87980d0e`, with
XChangeActivePointerGrab 1/2 and XGrabPointer 20 passing only after
it. The drop happened between run #7 and `c0bd7bfc`; run #7 has no
per-purpose data, so which purposes it is cannot be named.

For context, Xorg itself only passes 77% of the test suite.

| scenario  | cases | tests | PASS | FAIL | UNRES | UNTST | UNSUP | NOTIU | Δ PASS |
|-----------|------:|------:|-----:|-----:|------:|------:|------:|------:|-------:|
| Xproto    |   122 |   389 |  356 |    9 |     3 |    19 |     2 |     0 |      0 |
| Xlib3     |   109 |   162 |  108 |   18 |     2 |    21 |     6 |     1 |      0 |
| Xlib4     |    29 |   324 |  190 |   95 |     3 |    20 |    11 |     5 |     +8 |
| Xlib5     |    15 |    84 |   60 |   17 |     0 |     5 |     2 |     0 |      0 |
| Xlib6     |     8 |    50 |    9 |   12 |     0 |    29 |     0 |     0 |     +2 |
| Xlib7     |    58 |   172 |   87 |   27 |     0 |    13 |    45 |     0 |      0 |
| Xlib8     |    29 |   165 |   93 |   36 |     4 |    22 |    10 |     0 |     +1 |
| Xlib9     |    46 |  1472 |  835 |  374 |     0 |    36 |    23 |   201 |      0 |
| Xlib10    |    23 |    95 |   26 |   36 |     4 |    28 |     1 |     0 |     +1 |
| Xlib11    |    33 |   195 |   95 |   28 |     3 |     4 |    22 |    43 |     +8 |
| Xlib12    |    27 |   138 |   99 |   10 |     1 |    14 |     2 |    12 |     +2 |
| Xlib13    |    32 |   269 |  195 |   38 |    20 |    10 |     3 |     3 |    −10 |
| Xlib14    |    45 |    58 |   46 |    7 |     0 |     5 |     0 |     0 |      0 |
| Xlib15    |    45 |   159 |  125 |    1 |     0 |    33 |     0 |     0 |      0 |
| Xlib16    |    30 |   105 |   82 |    0 |     0 |    22 |     1 |     0 |      0 |
| Xlib17    |    55 |   131 |  103 |    7 |     0 |    21 |     0 |     0 |     +1 |
| Xopen     |     8 |   127 |  123 |    2 |     0 |     0 |     2 |     0 |     +1 |
| Xt3       |    21 |    73 |   73 |    0 |     0 |     0 |     0 |     0 |      0 |
| Xt4       |    33 |   192 |   94 |    0 |     0 |    98 |     0 |     0 |      0 |
| Xt5       |    10 |    69 |   26 |    0 |     0 |    41 |     0 |     0 |      0 |
| Xt6       |     7 |    71 |   67 |    4 |     0 |     0 |     0 |     0 |      0 |
| Xt7       |    11 |   106 |   96 |    1 |     0 |     6 |     0 |     3 |      0 |
| Xt8       |     7 |    43 |   35 |    4 |     0 |     4 |     0 |     0 |      0 |
| Xt9       |    33 |   189 |  122 |    2 |     8 |    55 |     2 |     0 |      0 |
| Xt10      |     8 |    17 |   16 |    0 |     0 |     1 |     0 |     0 |      0 |
| Xt11      |    58 |   285 |  248 |    1 |     0 |    34 |     0 |     0 |      0 |
| Xt12      |    22 |    67 |   55 |    0 |     1 |    11 |     0 |     0 |      0 |
| Xt13      |    39 |   178 |  124 |    5 |     2 |    47 |     0 |     0 |      0 |
| Xt14      |     2 |    18 |   18 |    0 |     0 |     0 |     0 |     0 |      0 |
| Xt15      |     1 |     2 |    0 |    0 |     0 |     0 |     2 |     0 |      0 |
| XtC       |    29 |   147 |   88 |    0 |     2 |    56 |     1 |     0 |      0 |
| XtE       |     1 |     1 |    1 |    0 |     0 |     0 |     0 |     0 |      0 |
| ShapeExt  |    11 |    11 |   11 |    0 |     0 |     0 |     0 |     0 |      0 |
| XI        |    36 |   316 |  215 |   54 |    10 |    31 |     1 |     5 |     −2 |
| XIproto   |    35 |   107 |  105 |    2 |     0 |     0 |     0 |     0 |     +3 |
| **total** | **1078** | **5987** | **4026** | **790** | **63** | **686** | **136** | **273** | **+15** |

ShapeExt, Xlib16 and Xt3/4/5/10/14/XtE are fully clean (zero
FAIL/UNRES). 2 NORESULTs, unchanged; 5 WARNINGs.

Note on what xts5 can and cannot show: the #141 fix (an XI2 selection
absorbs the core press, so core propagation stops there) cannot move a
single number here. xts5 has **no XI2 at all** — its `XI` and `XIproto`
suites are XInput 1.x (`XOpenDevice`, `AllowDeviceEvents`,
`ChangeDeviceKeyMapping`). The same was true of the earlier e27 work.
Both were caught instead by `tools/replay-propagation-probe.c` under
`tools/vng-scenarios/replay-propagation.sh`, which diffs the same probe
binary against Xorg and yserver in one harness.

The XI bucket is spread thin rather than concentrated: 51 FAIL (run #7; 54 in run #8) across
~20 files, led by `XSelectExtensionEvent` 7, `ChangeKeyboardDevice` 6,
`ChangePointerDevice` 5, `AllowDeviceEvents` 5, `GrabDeviceKey` 4.
Beware of ranking these by report-line volume —
`ChangeDeviceKeyMapping` emits 612 keysym lines from just 2 FAILs.
Delivery-shaped reports are a small minority: "not delivered" 15, "too
many events sent" 2, "incorrectly delivered" 2.

Largest FAIL buckets / next targets:
1. **Xlib9 (374)** — remaining drawing/GetImage content semantics.
   Biggest single bucket by a wide margin.
2. **Xlib4 (95)** — depth-mismatch BadMatch (CWBorderPixmap parser
   needed), colormap visual-type checks, bit-gravity pixel cluster,
   stacking-order pixel checks, BadAccess event-mask conflicts.
3. **XI (54)** — XInput-1.x device functions, now the third bucket.
4. **Xlib8 (36)** / **Xlib10 (36)** — events / colormap sections.
5. **Xlib11 (28)** — residual grab semantics, down from 49.

Previous full runs:
- #7 — 2026-09-12 (bee, HW): 4011/5987 PASS (67.0%) —
  `xts/results/2026-09-12-09:40:39/` on bee; per-suite counts only.
- #6 — 2026-06-25 (eiger, aarch64): 3993/5987 PASS (66.7%); results on
  that box.
- #5 — 2026-06-07 22:03:01 (bee, HW): 3961/5987 PASS (66.2%) —
  `xts/results/2026-06-07-22:03:01/`.
- #4 — 2026-06-07 17:14:17 (bee, HW): 3747/5987 PASS (62.6%) —
  `xts/results/2026-06-07-17:14:17/`. Last run before the Xlib4
  BadX work and the desktop-input-fixes branch.
- #3 — 2026-06-06 (air, M1): 3419/5987 PASS (57.1%) —
  `xts/results/2026-06-06-20:26:54/`.
- #2 — 2026-06-05 (M2) + 2026-06-06 air XI row: 3370/5987 PASS (56.3%)
  — `xts/results/2026-06-05-13:20:07/` (+ `2026-06-06-00:58:03` for XI).
- #1 — 2026-06-04 (first ever to complete): 2784/5987 PASS (46.5%) —
  `xts/results/2026-06-04-15:48:44/`.

Aborted run between #3 and #4 (`xts/results/2026-06-07-14:01:34/`):
2999/5987 PASS, 1290 UNRES — the GetImage BadMatch cascade caused
by an unguarded `XConfigureWindow` on the root window, fixed by
`77f785b` before run #4.

## rendercheck — bare-metal 2026-06-04, rendercheck 1.6, 900 s/test

| category    |  PASS | TOTAL |
|-------------|------:|------:|
| fill        |    64 |    64 |
| dcoords     |     2 |     2 |
| scoords     |     1 |     1 |
| mcoords     |     1 |     1 |
| tscoords    |     2 |     2 |
| tmcoords    |     2 |     2 |
| blend       |     5 |     5 |
| composite   |     5 |     5 |
| cacomposite |     5 |     5 |
| gradients   |  6081 |  6081 |
| repeat      |   380 |   380 |
| triangles   |   570 |   570 |
| bug7366     |     1 |     1 |
| **total**   | **7119** | **7119** |

**100% pass.**

> Use rendercheck ≥ 1.6. Version 1.5 has a bug in
> `gradients::render_to_gradient_test` that trips even against the
> host X server.
