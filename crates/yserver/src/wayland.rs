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
//! The seat -- keys, pointer, wheel, focus and cursor -- is [`input`]'s,
//! and the clipboard, X's selections both ways, [`clipboard`]'s.
//!
//! Not yet: override-redirect windows (menus, as popups), and the
//! compositor's own size and close for a window (docs/YSERVER.md, Y5).
//! Until then an X window keeps the size its client gave it, drawn at the
//! top left of the window the compositor tiles, on black.
//!
//! # Layout
//!
//! The Wayland side lives here and knows the X server only through
//! `&ServerState` and [`WindowImages`]; the renderer side lives in
//! `KmsBackend` (`kms/render/backend.rs`), each piece behind
//! `feature = "wayland"`:
//!
//! * `KmsBackend::attach_wayland` takes the [`WaylandLink`] at startup, and
//!   the compositor's keyboard layout for the server's keymap;
//! * `Backend::on_wayland_ready` calls [`WaylandLink::dispatch`], which
//!   hands each [`Event`] to `WaylandLink::handle`, where the seat's events
//!   are kept, then [`WaylandLink::deliver_input`], which hands them to X
//!   ([`input`]);
//! * `Backend::poll_deferred_input` calls `KmsBackend::sync_wayland` once
//!   per loop iteration: the server's own redirect of the root's children
//!   (`yserver_core`'s `redirect_subwindows_for_server`), then
//!   [`WaylandLink::sync`], [`WaylandLink::sync_cursor`] and
//!   [`WaylandLink::sync_clipboard`];
//! * `Backend::on_window_property_changed` calls
//!   [`WaylandLink::property_changed`];
//! * `impl WindowImages for KmsBackend` reads a window back, and
//!   `impl CursorSource for KmsBackend` gives the cursor in effect.
//!
//! Two core pieces serve the seat: `ServerState::pointer_scope`, which keeps
//! the pointer's hit test to the top-level the compositor has it on, and
//! `set_input_focus_for_server` in `process_request.rs`, the server's own
//! SetInputFocus for the keyboard's enter and leave.
//!
//! The clipboard's core pieces are `ServerState::server_selection_events`,
//! where what the selection protocol sends the server's own window is
//! queued, and the server's own selection requests in `process_request.rs`
//! (`selection_window_for_server` and its siblings).
//!
//! A top-level is keyed by its X window's host XID ([`WaylandLink`]'s
//! `toplevels`), and `WaylandLink::by_surface` finds one from the
//! compositor's side.

use std::{
    collections::{HashMap, HashSet},
    io,
    time::Duration,
};

use compositor_toolkit::{Client, Event, SurfaceId, ToplevelOptions};
use yserver_core::{
    resources::{COMPOSITE_OVERLAY_WINDOW, MapState, ROOT_WINDOW, Window, WindowClass},
    server::ServerState,
};
use yserver_protocol::x11::{AtomId, ResourceId};

/// What the compositor asked of an X window, which the server carries out
/// with its state in hand (`KmsBackend::sync_wayland`, through
/// `yserver_core`'s `configure_window_for_server` and
/// `close_window_for_server`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// `xdg_toplevel.configure` with a size the window does not have.
    Resize {
        /// The X window.
        window: ResourceId,
        /// Its new width.
        width: u16,
        /// Its new height.
        height: u16,
    },
    /// `xdg_toplevel.close`.
    Close {
        /// The X window.
        window: ResourceId,
    },
}

mod clipboard;
mod input;
mod popups;
pub use input::CursorSource;

/// What the link needs of the renderer: each top-level's image, by the
/// window's host XID.
pub trait WindowImages {
    /// A number that changes whenever anything is drawn into the window's
    /// image; `None` for a window the renderer does not know.
    fn image_version(&self, host_xid: u32) -> Option<u64>;

    /// The window's image, its `width` × `height` content with `border`
    /// pixels of its border around it (0 for none), each four bytes in
    /// memory order B, G, R and alpha (unused below depth 32) -- an X
    /// `ZPixmap` at 32 bits per pixel -- read into `out`, which holds at
    /// least that many bytes; whether it was read.
    fn read_image_into(
        &mut self,
        host_xid: u32,
        width: u16,
        height: u16,
        border: u16,
        out: &mut [u8],
    ) -> bool;
}

