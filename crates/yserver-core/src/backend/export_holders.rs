//! Core half of the `export holders` diagnostic: client resources per host pixmap xid.

use std::{collections::HashMap, fmt::Write as _};

use crate::server::{GlxDrawableKind, ServerState};

/// How a core resource refers to a host pixmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoreHolderKind {
    /// A plain pixmap resource whose host handle is the backing.
    Pixmap,
    /// A `NameWindowPixmap` alias of client window `window`.
    Named { window: u32 },
    /// A `NameWindowPixmap` record on `window` whose pixmap resource is gone.
    NamedStale { window: u32 },
    /// A GLXPixmap holding an export ref on the backing.
    Glx { x_drawable: u32 },
    /// Client window `window`'s current redirect backing.
    Redirect { window: u32 },
}

/// One core resource pointing at a host pixmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CoreHolder {
    pub client: u32,
    pub xid: u32,
    pub kind: CoreHolderKind,
}

/// Core holders by host pixmap xid, each list sorted.
pub type CoreHolders = HashMap<u32, Vec<CoreHolder>>;

/// Walk windows, pixmaps and GLX drawables for every host-pixmap reference.
#[must_use]
pub fn collect_core_holders(state: &ServerState) -> CoreHolders {
    let mut out: CoreHolders = HashMap::new();
    let mut named: HashMap<u32, u32> = HashMap::new();
    for w in state.resources.windows_iter() {
        for alias in &w.composite_named_pixmaps {
            named.insert(alias.client_pixmap.0, w.id.0);
            if state.resources.pixmap(alias.client_pixmap).is_none() {
                out.entry(alias.host_pixmap.as_raw())
                    .or_default()
                    .push(CoreHolder {
                        client: w.owner.0,
                        xid: alias.client_pixmap.0,
                        kind: CoreHolderKind::NamedStale { window: w.id.0 },
                    });
            }
        }
        if let Some(b) = w.redirected_backing.as_ref() {
            out.entry(b.host_pixmap.as_raw())
                .or_default()
                .push(CoreHolder {
                    client: w.owner.0,
                    xid: w.id.0,
                    kind: CoreHolderKind::Redirect { window: w.id.0 },
                });
        }
    }
    for p in state.resources.pixmaps_iter() {
        let Some(host) = p.host_xid else { continue };
        let kind = named
            .get(&p.id.0)
            .map_or(CoreHolderKind::Pixmap, |&window| CoreHolderKind::Named {
                window,
            });
        out.entry(host.as_raw()).or_default().push(CoreHolder {
            client: p.owner.0,
            xid: p.id.0,
            kind,
        });
    }
    for (&glx_xid, d) in &state.glx_drawables {
        if d.kind != GlxDrawableKind::Pixmap {
            continue;
        }
        let Some(host) = d.glx_export_host_xid else {
            continue;
        };
        out.entry(host).or_default().push(CoreHolder {
            client: d.owner.0,
            xid: glx_xid,
            kind: CoreHolderKind::Glx {
                x_drawable: d.x_drawable,
            },
        });
    }
    for v in out.values_mut() {
        v.sort_unstable();
    }
    out
}

/// The `core=[..]` field of one holders line; `none` when nothing points at it.
#[must_use]
pub fn format_core_holders(holders: Option<&[CoreHolder]>) -> String {
    let Some(holders) = holders.filter(|h| !h.is_empty()) else {
        return "none".to_owned();
    };
    let mut s = String::new();
    for (i, h) in holders.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "c{}:", h.client);
        let _ = match h.kind {
            CoreHolderKind::Pixmap => write!(s, "pixmap 0x{:x}", h.xid),
            CoreHolderKind::Named { window } => {
                write!(s, "named 0x{:x}(win 0x{window:x})", h.xid)
            }
            CoreHolderKind::NamedStale { window } => {
                write!(s, "named-stale 0x{:x}(win 0x{window:x})", h.xid)
            }
            CoreHolderKind::Glx { x_drawable } => {
                write!(s, "glxpixmap 0x{:x}(of 0x{x_drawable:x})", h.xid)
            }
            CoreHolderKind::Redirect { window } => write!(s, "redirect-of win 0x{window:x}"),
        };
    }
    s
}

#[cfg(test)]
mod tests {
    use yserver_protocol::x11::{ClientId, CreatePixmapRequest, CreateWindowRequest, ResourceId};

    use super::*;
    use crate::{
        backend::PixmapHandle,
        resources::{NamedCompositePixmap, ROOT_WINDOW, RedirectedBacking},
        server::GlxDrawable,
    };

    const BACKING: u32 = 0x0050_0102;

    fn state_with_named_window() -> ServerState {
        let mut state = ServerState::new();
        state.resources.create_window(
            ClientId(3),
            CreateWindowRequest {
                depth: 24,
                window: ResourceId(0x0060_0001),
                parent: ROOT_WINDOW,
                width: 64,
                height: 32,
                class: 1,
                visual: crate::resources::ROOT_VISUAL,
                ..Default::default()
            },
        );
        let handle = PixmapHandle::from_raw_for_test(BACKING);
        state.resources.create_pixmap(
            ClientId(7),
            CreatePixmapRequest {
                pixmap: ResourceId(0x0070_0002),
                drawable: ResourceId(0x0060_0001),
                width: 64,
                height: 32,
                depth: 24,
            },
        );
        assert!(
            state
                .resources
                .set_pixmap_host_xid(ResourceId(0x0070_0002), handle)
        );
        let w = state.resources.window_mut(ResourceId(0x0060_0001)).unwrap();
        w.redirected_backing = Some(RedirectedBacking {
            host_pixmap: handle,
            width: 64,
            height: 32,
            depth: 24,
        });
        w.composite_named_pixmaps.push(NamedCompositePixmap {
            client_pixmap: ResourceId(0x0070_0002),
            host_pixmap: handle,
            width: 64,
            height: 32,
        });
        state.glx_drawables.insert(
            0x0070_0003,
            GlxDrawable {
                owner: ClientId(7),
                kind: GlxDrawableKind::Pixmap,
                x_drawable: 0x0070_0002,
                fbconfig: 0x21,
                width: 0,
                height: 0,
                event_mask: 0,
                glx_export_host_xid: Some(BACKING),
                texture_target: yserver_protocol::x11::glx::GLX_TEXTURE_2D_EXT,
            },
        );
        state
    }

    #[test]
    fn collects_named_redirect_and_glx_holders() {
        let state = state_with_named_window();
        let holders = collect_core_holders(&state);
        assert_eq!(
            format_core_holders(holders.get(&BACKING).map(Vec::as_slice)),
            "c3:redirect-of win 0x600001 c7:named 0x700002(win 0x600001) \
             c7:glxpixmap 0x700003(of 0x700002)"
        );
    }

    #[test]
    fn freed_named_pixmap_reports_stale_record() {
        let mut state = state_with_named_window();
        state.resources.free_pixmap(ResourceId(0x0070_0002));
        state.glx_drawables.clear();
        let holders = collect_core_holders(&state);
        assert_eq!(
            format_core_holders(holders.get(&BACKING).map(Vec::as_slice)),
            "c3:redirect-of win 0x600001 c3:named-stale 0x700002(win 0x600001)"
        );
    }

    #[test]
    fn nothing_pointing_formats_none() {
        assert_eq!(format_core_holders(None), "none");
        assert_eq!(format_core_holders(Some(&[])), "none");
    }
}
