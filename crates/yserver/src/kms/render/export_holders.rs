//! `export holders` diagnostic: per exported backing, the refcounts keeping it alive.

use std::fmt::Write as _;

use yserver_core::backend::export_holders::{CoreHolders, format_core_holders};

use crate::kms::vk::mem_accounting::MemCategory;

/// Store-side state of a backing; `None` on a row means the store entry is gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoreRow {
    pub drawable_id: u64,
    pub category: Option<MemCategory>,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub refcount: u32,
    pub pending_retire: bool,
    /// Whether the store's xid map still points at this entry.
    pub xid_attached: bool,
    /// Backend pictures holding a store ref on it.
    pub pictures: u32,
}

/// `ExportedBacking` bookkeeping for a backing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExportRow {
    pub glx_refs: u32,
    /// DRI3 `BuffersFromPixmap` export fd dup present.
    pub dri3_fd: bool,
    pub lifetime_held: bool,
    pub lifetime_via_alias: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HolderRow {
    pub host_xid: u32,
    pub store: Option<StoreRow>,
    pub alias_refcount: Option<u32>,
    pub export: Option<ExportRow>,
    /// The store's parallel sync-fd dup is installed.
    pub sync_dup: bool,
    /// Host window whose redirect backing this currently is.
    pub redirect_of: Option<u32>,
}

impl HolderRow {
    fn bytes(&self) -> u64 {
        self.store.as_ref().map_or(0, |s| s.bytes)
    }

    fn sort_key(&self) -> (u32, u64) {
        (
            self.host_xid,
            self.store.as_ref().map_or(0, |s| s.drawable_id),
        )
    }
}

/// Keeps the last reported rows; a report is due only when they change.
#[derive(Default)]
pub(crate) struct ExportHoldersReporter {
    last: Vec<HolderRow>,
}

impl ExportHoldersReporter {
    /// Sort `rows` and record them; `true` if they differ from the last call.
    pub(crate) fn observe(&mut self, mut rows: Vec<HolderRow>) -> bool {
        rows.sort_by_key(HolderRow::sort_key);
        if rows == self.last {
            return false;
        }
        self.last = rows;
        true
    }

    pub(crate) fn rows(&self) -> &[HolderRow] {
        &self.last
    }
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn yn(b: bool) -> char {
    if b { 'y' } else { 'n' }
}

/// Header plus one line per row; core holders join only rows that still own their xid.
pub(crate) fn format_report(rows: &[HolderRow], core: &CoreHolders) -> Vec<String> {
    let total: u64 = rows.iter().map(HolderRow::bytes).sum();
    let mut out = Vec::with_capacity(rows.len() + 1);
    out.push(format!(
        "export holders: {} backings, {:.1} MiB",
        rows.len(),
        mib(total)
    ));
    for r in rows {
        let mut s = format!("  0x{:x}", r.host_xid);
        let attached = match &r.store {
            Some(st) => {
                let cat = st.category.map_or("untracked", MemCategory::label);
                let _ = write!(
                    s,
                    " id={} {cat} {}x{} {}KiB store_rc={} pending_retire={} xid={} pics={}",
                    st.drawable_id,
                    st.width,
                    st.height,
                    st.bytes.div_ceil(1024),
                    st.refcount,
                    yn(st.pending_retire),
                    if st.xid_attached {
                        "attached"
                    } else {
                        "detached"
                    },
                    st.pictures,
                );
                st.xid_attached
            }
            None => {
                s.push_str(" store=gone");
                false
            }
        };
        match r.alias_refcount {
            Some(n) => {
                let _ = write!(s, " alias_rc={n}");
            }
            None => s.push_str(" alias_rc=-"),
        }
        match r.export {
            Some(e) => {
                let lifetime = match (e.lifetime_held, e.lifetime_via_alias) {
                    (false, _) => "none",
                    (true, true) => "alias",
                    (true, false) => "store",
                };
                let _ = write!(
                    s,
                    " export=[glx_refs={} dri3_fd={} lifetime={lifetime}]",
                    e.glx_refs,
                    yn(e.dri3_fd),
                );
            }
            None => s.push_str(" export=-"),
        }
        let _ = write!(s, " sync_dup={}", yn(r.sync_dup));
        match r.redirect_of {
            Some(w) => {
                let _ = write!(s, " redirect_of=0x{w:x}");
            }
            None => s.push_str(" redirect_of=-"),
        }
        if attached {
            let _ = write!(
                s,
                " core=[{}]",
                format_core_holders(core.get(&r.host_xid).map(Vec::as_slice))
            );
        } else {
            s.push_str(" core=n/a");
        }
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use yserver_core::backend::export_holders::{CoreHolder, CoreHolderKind};

    use super::*;

    fn row(host_xid: u32, glx_refs: u32) -> HolderRow {
        HolderRow {
            host_xid,
            store: Some(StoreRow {
                drawable_id: 9,
                category: Some(MemCategory::RedirectExport),
                width: 640,
                height: 480,
                bytes: 3 << 20,
                refcount: 2,
                pending_retire: false,
                xid_attached: true,
                pictures: 1,
            }),
            alias_refcount: Some(2),
            export: Some(ExportRow {
                glx_refs,
                dri3_fd: true,
                lifetime_held: true,
                lifetime_via_alias: true,
            }),
            sync_dup: true,
            redirect_of: Some(0x0020_0005),
        }
    }

    #[test]
    fn observe_reports_only_changes() {
        let mut r = ExportHoldersReporter::default();
        assert!(!r.observe(Vec::new()));
        assert!(r.observe(vec![row(0x20, 1), row(0x10, 1)]));
        assert_eq!(r.rows()[0].host_xid, 0x10);
        assert!(!r.observe(vec![row(0x10, 1), row(0x20, 1)]));
        assert!(r.observe(vec![row(0x10, 1), row(0x20, 0)]));
        assert!(r.observe(Vec::new()));
        assert!(!r.observe(Vec::new()));
    }

    #[test]
    fn format_joins_core_holders_on_attached_rows() {
        let mut core = CoreHolders::new();
        core.insert(
            0x10,
            vec![CoreHolder {
                client: 7,
                xid: 0x0070_0002,
                kind: CoreHolderKind::Named {
                    window: 0x0060_0001,
                },
            }],
        );
        let mut gone = row(0x20, 1);
        gone.store = None;
        gone.alias_refcount = None;
        gone.redirect_of = None;
        let lines = format_report(&[row(0x10, 1), gone], &core);
        assert_eq!(lines[0], "export holders: 2 backings, 3.0 MiB");
        assert_eq!(
            lines[1],
            "  0x10 id=9 redirect_export 640x480 3072KiB store_rc=2 pending_retire=n \
             xid=attached pics=1 alias_rc=2 export=[glx_refs=1 dri3_fd=y lifetime=alias] \
             sync_dup=y redirect_of=0x200005 core=[c7:named 0x700002(win 0x600001)]"
        );
        assert_eq!(
            lines[2],
            "  0x20 store=gone alias_rc=- export=[glx_refs=1 dri3_fd=y lifetime=alias] \
             sync_dup=y redirect_of=- core=n/a"
        );
    }

    #[test]
    fn empty_report_is_header_only() {
        assert_eq!(
            format_report(&[], &CoreHolders::new()),
            vec!["export holders: 0 backings, 0.0 MiB".to_owned()]
        );
    }
}
