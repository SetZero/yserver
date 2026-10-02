//! Per-process GPU engine time and memory, read from DRM fdinfo.
//!
//! Motivation: Vulkan has no GPU-utilisation API. It offers
//! timestamp queries — which we already use for `gpu_render_ns`
//! (`kms::render::telemetry`) — but those measure one pass of our
//! own submitted work, not what share of the GPU this process is
//! taking. So when a contributor reports "GPU load is 60-70%", we
//! have had no way to say whether that load is ours or the client's.
//!
//! The kernel's DRM fdinfo interface answers exactly that. Each open
//! DRM file description exposes cumulative per-engine busy time under
//! `/proc/self/fdinfo/<fd>`:
//!
//! ```text
//! drm-driver:        amdgpu
//! drm-client-id:     12
//! drm-engine-gfx:    1234567890 ns
//! drm-engine-compute:          0 ns
//! drm-memory-vram:       524288 KiB
//! ```
//!
//! Both amdgpu and i915 implement it; it is the same source
//! `nvtop` and `intel_gpu_top` use to attribute load per client. A
//! delta over a known interval gives a busy fraction directly.
//!
//! ★ Why every fd and not just ours: the KMS device we open is used
//! for modesetting and page flips, but the *rendering* is submitted
//! through the DRM fd **Mesa** opens for the Vulkan device, which we
//! never see. Reading only our own fd would report a GPU that is
//! almost idle while the driver is busy on our behalf. Both fds
//! belong to this process, so the honest figure is the sum over
//! every DRM fd the process holds.
//!
//! ★ Why dedupe on `drm-client-id`: fdinfo is per *file
//! description*, so a `dup()`'d fd exposes the same counters twice.
//! Summing naively would double-count. The path alone cannot
//! distinguish a dup from a second independent open — the client id
//! can.
//!
//! ★ Why results are grouped by `drm-pdev`: silence is dual-GPU
//! (i915 at `0000:00:02.0`, RX 6800 at `0000:03:00.0`), and so are
//! the hybrid laptops. Summing engine time across two physical GPUs
//! produces a number that describes neither. Each device is reported
//! on its own line; a single-GPU box simply gets one line.
//!
//! Everything here is read-only `/proc` access. No ioctls, no
//! privileges, no vendor tooling, and it degrades to `None` on a
//! kernel or driver that does not implement the interface.

use std::collections::{BTreeMap, HashSet};

/// Cumulative counters for one physical GPU.
///
/// `engine_ns` keys are driver-defined (`gfx`/`compute` on amdgpu,
/// `render`/`copy`/`video`/`video-enhance` on i915), kept verbatim
/// rather than normalised — a made-up common vocabulary would hide
/// which engine actually ran. amdgpu emits no `drm-engine-*` at all
/// until work has been submitted, so an empty map means idle-or-
/// unused, not unsupported.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DeviceLoad {
    /// `drm-driver`, e.g. `amdgpu` or `i915`.
    pub driver: String,
    /// Engine name (without the `drm-engine-` prefix) → nanoseconds.
    pub engine_ns: BTreeMap<String, u64>,
    /// `drm-memory-vram` summed over this device's clients, bytes.
    /// `None` when the driver does not emit the key — i915 uses
    /// `drm-total-<region>` instead, and reporting 0 there would
    /// read as "no memory in use".
    pub vram_bytes: Option<u64>,
    /// `drm-memory-gtt` summed, bytes.
    pub gtt_bytes: Option<u64>,
    /// Distinct DRM clients this process holds on this device.
    pub clients: u32,
}

impl DeviceLoad {
    /// Total engine time. Can exceed wall time on a multi-engine
    /// GPU — two engines each busy for a second inside one second is
    /// 2 s of engine time, which is real, not an error.
    #[must_use]
    pub fn total_ns(&self) -> u64 {
        self.engine_ns
            .values()
            .copied()
            .fold(0, u64::saturating_add)
    }
}

/// One cumulative reading, grouped by PCI address.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GpuLoadSample {
    /// `drm-pdev` → that device's counters. A device with no
    /// `drm-pdev` (rare, some virtual drivers) is keyed by driver
    /// name so it is still reported rather than merged into another.
    pub devices: BTreeMap<String, DeviceLoad>,
}

