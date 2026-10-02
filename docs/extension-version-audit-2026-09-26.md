# Extension version audit — every advertised version vs Xorg 21.1.24

Audit date: 2026-09-26, branch `fix/173-xi2-raw-keys` @ `61beff4f`. Read-only audit: no code
changed.

**Why:** in #173, yserver answered XI1 `GetExtensionVersion` with a hard-coded 2.0 while
`XIQueryVersion` negotiated 2.4. libXi uses the XI1 answer to decide whether XI 2.2+ fields are
valid, so it zeroed the `sourceid` of raw events (fixed in `61beff4f`). A wrong version in either
direction quietly changes what clients do:

- **Too high:** clients use requests, events or fields that yserver doesn't implement.
- **Too low, or inconsistent between requests:** clients fall back to older paths or decode
  replies and events wrongly.

This audit checks every extension in `nested.rs::EXTENSIONS` for three things: the version
yserver reports, Xorg's answer, and whether yserver actually implements what that version
promises.

**Method**
- **yserver:** read every version handler in
  `crates/yserver-core/src/core_loop/process_request.rs` (called `PR:` below) and its reply
  encoder in `crates/yserver-protocol/src/x11/*.rs`. yserver was **not** run; its answers come
  from the source.
- **Xorg, live:** captured from `Xvfb :93 -ac -noreset` (package `xorg-server-xvfb 21.1.24-1`)
  with a raw-socket probe. Each request got a fresh connection, and the whole run was repeated
  from a big-endian (`'B'`) connection. Each version request was sent with the newest client
  version, an older one, and a higher major, so the negotiation rule shows up in the answers.
  The probe (`extver.py`) lives in the session scratchpad and is not committed.
- **Xorg, source:** `/home/jos/Projects/xserver` is master (`21.0.99.1-1654`). Where 21.1 and
  master differ, the values come from tag `xorg-server-21.1.22` (the newest 21.1 tag in that
  checkout).
- **Xorg with modesetting** has extensions Xvfb lacks (DPMS, DRI3, XFree86-VidMode). Their values
  come from source only.
- **Clients:** checked against the source of mesa (main @ `82d4f86`, sparse checkout), libXi,
  libXrandr, libXfixes, libXrender, libXres, GTK 3.24, Qt 6.8 xcb, SDL2 and cairo, fetched into
  the scratchpad. Anything not checked against source is marked *unverified*.

**Verdict legend:** ✅ OK · 🔺 over-advertised · 🔻 under-advertised · ⚠ inconsistent/negotiation
deviation · 🐞 reply bug

---

## Summary table

