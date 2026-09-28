//! Override-redirect X windows -- menus, drop-downs, tooltips -- as the
//! compositor's popups (Ferrix's `docs/YSERVER.md` §4.2, Y5).
//!
//! An override-redirect window places itself, in root coordinates, and a
//! Wayland client cannot place a surface on the screen. So each becomes an
//! `xdg_popup` on the window of the compositor's it belongs to, hung from a
//! 1×1 anchor rectangle at its own X position relative to that window,
//! growing right and down and never moved by the compositor: where X put it
//! relative to its parent is where it shows, which is also where the
//! pointer translation of [`super::input`] expects it to be.
//!
//! **The parent**, in order: the window `WM_TRANSIENT_FOR` names; the
//! window the pointer is on, which is where the click that opened a menu
//! was, and the parent menu for a submenu; the topmost shown window that
//! holds the popup's position; the topmost top-level. A popup whose parent
//! goes goes first, as `xdg_popup` requires.
//!
//! A popup the compositor takes away (`popup_done`) is not made again until
//! its X window unmaps: the X client still thinks its menu is up, and
//! remaking it would put back what the person just dismissed.

use std::collections::HashSet;

use compositor_toolkit::{PopupOptions, Rect, SurfaceId};
use yserver_core::{
    resources::{COMPOSITE_OVERLAY_WINDOW, MapState, ROOT_WINDOW, Window, WindowClass},
    server::ServerState,
};
use yserver_protocol::x11::ResourceId;

use super::{Frames, WaylandLink, WindowImages, show, transient_for};

/// `xdg_positioner.anchor` top left, and `gravity` bottom right.
const TOP_LEFT: u32 = 5;
/// See [`TOP_LEFT`].
const BOTTOM_RIGHT: u32 = 8;

/// An override-redirect X window as the compositor's popup.
#[derive(Debug)]
pub(super) struct Popup {
    /// The X window's host XID.
    pub(super) host_xid: u32,
    /// The compositor's popup.
    pub(super) surface: SurfaceId,
    /// The host XID of the window or popup it hangs from.
    parent: u32,
    /// Where it hangs from, relative to the parent, and its size.
    placed: Placement,
    /// Its frames.
    pub(super) frames: Frames,
}

/// A popup's position relative to its parent's content and its size, which
/// is what its positioner says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placement {
    at: (i32, i32),
    size: (u16, u16),
}

impl Placement {
    fn options(self) -> PopupOptions {
        PopupOptions {
            size: (i32::from(self.size.0), i32::from(self.size.1)),
            anchor_rect: Rect {
                x: self.at.0,
                y: self.at.1,
                width: 1,
                height: 1,
            },
            anchor: TOP_LEFT,
            gravity: BOTTOM_RIGHT,
            constraint_adjustment: 0,
            offset: (0, 0),
            // An X menu grabs the pointer itself, inside the server.
            grab: false,
        }
    }
}

impl WaylandLink {
    /// Destroy the popups that go: their X window is gone or unmapped, or
    /// what they hang from goes (`kept`, the top-levels that stay, and the
    /// popups kept before them). Children before parents, as `xdg_popup`
    /// requires, which is the reverse of the order they were made in.
    pub(super) fn retire_popups(&mut self, state: &ServerState, kept: &HashSet<u32>) {
        let wanted: HashSet<u32> = menus(state).iter().map(|&(host_xid, _)| host_xid).collect();
        self.dismissed.retain(|host_xid| wanted.contains(host_xid));
        let mut staying: HashSet<u32> = HashSet::new();
        let keep: Vec<bool> = self
            .popups
            .iter()
            .map(|popup| {
                let stays = wanted.contains(&popup.host_xid)
                    && (kept.contains(&popup.parent) || staying.contains(&popup.parent));
                if stays {
                    let _ = staying.insert(popup.host_xid);
                }
                stays
            })
            .collect();
        for (at, stays) in keep.iter().enumerate().rev() {
            if !stays {
                let popup = self.popups.remove(at);
                log::info!(
                    "wayland: popup 0x{:x} is gone from the compositor",
                    popup.host_xid
                );
                self.client.destroy(popup.surface);
            }
        }
    }

