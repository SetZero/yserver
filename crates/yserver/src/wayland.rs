//! The rootless Wayland backend's connection to its compositor, and the
//! compositor's windows the X server's top-level windows become.
//!
//! With `YSERVER_BACKEND=wayland` the server is a client of the Wayland
//! compositor `WAYLAND_DISPLAY` names, rather than the owner of a DRM card:
//! the renderer runs headless (lavapipe, or a render node), the root window
//! takes the size of the compositor's first screen, and each top-level X
//! window becomes an `xdg_toplevel` of the compositor's. The design is
//! Ferrix's `docs/YSERVER.md`.
//!
//! The Wayland side is Ferrix's own client runtime, `compositor-toolkit`,
//! driven from the server's core loop: its socket is polled there
//! ([`crate::kms::render::KmsBackend`] registers it as
//! `BackendFdKind::Wayland`), and each readiness is one non-blocking
//! [`Client::dispatch`].
//!
//! # Windows
//!
//! [`WaylandLink::sync`] runs once per loop iteration. It makes a window of
//! the compositor's for each child of the root that is viewable, draws
//! (an `InputOutput` window) and is not override-redirect, and destroys the
//! ones whose X window went. The server redirects the root's children for
//! itself, so each top-level's image, its subwindows included, is one
//! backing ([`WindowImages`]). A window's image is read back and handed over
//! as `wl_shm` when it changed and the compositor's last frame callback for
//! it has come, so an X client that draws faster than the screen shows is
//! read back once per frame, and an idle one not at all.
//!
//! The title is `_NET_WM_NAME`, or `WM_NAME`; the app id is `WM_CLASS`'s
//! class, what a window rule matches. Both follow the properties.
//!
//! Not yet: override-redirect windows (menus, as popups), input, and the
//! compositor's own size and close for a window (docs/YSERVER.md, Y4 and
//! Y5). Until then an X window keeps the size its client gave it, drawn at
//! the top left of the window the compositor tiles, on black.
//!
//! # Layout
//!
//! The Wayland side lives here and knows the X server only through
//! `&ServerState` and [`WindowImages`]; the renderer side lives in
//! `KmsBackend` (`kms/render/backend.rs`), each piece behind
//! `feature = "wayland"`:
//!
//! * `KmsBackend::attach_wayland` takes the [`WaylandLink`] at startup;
//! * `Backend::on_wayland_ready` calls [`WaylandLink::dispatch`], which
//!   hands each [`Event`] to `WaylandLink::handle` -- where the compositor's
//!   input will go;
//! * `Backend::poll_deferred_input` calls `KmsBackend::sync_wayland` once
//!   per loop iteration: the server's own redirect of the root's children
//!   (`yserver_core`'s `redirect_subwindows_for_server`), then
//!   [`WaylandLink::sync`];
//! * `Backend::on_window_property_changed` calls
//!   [`WaylandLink::property_changed`];
//! * `impl WindowImages for KmsBackend` reads a window back.
//!
//! A top-level is keyed by its X window's host XID ([`WaylandLink`]'s
//! `toplevels`), and `WaylandLink::by_surface` finds one from the
//! compositor's side.

use std::collections::{HashMap, HashSet};
use std::io;
use std::time::Duration;

use compositor_toolkit::tiny_skia::PixmapMut;
use compositor_toolkit::{Client, Event, SurfaceId, ToplevelOptions};
use yserver_core::resources::{
    COMPOSITE_OVERLAY_WINDOW, MapState, ROOT_WINDOW, Window, WindowClass,
};
use yserver_core::server::ServerState;
use yserver_protocol::x11::AtomId;

/// What the link needs of the renderer: each top-level's image, by the
/// window's host XID.
pub trait WindowImages {
    /// A number that changes whenever anything is drawn into the window's
    /// image; `None` for a window the renderer does not know.
    fn image_version(&self, host_xid: u32) -> Option<u64>;

    /// The window's image, `width` × `height` pixels without its border, each
    /// four bytes in memory order B, G, R and alpha (unused below depth 32):
    /// an X `ZPixmap` at 32 bits per pixel.
    fn read_image(&mut self, host_xid: u32, width: u16, height: u16) -> Option<Vec<u8>>;
}