| Extension | yserver reports | Xorg 21.1.24 | Negotiation rule (yserver / Xorg) | Verdict |
|---|---|---|---|---|
| BIG-REQUESTS | max 4194303 | max 4194303 | — | ✅ |
| XC-MISC | 1.1 | 1.1 | fixed / fixed | ✅ |
| Generic Event Extension | 1.0 | 1.0 | fixed / fixed | 🐞 reply always little-endian (§8) |
| RANDR | 1.5 | 1.6 | major pinned to 1; the client's minor is used only if client major < 1, **so a client asking 1.2 gets 1.5** / lower of the two versions | 🔺 (CreateMode, SetMonitor/DeleteMonitor, mode add/delete, transforms, panning) ⚠ negotiation (§4) |
| RENDER | 0.11 | 0.11 | fixed / lower of the two | 🔺 0.11 PDF blend ops accepted but draw nothing; filters ignored (§2) ⚠ |
| XINERAMA | 1.1 | 1.1 | fixed / fixed | ✅ |
| XKEYBOARD | 1.0 (UseExtension 1.x or 0.65 → supported) | 1.0, same rule | same | ✅ version; 🐞 requests and replies are little-endian only (§8) |
| XInputExtension | XIQueryVersion: lower of the two, capped at 2.4. XI1 GetExtensionVersion: 2.4 | 2.4 / 2.4 | lower of the two, without Xorg's sticky per-client rule (documented, `PR:26475`) / lower of the two + sticky | 🔺 2.0 requests 41/43/49 silently dropped; 2.2 touch and 2.4 gestures absent (§1) |
| XFIXES | 5.0 | 6.0 | fixed, **even when the client asks for less** / lower of the two + per-client request gate | 🔻 legal (6.0 isn't implemented) · 🔺 inside 5.0 (ChangeCursor, ExpandRegion, Hide/ShowCursor, CursorNotify) · ⚠ (§5) |
| SHAPE | 1.1 | 1.1 | fixed / fixed | ✅ |
| SYNC | 3.1 | 3.1 | lower major, then the client's minor if client major < 3 / fixed 3.1 | 🔺 Await and AwaitFence don't block, CounterNotify never sent (§3) |
| DAMAGE | 1.1 | 1.1 | same rule as Xorg | ✅ (DamageAdd sends no DamageNotify, §9) |
| Composite | 0.4 | 0.4 | fixed / fixed (the client's version wins only if major < 0, which can't happen) | ✅ |
| Present | 1.4 | **1.2** (master / Xwayland 24.1: 1.4 with DRI3, 1.3 without) | fixed, client ignored / "lower of the two", but buggy: client 1.4 → 1.2 and client 2.0 → **2.0** (captured) | ✅ matches master. ⚠ ignores the client's version and reports 1.4 even without DRI3 (§6) |
| DPMS | 1.2 | 1.2 (source; Xvfb has no DPMS) | fixed / fixed | ✅ |
| MIT-SCREEN-SAVER | 1.1 | 1.1 | fixed / fixed | ✅ (Xvfb 21.1 has a big-endian reply bug, see §8; yserver is correct) |
| MIT-SHM | 1.2, sharedPixmaps=0, **pixmapFormat=2**, uid/gid 0 | 1.2, shared=1, fmt=2, real uid/gid | fixed / fixed | 🐞 pixmapFormat should be 0 when sharedPixmaps=0 (§7) |
| XTEST | 2.2 | 2.2 | fixed / fixed | ✅ (GrabControl is a no-op) |
| DRI3 | 1.4 with syncobj, else 1.3; hidden without Vulkan or a render node | 21.1: **1.2** (master: 1.4, dropping to 1.2 without syncobj and 1.0 without modifiers). Xvfb has no DRI3 | major and minor capped separately / the whole version compared at once | ✅ in practice. ⚠ theoretical (§6) |
| GLX | client's version, each component capped at 1.4; GLX_VERSION "1.4"; vendor "yserver" | 1.4 fixed; "1.4"; vendor "SGI" | major and minor capped separately / fixed | 🔺 GLX 1.0 CreateGLXPixmap/DestroyGLXPixmap missing; QueryContext is a stub although GLX_EXT_import_context is listed (§6) |
| X-Resource | client's version, each component capped at 1.2 (client 1.0 → 1.0) | 1.2 fixed | major and minor capped separately / fixed | ✅ (QueryResourceBytes is a stub) ⚠ minor |
| XFree86-VidModeExtension | 2.2 | 2.2 (source; Xvfb has no VidMode) | fixed / fixed | ✅ (writes deliberately return ClientNotLocal) |

**Not advertised by yserver but present in Xvfb/Xorg:** DOUBLE-BUFFER 1.0, RECORD 1.13, SECURITY 1.0,
XVideo 2.2, plus DRI2, XFree86-DGA and XFree86-DRI on Xorg with modesetting. That is legal, since
clients have to probe for them, and out of scope here.

### Xvfb 21.1.24 capture (little-endian client; big-endian identical except MIT-SCREEN-SAVER)

```
BIG-REQUESTS  Enable -> max=4194303
XC-MISC       GetVersion 1.1/1.0/2.0 -> 1.1 / 1.1 / 1.1
GE            QueryVersion 1.0/0.5/2.0 -> 1.0 / 1.0 / 1.0
RANDR         QueryVersion 1.6/1.5/1.2/1.0/2.0/99.99 -> 1.6 / 1.5 / 1.2 / 1.0 / 1.6 / 1.6
RENDER        QueryVersion 0.11/0.10/0.2/1.0 -> 0.11 / 0.10 / 0.2 / 0.11
XINERAMA      QueryVersion 1.1/1.0/2.0 -> 1.1 / 1.1 / 1.1
XKEYBOARD     UseExtension 1.0/0.65/2.0 -> supported=1/1/0, server=1.0
XInput        GetExtensionVersion -> RepType=1 2.4 present=1
              XIQueryVersion 2.4/2.0/2.9/3.0/1.5 -> 2.4 / 2.0 / 2.4 / 2.4 / BadValue(value=1)
XFIXES        QueryVersion 6.0/5.0/4.0/2.0/7.0 -> 6.0 / 5.0 / 4.0 / 2.0 / 6.0
SHAPE         1.1
SYNC          Initialize 3.1/3.0/2.0/4.0 -> 3.1 / 3.1 / 3.1 / 3.1
DAMAGE        QueryVersion 1.1/1.0/0.5/2.0 -> 1.1 / 1.0 / 0.5 / 1.1
Composite     QueryVersion 0.4/0.2/1.0 -> 0.4 / 0.4 / 0.4
Present       QueryVersion 1.4/1.2/1.0/2.0 -> 1.2 / 1.2 / 1.0 / 2.0   (Xorg's rule echoes 2.0)
MIT-SCREEN-SAVER QueryVersion 1.1/1.0 -> 1.1 / 1.1   (big-endian: 256.256)
MIT-SHM       shared=1 1.2 uid=1000 gid=1000 fmt=2
XTEST         GetVersion 2.2/2.1/1.0 -> 2.2 / 2.2 / 2.2
GLX           QueryVersion 1.4/1.2/2.0 -> 1.4 / 1.4 / 1.4; vendor 'SGI'; version '1.4'
X-Resource    QueryVersion 1.2/1.0/2.0 -> 1.2 / 1.2 / 1.2
DPMS, DRI3, XFree86-VidModeExtension: not present in Xvfb
```

### yserver version handlers (file:line)

| Extension | Handler | Encoder / constant |
|---|---|---|
| RANDR | `PR:3094-3116` | `randr.rs:33-34`, `randr.rs:619` |
| RENDER | `PR:2023-2033` → `backend.render_query_version` = `(0,11)` at `kms/render/backend.rs:26363` | `x11/mod.rs:7410` |
| SYNC | `PR:5228-5243` | `sync.rs:69-75`, `sync.rs:303` |
| XINERAMA | `PR:5677-5680` | `xinerama.rs:11-12`, `:71` |
| SHAPE | `PR:6004-6011` | `shape.rs:17-18`, `:245` |
| XFIXES | `PR:6277-6289` | `xfixes.rs:62-63`, `:453` |
| Composite | `PR:7174-7189` | `composite.rs:16-17`, `:56` |
| MIT-SHM | `PR:7752-7758` | `mit_shm.rs:27-28`, `:249-267` |
| DAMAGE | `PR:8462-8481` | `damage.rs:12-13`, `:64` |
| XTEST | `PR:8736-8758` | `xtest.rs:16-17`, `:55` |
| DPMS | `PR:9380-9390` | `dpms.rs:23-24`, `:78` |
| MIT-SCREEN-SAVER | `PR:9588-9598` | `screensaver.rs:20-21`, `:79` |
| Present | `PR:11257-11276` | `present.rs:13-14`, `:216` |
| DRI3 | `PR:12769-12788`; `dri3_version_for` at `kms/render/backend.rs:18682` | `dri3.rs:283` |
| X-Resource | `PR:13897-13906` | `x_resource.rs:25-26`, `:53` |
| XFree86-VidMode | `PR:14271-14285` | `xf86vidmode.rs:29-30`, `:288` |
| GLX | QueryVersion `PR:14602-14615`; QueryServerString `PR:14616-14650` (`STRING_VERSION => "1.4"` at `PR:14624`) | `glx.rs:60-61`, `:228` |
| XI | GetExtensionVersion `PR:15673-15703`; XIQueryVersion `PR:15854-15887` | `PR:82-83` |
| XKB | `xkb_select.rs:66-77` (supported rule); reply built at `kms/xkb.rs:183-192`, patched at `PR:20803-20812` | — |
| GE | `PR:25709-25733` | `x11/mod.rs:2918-2929` |
| BIG-REQUESTS | `PR:25736-25786` | `x11/mod.rs:2933-2945` |
| XC-MISC | `PR:31879-31897` | `x11/mod.rs:7430-7442` |

---

## Flagged items, ranked by client impact

### 1. 🔺 XInputExtension 2.4 — three XI 2.0 requests are silently dropped; touch (2.2) and gestures (2.4) are absent

**Evidence**
- Unmatched XI minors 1–61 fall into `_ => { debug!("known unsupported XI request") ; return
  Ok(RequestOutcome::Handled) }` (`PR:20540-20545`). The client gets no error and nothing happens.
  Only minor 0 and minors above 61 get BadRequest (`PR:20529`).
- Handler arms that exist in `handle_xi2_request`: 1–40, 42, 44–48, 50–61. **Missing: 41
  XIWarpPointer, 43 XIChangeHierarchy, 49 XISetFocus.** They have entries in the request-length
  table (`request_lengths.rs:214-222`) and the byte-swap table (`request_swap.rs:881-912`) but
  no handler.
- 2.2 touch:
  - XIQueryDevice has no TouchClass (`PR:15889-16164`), and no touch or RawTouch events are ever
    generated.
  - Passive grab types 4/5/6 (TouchBegin/GesturePinchBegin/GestureSwipeBegin) are "accepted,
    logged unsupported" and succeed with 0 failed modifiers (`PR:17182-17190`).
  - XIAllowEvents Accept/RejectTouch are no-ops (`PR:26176`).
- 2.4: there is no GestureClass and no gesture events. XISelectEvents keeps only the **first 32
  mask bits** (`PR:15813-15820`), so bit 32 (GestureSwipeEnd) is thrown away.
- 2.1 (scroll classes) and 2.3 (BarrierHit/Leave, XIBarrierReleasePointer 61 at `PR:20487`) are
  real.
- XI 2.2 `sourceid` on raw events is written (`x11/mod.rs:2248`). The XI1/XI2 version mismatch
  from #173 is fixed: `PR:15697-15699`, pinned by test
  `xi_get_extension_version_reports_the_server_version_like_xorg` at `PR:37994`.

**Client impact**
- **XIWarpPointer, dropped silently — highest impact in this audit.**
  - GTK 3.24 `gdk_device_warp` goes straight to `XIWarpPointer` (`gdk/x11/gdkdevice-xi2.c:308`).
  - SDL2 `X11_WarpMouseInternal` uses `X11_XIWarpPointer` whenever XInput2 is initialised and
    there is a single screen (`src/video/x11/SDL_x11mouse.c:336-347`). SDL2 games and emulators
    that warp the pointer (re-centring for relative mode, `SDL_WarpMouseInWindow`) therefore get
    **no warp and no error**.
  - Blender and other apps that warp via XI2 are hit the same way *(unverified which ones)*.
  - Core `WarpPointer` works; only the XI2 path is dead.
- **XISetFocus:** clients that set focus through XI2 get success with no effect. GTK3 focuses
  through core `XSetInputFocus` *(unverified for GTK4)*.
- **XIChangeHierarchy:** `xinput create-master` / `reattach` report success and do nothing, and
  no HierarchyChanged event is sent. That is rare outside xts.
- **Touch and gestures:** yserver never has a touch or gesture device, so a client sees the same
  thing as on Xorg with no touch device:
  - Qt 6 selects touch masks at XI ≥ 2.2 and gesture masks at ≥ 2.4 (`qxcbconnection_xi2.cpp:68-80`).
  - GTK3 asks for 2.4 (`gdkdevicemanager-x11.c:50-54`).
  - libXi sends the 2.2 long form of XIAllowEvents and allows touch/gesture passive grabs at
    2.2/2.4 (`XIAllowEvents.c:53`, `XIPassiveGrab.c:169-311`). yserver accepts both
    XIAllowEvents lengths (`request_lengths.rs:226`).

  This only becomes a real gap on hardware with touch devices, where the libinput touchpad
  gestures Xorg delivers as XI 2.4 events go missing.

**Recommendation**
1. Implement XIWarpPointer (41) as XI's form of `WarpPointer`: FP16.16 coordinates, `src_win`
   bounds check, and a BadDevice check like Xorg's `ProcXIWarpPointer`. Reuse the core
   WarpPointer path. Implement XISetFocus (49) on the core SetInputFocus path.
2. Make the catch-all arm emit BadRequest, or log at warn level, so the next silently-dropped
   request shows up.
3. Keep advertising 2.4. Xorg reports 2.4 regardless of hardware, and dropping to 2.3 would
   change libXi's grab gating for no gain. XISelectEvents should keep the whole mask, not just
   the first 32 bits.

**Risk:** low for 41 and 49, which have a well-defined core equivalent. Turning the catch-all into
BadRequest could break a client that relies on silent success, so check it against xts XI tests
first.

### 2. 🔺 SYNC 3.1 — Await and AwaitFence never block; CounterNotify never sent

**Evidence**
- Initialize: `PR:5228-5243`.
- `x11sync::AWAIT => { // Non-blocking stub. }` (`PR:5318-5320`).
- AwaitFence records a pending await and logs "request stream NOT suspended — known gap"
  (`PR:5515-5557`).
- CounterNotify has no emitter anywhere.
- SERVERTIME alarms never fire: there is no time source.
- There are no BadCounter/BadAlarm/BadFence errors: unknown XIDs are ignored, and QueryCounter
  replies 0 (`PR:5252-5317`, `PR:5408`, `PR:5506`).
- Real parts: fences (Create/Trigger/Reset/Destroy, TriggerFence also triggers xshmfence),
  alarms with Xorg re-arm semantics, and IDLETIME alarms (`run.rs:1305,1775,3197`).
  ListSystemCounters matches Xorg (`sync.rs:336-339`).

**Negotiation:** Xorg always replies 3.1. yserver gives `min(3, client_major)`, and if that is
below 3 it echoes the client's minor. Harmless, since every real client sends 3.1.

**Client impact:** a client that sends `SyncAwaitFence` and then renders expects its *following
requests* to be held until the fence triggers. yserver runs them right away.
- Mesa x11: the fence path is TriggerFence plus client-side `xshmfence_await`, and there is no
  `xcb_sync_await_fence` anywhere in mesa `src/` (grep). So mesa is unaffected.
- mutter's X11 sync ring uses `XSyncTriggerFence` plus GL `glWaitSync`
  (`meta-sync-ring.c:243,399`), not AwaitFence.
- Clients that do call AwaitFence *(unverified list: KWin/sonic-win X11 GLX sync, some Vulkan
  WSI/xshmfence consumers)* would have ordering races.
- Counter `Await` is used by old toolkits and xts. `_NET_WM_SYNC_REQUEST` uses alarms, which work.

**Recommendation:** implement blocking Await/AwaitFence as a per-client request-stream
suspension (Xorg `IgnoreClient`/`AttendClient` in `SyncAwaitFence`/`FreeAwait`). The core loop
already parks clients elsewhere (e.g. DRI3 async waits). Add CounterNotify and the resource errors.

**Risk:** medium. Suspending a client's request stream touches the scheduler. Advertising 3.0
instead doesn't help, because Await is 3.0.

### 3. 🔺 RENDER 0.11 — PDF blend ops accepted but draw nothing; SetPictureFilter stored but ignored

**Evidence**
- Version fixed at 0.11 (`kms/render/backend.rs:26363`, `PR:2024`).
- `StdPictOp::from_u8` covers ops 0–13, 16–27 and 32–43 only (`render_pipeline.rs:223-263`).
  Ops 0x30–0x3e (Multiply…HSLLuminosity, the 0.11 addition) raise no BadValue and hit
  `"gap: unsupported op"` (`engine.rs:7468`, `:8245`), so nothing is drawn.
- SetPictureFilter (0.6) is stored but drawing always uses nearest (`backend.rs:26304-26308`,
  `kms/core.rs:1809-1812`).
- CreateConicalGradient (0.10) is a stub (`PR:2869`); AddTraps (0.9) is a stub (`PR:2887`).
- Unused minors 3/9/14/15/16/21 are silently accepted (`PR:2914`), where Xorg returns
  BadImplementation or BadRequest.
- Negotiation: yserver always says 0.11, while Xorg returns the lower of the two versions
  (captured: 0.10→0.10, 0.2→0.2). Xorg also remembers the client's version and omits sub-pixel
  info from QueryPictFormats for clients below 0.6 (`render/render.c:271-272,362`).

**Client impact**
- **cairo-xlib** sends the PDF operators straight to the server when RENDER ≥ 0.11
  (`cairo-xlib-private.h:384-389`, `CAIRO_RENDER_SUPPORTS_OPERATOR`). Any GTK/cairo app using
  `CAIRO_OPERATOR_MULTIPLY/SCREEN/OVERLAY/...` on an xlib surface gets **missing content**
  instead of cairo's image fallback.
- **cairo** uses SetPictureFilter at ≥ 0.6 (`CAIRO_RENDER_HAS_FILTERS`), so scaled images
  (GTK3 icon and image scaling on the xlib backend) come out nearest-sampled instead of bilinear.
- Conical gradients and AddTraps are rarely used by current toolkits *(unverified)*.
- libXrender sends 0.11 and gates on minor ≥ 6 (`Filter.c:50`, `Xrender.c:473`).

**Recommendation:** implement ops 0x30–0x3e in the shader pipeline; pixman has reference
formulas. If that has to wait, advertise **0.10** so cairo falls back itself. Separately, honour
the bilinear filter.

**Risk:** reporting 0.10 is spec-legal and cairo handles it. Implementing the blend ops needs
rendercheck coverage (`blend` tests).

### 4. 🔺 RANDR 1.5 — mode and monitor requests fail or fake success; the reply goes above what the client asked for

**Evidence (version reply)**

`PR:3094-3116`:

```rust
let reply_major = x11randr::MAJOR_VERSION;
let reply_minor = if r.major < x11randr::MAJOR_VERSION { r.minor } else { x11randr::MINOR_VERSION };
```

A client asking **1.2 gets 1.5**, and one asking 0.x gets 1.x. Xorg returns the lower of the two
(captured 1.2→1.2, 1.0→1.0) and remembers the client's version: `RRClientKnowsRates`
(`randr/rrdispatch.c:27-32`) controls whether GetScreenInfo and ScreenChangeNotify carry rate
data. yserver keeps no per-client RANDR version.

**Evidence (coverage)**

| Version | Request | yserver status |
|---|---|---|
| 1.2 | CreateMode (16) | BadImplementation, marked `// TODO(unimplemented) … STOPGAP` (`PR:4906`) |
| 1.2 | DestroyMode, AddOutputMode, DeleteOutputMode (17–19) | silent success no-op (`PR:5109-5126`) |
| 1.3 | Set/GetCrtcTransform | identity only (`PR:3300-3359`) |
| 1.3 | Get/SetPanning | "no panning" (`PR:3798-3867`) |
| 1.4 | 6 provider-property requests | BadImplementation (`PR:4929-5037`) |
| 1.4 | SetProviderOffloadSink | BadImplementation (`PR:4034-4078`) |
| 1.5 | SetMonitor, DeleteMonitor (43/44) | silent success no-op (`PR:5113-5114`) |

Everything else in 1.2–1.5 is implemented. 1.6 (leases) is correctly not advertised, although
CreateLease (45) still answers BadImplementation and FreeLease answers BadValue.

**Client impact**
- `xrandr --newmode` (the cvt/gtf custom-resolution workflow, arandr, nvidia-settings-style
  tools) gets **BadImplementation**. With Xlib's default error handler that ends the client.
- `xrandr --addmode` / `--rmmode` / `--setmonitor` / `--delmonitor` report success but change
  nothing, which is worse than an error for scripts.
- Clients that set a scaling transform (`xrandr --scale`, fractional-scaling tools) get BadMatch.
- The version over-reply is low impact: libXrandr always sends 1.6 (`XrrConfig.c:223`,
  `Xrandr.c:386`), and Qt sends xcb-proto's version and gates monitors on ≥ 1.5
  (`qxcbconnection_basic.cpp:301-316`). But it breaks the spec's "no higher than requested"
  promise.
- Vulkan `VK_EXT_acquire_xlib_display` needs 1.6 leases. yserver correctly doesn't claim 1.6.

**Recommendation**
1. Make the QueryVersion rule Xorg's lexicographic lower-of-the-two, a 3-line change. Store the
   version per client if yserver wants to gate rates on it like Xorg.
2. Implement CreateMode/AddOutputMode/DeleteOutputMode/DestroyMode as user-mode bookkeeping, as
   in Xorg's `rrmode.c`.
3. Implement SetMonitor/DeleteMonitor as client-defined monitors that GetMonitors merges in,
   as in Xorg's `rrmonitor.c`.
4. Until then, return **BadImplementation** or **BadMatch** instead of faking success.

**Risk:** the negotiation fix is low-risk. User-defined modes interact with SetCrtcConfig
validation, so that part is medium risk.

### 5. 🔺/⚠ XFIXES 5.0 — several ≤5.0 requests are no-ops; the reply ignores the client's version

**Evidence**
- `PR:6277-6289` always replies 5.0. Xorg (6.0) uses the rule at `xfixes/xfixes.c:62-98`, stores
  the client's major, and **rejects every request above that major's request set with
  BadRequest** (`xfixes.c:156-158`). A client that never sent QueryVersion can only use
  QueryVersion on Xorg. yserver has no such gate.
- 6.0 (Set/GetClientDisconnectMode 33/34) → BadRequest (`PR:7135`). That is consistent with
  advertising 5.0.
- Inside 5.0:
  - ChangeCursor (v2, 26) and ExpandRegion (v3, 28) fall to the `other =>` debug arm and are
    silent no-ops (`PR:7146`).
  - ChangeCursorByName is a KMS no-op (`backend.rs:27431`).
  - HideCursor/ShowCursor (v4) are stubs (`PR:7123`).
  - SelectCursorInput stores the mask, but **DisplayCursorNotify is never sent**
    (`PR:6327-6335`).
  - GetCursorImageAndName always returns a blank name (`PR:6369`).
- Barriers (v5) are real.

**Client impact**
- libXfixes sends 6.0 and gates Hide/ShowCursor on major ≥ 4, barriers on ≥ 5 and disconnect
  mode on ≥ 6 (`libxfixes/src/Cursor.c:258-335`, `Disconnect.c:59,80`). With 5.0,
  `XFixesSetClientDisconnectMode` simply doesn't send. That is correct: it is the Xwayland/-terminate
  feature.
- **HideCursor stub:** video players (mpv, VLC), games, `unclutter-xfixes` and SDL
  `SDL_ShowCursor(0)` via XFixes *(SDL uses an invisible cursor; unverified)* can't hide the
  cursor.
- **No CursorNotify:** screen recorders and VNC servers (x11vnc, OBS xcomposite cursor capture)
  that follow cursor changes never see an update.
- **ChangeCursor no-op:** cursor-theme swapping (e.g. `xsetroot`-style live theme changes via
  libXcursor's `XFixesChangeCursorByName`) does nothing.
- mesa WSI asks for 6.0 and only requires major ≥ 2 (`wsi_common_x11.c:338-346`).

**Recommendation**
1. Implement HideCursor/ShowCursor: a per-client hide count, as in Xorg's `cursor.c`
   `CursorHideCount`.
2. Implement CursorNotify on cursor change.
3. Make ChangeCursor/ExpandRegion real, or return BadImplementation.
4. Negotiate with Xorg's rule instead of a fixed 5.0.

**Risk:** low. Each of these is self-contained. Keep 5.0: 6.0's disconnect mode is meaningless
without `-terminate` semantics.

### 6. ⚠/🔺 GLX, Present, DRI3 — mostly OK; GLX has real holes

**GLX**
- QueryVersion caps major and minor separately at 1.4 (`PR:14602-14610`). Xorg always replies
  1.4 (`glx/glxcmds.c:730-765`); mesa sends 1.4, so the reply is the same in practice.
- The GLX_VERSION string is "1.4", matching Xorg. The vendor string is "yserver" where Xorg says
  "SGI".
- **Missing:** GLX 1.0 **CreateGLXPixmap (13) / DestroyGLXPixmap (15)**, CopyContext (10),
  UseXFont (12), Render/RenderLarge. They reach `other` → GLXBadRenderRequest (`PR:15422-15438`).
- mesa's `glXCreateGLXPixmap` really sends opcode 13 even for direct contexts
  (`mesa src/glx/glx_pbuffer.c:846-851`). The client therefore gets an asynchronous X error, and
  **Xlib's default handler exits the app**. Old GL apps and some xscreensaver hacks use
  `glXCreateGLXPixmap` *(unverified list)*.
- **GLX_EXT_import_context** is advertised (`glx.rs:201`), but QueryContext replies with zero
  attributes (`PR:15101-15110`). mesa's `glXImportContextEXT` → `X_GLXQueryContext`
  (`glxcmds.c:1276`) gets nothing back.
- GLX_ARB_fbconfig_float is listed without any float fbconfig. That is legal but pointless.
- Indirect rendering is absent. That is acceptable but not what "1.4" implies:
  `LIBGL_ALWAYS_INDIRECT=1` clients fail.

**Present**
- Always 1.4, and the client's version is ignored (`PR:11257-11276`).
- Xorg 21.1.24 says 1.2. Master / Xwayland 24.1 say 1.4 with DRI3, 1.3 without
  (`include/protocol-versions.h:69-75`). Xorg's own rule is buggy: the captured 2.0 request came
  back as 2.0 (`present_request.c:47-51`).
- PixmapSynced (1.4) is really implemented (`PR:11721-12058`), and the Syncobj capability is
  gated on DRI3 syncobj (`kms/render/backend.rs:27158`). So a 1.4 client without syncobj doesn't
  get explicit sync:
  - mesa WSI needs DRI3 ≥ 1.4, Present ≥ 1.4 and `XCB_PRESENT_CAPABILITY_SYNCOBJ`
    (`wsi_common_x11.c:318-356`, `:3235-3239`).
  - yserver reports DRI3 1.3 without syncobj, so explicit sync stays off.
- Gaps (behaviour, not version):
  - `wait_fence` is validated but never waited on (`PR:11209-11213`).
  - The Fence capability is always set.
  - SelectInput doesn't validate.
- Recommendation: use the lower-of-the-two rule, and report 1.3 when DRI3 is hidden. Both are
  cosmetic.

**DRI3**
- The version is 1.4 with syncobj and 1.3 without (`kms/render/backend.rs:18682`). Negotiation
  caps major and minor separately (`PR:12776-12777`), where Xorg compares whole versions
  (`dri3/dri3_request.c:100-105`). The results differ only when a client sends a major above 1
  (client 2.0 → yserver 1.0, Xorg 1.4). No such client exists today, so this is theoretical.
- Xorg master drops to 1.0 when a screen can't do modifiers.
- yserver keeps 1.2+ and answers GetSupportedModifiers with `[LINEAR]` (`kms/vk/dri3.rs:77-79`).
  That is a valid answer, and mesa's `has_dri3_modifiers` path (`wsi_common_x11.c:355`) copes
  with it.
- PixmapFromBuffers is single-plane only: `num_buffers != 1` → BadAlloc (`PR:12937-12952`).
- SetDRMDeviceInUse (1.3) is a no-op, as it is in Xorg with modesetting.
- Recommendation: switch to the lexicographic rule (a one-line change, zero risk).

### 7. 🐞 MIT-SHM — pixmapFormat=ZPixmap while sharedPixmaps=0

**Evidence**
- `mit_shm.rs:249-267` always writes `PIXMAP_FORMAT_Z_PIXMAP` (2).
- Xorg writes `.pixmapFormat = sharedPixmaps ? ZPixmap : 0` (`Xext/shm.c:289`). uid and gid are
  0 in yserver, geteuid()/getegid() in Xorg (captured `uid=1000 gid=1000`).
- The version, 1.2, matches, and AttachFd (6) and CreateSegment (7) are real
  (`PR:7793-7835`, `PR:8340-8420`).
- Qt 6 turns on the SHM-fd path at 1.2 (`qxcbconnection_basic.cpp:236`); that path works.
- CreatePixmap (1.1) is a one-time copy, not a shared pixmap, which is consistent with
  sharedPixmaps=0 (`PR:7912-8055`). It leaks the host pixmap when the segment or offset checks
  fail (`PR:7982` vs `7993-8016`).

**Client impact:** low. libXext/xcb clients check `shared_pixmaps` before `pixmap_format`
*(unverified for every client)*. The uid/gid fields are informational.

**Recommendation:** send 0 when sharedPixmaps=0, and send the real euid/egid. Fix the
CreatePixmap leak.

**Risk:** none.

### 8. 🐞 Byte order — GE reply always little-endian; XKB little-endian only

**Evidence**
- **GE QueryVersion reply:** `reply.extend_from_slice(&[1, 0]); // major_version = 1`
  (`x11/mod.rs:2925-2926`). A big-endian client reads major = 256. This is the same class of bug
  as the XI1 one.
- **XKB:** there is no request-swap table for major 136 (`request_swap.rs:186-199` covers only
  RANDR 128, XI 137 and VidMode 153):
  - `use_extension` reads `wantedMajor`/`wantedMinor` with `from_le_bytes`
    (`xkb_select.rs:68-69`).
  - Replies are built little-endian throughout (`kms/xkb.rs`, `xkb_desc/reply.rs`, `set_map.rs`,
    about 117 `le_bytes` sites).
  - The sequence number is stamped with `to_le_bytes` (`PR:20716`, `PR:20805`).
  - XKB *events* do use the client's byte order.
- **XI:**
  - The XI1 `GetExtensionVersion` and `XIQueryVersion` reads use `from_le_bytes`
    (`PR:15683`, `PR:15856-15863`). They are **correct**, because `client_reader.rs:187` swaps XI
    request bodies before dispatch (`request_swap.rs:~762,~945`).
  - XIQueryDevice, XIGetSelectedEvents and DeviceChanged class blocks are written little-endian
    (`PR:15897-16164`, `PR:16458-16471`, `fanout.rs:482-524`).
  - The swap table leaves the XISelectEvents mask tail (46), the XIPassiveGrabDevice modifiers
    (54) and property values (37/57) unswapped.
- XTEST, DPMS, MIT-SCREEN-SAVER and X-Resource replies use the client's byte order, but their
  request parsers read little-endian only.
- Xvfb 21.1.24 answers MIT-SCREEN-SAVER QueryVersion to a big-endian client as **256.256**:
  its 21.1 `ProcScreenSaverQueryVersion` doesn't swap major/minor, and master fixed it. yserver
  (`screensaver.rs:79-91`) is correct, so don't copy Xvfb here.

**Client impact:** only big-endian clients are affected: s390x/ppc64 remote X, and xts5
byte-sex tests. No little-endian client is affected.

**Recommendation:** fix the GE reply by writing major/minor with `write_u16(byte_order, …)`, a
one-liner. The XKB/XI/small-extension byte-order gaps should be one scoped project (swap tables
plus reply encoders), not piecemeal.

**Risk:** GE: none. XKB: large surface, so do it with xts byte-sex coverage.

### 9. Lower-impact deviations (no fix needed for correctness today)

- **DAMAGE 1.1 `DamageAdd`** merges rects into damage objects but emits no DamageNotify, and
  treats region 0 or an unknown region as full/empty instead of BadRegion (`PR:8543-8561`).
  Xorg's `ProcDamageAdd` reports damage, so notify listeners fire. It's used by DRI2-era and
  VNC-style clients *(unverified)*.
- **DAMAGE and XFIXES per-client request gating:** Xorg returns BadRequest for requests newer
  than the negotiated major (`damageext.c:483-485`, `xfixes.c:156-158`). yserver doesn't gate.
  This is harmless leniency, since libraries always send QueryVersion first.
- **X-Resource:** yserver caps major and minor separately (client 1.0 → 1.0) where Xorg always
  says 1.2. libXRes sends 1.2 (`XRes.c:75-76`). QueryResourceBytes (1.2) returns zero sizes
  (`PR:14052`).
- **SYNC Initialize and Composite QueryVersion** use slightly different rules from Xorg but give
  the same answer for every real client version.
- **XI `XIQueryVersion`** doesn't reproduce Xorg's sticky per-client rule (a lower second query
  → BadValue). This is intentional and documented (`PR:26475-26494`); only the stored version
  follows Xorg.
- **XTEST GrabControl** is a no-op (`PR:8832`). **MIT-SCREEN-SAVER SetAttributes** always
  returns BadAccess (`PR:9680`). Both are behaviour gaps inside honest versions.

---

## Recommended fix order

1. **XIWarpPointer (41) / XISetFocus (49)**, plus a loud catch-all for unhandled XI minors (§1).
   This silently breaks GTK3 `gdk_device_warp` and SDL2 warps today.
2. **RENDER PDF blend ops**, or advertise 0.10 until they exist, plus the bilinear filter (§3).
   Cairo content currently goes missing.
3. **XFIXES Hide/ShowCursor + CursorNotify** (§5).
4. **RANDR:** QueryVersion lower-of-the-two rule, CreateMode and friends, SetMonitor/DeleteMonitor,
   with errors instead of fake success in the meantime (§4).
5. **SYNC blocking Await/AwaitFence** (§2). Higher effort, and no confirmed victim in the
   daily-driver stack yet.
6. **GLX CreateGLXPixmap/DestroyGLXPixmap**, and either implement QueryContext or remove
   GLX_EXT_import_context from the list (§6).
7. One-liners: GE reply byte order (§8), MIT-SHM pixmapFormat/uid/gid (§7), DRI3 and Present
   negotiation rules (§6).
8. Project: big-endian request/reply support for XKB, the XI reply blocks and the small
   extensions (§8).
