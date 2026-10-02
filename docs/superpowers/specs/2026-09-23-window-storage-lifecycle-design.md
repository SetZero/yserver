# Window storage: own it only while viewable (A), then per top-level (2)

Status: design, not implemented. Branch `feat/window-storage-lifecycle`.

## Problem

Every InputOutput window owns a full-size Vulkan image from CreateWindow to
DestroyWindow. Measured on a contributor's 4K awesome session with
`vram by use` (`feat/vram-by-category`): `window=1319MiB/62` with no
compositor running, 2.9 GiB total at peak. Xorg on the same session holds
~0.9 GiB; it allocates nothing per window unless the window is redirected.

Two independent causes:

1. **Hidden windows keep storage.** Unmap frees nothing
   (`backend.rs:20758` `unmap_subwindow` only clears `mapped` and
   `scene_participating`). Windows on other awesome tags, minimised apps and
   every descendant of an unmapped frame keep full-size images.
2. **Every descendant has its own storage.** `create_subwindow`
   (`backend.rs:20635`) allocates for every InputOutput window, so a WM frame
   and its client are two overlapping full-size images. A maximised 4K app
   costs 2 × 31.6 MiB, hidden or not.

A fixes (1). Option 2 fixes (2). A comes first and is built so that 2 changes
*which* windows own storage, not *when* storage exists.

## Reference: Xorg and Xwayland

- Xorg: a non-redirected window has no pixmap; it draws into the screen
  pixmap through its clip list, which is empty while unviewable. Composite
  allocates a redirect pixmap only for a *realized* window
  (`composite/compwindow.c:162` `should = pWin->realized && ...`) and
  destroys it at unrealize (`compUnrealizeWindow` → `compCheckRedirect` →
  `DestroyPixmap`, `compwindow.c:175-181, 284`). `NameWindowPixmap` holds its
  own reference (`compext.c:260`), so a compositor's named pixmap outlives
  the unmap (fade-out animations rely on this).
- Xwayland (rootless): only windows whose parent is the root get a surface
  and `compRedirectWindow(..., CompositeRedirectManual)`, at realize
  (`hw/xwayland/xwayland-window.c:1599` `xwl_realize_window` →
  `ensure_surface_for_window`, redirect at `:1559`); unredirected in
  `xwl_window_dispose` (`:1735`). Descendants draw into the top-level's
  pixmap. That is exactly A + 2.

## A: storage exists only while the window is viewable

### Rule

A window owns storage iff it is **viewable** (mapped, and every ancestor
mapped) and InputOutput. Two exemptions, outside the lifecycle entirely:
the **root** (always owns storage) and the **Composite overlay window**
(owns storage from claim to release, whatever its map state). The
lifecycle code must skip both xids, so no generic root-child or unmap path
can release them.

- CreateWindow allocates nothing (windows are created unmapped).
- The window becoming viewable allocates at its current size + border, fills
  the background (bg pixel / pixmap), paints the border ring.
- The window becoming unviewable drops the window's reference to its storage.
- Resize / border change of an unviewable window only updates geometry.

### Viewability delta: one transition set, computed in core

Viewability changes are subtree events: unmapping a frame makes its whole
mapped subtree unviewable; mapping it makes every descendant whose path is
fully mapped viewable. Core already promotes/demotes descendants' map state
on map, unmap and reparent (`resources.rs`, e.g. `reparent_window` at
`:1687`) but discards that list; the backend and the redirect hooks see
only the window the request named.

Core's resources layer returns a
`ViewabilityDelta { became_viewable: Vec<Window>, became_unviewable: Vec<Window> }`
from every operation that changes viewability while the windows survive
(MapWindow, MapSubwindows, UnmapWindow, UnmapSubwindows, ReparentWindow).
Destruction is **not** a viewability transition; it has its own path
(below). Core then drives, for every member:
storage release/allocation, redirect-backing unrealize/realize, Picture
rebinding, and the map-time background paint. The backend never infers a
subtree itself; `window_viewable` / `collect_viewable_bg_paint_targets`
(`backend.rs:5066`, `:5086`) become consumers of the delta, not a second
source of truth.

### Destruction is a separate path

DestroyWindow, DestroySubwindows and client-disconnect teardown do not go
through `ViewabilityDelta`. On unmap a window Picture stays valid and
rebinds on remap; on destruction Xorg frees every Picture attached to the
window, including Pictures owned by other clients (`PictureDestroyWindow`,
`render/picture.c:67`). Treating destroy as "became unviewable" would leave
dead Pictures valid or let them resurrect onto a reused xid.

Core takes a destruction snapshot of the doomed subtree **before** any
window record is removed, capturing each window's host handles, and then,
for every window in it:

