//! The rootless Wayland backend's connection to its compositor.
//!
//! With `YSERVER_BACKEND=wayland` the server is a client of the Wayland
//! compositor `WAYLAND_DISPLAY` names, rather than the owner of a DRM card:
//! the renderer runs headless (lavapipe, or a render node), the root window
//! takes the size of the compositor's first screen, and -- in the slices
//! that follow this one -- each top-level X window becomes a window of the
//! compositor's. The design is Ferrix's `docs/YSERVER.md`.
//!
//! The Wayland side is Ferrix's own client runtime, `compositor-toolkit`,
//! driven from the server's core loop: its socket is polled there
//! ([`crate::kms::render::KmsBackend`] registers it as
//! `BackendFdKind::Wayland`), and each readiness is one non-blocking
//! [`Client::dispatch`].

use std::io;
use std::time::Duration;

use compositor_toolkit::{Client, Event};

/// The connection, and what the compositor has said about its screens.
pub struct WaylandLink {
    client: Client,
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
        Ok(Self { client })
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

    /// Read what the compositor sent, without blocking, and hand back the
    /// events it made.
    ///
    /// # Errors
    ///
    /// The compositor closed the connection or refused a request.
    pub fn dispatch(&mut self) -> io::Result<Vec<Event>> {
        self.client
            .dispatch(Some(Duration::ZERO))
            .map_err(|error| io::Error::new(io::ErrorKind::ConnectionAborted, format!("{error}")))
    }

    /// Send what is queued.
    pub fn flush(&mut self) {
        if let Err(error) = self.client.flush() {
            log::warn!("wayland: flush: {error}");
        }
    }
}