    /// Make the popups there are not yet, move the ones whose X window
    /// moved or was resized, and hand over each image that changed.
    pub(super) fn show_popups(&mut self, state: &ServerState, images: &mut dyn WindowImages) {
        for (host_xid, window) in menus(state) {
            if self.dismissed.contains(&host_xid) {
                continue;
            }
            match self
                .popups
                .iter()
                .position(|popup| popup.host_xid == host_xid)
            {
                Some(at) => self.follow(state, at, window),
                None => self.open(state, host_xid, window),
            }
            let client = &mut self.client;
            if let Some(popup) = self
                .popups
                .iter_mut()
                .find(|popup| popup.host_xid == host_xid)
            {
                show(
                    client,
                    images,
                    popup.surface,
                    (host_xid, window),
                    &mut popup.frames,
                    true,
                );
            }
        }
    }

    /// The compositor took a popup away.
    pub(super) fn popup_dismissed(&mut self, surface: SurfaceId) {
        let Some(at) = self
            .popups
            .iter()
            .position(|popup| popup.surface == surface)
        else {
            return;
        };
        let popup = self.popups.remove(at);
        log::info!(
            "wayland: the compositor took popup 0x{:x} away",
            popup.host_xid
        );
        let _ = self.dismissed.insert(popup.host_xid);
        self.client.destroy(popup.surface);
    }

    /// Make `window` a popup on the window it belongs to.
    fn open(&mut self, state: &ServerState, host_xid: u32, window: &Window) {
        let Some(parent) = self.popup_parent(state, host_xid, window) else {
            log::debug!("wayland: popup 0x{host_xid:x} has no window to hang from yet");
            return;
        };
        let (Some(parent_surface), Some(placed)) = (
            self.surface_of(parent),
            self.placement(state, parent, window),
        ) else {
            return;
        };
        match self.client.popup(parent_surface, &placed.options()) {
            Ok(surface) => {
                log::info!(
                    "wayland: override-redirect window 0x{host_xid:x} ({}x{}) is a popup on \
                     0x{parent:x} at {:?}",
                    placed.size.0,
                    placed.size.1,
                    placed.at,
                );
                self.popups.push(Popup {
                    host_xid,
                    surface,
                    parent,
                    placed,
                    frames: Frames::default(),
                });
            }
            Err(error) => log::warn!("wayland: no popup for 0x{host_xid:x}: {error}"),
        }
    }

    /// Move the popup at `at` in [`WaylandLink::popups`] where its X window
    /// now is, if it moved or was resized.
    fn follow(&mut self, state: &ServerState, at: usize, window: &Window) {
        let Some(popup) = self.popups.get(at) else {
            return;
        };
        let Some(placed) = self.placement(state, popup.parent, window) else {
            return;
        };
        if placed == popup.placed {
            return;
        }
        let surface = popup.surface;
        self.client.reposition_popup(surface, &placed.options());
        if let Some(popup) = self.popups.get_mut(at) {
            popup.placed = placed;
            // A popup remade under the same number, where the compositor
            // could not move it, is configured afresh.
            popup.frames = Frames::default();
        }
    }

    /// Where `window` hangs from on `parent`: its outer corner, border and
    /// all, relative to the parent's content, both in root coordinates, and
    /// its size with the border.
    fn placement(&self, state: &ServerState, parent: u32, window: &Window) -> Option<Placement> {
        let parent = self.x_window_of(state, parent)?;
        let (parent_x, parent_y) = state.resources.window_absolute_position(parent);
        let (x, y) = state.resources.window_absolute_position(window.id);
        let border = window.border_width;
        Some(Placement {
            at: (
                x - i32::from(border) - parent_x,
                y - i32::from(border) - parent_y,
            ),
            size: (
                window.width.saturating_add(border.saturating_mul(2)),
                window.height.saturating_add(border.saturating_mul(2)),
            ),
        })
    }