/// Read the current cumulative counters.
///
/// Returns `None` when the process holds no DRM fd that reports
/// fdinfo — an old kernel, a driver without the interface, or simply
/// before the device is opened. Callers should omit their output
/// rather than print zeros, which would read as "the GPU is idle".
#[must_use]
pub fn sample() -> Option<GpuLoadSample> {
    let mut out = GpuLoadSample::default();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for entry in std::fs::read_dir("/proc/self/fd").ok()?.flatten() {
        // The link target tells us this is a DRM device before we
        // pay for reading and parsing the fdinfo of every open file.
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if !target.to_string_lossy().starts_with("/dev/dri/") {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(format!("/proc/self/fdinfo/{name}")) else {
            // Racy by nature: the fd can close between the readdir
            // and the read. A vanished fd is not an error.
            continue;
        };
        accumulate(&text, &mut out, &mut seen);
    }

    if out.devices.is_empty() {
        return None;
    }
    Some(out)
}

/// Fold one fdinfo file into the running totals, skipping a client
/// already counted.
///
/// Split out from [`sample`] so the parsing is testable against
/// captured real-world fdinfo text without a GPU.
fn accumulate(text: &str, out: &mut GpuLoadSample, seen: &mut HashSet<(String, String)>) {
    let mut driver = String::new();
    let mut pdev = String::new();
    let mut client_id = String::new();
    let mut engines: Vec<(String, u64)> = Vec::new();
    let mut vram: Option<u64> = None;
    let mut gtt: Option<u64> = None;

    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "drm-driver" => driver = value.to_owned(),
            "drm-pdev" => pdev = value.to_owned(),
            "drm-client-id" => client_id = value.to_owned(),
            _ => {
                if let Some(engine) = key.strip_prefix("drm-engine-") {
                    // ★ Engine TIME is "<n> ns". The same prefix also
                    // carries `drm-engine-capacity-<name>: <n>`, an
                    // engine COUNT with no unit — i915 emits
                    // `drm-engine-capacity-video: 2` on this box.
                    // Keying on the `ns` unit rather than blacklisting
                    // the `capacity-` prefix rejects that, and any
                    // future unitless sibling, by construction.
                    if let Some(ns) = value
                        .strip_suffix("ns")
                        .map(str::trim_end)
                        .and_then(|n| n.parse::<u64>().ok())
                    {
                        engines.push((engine.to_owned(), ns));
                    }
                } else if let Some(region) = key.strip_prefix("drm-memory-")
                    && let Some(bytes) = parse_size(value)
                {
                    match region {
                        "vram" => vram = Some(bytes),
                        "gtt" => gtt = Some(bytes),
                        _ => {}
                    }
                }
            }
        }
    }

    // A file with no DRM keys at all is not a DRM client: the fd may
    // have been closed and the number reused between the readlink
    // and the read.
    if driver.is_empty() && engines.is_empty() {
        return;
    }
    // Dedupe per file description: a `dup()`'d fd exposes the same
    // counters under a second fd number, and summing both would
    // double the reported load.
    if !seen.insert((driver.clone(), client_id)) {
        return;
    }

    // Group by physical device. Falling back to the driver name
    // keeps a pdev-less device separate rather than merging it into
    // whichever device happened to be first.
    let key = if pdev.is_empty() {
        driver.clone()
    } else {
        pdev
    };
    let dev = out.devices.entry(key).or_default();
    if dev.driver.is_empty() {
        dev.driver = driver;
    }
    dev.clients += 1;
    for (engine, ns) in engines {
        let slot = dev.engine_ns.entry(engine).or_insert(0);
        *slot = slot.saturating_add(ns);
    }
    if let Some(v) = vram {
        *dev.vram_bytes.get_or_insert(0) += v;
    }
    if let Some(g) = gtt {
        *dev.gtt_bytes.get_or_insert(0) += g;
    }
}

/// Parse a `drm-memory-*` value, which carries an explicit unit.
///
/// The interface specifies KiB, but the unit is written out and
/// drivers have shipped others, so honour what is actually there
/// rather than assuming. A bare number is taken as bytes.
fn parse_size(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let n: u64 = parts.next()?.parse().ok()?;
    Some(match parts.next() {
        None | Some("B") => n,
        Some("KiB") => n.saturating_mul(1024),
        Some("MiB") => n.saturating_mul(1024 * 1024),
        Some("GiB") => n.saturating_mul(1024 * 1024 * 1024),
        Some(_) => return None,
    })
}

