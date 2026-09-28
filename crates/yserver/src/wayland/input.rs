//! The compositor's seat, into X: keys, the pointer, the wheel, the
//! keyboard focus and the cursor (Ferrix's `docs/YSERVER.md` §4.4, Y4).
//!
//! The compositor tells a client about its own windows only: which one the
//! pointer is on and where in it, which one has the keyboard. The server
//! turns that into what an X server that owns the screen would have seen.
//!
//! * **The pointer.** An X top-level keeps the root position X gave it,
//!   wherever the compositor shows it, so a position on the compositor's
//!   window is that window's X origin plus the surface-local position. The
//!   hit test is kept to the top-level the pointer is on
//!   ([`PointerScope`]), because two X windows the compositor shows apart
//!   can overlap in root coordinates; off every X window the pointer is on
//!   the root, which gives the last window its `LeaveNotify`.
//! * **Buttons** pass through as evdev codes. **The wheel** becomes X's
//!   buttons 4 to 7, one press and release a click: the compositor's clicks
//!   where it counts them, else ten units of its scroll a click, which is
//!   what a wheel click is in `wl_pointer.axis`.
//! * **Keys** are evdev codes plus 8, cooked by the server's own XKB state,
//!   which takes the compositor's layout when it connects
//!   ([`WaylandLink::adopt_keyboard_layout`]). The toolkit's own repeat is
//!   dropped: the X server repeats a held key itself, per
//!   `ChangeKeyboardControl`.
//! * **Focus.** The keyboard entering a window gives its X top-level the
//!   input focus, as a window manager would; leaving takes it away (focus
//!   None) and lets go of every key the server still has down.
//! * **The cursor** is the X cursor in effect, copied into a buffer and
//!   given with `wl_pointer.set_cursor` whenever it changes; with none, the
//!   compositor's arrow.

use std::collections::BTreeSet;

use compositor_toolkit::{CursorShape, Event, KeyboardEvent, PointerEvent, SurfaceId};
use yserver_core::{
    backend::{ActiveCursorImage, Backend},
    core_loop::{
        message::{
            HostInputEvent, SYNTH_SCROLL_DOWN, SYNTH_SCROLL_LEFT, SYNTH_SCROLL_RIGHT,
            SYNTH_SCROLL_UP,
        },
        process_request::set_input_focus_for_server,
    },
    host_x11::HostKeyEvent,
    resources::ROOT_WINDOW,
    server::{PointerScope, ServerState},
};
use yserver_protocol::x11::ResourceId;

use super::WaylandLink;

/// How much `wl_pointer.axis` scroll is one wheel click, where the
/// compositor does not count clicks itself.
const SCROLL_PER_CLICK: f64 = 10.0;

/// What the renderer knows of the cursor in effect.
pub trait CursorSource {
    /// Which cursor is in effect, as the cursor's XID and a number that
    /// changes whenever its picture does; `None` for no cursor at all.
    fn cursor_key(&self) -> Option<(u32, u64)>;

    /// The cursor in effect.
    fn cursor_image(&self) -> Option<ActiveCursorImage>;
}

/// The seat's state between loop iterations.
#[derive(Debug, Default)]
pub(super) struct Seat {
    /// What the compositor said since the last [`WaylandLink::deliver_input`].
    queue: Vec<Event>,
    /// The compositor's window the pointer is on.
    pointer: Option<SurfaceId>,
    /// Where the pointer last was, in root coordinates.
    at: (i32, i32),
    /// Evdev buttons the server has down.
    buttons: BTreeSet<u32>,
    /// X keycodes the server has down.
    keys: BTreeSet<u8>,
    /// Scroll that is not yet a whole click: down, right.
    scroll: (f64, f64),
    /// The cursor last given to the compositor ([`CursorSource::cursor_key`]);
    /// the outer `None` before the first.
    cursor: Option<Option<(u32, u64)>>,
}

impl Seat {
    /// Keep a seat event for [`WaylandLink::deliver_input`]; anything else
    /// back to the caller.
    pub(super) fn take(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::Keyboard(_) | Event::Pointer(_) => {
                self.queue.push(event);
                None
            }
            other => Some(other),
        }
    }
}