/// A top-level X window as the compositor's window.
#[derive(Debug)]
struct Toplevel {
    /// The compositor's window.
    surface: SurfaceId,
    /// What it was last given.
    title: String,
    /// What it was last given.
    app_id: String,
    /// A frame callback was asked for and has not come.
    frame_pending: bool,
    /// The [`WindowImages::image_version`] last drawn.
    shown: Option<u64>,
    /// The compositor configured it since it was last drawn.
    configured: bool,
}

/// The connection, what the compositor has said about its screens, and the
/// windows made on it.
pub struct WaylandLink {
    client: Client,
    /// By the X window's host XID.
    toplevels: HashMap<u32, Toplevel>,
    /// Host XIDs whose title or class changed since the last sync.
    renamed: HashSet<u32>,
}

impl WaylandLink {
    /// Connect to the compositor `WAYLAND_DISPLAY` names, and wait until it
    /// has described every screen.
    ///
    /// # Errors
    ///
    /// No `WAYLAND_DISPLAY`, a socket that refuses, or a compositor that
    /// closes the connection during the first round trips.
    pub fn connect() -> io::Result<Self> {
        let client = Client::connect().map_err(|error| {
            io::Error::new(io::ErrorKind::NotConnected, format!("wayland: {error}"))
        })?;
        Ok(Self {
            client,
            toplevels: HashMap::new(),
            renamed: HashSet::new(),
        })
    }

    /// The socket, for the core loop's poller.
    #[must_use]
    pub fn fd(&self) -> std::os::fd::RawFd {
        self.client.as_raw_fd()
    }

    /// The size of the compositor's first screen in logical pixels, which
    /// the root window takes; `None` while it has no screen.
    #[must_use]
    pub fn screen_size(&self) -> Option<(u16, u16)> {
        let output = self
            .client
            .outputs()
            .into_iter()
            .find(|output| output.done)?;
        let (width, height) = output.logical_size();
        let width = u16::try_from(width).ok().filter(|&width| width > 0)?;
        let height = u16::try_from(height).ok().filter(|&height| height > 0)?;
        Some((width, height))
    }