/// Busy fraction per engine for one device, as a percentage of wall
/// time.
///
/// `elapsed_ns` is the wall interval between `prev` and `cur`.
/// Engines present in `cur` but not `prev` count from zero. A
/// counter that went backwards (a client closed and a new one took
/// its id) yields 0 rather than a nonsense spike.
#[must_use]
pub fn busy_percent(
    prev: Option<&DeviceLoad>,
    cur: &DeviceLoad,
    elapsed_ns: u64,
) -> Vec<(String, f64)> {
    if elapsed_ns == 0 {
        return Vec::new();
    }
    cur.engine_ns
        .iter()
        .map(|(engine, &now)| {
            let before = prev
                .and_then(|p| p.engine_ns.get(engine))
                .copied()
                .unwrap_or(0);
            let delta = now.saturating_sub(before);
            (engine.clone(), (delta as f64 / elapsed_ns as f64) * 100.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim capture from `/proc/self/fdinfo/<fd>` for a freshly
    /// opened `/dev/dri/renderD128` on silence (i915 iGPU,
    /// 2026-09-22). Real driver output, not a hand-written guess —
    /// it is what exposed `drm-engine-capacity-video`.
    const I915_REAL: &str = "\
drm-driver:\ti915
drm-client-id:\t537
drm-pdev:\t0000:00:02.0
drm-total-system0:\t0
drm-resident-system0:\t0
drm-engine-render:\t0 ns
drm-engine-copy:\t0 ns
drm-engine-video:\t0 ns
drm-engine-capacity-video:\t2
drm-engine-video-enhance:\t0 ns
";

    /// Verbatim capture from `/dev/dri/renderD129` on silence
    /// (amdgpu RX 6800, 2026-09-22). Note the trailing space before
    /// the tab on `drm-memory-gtt`, and that amdgpu emits NO
    /// `drm-engine-*` until work has been submitted.
    const AMDGPU_REAL: &str = "\
drm-driver:\tamdgpu
drm-client-id:\t544
drm-pdev:\t0000:03:00.0
drm-total-gtt:\t2 MiB
drm-resident-gtt:\t2 MiB
drm-total-vram:\t12 KiB
drm-resident-vram:\t12 KiB
drm-memory-vram:\t12 KiB
drm-memory-gtt: \t2048 KiB
drm-memory-cpu: \t0 KiB
";

    fn one(text: &str) -> GpuLoadSample {
        let mut out = GpuLoadSample::default();
        let mut seen = HashSet::new();
        accumulate(text, &mut out, &mut seen);
        out
    }

    fn only(s: &GpuLoadSample) -> &DeviceLoad {
        assert_eq!(s.devices.len(), 1, "expected one device: {:?}", s.devices);
        s.devices.values().next().unwrap()
    }

    /// ★ The case that matters on silence and the hybrid laptops:
    /// two physical GPUs must NOT be summed into one figure.
    #[test]
    fn two_gpus_are_reported_separately_not_summed() {
        let mut out = GpuLoadSample::default();
        let mut seen = HashSet::new();
        accumulate(I915_REAL, &mut out, &mut seen);
        accumulate(AMDGPU_REAL, &mut out, &mut seen);

        assert_eq!(out.devices.len(), 2);
        let intel = &out.devices["0000:00:02.0"];
        let amd = &out.devices["0000:03:00.0"];
        assert_eq!(intel.driver, "i915");
        assert_eq!(amd.driver, "amdgpu");
        // The Intel engines must not appear under the AMD device.
        assert!(amd.engine_ns.is_empty());
        assert_eq!(intel.engine_ns.len(), 4);
        // Nor its memory.
        assert_eq!(intel.vram_bytes, None);
        assert_eq!(amd.vram_bytes, Some(12 * 1024));
    }

    /// `drm-engine-capacity-<name>` is an engine COUNT, not a time.
    /// Treating it as nanoseconds would invent an engine called
    /// "capacity-video" that was busy for 2 ns.
    #[test]
    fn engine_capacity_is_not_mistaken_for_engine_time() {
        let s = one(I915_REAL);
        let d = only(&s);
        assert!(
            !d.engine_ns.contains_key("capacity-video"),
            "capacity parsed as an engine: {:?}",
            d.engine_ns
        );
        let mut names: Vec<&str> = d.engine_ns.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["copy", "render", "video", "video-enhance"]);
        assert_eq!(d.total_ns(), 0);
    }

    /// i915 reports memory as `drm-total-<region>`, not
    /// `drm-memory-*`. Report unavailable rather than 0, so it is not
    /// misread as "no memory in use".
    #[test]
    fn i915_memory_is_unavailable_not_zero() {
        let s = one(I915_REAL);
        let d = only(&s);
        assert_eq!(d.vram_bytes, None);
        assert_eq!(d.gtt_bytes, None);
    }

    /// amdgpu does carry the legacy `drm-memory-*` keys, including
    /// one with a stray space before the tab.
    #[test]
    fn amdgpu_memory_keys_parse_including_the_stray_space() {
        let s = one(AMDGPU_REAL);
        let d = only(&s);
        assert_eq!(d.vram_bytes, Some(12 * 1024));
        assert_eq!(d.gtt_bytes, Some(2048 * 1024));
        assert_eq!(d.clients, 1);
    }

    /// A `dup()`'d fd exposes the same counters twice.
    #[test]
    fn the_same_client_seen_twice_is_counted_once() {
        let mut out = GpuLoadSample::default();
        let mut seen = HashSet::new();
        accumulate(AMDGPU_REAL, &mut out, &mut seen);
        accumulate(AMDGPU_REAL, &mut out, &mut seen);
        assert_eq!(only(&out).clients, 1);
        assert_eq!(only(&out).vram_bytes, Some(12 * 1024));
    }

    /// Two distinct clients on the SAME device — Mesa's render fd
    /// alongside our KMS fd — must sum. This is why we scan every fd
    /// rather than only our own.
    #[test]
    fn distinct_clients_on_one_device_sum() {
        let second = AMDGPU_REAL.replace("drm-client-id:\t544", "drm-client-id:\t99");
        let mut out = GpuLoadSample::default();
        let mut seen = HashSet::new();
        accumulate(AMDGPU_REAL, &mut out, &mut seen);
        accumulate(&second, &mut out, &mut seen);
        let d = only(&out);
        assert_eq!(d.clients, 2);
        assert_eq!(d.vram_bytes, Some(2 * 12 * 1024));
    }

    /// An fd that is not a DRM client (closed and reused between the
    /// readlink and the read) contributes nothing.
    #[test]
    fn a_non_drm_file_contributes_nothing() {
        let s = one("pos:\t0\nflags:\t02\nmnt_id:\t24\n");
        assert!(s.devices.is_empty());
    }

    #[test]
    fn busy_percent_is_the_delta_over_wall_time() {
        let mut prev = DeviceLoad::default();
        prev.engine_ns.insert("gfx".into(), 1_000_000_000);
        let mut cur = DeviceLoad::default();
        // Half a second of engine time in one second of wall time.
        cur.engine_ns.insert("gfx".into(), 1_500_000_000);

        let pct = busy_percent(Some(&prev), &cur, 1_000_000_000);
        assert_eq!(pct.len(), 1);
        assert_eq!(pct[0].0, "gfx");
        assert!((pct[0].1 - 50.0).abs() < 1e-9, "got {}", pct[0].1);
    }

    /// A client closing and its id being reused can make a counter
    /// go backwards. That must read as 0, not a huge spike.
    #[test]
    fn a_counter_going_backwards_does_not_spike() {
        let mut prev = DeviceLoad::default();
        prev.engine_ns.insert("gfx".into(), 5_000_000_000);
        let mut cur = DeviceLoad::default();
        cur.engine_ns.insert("gfx".into(), 1_000_000);

        let pct = busy_percent(Some(&prev), &cur, 1_000_000_000);
        assert!((pct[0].1 - 0.0).abs() < 1e-9, "got {}", pct[0].1);
    }

    /// First sample of a device has no predecessor; it must count
    /// from zero rather than panic or report nothing.
    #[test]
    fn a_device_seen_for_the_first_time_counts_from_zero() {
        let mut cur = DeviceLoad::default();
        cur.engine_ns.insert("gfx".into(), 250_000_000);
        let pct = busy_percent(None, &cur, 1_000_000_000);
        assert!((pct[0].1 - 25.0).abs() < 1e-9, "got {}", pct[0].1);
    }

    #[test]
    fn memory_units_are_honoured() {
        assert_eq!(parse_size("1024 KiB"), Some(1024 * 1024));
        assert_eq!(parse_size("2 MiB"), Some(2 * 1024 * 1024));
        assert_eq!(parse_size("512"), Some(512));
        assert_eq!(parse_size("7 furlongs"), None);
    }
}