impl WaylandLink {
    /// Take the compositor's keyboard layout for the server's keymap, so a
    /// key means in X what it means on the compositor's other windows. Once,
    /// before any client connects: afterwards a new keymap would owe the
    /// clients a MappingNotify, and the compositor's layout is read only
    /// when it starts.
    pub(crate) fn adopt_keyboard_layout(&mut self, core: &mut crate::kms::core::KmsCore) {
        if self.client.keyboard_layout().is_none() {
            // The keymap comes after the seat's keyboard is made, which the
            // first round trips only start.
            match self.client.roundtrip() {
                Ok(events) => {
                    for event in events {
                        self.handle(event);
                    }
                }
                Err(error) => log::warn!("wayland: waiting for the keymap: {error}"),
            }
        }
        let Some((layout, _)) = self.client.keyboard_layout() else {
            log::info!(
                "wayland: the compositor sent no keymap; the keyboard stays {}",
                core.xkb_rmlvo.layout
            );
            return;
        };
        let rmlvo = crate::kms::core::XkbRmlvo {
            layout: layout.name.to_owned(),
            variant: layout.variant.to_owned(),
            ..core.xkb_rmlvo.clone()
        };
        if core.recompile_keymap(&rmlvo).is_some() {
            log::info!(
                "wayland: the keyboard is the compositor's, {} {:?}",
                layout.name,
                layout.variant
            );
        }
    }

    /// Hand what the compositor's seat did since the last call to X.
    pub fn deliver_input(&mut self, state: &mut ServerState, backend: &mut dyn Backend) {
        for event in std::mem::take(&mut self.seat.queue) {
            match event {
                Event::Pointer(event) => self.pointer_event(state, backend, event),
                Event::Keyboard(event) => self.keyboard_event(state, backend, event),
                _ => {}
            }
        }
    }

    /// The compositor's window the pointer is on, if any.
    pub(super) fn pointer_surface(&self) -> Option<SurfaceId> {
        self.seat.pointer
    }