    /// Read what the compositor sent, without blocking, and act on it.
    ///
    /// # Errors
    ///
    /// The compositor closed the connection or refused a request.
    pub fn dispatch(&mut self) -> io::Result<()> {
        let events = self
            .client
            .dispatch(Some(Duration::ZERO))
            .map_err(|error| {
                io::Error::new(io::ErrorKind::ConnectionAborted, format!("{error}"))
            })?;
        for event in events {
            self.handle(event);
        }
        Ok(())
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Frame { surface, .. } => {
                if let Some(toplevel) = self.by_surface(surface) {
                    toplevel.frame_pending = false;
                }
            }
            Event::Configure { surface, .. } => {
                if let Some(toplevel) = self.by_surface(surface) {
                    toplevel.configured = true;
                }
            }
            Event::CloseRequested(surface) => {
                log::info!("wayland: the compositor asked to close {surface:?}; not yet done");
            }
            other => log::debug!("wayland: {other:?}"),
        }
    }

    fn by_surface(&mut self, surface: SurfaceId) -> Option<&mut Toplevel> {
        self.toplevels
            .values_mut()
            .find(|toplevel| toplevel.surface == surface)
    }

    /// A window's title or class may have changed: `property` was set or
    /// deleted on the window with host XID `host_xid`.
    pub fn property_changed(&mut self, state: &ServerState, host_xid: u32, property: AtomId) {
        if is_name(state, property) {
            let _ = self.renamed.insert(host_xid);
        }
    }

    /// Bring the compositor's windows in line with the X server's
    /// top-levels, and hand over each image that changed and whose last
    /// frame the compositor has shown. Once per loop iteration.
    pub fn sync(&mut self, state: &ServerState, images: &mut dyn WindowImages) {
        let wanted: Vec<(u32, &Window)> = state
            .resources
            .children(ROOT_WINDOW)
            .iter()
            .filter_map(|&child| {
                let window = state.resources.window(child)?;
                let host_xid = window.host_xid?.as_raw();
                is_toplevel(child, window).then_some((host_xid, window))
            })
            .collect();

        let client = &mut self.client;
        self.toplevels.retain(|host_xid, toplevel| {
            let keep = wanted.iter().any(|(wanted, _)| wanted == host_xid);
            if !keep {
                log::info!("wayland: window 0x{host_xid:x} is gone from the compositor");
                client.destroy(toplevel.surface);
            }
            keep
        });

        for &(host_xid, window) in &wanted {
            let toplevel = match self.toplevels.entry(host_xid) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let (title, app_id) = names(state, window);
                    let options = ToplevelOptions {
                        title: title.clone(),
                        app_id: app_id.clone(),
                        size: (u32::from(window.width), u32::from(window.height)),
                    };
                    match self.client.toplevel(&options) {
                        Ok(surface) => {
                            log::info!(
                                "wayland: window 0x{host_xid:x} ({}x{}) is the compositor's \
                                 window {surface:?}, {title:?} of {app_id:?}",
                                window.width,
                                window.height,
                            );
                            entry.insert(Toplevel {
                                surface,
                                title,
                                app_id,
                                frame_pending: false,
                                shown: None,
                                configured: false,
                            })
                        }
                        Err(error) => {
                            log::warn!("wayland: no window for 0x{host_xid:x}: {error}");
                            continue;
                        }
                    }
                }
            };

            if self.renamed.contains(&host_xid) {
                let (title, app_id) = names(state, window);
                if title != toplevel.title {
                    self.client.set_title(toplevel.surface, &title);
                    toplevel.title = title;
                }
                if app_id != toplevel.app_id {
                    self.client.set_app_id(toplevel.surface, &app_id);
                    toplevel.app_id = app_id;
                }
            }

            if toplevel.frame_pending || self.client.size(toplevel.surface).is_none() {
                continue;
            }
            let version = images.image_version(host_xid);
            if version.is_some() && version == toplevel.shown && !toplevel.configured {
                continue;
            }
            let Some(pixels) = images.read_image(host_xid, window.width, window.height) else {
                continue;
            };
            let opaque = window.depth != 32;
            let (width, height) = (window.width, window.height);
            match self.client.draw(toplevel.surface, |pixmap| {
                blit(pixmap, &pixels, width, height, opaque);
            }) {
                Ok(true) => {
                    self.client.request_frame(toplevel.surface);
                    toplevel.frame_pending = true;
                    toplevel.shown = version;
                    toplevel.configured = false;
                }
                Ok(false) => {}
                Err(error) => log::warn!("wayland: drawing 0x{host_xid:x}: {error}"),
            }
        }
        self.renamed.clear();
        self.flush();
    }

    /// Send what is queued.
    pub fn flush(&mut self) {
        if let Err(error) = self.client.flush() {
            log::warn!("wayland: flush: {error}");
        }
    }
}

/// Whether a child of the root is shown as a window of the compositor's:
/// viewable, drawing, and not override-redirect (a menu or a tooltip, which
/// becomes a popup in a later slice).
fn is_toplevel(id: yserver_protocol::x11::ResourceId, window: &Window) -> bool {
    id != COMPOSITE_OVERLAY_WINDOW
        && window.map_state == MapState::Viewable
        && window.class != WindowClass::InputOnly
        && !window.override_redirect
        && window.width > 0
        && window.height > 0
}

/// Whether `property` is one [`names`] reads.
fn is_name(state: &ServerState, property: AtomId) -> bool {
    ["_NET_WM_NAME", "WM_NAME", "WM_CLASS"]
        .iter()
        .any(|name| state.atoms.id_for(name) == Some(property))
}

/// A window's title and app id: `_NET_WM_NAME` (UTF-8) or else `WM_NAME`
/// (Latin-1), and `WM_CLASS`'s class ([`class`]). Missing ones are empty.
fn names(state: &ServerState, window: &Window) -> (String, String) {
    let property = |name: &str| {
        let atom = state.atoms.id_for(name)?;
        window
            .properties
            .get(&atom)
            .map(|value| value.data.as_slice())
    };
    let title = property("_NET_WM_NAME")
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .or_else(|| property("WM_NAME").map(latin1))
        .unwrap_or_default();
    let app_id = property("WM_CLASS").map(class).unwrap_or_default();
    (title, app_id)
}

