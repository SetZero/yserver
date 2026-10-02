# Window storage only while viewable (A) — implementation plan

Implements part A of `../specs/2026-09-23-window-storage-lifecycle-design.md`
(reviewed by codex, `3ffb770a`). Read that first; this plan does not restate
its reasoning. Option 2 is out of scope.

**Branch:** `feat/window-storage-lifecycle`.

**Deliverable:** a window owns storage only while it is viewable. On the
reporter's awesome session, windows on hidden tags and minimised apps cost
no VRAM; `vram by use` shows `window=` tracking the visible set.

## Ordering principle

**Make every consumer cope with "no storage" before anything stops
allocating it.** Steps 1–4 each change one thing while every window still
owns storage, so a regression in any of them is attributable and none of
them can crash on a missing image. Step 5 is the only step that frees
storage, and by then it is mostly deletion of the unconditional allocation.

## Prerequisites

- Gates before every commit, exactly CI's lines: `cargo +nightly fmt --
  --check`, `cargo clippy --all-targets -- -D warnings` (also with
  `--features tcp-transport` and `--features xdmcp`), `cargo test`.
- Lavapipe ignored suite: `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
  cargo test -p yserver --lib -- --ignored`, plus `--test render_acceptance
  -- --ignored`.
- `rendercheck` A/B (`just rendercheck-yserver`) for steps that change Render
  handling. No per-step xts A/B: its runs are not deterministic.
- `feat/vram-by-category` merged first (HW-checked on bee): step 6 measures
  with its `vram by use` line.

## Step 1 — Core: `ViewabilityDelta` (no consumer yet)

Pure refactor; no behaviour change.

- `resources.rs`: `map_window` (`:1257`), `unmap_window` (`:1374`) and
  `reparent_window` (`:1687`) return a
  `MapTransition { mapping_changed: bool, delta: ViewabilityDelta }`, where
  `ViewabilityDelta { became_viewable, became_unviewable }` replaces the
  discarded lists built by
  `promote_unviewable_descendants` (`:1302`) and `demote_viewable_descendants`
  (`:1342`); see the discarded vectors at `:1398-1400` and `:1753-1758`.
  Fold `map_window_with_promoted_descendants` (`:1262`) into it.
  `mapping_changed` is kept separate from the delta because the two
  diverge: mapping a window under an unmapped parent (Unmapped →
  Unviewable) has an empty delta but still sends MapNotify; unmapping an
  unviewable window (Unviewable → Unmapped) has an empty delta but still
  sends UnmapNotify and runs the grab/focus handling. Today's `bool`
  returns carry exactly `mapping_changed`.
- `process_request.rs`: `handle_map_window` (`:24563`),
  `handle_map_subwindows` (`:24765`), `handle_unmap_window` (`:24851`),
  `handle_unmap_subwindows` (`:24935`) and `handle_reparent_window`
  (`:20397`) receive the delta. The Subwindows handlers union the per-child
  deltas. **Fix while here:** `handle_map_subwindows` calls plain
  `map_window` per child (`:24791`), so grandchild promotions never surface
  today.
- The existing damage reset that uses `promoted_descendants`
  (`:24740-24743`) switches to `delta.became_viewable`.
- Tests (core, no Vulkan): map/unmap/reparent/MapSubwindows/UnmapSubwindows
  of a frame with nested children, some unmapped, yield the exact sets. An
  already-viewable remap yields `mapping_changed = false` and an empty
  delta. Map under an unmapped parent and unmap of an unviewable window
  yield `mapping_changed = true` with an empty delta, and still send
  MapNotify / UnmapNotify.

### Delta processing order (all later steps)

Every consumer walks `became_viewable` **parent before child**, so
background-None seeding and redirect-backing seeding always find the parent
already materialized, and walks `became_unviewable` **child before parent**.
The delta's vectors are produced in that order (pre-order for viewable,
post-order for unviewable) and a test asserts it.

## Step 2 — Core: protocol results for unviewable windows

Correct today already, with the storage still present.

- **CopyArea / CopyPlane:** `copy_area_source_split` (`process_request.rs:26752`)
  treats an unviewable source *window* as wholly missing, so `emit_copy_exposures`
  (`:26828`, called at `:27032`) sends GraphicsExpose for the whole
  destination area and no NoExpose. Same shape in `handle_copy_plane`
  (`:27278`, `:27333-27347`). Pixmap sources are unaffected.
- **Present to an unviewable window:** before `try_present_direct`
  (`:10514`), skip direct scanout and the copy (`:10543`, `:10556`); record no
  damage; still `enqueue_present_completion` (`:10603`) with the completion
  built at `:10479-10494` (CompleteModeCopy).
- **Drawing damage:** already gated on `map_state != Viewable` in
  `accumulate_damage_to_state` (`damage_fanout.rs:315-319`). Add the test
  only.
- Tests: CopyArea from an unmapped window with graphics-exposures on →
  GraphicsExpose covering the destination rect; Present to an unmapped
  window → idle + complete, no damage, no backend copy; PolyFill on an
  unmapped window → no DamageNotify.
- Check the Present behaviour against `../xserver/present/` before
  committing; the design records Xorg's result.

## Step 3 — Pictures on windows

- Backend: `render_create_picture` (`backend.rs:24307`) stops increfing the
  window's leaf (`:24327-24329`) and the `pending_picture_drawable_refs`
  deferral (`:24337`, `apply_pending_picture_refs` `:9208`) goes away for
  window drawables. A window Picture resolves its window at each use through
  `resolve_paint_target`; `None` clips the operation away.
  `render_free_picture` (`:24422`) no longer decrefs for window Pictures.
- Core destruction: `destroy_window_subtree` (`process_request.rs:1748`)
  already snapshots the subtree into `pending` (`:1769-1792`) before
  `resources.destroy_window` (`:1839`). Extend the snapshot with every Picture
  whose `drawable` (`resources.rs:178` `PictureState`) is in the subtree, for
  **all** clients, and free those Picture resources and their backend
  records. Same in the disconnect path (`process_disconnect.rs:180-264`
  snapshot; teardown `:519-527`, `:728`). Core has no window→Picture index;
  a scan of `resources.pictures` filtered against the snapshot is enough to
  start.
- Tests: the design's cross-client test (client B's Picture on client A's
  window → BadPicture after A's DestroyWindow, and after A disconnects; a new
  window reusing the host storage does not revive it); a window Picture
  composites nothing while the window is unmapped and works after remap.

## Step 4 — Composite: unrealize is not UnredirectWindow

- Split `teardown_redirect_for_window` (`process_disconnect.rs:598`), which
  today releases the backing (`:613`) *and* restores scene participation
  (`:636`):
  - `unrealize_redirect_backing`: `take()` the window's `redirected_backing`
    (as the full teardown does at `:604-609`) and release it; the
    `composite_redirects` record (`server.rs:1135`, `RedirectRecord`
    `:817-820`) and the window's scene exclusion stay. Clearing
    `redirected_backing` matters: a stale value would make the remap path
    think a backing exists and skip reallocating it.
  - the full teardown stays for UnredirectWindow, UnredirectSubwindows, the
    reparent REVOKE (`process_request.rs:20568`) and destruction.
- Consume the step-1 delta: `became_unviewable` of a redirected window →
  `unrealize_redirect_backing`; `became_viewable` → allocate a fresh backing
  for the existing redirect (reuse the path `reapply_redirect_mode_after_map`
  and `maybe_activate_child_under_redirected_parent` already take,
  `:24668-24669`, `:24803-24804`).
- Named pixmaps keep a released backing alive through the `AliasRegistry`
  (`kms/core.rs:1743`; `release_redirected_backing` `backend.rs:21508`,
  decref `:21557-21559`).
- Tests: unmap of a manually redirected window frees the backing, keeps the
  redirect record and keeps it out of the scene; remap re-creates a backing;
  a NameWindowPixmap taken before the unmap stays valid and readable after
  it.
- HW: awesome + picom, open/close windows (fade-outs read named pixmaps).

## Step 5 — The switch: window storage follows viewability

- Split `allocate_window_storage` (`backend.rs:15452`), which today does
  both and returns early when the geometry is already recorded (`:15464`
  `self.windows.contains_key`), into:
  - `register_window_geometry`: records geometry, depth, parent, background
    and border; called by `create_subwindow` (`:20635`, today's call at
    `:20668`);
  - `allocate_window_leaf`: allocates, fills and seeds the leaf, idempotent
    on `store.lookup(host_xid)` rather than on `self.windows`.
- New backend entry points driven by the delta (core calls them for every
  member, never inferring a subtree):
  - `realize_window_storage(xid)`: allocate at the current bordered size
    through `allocate_window_leaf`, fill the background, paint
    the border ring, and for bg None seed from the parent with
    `seed_backing_from_parent` (`:7669`) (accepted deviation: no lower
    siblings).
  - `release_window_storage(xid)`: drop the window's store reference; the
    store's fence-deferred decref keeps a pinned or in-flight image alive.
- Exemptions: the root and the COW (`get_overlay_window` `:21594`,
  `cow_id` `:1297`, `finish_cow_release` `:1962`) never go through these.
- `register_top_level` (`:21075`) and `register_subwindow` (`:21102`)
  allocate 1×1 placeholders for unknown xids; make them follow the same rule
  (no storage unless viewable).
- `configure_subwindow` of an unviewable window updates geometry only; the
  border-attribute path records the border without painting
  (`border_ring_thickness` `:4829` must handle a missing leaf).
- `resolve_paint_target` (`:6401`) gets a **functional** viewability gate
  before it walks to a redirected ancestor: an unviewable window resolves to
  `None` even if a viewable ancestor has a redirect backing. Without it, a
  hidden child with no leaf would resolve to that ancestor's backing and
  paint into it. The gate reads the backend's mirror of core's viewability,
  kept current by the delta.
- `map_subwindow` / `unmap_subwindow` (`:20716`, `:20758`) and
  `collect_viewable_bg_paint_targets` (`:5086`) become delta consumers;
  `window_viewable` (`:5066`) is replaced by the delta-maintained mirror.
- Audit every window-path `store.lookup(host_xid)` in `backend.rs` (128
  occurrences including tests) for a missing leaf being an error, warning or
  panic. The redirect route flip (`:21357`, `:21526`, `:21539`) must accept
  a redirected window without a leaf.
- Tests: storage absent after an ancestor's unmap and present after its
  map; an unmapped child of a viewable redirected window resolves to `None`,
  not to the ancestor's backing, and a draw to it leaves the backing
  unchanged; a window Picture rebinding to the new leaf; drawing to an unmapped
  window writes nothing; a pool hit on remap of the same size.

## Step 6 — Measure and smoke

- `vram by use` on awesome (air or bee): open large windows on tag 1, switch
  to an empty tag, `window=` drops to the visible set; switch back, it
  returns. Then ask the reporter for one run.
- HW smoke: Cinnamon (Muffin, direct scanout; lock screen), awesome + picom
  (fade-outs), MATE, and a menu/tooltip-heavy app for map/unmap churn.
- rendercheck A/B against the step-2 baseline.

## Atomic-switch check

At every step boundary the tree must run: after steps 1–4 every window
still owns storage and the new paths only add correct behaviour, so a
partial landing is safe. Step 5 changes allocation and every consumer in
one commit; it must not be split into "stop allocating" and "cope with
absence".