/// Where a window of the compositor's is in drawing it: the frame callback
/// it waits for and the image it last showed.
#[derive(Debug, Default)]
struct Frames {
    /// A frame callback was asked for and has not come.
    pending: bool,
    /// The [`WindowImages::image_version`] last drawn.
    shown: Option<u64>,
    /// The compositor configured it since it was last drawn.
    configured: bool,
    /// A hash of each row of the image last handed over, which the next
    /// is compared with to say which rows changed.
    rows: Vec<u64>,
}

/// A top-level X window as the compositor's window.
#[derive(Debug)]
struct Toplevel {
    /// The X window.
    window: ResourceId,
    /// The compositor's window.
    surface: SurfaceId,
    /// What it was last given.
    title: String,
    /// What it was last given.
    app_id: String,
    /// Its frames.
    frames: Frames,
    /// The size the compositor's last configure gave, not yet asked of the
    /// X window.
    resize: Option<(u32, u32)>,
    /// The compositor asked to close it, not yet passed on.
    close: bool,
    /// The host XID of the window it was last made a dialog of
    /// (`WM_TRANSIENT_FOR`).
    parent: Option<u32>,
    /// The least and greatest size it was last given (`WM_NORMAL_HINTS`).
    limits: SizeLimits,
}

/// The connection, what the compositor has said about its screens, and the
/// windows made on it.
pub struct WaylandLink {
    client: Client,
    /// By the X window's host XID.
    toplevels: HashMap<u32, Toplevel>,
    /// Host XIDs whose title or class changed since the last sync.
    renamed: HashSet<u32>,
    /// The keyboard and the pointer ([`input`]).
    seat: input::Seat,
    /// What the compositor asked, for the server to carry out.
    requests: Vec<Request>,
    /// Override-redirect windows as the compositor's popups, in the order
    /// they were made, which a child popup is after its parent in
    /// ([`popups`]).
    popups: Vec<popups::Popup>,
    /// Host XIDs of popups the compositor took away (`popup_done`), not
    /// made again until their X window unmaps.
    dismissed: HashSet<u32>,
    /// X's selections and the compositor's clipboard, one in both
    /// directions ([`clipboard`]); `None` when the compositor has no
    /// `ext_data_control_v1`.
    clipboard: Option<clipboard::Clipboard>,
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
        let mut client = Client::connect().map_err(|error| {
            io::Error::new(io::ErrorKind::NotConnected, format!("wayland: {error}"))
        })?;
        let clipboard = clipboard::Clipboard::bind(&mut client);
        if clipboard.is_none() {
            log::warn!("wayland: the compositor has no ext_data_control_v1; no clipboard");
        }
        Ok(Self {
            clipboard,
            client,
            toplevels: HashMap::new(),
            renamed: HashSet::new(),
            seat: input::Seat::default(),
            requests: Vec::new(),
            popups: Vec::new(),
            dismissed: HashSet::new(),
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
        let Some(event) = self.seat.take(event) else {
            return;
        };
        match event {
            Event::Frame { surface, .. } => {
                if let Some(frames) = self.frames_of(surface) {
                    frames.pending = false;
                }
            }
            Event::Configure {
                surface,
                width,
                height,
            } => {
                if let Some(toplevel) = self.by_surface(surface) {
                    toplevel.resize = Some((width, height));
                }
                if let Some(frames) = self.frames_of(surface) {
                    frames.configured = true;
                }
            }
            Event::Closed(surface) => self.popup_dismissed(surface),
            Event::CloseRequested(surface) => {
                if let Some(toplevel) = self.by_surface(surface) {
                    toplevel.close = true;
                }
            }
            Event::Object {
                object,
                opcode,
                ref args,
                ..
            } if self.clipboard.as_mut().is_some_and(|clipboard| {
                clipboard.event(&mut self.client, object, opcode, args)
            }) => {}
            other => log::debug!("wayland: {other:?}"),
        }
    }

    /// Follow X's selections and the compositor's on each other, and move
    /// what pastes are under way ([`clipboard`]). Once per loop iteration.
    pub fn sync_clipboard(
        &mut self,
        state: &mut ServerState,
        backend: &mut dyn yserver_core::backend::Backend,
    ) {
        if let Some(clipboard) = self.clipboard.as_mut() {
            clipboard.sync(&mut self.client, state, backend);
        }
        self.flush();
    }

    /// Whether a paste is under way, for which the loop must come round
    /// again soon.
    #[must_use]
    pub fn clipboard_busy(&self) -> bool {
        self.clipboard
            .as_ref()
            .is_some_and(clipboard::Clipboard::busy)
    }

    fn by_surface(&mut self, surface: SurfaceId) -> Option<&mut Toplevel> {
        self.toplevels
            .values_mut()
            .find(|toplevel| toplevel.surface == surface)
    }

    /// The frames of the window or popup `surface` is.
    fn frames_of(&mut self, surface: SurfaceId) -> Option<&mut Frames> {
        let Self {
            toplevels, popups, ..
        } = self;
        toplevels
            .values_mut()
            .find(|toplevel| toplevel.surface == surface)
            .map(|toplevel| &mut toplevel.frames)
            .or_else(|| {
                popups
                    .iter_mut()
                    .find(|popup| popup.surface == surface)
                    .map(|popup| &mut popup.frames)
            })
    }

    /// The host XID of the X window the compositor's window or popup
    /// `surface` shows.
    pub(crate) fn host_xid_of(&self, surface: SurfaceId) -> Option<u32> {
        self.toplevels
            .iter()
            .find(|(_, toplevel)| toplevel.surface == surface)
            .map(|(&host_xid, _)| host_xid)
            .or_else(|| {
                self.popups
                    .iter()
                    .find(|popup| popup.surface == surface)
                    .map(|popup| popup.host_xid)
            })
    }

    /// A window's title or class may have changed: `property` was set or
    /// deleted on the window with host XID `host_xid`.
    pub fn property_changed(&mut self, state: &ServerState, host_xid: u32, property: AtomId) {
        if is_name(state, property) {
            let _ = self.renamed.insert(host_xid);
        }
    }

    /// What the compositor asked since the last call, for the server to
    /// carry out.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// Bring the compositor's windows in line with the X server's
    /// top-levels, and hand over each image that changed and whose last
    /// frame the compositor has shown. Once per loop iteration.
    pub fn sync(&mut self, state: &ServerState, images: &mut dyn WindowImages) {
        let mut wanted: Vec<(u32, &Window)> = state
            .resources
            .children(ROOT_WINDOW)
            .iter()
            .filter_map(|&child| {
                let window = state.resources.window(child)?;
                let host_xid = window.host_xid?.as_raw();
                is_toplevel(child, window).then_some((host_xid, window))
            })
            .collect();
        // Dialogs after the windows they belong to, so that a dialog is
        // made with its parent in hand: the compositor decides at the first
        // commit whether a window floats.
        wanted.sort_by_key(|(_, window)| transient_for(state, window).is_some());

        // Popups go before the windows they hang from, and a child popup
        // before its parent.
        let kept: HashSet<u32> = wanted.iter().map(|&(host_xid, _)| host_xid).collect();
        self.retire_popups(state, &kept);

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
            // The window it is a dialog of, when that is one of the
            // compositor's windows already.
            let parent = transient_for(state, window)
                .filter(|parent| *parent != host_xid && self.toplevels.contains_key(parent));
            let parent_surface = parent
                .and_then(|parent| self.toplevels.get(&parent))
                .map(|parent| parent.surface);
            let limits = size_limits_of(state, window);
            let toplevel = match self.toplevels.entry(host_xid) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let (title, app_id) = names(state, window);
                    // Sent before the first commit, where the compositor
                    // decides whether a window of a fixed size floats.
                    let options = ToplevelOptions {
                        title: title.clone(),
                        app_id: app_id.clone(),
                        size: (u32::from(window.width), u32::from(window.height)),
                        parent: parent_surface,
                        min_size: limits.min,
                        max_size: limits.max,
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
                                window: window.id,
                                surface,
                                title,
                                app_id,
                                frames: Frames::default(),
                                resize: None,
                                close: false,
                                parent,
                                limits,
                            })
                        }
                        Err(error) => {
                            log::warn!("wayland: no window for 0x{host_xid:x}: {error}");
                            continue;
                        }
                    }
                }
            };

            // A window made a dialog, or no longer one, after it mapped.
            if parent != toplevel.parent {
                toplevel.parent = parent;
                self.client.set_parent(toplevel.surface, parent_surface);
            }

            // Size hints set or changed after it mapped.
            if limits != toplevel.limits {
                toplevel.limits = limits;
                self.client
                    .set_size_limits(toplevel.surface, limits.min, limits.max);
            }

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

            // What the compositor asked, carried out by the server after
            // this.
            if let Some((width, height)) = toplevel.resize.take()
                && (width, height) != (u32::from(window.width), u32::from(window.height))
            {
                self.requests.push(Request::Resize {
                    window: window.id,
                    width: u16::try_from(width).unwrap_or(u16::MAX),
                    height: u16::try_from(height).unwrap_or(u16::MAX),
                });
            }
            if std::mem::take(&mut toplevel.close) {
                log::info!("wayland: the compositor closes window 0x{host_xid:x}");
                self.requests.push(Request::Close { window: window.id });
            }

            show(
                &mut self.client,
                images,
                toplevel.surface,
                (host_xid, window),
                &mut toplevel.frames,
                false,
            );
        }
        self.show_popups(state, images);
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