- releases the window's storage and its redirect backing;
- removes the redirect intent (full teardown, as UnredirectWindow);
- frees every window-backed Picture resource and its backend record,
  whichever client owns it, rather than parking it for a possible remap.

### Why content loss is legal and mostly already handled

X does not preserve window contents across unmap (no backing store). We
already behave that way: `map_subwindow` (`backend.rs:20716-20756`) re-tiles
the background of every newly viewable window and core sends Expose for the
subtree. `backing_store` / `save_under` are stored and echoed only; nothing
honours them. So A removes storage nobody was allowed to rely on.

Visible change: a **background None** window currently shows its old pixels
on remap; after A its fresh storage has undefined contents. Xorg shows
whatever was on screen underneath: the parent *and* any lower overlapping
siblings. A seeds a bg-None window from the parent's current pixels only,
reusing the existing seeding path (`seed_backing_from_parent`,
`backend.rs:7669`), which already defers the IncludeInferiors-equivalent
case. **Accepted deviation:** pixels from lower overlapping siblings are
missing from the seed. The client repaints on the Expose it gets anyway; a
composed-underlay seed is a later refinement if the difference is visible
in practice.

### One resolver for "which image does this window draw into"

All paths must get the image through `resolve_paint_target`
(`backend.rs:6401`), which already returns `Option` and already walks to a
redirected ancestor's backing. A unviewable window resolves to `None`.
**`None` means "clipped away", never an error or a warning.** Option 2 later
changes only this function (child → top-level's image at an offset).

`None` from the backend is necessary but not sufficient: a backend call
that "succeeds" as a no-op must not let core report results as if pixels
moved. Core decides the protocol-visible outcome from viewability itself:

- **CopyArea / CopyPlane from an unviewable source window:** the whole
  source region is unavailable, so with graphics-exposures set, core sends
  GraphicsExpose for the corresponding destination area, never NoExpose.
- **Any request drawing to an unviewable destination:** no damage is
  recorded and no DamageNotify is sent.
- **PresentPixmap to an unviewable window:** no copy, no damage, no direct
  scanout, and the normal idle and complete events (CompleteModeCopy), as
  Xorg does.

Paths that currently assume a window has storage and must accept `None`
(from a source survey; verify each):

| Path | Today | Under A |
|---|---|---|
| Core drawing (PolyFill, PutImage, text, …) to an unviewable window | writes the leaf | no-op, no damage (Xorg: empty clip) |
| CopyArea / CopyPlane with an unviewable **source** (`process_request.rs:26869`, no viewability check) | reads the leaf | no copy; GraphicsExpose for the source area if the GC asks for it |
| CopyArea with an unviewable **destination** | writes the leaf | no-op |
| Present copy path into an unviewable window (`process_request.rs:10543-10556`) | writes the leaf | Xorg accepts it (not BadMatch): the copy goes through a GC validated against the empty clip, writes nothing, and still delivers the normal idle/complete events with CompleteModeCopy. Under A: bypass direct scanout, no copy and no damage when the resolver returns `None`, complete normally |
| Render Picture on a window (`render_create_picture`, `backend.rs:24307`; `apply_pending_picture_refs`, `:9208`) | increfs the window's leaf | **lifetime conflict:** that reference would keep the leaf alive past unmap. A window Picture holds no store reference; it resolves its window at each use, yields no target while the window is hidden (the op is clipped away), and binds to the new leaf when the window is realized |
| GetImage | BadMatch when unviewable (`process_request.rs:26045-26061`) | unchanged |
| Redirect seed / restore (`restore_leaves_from_backing`, `backend.rs:3920`) | reads/writes leaves; planner skips unmapped subtrees (test `backend.rs:39011`) | must skip windows without storage |
| Direct scanout / pinned frames referencing the drawable (`direct_frame_references_host_drawable`) | unflip requested on unmap | unchanged; storage release goes through the store's fence-deferred decref (`store.rs:1062-1136`), so a pinned image outlives the window's reference |
| DRI3 BuffersFromPixmap / GLX TFP on a window | no client path exports a leaf: DRI3 accepts only pixmaps (`process_request.rs:12803`); GLX takes an export lifetime ref only for CreatePixmap, CreateWindow has no export host xid (`process_request.rs:14622`) | unchanged; internal users (Present/direct-frame pins, Pictures, debug dumps) must tolerate the leaf disappearing |
| Border ring paint on CWBorderPixel/Pixmap for an unviewable window | paints the leaf | record only; paint at realize |

### Composite redirect under A

A redirected window's redirect backing follows the same rule, as in Xorg:
allocated when the window is (redirected and) viewable, released when it
becomes unviewable.

**Unrealize is not UnredirectWindow.** Today's teardown
(`teardown_redirect_for_window`, `process_disconnect.rs:598`) releases the
backing *and* restores the window's scene participation, which is right
for UnredirectWindow and wrong for an unmapped, manually redirected
window. Under A, becoming unviewable drops only the backing: the redirect
intent (mode, owning client) stays recorded and the window stays out of
the scene. Becoming viewable again allocates a fresh backing for the
existing redirect. UnredirectWindow keeps using the full teardown. The backing is released by dropping the window's
reference; a `NameWindowPixmap` alias keeps its own (`name_window_pixmap`,
`backend.rs:21264`, and the `alias_registry`), so compositors keep their
pixmap across the unmap exactly as on Xorg. Verified: NameWindowPixmap
increments the `AliasRegistry` (`kms/core.rs:1743`), and the backing's
store reference is released only when that count reaches zero
(`free_pixmap`, `backend.rs:21835`). Indirect rather than a per-alias
`DrawableStore::incref`, but it keeps a fade-out backing alive.

The window's own leaf is dead weight while it is redirected (all paint goes
to the backing, `backend.rs:6472-6511`). Out of scope for A, but cheap
follow-up B: keep the leaf at 1×1 while redirected; unredirect already
reallocates it (`backend.rs:21536-21558`).