    /// The X window the compositor's window or popup `surface` is, if it
    /// still is one.
    fn window_of(&self, state: &ServerState, surface: SurfaceId) -> Option<ResourceId> {
        let host_xid = self.host_xid_of(surface)?;
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

    /// Where a position on the compositor's window `surface` is on the root.
    fn root_position(
        &self,
        state: &ServerState,
        surface: SurfaceId,
        x: f64,
        y: f64,
    ) -> Option<(i32, i32)> {
        let window = self.window_of(state, surface)?;
        let (left, top) = state.resources.window_absolute_position(window);
        Some(root_point((left, top), (x, y)))
    }

    fn pointer_event(
        &mut self,
        state: &mut ServerState,
        backend: &mut dyn Backend,
        event: PointerEvent,
    ) {
        match event {
            PointerEvent::Enter { surface, x, y } => {
                self.seat.pointer = Some(surface);
                state.pointer_scope = self
                    .window_of(state, surface)
                    .map_or(PointerScope::Nowhere, PointerScope::Within);
                if let Some(at) = self.root_position(state, surface, x, y) {
                    self.seat.at = at;
                }
                self.move_pointer(state, backend);
            }
            PointerEvent::Leave { .. } => {
                self.seat.pointer = None;
                for button in std::mem::take(&mut self.seat.buttons) {
                    press(state, backend, button, false);
                }
                state.pointer_scope = PointerScope::Nowhere;
                self.move_pointer(state, backend);
            }
            PointerEvent::Motion { surface, x, y } => {
                if let Some(at) = self.root_position(state, surface, x, y) {
                    self.seat.at = at;
                    self.move_pointer(state, backend);
                }
            }
            PointerEvent::Button {
                surface,
                x,
                y,
                button,
                pressed,
                ..
            } => {
                if let Some(at) = self.root_position(state, surface, x, y)
                    && at != self.seat.at
                {
                    self.seat.at = at;
                    self.move_pointer(state, backend);
                }
                let changed = if pressed {
                    self.seat.buttons.insert(button)
                } else {
                    self.seat.buttons.remove(&button)
                };
                if changed {
                    press(state, backend, button, pressed);
                }
            }
            PointerEvent::Axis {
                vertical,
                horizontal,
                discrete,
                ..
            } => {
                let down = clicks(&mut self.seat.scroll.0, vertical, discrete.0);
                let right = clicks(&mut self.seat.scroll.1, horizontal, discrete.1);
                for (count, forward, backward) in [
                    (down, SYNTH_SCROLL_DOWN, SYNTH_SCROLL_UP),
                    (right, SYNTH_SCROLL_RIGHT, SYNTH_SCROLL_LEFT),
                ] {
                    let button = if count > 0 { forward } else { backward };
                    for _ in 0..count.unsigned_abs() {
                        press(state, backend, u32::from(button), true);
                        press(state, backend, u32::from(button), false);
                    }
                }
            }
        }
    }

    /// Tell X the pointer is at [`Seat::at`], which also recomputes the
    /// window it is in and sends the crossings.
    fn move_pointer(&self, state: &mut ServerState, backend: &mut dyn Backend) {
        let (x, y) = self.seat.at;
        yserver_core::core_loop::handle_host_input(
            state,
            backend,
            HostInputEvent::PointerMotion {
                x,
                y,
                time: crate::clock::server_time_ms(),
                relative: false,
                dx: 0,
                dy: 0,
            },
        );
    }

    fn keyboard_event(
        &mut self,
        state: &mut ServerState,
        backend: &mut dyn Backend,
        event: KeyboardEvent,
    ) {
        match event {
            KeyboardEvent::Enter(surface) => {
                let Some(window) = self.window_of(state, surface) else {
                    return;
                };
                if set_input_focus_for_server(state, Some(window)) {
                    log::info!("wayland: the keyboard is on window 0x{:x}", window.0);
                }
            }
            KeyboardEvent::Leave(_) => {
                for keycode in std::mem::take(&mut self.seat.keys) {
                    self.key(state, backend, keycode, false);
                }
                let _ = set_input_focus_for_server(state, None);
            }
            KeyboardEvent::Key(key) => {
                // The server repeats a held key itself.
                if key.repeat {
                    return;
                }
                let Some(keycode) = x_keycode(key.code) else {
                    log::debug!("wayland: key {} has no X keycode", key.code);
                    return;
                };
                let changed = if key.pressed {
                    self.seat.keys.insert(keycode)
                } else {
                    self.seat.keys.remove(&keycode)
                };
                if changed {
                    self.key(state, backend, keycode, key.pressed);
                }
            }
            // The server's XKB state follows the keys themselves.
            KeyboardEvent::Modifiers(_) => {}
        }
    }

    fn key(&self, state: &mut ServerState, backend: &mut dyn Backend, keycode: u8, pressed: bool) {
        let (x, y) = self.seat.at;
        let x = i16::try_from(x).unwrap_or(i16::MAX);
        let y = i16::try_from(y).unwrap_or(i16::MAX);
        yserver_core::core_loop::handle_host_input(
            state,
            backend,
            HostInputEvent::Key(HostKeyEvent {
                pressed,
                keycode,
                time: crate::clock::server_time_ms(),
                root_x: x,
                root_y: y,
                event_x: x,
                event_y: y,
                state: 0,
            }),
        );
    }

    /// Give the compositor the X cursor in effect if it changed since it was
    /// last given. Once per loop iteration.
    pub fn sync_cursor(&mut self, cursors: &dyn CursorSource) {
        let key = cursors.cursor_key();
        if self.seat.cursor == Some(key) {
            return;
        }
        self.seat.cursor = Some(key);
        let Some(image) = key.and_then(|_| cursors.cursor_image()) else {
            self.client.set_cursor(CursorShape::default());
            return;
        };
        let pixels = premultiplied(&image.bgra_bytes);
        match self.client.set_cursor_image(
            u32::from(image.width),
            u32::from(image.height),
            (i32::from(image.hot_x), i32::from(image.hot_y)),
            &pixels,
        ) {
            Ok(()) => log::debug!(
                "wayland: the cursor is {}x{} at ({}, {})",
                image.width,
                image.height,
                image.hot_x,
                image.hot_y
            ),
            Err(error) => {
                log::warn!("wayland: the cursor: {error}");
                self.client.set_cursor(CursorShape::default());
            }
        }
    }
}

/// Press or let go of pointer button `button` (an evdev code, or one of
/// the server's synthetic scroll codes).
fn press(state: &mut ServerState, backend: &mut dyn Backend, button: u32, pressed: bool) {
    let Ok(button) = u16::try_from(button) else {
        return;
    };
    yserver_core::core_loop::handle_host_input(
        state,
        backend,
        HostInputEvent::PointerButton {
            button,
            pressed,
            time: crate::clock::server_time_ms(),
        },
    );
}

/// A surface-local position on a window whose X origin is `origin`, on
/// the root.
fn root_point(origin: (i32, i32), (x, y): (f64, f64)) -> (i32, i32) {
    #[allow(clippy::cast_possible_truncation)]
    let local = (x.floor() as i32, y.floor() as i32);
    (
        origin.0.saturating_add(local.0),
        origin.1.saturating_add(local.1),
    )
}

/// Whole wheel clicks from one axis event: the compositor's own count when
/// it gives one, which also forgets what was left over, else `value`
/// added to `residue`, a click every [`SCROLL_PER_CLICK`].
fn clicks(residue: &mut f64, value: f64, discrete: i32) -> i32 {
    if discrete != 0 {
        *residue = 0.0;
        return discrete;
    }
    *residue += value;
    let whole = (*residue / SCROLL_PER_CLICK).trunc();
    *residue -= whole * SCROLL_PER_CLICK;
    #[allow(clippy::cast_possible_truncation)]
    let whole = whole as i32;
    whole
}

/// An evdev key code as an X keycode, which is 8 more and at most 255.
fn x_keycode(code: u32) -> Option<u8> {
    u8::try_from(code.checked_add(8)?).ok()
}

/// A cursor's straight-alpha B, G, R, A pixels as `wl_shm`'s
/// premultiplied ARGB8888, which is B, G, R, A in memory too.
fn premultiplied(bgra: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bgra.len());
    for pixel in bgra.chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        let scale = |channel: u8| {
            #[allow(clippy::cast_possible_truncation)]
            let scaled = ((u16::from(channel) * alpha + 127) / 255) as u8;
            scaled
        };
        out.extend_from_slice(&[scale(pixel[0]), scale(pixel[1]), scale(pixel[2]), pixel[3]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_on_a_window_is_its_origin_plus_the_local_one() {
        assert_eq!(root_point((100, 50), (10.7, 0.2)), (110, 50));
        assert_eq!(root_point((0, 0), (-0.5, 3.0)), (-1, 3));
    }

    #[test]
    fn the_wheel_counts_the_compositors_clicks_or_ten_units_a_click() {
        let mut residue = 0.0;
        assert_eq!(clicks(&mut residue, 15.0, 1), 1, "its own count");
        assert_eq!(residue, 0.0);
        assert_eq!(clicks(&mut residue, 4.0, 0), 0);
        assert_eq!(clicks(&mut residue, 7.0, 0), 1, "eleven units: a click");
        assert!((residue - 1.0).abs() < 1e-9);
        assert_eq!(clicks(&mut residue, -21.0, 0), -2, "up");
        assert_eq!(clicks(&mut residue, 0.0, -3), -3);
    }

    #[test]
    fn an_x_keycode_is_evdev_plus_eight() {
        assert_eq!(x_keycode(30), Some(38), "KEY_A is X's 38");
        assert_eq!(x_keycode(247), Some(255));
        assert_eq!(x_keycode(248), None);
    }

    #[test]
    fn a_cursor_is_premultiplied() {
        let straight = [
            0xff, 0x80, 0x00, 0x80, 0x10, 0x20, 0x30, 0xff, 0xff, 0xff, 0xff, 0,
        ];
        assert_eq!(
            premultiplied(&straight),
            [0x80, 0x40, 0x00, 0x80, 0x10, 0x20, 0x30, 0xff, 0, 0, 0, 0]
        );
    }
}