/// Hand the compositor `window`'s image when it changed and its last frame
/// has been shown, at the window's own size: until the X window has taken a
/// size the compositor asked for, it shows at the one it has.
///
/// A window with `bordered` is shown with its X border around it: an
/// override-redirect window, whose border X shows as its own since no
/// window manager draws one. A top-level's border is not shown, as a window
/// manager's frame would take its place.
///
/// The image is read straight into the compositor's buffer -- X's 32-bit
/// `ZPixmap` is `wl_shm`'s layout, `XRGB8888` below depth 32 and
/// premultiplied `ARGB8888` at it -- and the compositor is told only the
/// band of rows that differ from the image before, found by a hash of each
/// row ([`Frames::rows`]): what X drew is not needed for that, so no
/// drawing path can be missed.
fn show(
    client: &mut Client,
    images: &mut dyn WindowImages,
    surface: SurfaceId,
    (host_xid, window): (u32, &Window),
    frames: &mut Frames,
    bordered: bool,
) {
    if frames.pending || client.size(surface).is_none() {
        return;
    }
    let version = images.image_version(host_xid);
    if version.is_some() && version == frames.shown && !frames.configured {
        return;
    }
    let border = if bordered { window.border_width } else { 0 };
    let (width, height) = (
        window.width.saturating_add(border.saturating_mul(2)),
        window.height.saturating_add(border.saturating_mul(2)),
    );
    let opaque = window.depth != 32;
    let size = (u32::from(width), u32::from(height));
    // Whether the compositor has this size's image already: a configure
    // since, or a first frame, or another size, is all of it changed.
    let whole =
        frames.configured || frames.shown.is_none() || frames.rows.len() != usize::from(height);
    let rows = &mut frames.rows;
    match client.draw_pixels(surface, size, opaque, |out| {
        if !images.read_image_into(host_xid, window.width, window.height, border, out) {
            return None;
        }
        let stride = usize::from(width) * 4;
        let hashes = out
            .chunks_exact(stride)
            .take(usize::from(height))
            .map(row_hash);
        let mut band: Option<(u32, u32)> = None;
        let mut damage = Vec::new();
        if whole {
            rows.clear();
            rows.extend(hashes);
        } else {
            for (y, hash) in hashes.enumerate() {
                if rows[y] != hash {
                    rows[y] = hash;
                    let y = y as u32;
                    band = Some(band.map_or((y, y), |(first, _)| (first, y)));
                }
            }
            // Nothing changed that shows: a version moved by a draw that
            // drew the same pixels. One row keeps the commit meaningful.
            let (first, last) = band.unwrap_or((0, 0));
            damage.push((0, first, size.0, last - first + 1));
        }
        Some(damage)
    }) {
        Ok(true) => {
            client.request_frame(surface);
            frames.pending = true;
            frames.shown = version;
            frames.configured = false;
        }
        Ok(false) => {}
        Err(error) => log::warn!("wayland: drawing 0x{host_xid:x}: {error}"),
    }
}