/// `WM_CLASS`'s class: the second of the two strings it holds, each ended
/// by a NUL, or the only one where a program set just one.
fn class(bytes: &[u8]) -> String {
    let mut strings = bytes.split(|&byte| byte == 0);
    let instance = strings.next().unwrap_or_default();
    let class = strings.next().filter(|class| !class.is_empty());
    latin1(class.unwrap_or(instance))
}

/// ICCCM `STRING` text, which is Latin-1.
fn latin1(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| char::from(byte))
        .collect()
}

/// Copy an X image (B, G, R, alpha at 32 bits per pixel, `width` ×
/// `height`) to the top left of the compositor's buffer, which is
/// tiny-skia's premultiplied R, G, B, alpha, over opaque black. An `opaque`
/// image, below depth 32, has no alpha of its own.
fn blit(pixmap: &mut PixmapMut<'_>, pixels: &[u8], width: u16, height: u16, opaque: bool) {
    let buffer_width = pixmap.width() as usize;
    let buffer_height = pixmap.height() as usize;
    let data = pixmap.data_mut();
    for pixel in data.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[0, 0, 0, 0xff]);
    }
    let stride = usize::from(width) * 4;
    let columns = usize::from(width).min(buffer_width);
    let rows = usize::from(height).min(buffer_height);
    for row in 0..rows {
        let (Some(from), Some(to)) = (
            pixels.get(row * stride..row * stride + columns * 4),
            data.get_mut(row * buffer_width * 4..(row * buffer_width + columns) * 4),
        ) else {
            break;
        };
        for (from, to) in from.chunks_exact(4).zip(to.chunks_exact_mut(4)) {
            to[0] = from[2];
            to[1] = from[1];
            to[2] = from[0];
            to[3] = if opaque { 0xff } else { from[3] };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blit_swaps_red_and_blue_and_pads_with_black() {
        // A 2x1 X image: blue, then half-transparent red.
        let pixels = [0xff, 0, 0, 0, 0, 0, 0x80, 0x80];
        let mut bytes = vec![0x55; 3 * 2 * 4];
        let mut pixmap = PixmapMut::from_bytes(&mut bytes, 3, 2).expect("a 3x2 pixmap");
        blit(&mut pixmap, &pixels, 2, 1, true);
        assert_eq!(&bytes[0..4], &[0, 0, 0xff, 0xff], "blue, opaque");
        assert_eq!(&bytes[4..8], &[0x80, 0, 0, 0xff], "red, alpha ignored");
        assert_eq!(&bytes[8..12], &[0, 0, 0, 0xff], "past the image: black");
        assert_eq!(&bytes[12..16], &[0, 0, 0, 0xff], "the next row: black");

        let mut bytes = vec![0; 2 * 4];
        let mut pixmap = PixmapMut::from_bytes(&mut bytes, 2, 1).expect("a 2x1 pixmap");
        blit(&mut pixmap, &pixels, 2, 1, false);
        assert_eq!(
            &bytes[4..8],
            &[0x80, 0, 0, 0x80],
            "depth 32 keeps its alpha"
        );
    }

    #[test]
    fn blit_clips_an_image_larger_than_the_buffer() {
        let pixels = [0x10; 4 * 4 * 4];
        let mut bytes = vec![0; 2 * 2 * 4];
        let mut pixmap = PixmapMut::from_bytes(&mut bytes, 2, 2).expect("a 2x2 pixmap");
        blit(&mut pixmap, &pixels, 4, 4, true);
        assert!(
            bytes
                .chunks_exact(4)
                .all(|pixel| pixel == [0x10, 0x10, 0x10, 0xff])
        );
    }

    #[test]
    fn the_class_is_the_second_string_or_the_only_one() {
        assert_eq!(class(b"xmessage\0Xmessage\0"), "Xmessage");
        assert_eq!(class(b"Xev"), "Xev");
        assert_eq!(class(b"xev\0"), "xev");
        assert_eq!(class(b""), "");
    }

    #[test]
    fn latin1_stops_at_a_nul() {
        assert_eq!(latin1(b"xev\0Xev"), "xev");
        assert_eq!(latin1(&[0x63, 0x61, 0x66, 0xe9]), "caf\u{e9}");
    }
}