    /// The window or popup `window` hangs from, by host XID.
    fn popup_parent(&self, state: &ServerState, host_xid: u32, window: &Window) -> Option<u32> {
        // Only what the compositor has drawn once: an `xdg_popup`'s parent
        // must be mapped before it.
        let shown = |candidate: u32| {
            candidate != host_xid
                && (self
                    .toplevels
                    .get(&candidate)
                    .is_some_and(|toplevel| toplevel.frames.shown.is_some())
                    || self
                        .popups
                        .iter()
                        .any(|popup| popup.host_xid == candidate && popup.frames.shown.is_some()))
        };
        if let Some(parent) = transient_for(state, window).filter(|&parent| shown(parent)) {
            return Some(parent);
        }
        if let Some(parent) = self
            .pointer_surface()
            .and_then(|surface| self.host_xid_of(surface))
            .filter(|&parent| shown(parent))
        {
            return Some(parent);
        }
        let (x, y) = state.resources.window_absolute_position(window.id);
        let holds = |candidate: u32| {
            self.x_window_of(state, candidate).is_some_and(|id| {
                let Some(found) = state.resources.window(id) else {
                    return false;
                };
                let (left, top) = state.resources.window_absolute_position(id);
                x >= left
                    && y >= top
                    && x < left + i32::from(found.width)
                    && y < top + i32::from(found.height)
            })
        };
        let popups = self.popups.iter().rev().map(|popup| popup.host_xid);
        let stacked = self.toplevels_top_down(state);
        popups
            .chain(stacked.iter().copied())
            .find(|&candidate| shown(candidate) && holds(candidate))
            .or_else(|| stacked.into_iter().find(|&candidate| shown(candidate)))
    }

    /// The shown top-levels in X's stacking order, topmost first.
    fn toplevels_top_down(&self, state: &ServerState) -> Vec<u32> {
        state
            .resources
            .children(ROOT_WINDOW)
            .iter()
            .rev()
            .filter_map(|&child| {
                let host_xid = state.resources.window(child)?.host_xid?.as_raw();
                self.toplevels.contains_key(&host_xid).then_some(host_xid)
            })
            .collect()
    }

    /// The compositor's surface for a window or popup, by host XID.
    fn surface_of(&self, host_xid: u32) -> Option<SurfaceId> {
        self.toplevels
            .get(&host_xid)
            .map(|toplevel| toplevel.surface)
            .or_else(|| {
                self.popups
                    .iter()
                    .find(|popup| popup.host_xid == host_xid)
                    .map(|popup| popup.surface)
            })
    }

    /// The X window a shown window or popup is, by host XID.
    fn x_window_of(&self, state: &ServerState, host_xid: u32) -> Option<ResourceId> {
        if let Some(toplevel) = self.toplevels.get(&host_xid) {
            return Some(toplevel.window);
        }
        state
            .resources
            .children(ROOT_WINDOW)
            .iter()
            .copied()
            .find(|&child| {
                state
                    .resources
                    .window(child)
                    .and_then(|window| window.host_xid)
                    .is_some_and(|xid| xid.as_raw() == host_xid)
            })
    }
}

/// The children of the root shown as popups, bottom to top: viewable,
/// drawing and override-redirect.
fn menus(state: &ServerState) -> Vec<(u32, &Window)> {
    state
        .resources
        .children(ROOT_WINDOW)
        .iter()
        .filter_map(|&child| {
            let window = state.resources.window(child)?;
            let host_xid = window.host_xid?.as_raw();
            is_popup(child, window).then_some((host_xid, window))
        })
        .collect()
}

/// Whether a child of the root is shown as a popup.
fn is_popup(id: ResourceId, window: &Window) -> bool {
    id != COMPOSITE_OVERLAY_WINDOW
        && window.map_state == MapState::Viewable
        && window.class != WindowClass::InputOnly
        && window.override_redirect
        && window.width > 0
        && window.height > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_popup_hangs_from_its_position_and_is_never_moved() {
        let options = Placement {
            at: (40, -12),
            size: (120, 90),
        }
        .options();
        assert_eq!(options.size, (120, 90));
        assert_eq!(
            options.anchor_rect,
            Rect {
                x: 40,
                y: -12,
                width: 1,
                height: 1
            }
        );
        assert_eq!((options.anchor, options.gravity), (TOP_LEFT, BOTTOM_RIGHT));
        assert_eq!(options.constraint_adjustment, 0);
        assert!(!options.grab);
    }
}