### Churn

Menus and tooltips map/unmap often. Released storage goes back through the
pixmap pool when its bucket has room (`store.rs:444-468`, cap
`pixmap_pool.rs:347-349`), and the next same-size map is a pool hit. Measure
before adding any hysteresis.

### Verification

- `vram by use`: `window=` must track visible windows. On awesome: open
  large windows on tag 1, switch to an empty tag → `window` drops to the
  visible set; switch back → returns.
- xts A/B (Xlib4 + Xlib9, zero PASS→FAIL): map/unmap/expose/background
  tests exercise exactly these paths.
- Core tests for the delta: map/unmap/reparent/destroy of a frame yields
  the exact descendant sets; a CopyArea from an unviewable source yields
  GraphicsExpose, not NoExpose; drawing to an unviewable window records no
  damage; Present to one completes with no copy.
- Cross-client destruction: client B creates a Picture on client A's
  window; destroying A's window (and, separately, disconnecting A) makes
  B's Picture invalid (BadPicture on use), and a new window reusing the
  host storage does not revive it.
- Unit tests at the backend level: storage absent after unmap of an
  ancestor; present after map; a window Picture yields no target while
  hidden and rebinds after remap; drawing to an unviewable window is a no-op
  without damage; CopyArea from an unviewable source copies nothing.
- HW smoke on Cinnamon (Muffin, direct scanout) and awesome + picom (named
  pixmaps, fade-out).

### Risks

- A path that dereferences a missing storage and warns or panics — the table
  above is the checklist; grep every `store.lookup(host_xid)` in window paths.
- bg-None remap seeding wrong (flash of garbage instead of parent pixels).
- A viewability transition that bypasses the delta (a path that changes map
  state or the tree without going through resources) leaves storage out of
  step with viewability. The delta must be the only source.

## Option 2 (outline): storage per top-level only

Adopt Xwayland's model: only top-level windows (children of the root, i.e.
WM frames and override-redirect windows) own storage; every descendant
draws into its top-level's image at its offset, clipped to its own visible
region. Equivalent to an automatic, internal redirect of every top-level.

- The mechanism exists: descendants of a redirected window already paint
  into the ancestor's backing through `resolve_paint_target`
  (`backend.rs:6472-6511`); every compositor session depends on it.
- Effect: a framed app costs 1× (the frame's image contains the client)
  instead of 2×; with A, memory ≈ the visible top-levels.

Known hard parts:

- **Depth mismatch.** A depth-24 client in a depth-32 frame shares pixels;
  its undefined alpha must be stamped opaque (`stamp_opaque_alpha_if_shared`,
  landed for the redirect path in `e70d5baf`) on every write path.
- **Clip correctness.** Overlapping siblings and shaped children now share
  pixels; each child write must be clipped to its visible region exactly, and
  a child's background paint must not overwrite a sibling.
- **Child Present / DRI3 flips** (video subwindows) become copies into the
  top-level's image; direct scanout stays at top-level granularity.
- **Readback.** GetImage / CopyArea from a child read its region of the
  top-level image; border and bg-None semantics per child.
- **Scene / damage.** The scene composes top-level images only; child
  damage is damage on the top-level.

Boundary: children of the root. Right for ordinary reparenting WMs (the
outer frame stays a root child) and the same criterion Xwayland uses for
rootless realization (`xwayland-window.c:1638`). Not universal: a
virtual-root / container WM makes its container the only root child and
would collapse every frame into one image. Start with root children and
treat virtual-root WMs as an explicit compatibility case to probe, not an
assumed fit.