/// A row's hash, for [`show`]'s comparison with the image before: FxHash's
/// step over its 8-byte words, which is as fast as reading them.
fn row_hash(row: &[u8]) -> u64 {
    let mut hash: u64 = 0;
    let mut words = row.chunks_exact(8);
    for word in &mut words {
        let word = u64::from_le_bytes(word.try_into().unwrap_or([0; 8]));
        hash = (hash.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    for &byte in words.remainder() {
        hash = (hash.rotate_left(5) ^ u64::from(byte)).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    hash
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

/// The host XID of the window `window` is a dialog of: `WM_TRANSIENT_FOR`
/// (ICCCM 4.1.2.6), when it names a window that has one.
fn transient_for(state: &ServerState, window: &Window) -> Option<u32> {
    let atom = state.atoms.id_for("WM_TRANSIENT_FOR")?;
    let data = window.properties.get(&atom)?.data.get(0..4)?;
    let id = u32::from_le_bytes(data.try_into().ok()?);
    let parent = state.resources.window(ResourceId(id))?;
    (parent.id != window.id)
        .then_some(parent.host_xid)
        .flatten()
        .map(|host| host.as_raw())
}

/// The least and greatest size a window may be given, in pixels; 0 is no
/// limit, as `xdg_toplevel.set_min_size` and `set_max_size` take it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SizeLimits {
    min: (u32, u32),
    max: (u32, u32),
}

/// `window`'s [`SizeLimits`] from `WM_NORMAL_HINTS`.
fn size_limits_of(state: &ServerState, window: &Window) -> SizeLimits {
    state
        .atoms
        .id_for("WM_NORMAL_HINTS")
        .and_then(|atom| window.properties.get(&atom))
        .map(|value| size_limits(&value.data))
        .unwrap_or_default()
}

/// The least and greatest size in a `WM_SIZE_HINTS` value (ICCCM 4.1.2.3):
/// 32-bit words, `flags` first, `min_width` and `min_height` the sixth and
/// seventh, `max_width` and `max_height` the eighth and ninth, each pair
/// meant only when its flag (`PMinSize`, `PMaxSize`) is set. A window whose
/// least size is its greatest is one of a fixed size, which the compositor
/// floats, as Hyprland does an X window of one.
fn size_limits(data: &[u8]) -> SizeLimits {
    const P_MIN_SIZE: u32 = 1 << 4;
    const P_MAX_SIZE: u32 = 1 << 5;
    let word = |index: usize| {
        data.get(index * 4..index * 4 + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
    };
    // A size is a positive `INT32`; anything else is no limit.
    let size = |index: usize| {
        word(index)
            .filter(|&value| i32::try_from(value).is_ok())
            .unwrap_or(0)
    };
    let pair = |flag: u32, first: usize| match word(0) {
        Some(flags) if flags & flag != 0 && word(first + 1).is_some() => {
            (size(first), size(first + 1))
        }
        _ => (0, 0),
    };
    SizeLimits {
        min: pair(P_MIN_SIZE, 5),
        max: pair(P_MAX_SIZE, 7),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_class_is_the_second_string_or_the_only_one() {
        assert_eq!(class(b"xmessage\0Xmessage\0"), "Xmessage");
        assert_eq!(class(b"Xev"), "Xev");
        assert_eq!(class(b"xev\0"), "xev");
        assert_eq!(class(b""), "");
    }

    /// A `WM_SIZE_HINTS` value of 18 words with `flags`, the least size and
    /// the greatest.
    fn hints(flags: u32, min: (u32, u32), max: (u32, u32)) -> Vec<u8> {
        let mut words = [0_u32; 18];
        words[0] = flags;
        (words[5], words[6], words[7], words[8]) = (min.0, min.1, max.0, max.1);
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[test]
    fn size_hints_give_limits_only_where_their_flags_say() {
        let fixed = size_limits(&hints(0x30, (640, 480), (640, 480)));
        assert_eq!(fixed.min, (640, 480));
        assert_eq!(fixed.max, (640, 480));
        let least = size_limits(&hints(0x10, (200, 100), (900, 900)));
        assert_eq!(
            (least.min, least.max),
            ((200, 100), (0, 0)),
            "PMaxSize unset"
        );
        assert_eq!(
            size_limits(&hints(0x0c, (1, 1), (2, 2))),
            SizeLimits::default(),
            "USSize and PPosition alone"
        );
        let negative = size_limits(&hints(0x30, (u32::MAX, 10), (640, 480)));
        assert_eq!(negative.min, (0, 10), "a negative size is no limit");
        assert_eq!(
            size_limits(&[0x30, 0, 0, 0]),
            SizeLimits::default(),
            "cut short"
        );
        assert_eq!(size_limits(&[]), SizeLimits::default());
    }

    #[test]
    fn latin1_stops_at_a_nul() {
        assert_eq!(latin1(b"xev\0Xev"), "xev");
        assert_eq!(latin1(&[0x63, 0x61, 0x66, 0xe9]), "caf\u{e9}");
    }
}
