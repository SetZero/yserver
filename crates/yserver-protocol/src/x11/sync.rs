use super::{
    ClientByteOrder, SequenceNumber,
    wire::{read_u32, write_u16, write_u32},
};

pub const INITIALIZE: u8 = 0;
pub const LIST_SYSTEM_COUNTERS: u8 = 1;
pub const CREATE_COUNTER: u8 = 2;
pub const SET_COUNTER: u8 = 3;
pub const CHANGE_COUNTER: u8 = 4;
pub const QUERY_COUNTER: u8 = 5;
pub const DESTROY_COUNTER: u8 = 6;
pub const AWAIT: u8 = 7;
pub const CREATE_ALARM: u8 = 8;
pub const CHANGE_ALARM: u8 = 9;
pub const QUERY_ALARM: u8 = 10;
pub const DESTROY_ALARM: u8 = 11;
pub const SET_PRIORITY: u8 = 12;
pub const GET_PRIORITY: u8 = 13;
pub const CREATE_FENCE: u8 = 14;
pub const TRIGGER_FENCE: u8 = 15;
pub const RESET_FENCE: u8 = 16;
pub const DESTROY_FENCE: u8 = 17;
pub const QUERY_FENCE: u8 = 18;
pub const AWAIT_FENCE: u8 = 19;

pub const SERVERTIME_COUNTER: u32 = 0x106;
pub const IDLETIME_COUNTER: u32 = 0x107;
/// Per-master-device IDLETIME counters. yserver hard-codes the XI2
/// master pair: VCP=2, VCK=3 (see `key_fanout.rs:29`,
/// `pointer_fanout.rs:30`). Counter IDs picked to avoid collision
/// with anything in `resources.rs`.
pub const IDLETIME_DEVICE_VCP: u32 = 0x108;
pub const IDLETIME_DEVICE_VCK: u32 = 0x109;

// Alarm trigger semantics, sourced from
// `/usr/include/X11/extensions/syncconst.h`.
//
// XSyncValueType.
pub const VALUE_TYPE_ABSOLUTE: u32 = 0;
pub const VALUE_TYPE_RELATIVE: u32 = 1;
// XSyncTestType.
pub const TEST_POSITIVE_TRANSITION: u32 = 0;
pub const TEST_NEGATIVE_TRANSITION: u32 = 1;
pub const TEST_POSITIVE_COMPARISON: u32 = 2;
pub const TEST_NEGATIVE_COMPARISON: u32 = 3;
// XSyncAlarmState.
pub const ALARM_STATE_ACTIVE: u8 = 0;
pub const ALARM_STATE_INACTIVE: u8 = 1;
pub const ALARM_STATE_DESTROYED: u8 = 2;
// CreateAlarm/ChangeAlarm value-mask bits (XSyncCA*).
pub const CA_COUNTER: u32 = 1 << 0;
pub const CA_VALUE_TYPE: u32 = 1 << 1;
pub const CA_VALUE: u32 = 1 << 2;
pub const CA_TEST_TYPE: u32 = 1 << 3;
pub const CA_DELTA: u32 = 1 << 4;
pub const CA_EVENTS: u32 = 1 << 5;
// Event sub-codes relative to the SYNC first-event base
// (CounterNotify=0, AlarmNotify=1).
pub const COUNTER_NOTIFY_KIND: u8 = 0;
pub const ALARM_NOTIFY_KIND: u8 = 1;

// Error codes relative to the SYNC first-error base (`syncconst.h`).
pub const BAD_COUNTER: u8 = 0;
pub const BAD_ALARM: u8 = 1;
pub const BAD_FENCE: u8 = 2;

/// Wire size of one `WAITCONDITION` in `Await`: trigger (counter,
/// value-type, INT64 wait-value, test-type) plus the INT64 event threshold.
pub const WAIT_CONDITION_LEN: usize = 28;

/// One `Await` wait condition as sent (`xSyncWaitCondition`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitCondition {
    pub counter: u32,
    pub value_type: u32,
    pub wait_value: i64,
    pub test_type: u32,
    pub event_threshold: i64,
}

fn read_i64_ordered(byte_order: ClientByteOrder, bytes: &[u8]) -> i64 {
    // INT64 on the wire is hi (INT32) then lo (CARD32).
    #[allow(clippy::cast_possible_wrap)]
    let hi = read_u32(byte_order, bytes) as i32;
    (i64::from(hi) << 32) | i64::from(read_u32(byte_order, &bytes[4..]))
}

/// Parse an `Await` body: a list of wait conditions. `None` when the length
/// is not a whole number of conditions (Xorg: BadLength). An empty list
/// parses; the handler rejects it with BadValue as Xorg does.
#[must_use]
pub fn parse_await(byte_order: ClientByteOrder, body: &[u8]) -> Option<Vec<WaitCondition>> {
    if !body.len().is_multiple_of(WAIT_CONDITION_LEN) {
        return None;
    }
    Some(
        body.chunks_exact(WAIT_CONDITION_LEN)
            .map(|c| WaitCondition {
                counter: read_u32(byte_order, c),
                value_type: read_u32(byte_order, &c[4..]),
                wait_value: read_i64_ordered(byte_order, &c[8..]),
                test_type: read_u32(byte_order, &c[16..]),
                event_threshold: read_i64_ordered(byte_order, &c[20..]),
            })
            .collect(),
    )
}

/// Encode a `CounterNotify` event (32 bytes, `xSyncCounterNotifyEvent`):
/// sent only to a client whose `Await` / `AwaitFence` just unblocked.
/// `count` is the number of events still to follow for that request.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn encode_counter_notify_event(
    byte_order: ClientByteOrder,
    first_event: u8,
    sequence: SequenceNumber,
    counter: u32,
    wait_value: i64,
    counter_value: i64,
    time: u32,
    count: u16,
    destroyed: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.push(first_event.wrapping_add(COUNTER_NOTIFY_KIND));
    out.push(COUNTER_NOTIFY_KIND);
    write_u16(byte_order, &mut out, sequence.0);
    write_u32(byte_order, &mut out, counter);
    write_i64(byte_order, &mut out, wait_value);
    write_i64(byte_order, &mut out, counter_value);
    write_u32(byte_order, &mut out, time);
    write_u16(byte_order, &mut out, count);
    out.push(u8::from(destroyed));
    out.push(0);
    debug_assert_eq!(out.len(), 32);
    out
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreateFenceRequest {
    pub drawable: u32,
    pub fence: u32,
    pub initially_triggered: bool,
}

pub const MAJOR_VERSION: u8 = 3;
// 3.1 (not 3.0) because we already implement all the fence requests
// (opcodes 14–19, wired in 525529e for the GLX/DRI3 path). Modern WMs
// gate their sync-fence frame-timing fast path on `minor >= 1`; xfwm4
// logs `XSync extension too old (3.0)` and falls back without it.
// xcb-proto `sync.xml` is at 3.1 — canonical.
pub const MINOR_VERSION: u8 = 1;

fn read_u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_i32_le(bytes: &[u8]) -> i32 {
    i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_i64(hi: &[u8], lo: &[u8]) -> i64 {
    (i64::from(read_i32_le(hi)) << 32) | i64::from(read_u32_le(lo))
}

fn write_i64(byte_order: ClientByteOrder, out: &mut Vec<u8>, value: i64) {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let hi = (value >> 32) as u32;
    #[allow(clippy::cast_possible_truncation)]
    let lo = value as u32;
    write_u32(byte_order, out, hi);
    write_u32(byte_order, out, lo);
}

fn fixed_reply(byte_order: ClientByteOrder, sequence: SequenceNumber, length: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.push(1);
    out.push(0);
    write_u16(byte_order, &mut out, sequence.0);
    write_u32(byte_order, &mut out, length);
    out
}

#[must_use]
pub fn parse_counter_value(body: &[u8]) -> Option<(u32, i64)> {
    if body.len() < 12 {
        return None;
    }
    Some((read_u32_le(body), read_i64(&body[4..], &body[8..])))
}

#[must_use]
pub fn parse_resource(body: &[u8]) -> Option<u32> {
    if body.len() < 4 {
        return None;
    }
    Some(read_u32_le(body))
}

#[must_use]
pub fn parse_alarm_with_mask(body: &[u8]) -> Option<(u32, u32)> {
    if body.len() < 8 {
        return None;
    }
    Some((read_u32_le(body), read_u32_le(&body[4..])))
}

/// Body length (after the 4-byte request header) of a `CreateAlarm` /
/// `ChangeAlarm` whose value-mask is `mask`: id and mask, then one word
/// per mask bit plus a second word for each INT64 (`VALUE`, `DELTA`) —
/// Xorg's `Ones(vmask) + Ones(vmask & (XSyncCAValue | XSyncCADelta))`.
/// Every mask bit counts, known or not, as in Xorg.
#[must_use]
pub fn alarm_request_len(mask: u32) -> usize {
    let words = mask.count_ones() + (mask & (CA_VALUE | CA_DELTA)).count_ones();
    8 + 4 * words as usize
}

/// Attributes carried by a `CreateAlarm`/`ChangeAlarm` value-list.
/// Each field is `Some` only when its value-mask bit was set.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AlarmAttributes {
    pub counter: Option<u32>,
    pub value_type: Option<u32>,
    pub value: Option<i64>,
    pub test_type: Option<u32>,
    pub delta: Option<i64>,
    /// The raw `events` word: Xorg accepts only True (1) and False (0)
    /// and answers anything else with BadValue naming it.
    pub events: Option<u32>,
}

/// Parse a `CreateAlarm`/`ChangeAlarm` body: `alarm(4) value-mask(4)
/// value-list(var)`. Fields appear in ascending value-mask bit order;
/// CARD32/enum/BOOL fields occupy 4 bytes, INT64 (`VALUE`, `DELTA`)
/// occupy 8 (INT32 hi, CARD32 lo). Returns `None` on truncation.
#[must_use]
pub fn parse_alarm_attributes(body: &[u8]) -> Option<(u32, AlarmAttributes)> {
    if body.len() < 8 {
        return None;
    }
    let alarm = read_u32_le(body);
    let mask = read_u32_le(&body[4..]);
    let list = &body[8..];
    let mut off = 0usize;
    let mut attrs = AlarmAttributes::default();

    if mask & CA_COUNTER != 0 {
        if list.len() < off + 4 {
            return None;
        }
        attrs.counter = Some(read_u32_le(&list[off..]));
        off += 4;
    }
    if mask & CA_VALUE_TYPE != 0 {
        if list.len() < off + 4 {
            return None;
        }
        attrs.value_type = Some(read_u32_le(&list[off..]));
        off += 4;
    }
    if mask & CA_VALUE != 0 {
        if list.len() < off + 8 {
            return None;
        }
        attrs.value = Some(read_i64(&list[off..], &list[off + 4..]));
        off += 8;
    }
    if mask & CA_TEST_TYPE != 0 {
        if list.len() < off + 4 {
            return None;
        }
        attrs.test_type = Some(read_u32_le(&list[off..]));
        off += 4;
    }
    if mask & CA_DELTA != 0 {
        if list.len() < off + 8 {
            return None;
        }
        attrs.delta = Some(read_i64(&list[off..], &list[off + 4..]));
        off += 8;
    }
    if mask & CA_EVENTS != 0 {
        if list.len() < off + 4 {
            return None;
        }
        attrs.events = Some(read_u32_le(&list[off..]));
    }
    Some((alarm, attrs))
}

/// Does a counter transition from `old` to `new` satisfy an alarm's
/// trigger test against `wait_value`? Transition tests require an actual
/// crossing; comparison tests look only at the new value (see the X
/// Synchronization Extension spec, §"Alarms").
#[must_use]
pub fn trigger_fires(test_type: u32, old: i64, new: i64, wait_value: i64) -> bool {
    match test_type {
        TEST_POSITIVE_TRANSITION => old < wait_value && new >= wait_value,
        TEST_NEGATIVE_TRANSITION => old > wait_value && new <= wait_value,
        TEST_POSITIVE_COMPARISON => new >= wait_value,
        TEST_NEGATIVE_COMPARISON => new <= wait_value,
        _ => false,
    }
}

/// The comparison underlying a test type, used when re-arming a
/// triggered alarm: advance `wait_value` by `delta` until this is false
/// for the current counter value, so the alarm fires again on the next
/// crossing rather than immediately.
#[must_use]
pub fn comparison_satisfied(test_type: u32, value: i64, wait_value: i64) -> bool {
    match test_type {
        TEST_POSITIVE_TRANSITION | TEST_POSITIVE_COMPARISON => value >= wait_value,
        TEST_NEGATIVE_TRANSITION | TEST_NEGATIVE_COMPARISON => value <= wait_value,
        _ => false,
    }
}

/// Encode an `AlarmNotify` event (32 bytes). `first_event` is the SYNC
/// extension's advertised first-event base; the event type is
/// `first_event + ALARM_NOTIFY_KIND`. Layout matches
/// `xSyncAlarmNotifyEvent` in `syncproto.h`.
#[must_use]
pub fn encode_alarm_notify_event(
    byte_order: ClientByteOrder,
    first_event: u8,
    sequence: SequenceNumber,
    alarm: u32,
    counter_value: i64,
    alarm_value: i64,
    time: u32,
    state: u8,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.push(first_event.wrapping_add(ALARM_NOTIFY_KIND));
    out.push(ALARM_NOTIFY_KIND);
    write_u16(byte_order, &mut out, sequence.0);
    write_u32(byte_order, &mut out, alarm);
    write_i64(byte_order, &mut out, counter_value);
    write_i64(byte_order, &mut out, alarm_value);
    write_u32(byte_order, &mut out, time);
    out.push(state);
    out.extend_from_slice(&[0u8; 3]);
    debug_assert_eq!(out.len(), 32);
    out
}

#[must_use]
pub fn parse_create_fence(body: &[u8]) -> Option<CreateFenceRequest> {
    // Body: drawable(4) fence(4) initially_triggered(1) pad(3) = 12B.
    if body.len() < 12 {
        return None;
    }
    Some(CreateFenceRequest {
        drawable: read_u32_le(body),
        fence: read_u32_le(&body[4..]),
        initially_triggered: body[8] != 0,
    })
}

/// Parse an `AwaitFence` body: `FENCE[n]`, n implicit in the length.
/// `None` when the length is not a multiple of 4 (Xorg: BadLength).
#[must_use]
pub fn parse_await_fence(byte_order: ClientByteOrder, body: &[u8]) -> Option<Vec<u32>> {
    if !body.len().is_multiple_of(4) {
        return None;
    }
    Some(
        body.chunks_exact(4)
            .map(|c| read_u32(byte_order, c))
            .collect(),
    )
}

#[must_use]
pub fn encode_query_fence_reply(
    byte_order: ClientByteOrder,
    sequence: SequenceNumber,
    triggered: bool,
) -> Vec<u8> {
    let mut out = fixed_reply(byte_order, sequence, 0);
    out.push(u8::from(triggered));
    out.extend_from_slice(&[0u8; 23]);
    debug_assert_eq!(out.len(), 32);
    out
}

#[must_use]
pub fn encode_initialize_reply(
    byte_order: ClientByteOrder,
    sequence: SequenceNumber,
    major: u8,
    minor: u8,
) -> Vec<u8> {
    let mut out = fixed_reply(byte_order, sequence, 0);
    out.push(major);
    out.push(minor);
    write_u16(byte_order, &mut out, 0);
    out.extend_from_slice(&[0u8; 20]);
    debug_assert_eq!(out.len(), 32);
    out
}

#[must_use]
pub fn encode_list_system_counters_empty_reply(
    byte_order: ClientByteOrder,
    sequence: SequenceNumber,
) -> Vec<u8> {
    let mut out = fixed_reply(byte_order, sequence, 0);
    write_u32(byte_order, &mut out, 0); // counters_len = 0
    out.extend_from_slice(&[0u8; 20]);
    debug_assert_eq!(out.len(), 32);
    out
}

#[must_use]
pub fn encode_list_system_counters_reply(
    byte_order: ClientByteOrder,
    sequence: SequenceNumber,
) -> Vec<u8> {
    // Newest first, as Xorg's list (SyncCreateSystemCounter prepends). It
    // also keeps libxcb's systemcounter iterator in step: it assumes a
    // 16-byte entry header (the wire has 14) and loses step after
    // SERVERTIME, which is therefore last.
    const COUNTERS: &[(u32, i64, &[u8])] = &[
        (IDLETIME_DEVICE_VCK, 4, b"DEVICEIDLETIME 3"),
        (IDLETIME_DEVICE_VCP, 4, b"DEVICEIDLETIME 2"),
        (IDLETIME_COUNTER, 4, b"IDLETIME"),
        (SERVERTIME_COUNTER, 4, b"SERVERTIME"),
    ];

    let payload_len: usize = COUNTERS
        .iter()
        .map(|(_, _, name)| (14 + name.len()).next_multiple_of(4))
        .sum();

    let mut out = fixed_reply(
        byte_order,
        sequence,
        u32::try_from(payload_len / 4).expect("system counter reply length fits u32"),
    );
    write_u32(
        byte_order,
        &mut out,
        u32::try_from(COUNTERS.len()).expect("system counter count fits u32"),
    );
    out.extend_from_slice(&[0u8; 20]);
    debug_assert_eq!(out.len(), 32);

    for &(counter, resolution_ms, name) in COUNTERS {
        let entry_start = out.len();
        write_u32(byte_order, &mut out, counter);
        write_i64(byte_order, &mut out, resolution_ms);
        write_u16(
            byte_order,
            &mut out,
            u16::try_from(name.len()).expect("system counter name length fits u16"),
        );
        out.extend_from_slice(name);
        out.resize(entry_start + (14 + name.len()).next_multiple_of(4), 0);
    }
    out
}

#[must_use]
pub fn encode_query_counter_reply(
    byte_order: ClientByteOrder,
    sequence: SequenceNumber,
    value: i64,
) -> Vec<u8> {
    let mut out = fixed_reply(byte_order, sequence, 0);
    write_i64(byte_order, &mut out, value);
    out.extend_from_slice(&[0u8; 16]);
    debug_assert_eq!(out.len(), 32);
    out
}

#[must_use]
pub fn encode_query_alarm_reply(
    byte_order: ClientByteOrder,
    sequence: SequenceNumber,
    counter: u32,
    wait_value: i64,
    test_type: u32,
    delta: i64,
    events: bool,
    state: u8,
) -> Vec<u8> {
    let mut out = fixed_reply(byte_order, sequence, 2);
    write_u32(byte_order, &mut out, counter);
    // Xorg `ProcSyncQueryAlarm` always answers Absolute with the resolved
    // test value (its value-type branch is `#if 0`'d out).
    write_u32(byte_order, &mut out, VALUE_TYPE_ABSOLUTE);
    write_i64(byte_order, &mut out, wait_value);
    write_u32(byte_order, &mut out, test_type);
    write_i64(byte_order, &mut out, delta);
    out.push(u8::from(events));
    out.push(state);
    out.extend_from_slice(&[0u8; 2]);
    debug_assert_eq!(out.len(), 40);
    out
}

#[must_use]
pub fn encode_get_priority_reply(
    byte_order: ClientByteOrder,
    sequence: SequenceNumber,
    priority: i32,
) -> Vec<u8> {
    let mut out = fixed_reply(byte_order, sequence, 0);
    #[allow(clippy::cast_sign_loss)]
    let p = priority as u32;
    write_u32(byte_order, &mut out, p);
    out.extend_from_slice(&[0u8; 20]);
    debug_assert_eq!(out.len(), 32);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_reply_shape() {
        let reply = encode_initialize_reply(ClientByteOrder::LittleEndian, SequenceNumber(2), 3, 0);
        assert_eq!(reply.len(), 32);
        assert_eq!(reply[0], 1);
        assert_eq!(u32::from_le_bytes(reply[4..8].try_into().unwrap()), 0);
        assert_eq!(reply[8], 3);
        assert_eq!(reply[9], 0);
    }

    #[test]
    fn query_counter_reply_shape() {
        let reply = encode_query_counter_reply(ClientByteOrder::LittleEndian, SequenceNumber(2), 5);
        assert_eq!(reply.len(), 32);
        assert_eq!(i32::from_le_bytes(reply[8..12].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(reply[12..16].try_into().unwrap()), 5);
    }

    /// Xorg lists system counters newest first (SyncCreateSystemCounter
    /// prepends; Xvfb 21.1.24: DEVICEIDLETIME 7..2, IDLETIME, SERVERTIME).
    /// The order matters beyond fidelity: libxcb's systemcounter iterator
    /// assumes a 16-byte entry header where the wire has 14, so it loses
    /// step after a name whose length is 2 mod 4 — SERVERTIME. Listed last,
    /// as on Xorg, every xcb client still finds IDLETIME; listed first, it
    /// read the following entries as counter 0.
    #[test]
    fn list_system_counters_newest_first_like_xorg() {
        let reply =
            encode_list_system_counters_reply(ClientByteOrder::LittleEndian, SequenceNumber(0x88));
        assert_eq!(
            u32::from_le_bytes(reply[8..12].try_into().unwrap()),
            4,
            "counters_len"
        );
        // Walk the wire layout: counter(4) resolution hi(4) lo(4) name_len(2)
        // name, padded to 4.
        let mut entries = Vec::new();
        let mut at = 32;
        while at < reply.len() {
            let counter = u32::from_le_bytes(reply[at..at + 4].try_into().unwrap());
            let hi = i32::from_le_bytes(reply[at + 4..at + 8].try_into().unwrap());
            let lo = u32::from_le_bytes(reply[at + 8..at + 12].try_into().unwrap());
            let len = usize::from(u16::from_le_bytes([reply[at + 12], reply[at + 13]]));
            let name = String::from_utf8(reply[at + 14..at + 14 + len].to_vec()).unwrap();
            entries.push((counter, (i64::from(hi) << 32) | i64::from(lo), name));
            at += (14 + len).next_multiple_of(4);
        }
        assert_eq!(
            entries,
            vec![
                (IDLETIME_DEVICE_VCK, 4, "DEVICEIDLETIME 3".to_owned()),
                (IDLETIME_DEVICE_VCP, 4, "DEVICEIDLETIME 2".to_owned()),
                (IDLETIME_COUNTER, 4, "IDLETIME".to_owned()),
                (SERVERTIME_COUNTER, 4, "SERVERTIME".to_owned()),
            ]
        );
        // libxcb's walk: header taken as 16 bytes, each entry padded to 4
        // from there (xcb_sync_systemcounter_sizeof). It must land on every
        // entry the real walk found.
        let mut xcb_at = 32;
        for (counter, _, name) in &entries {
            assert_eq!(
                u32::from_le_bytes(reply[xcb_at..xcb_at + 4].try_into().unwrap()),
                *counter,
                "xcb iterator in step at {name}"
            );
            xcb_at += (16 + name.len()).next_multiple_of(4);
        }
    }

    // Reconstructs the exact CreateAlarm muffin sends under Cinnamon
    // (captured in cinnamon.xtrace): all six attributes set, Relative
    // value 1, PositiveComparison, delta 1, events true. The body is
    // everything after the 4-byte generic SYNC request header.
    fn muffin_create_alarm_body() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&0x01e0_0019u32.to_le_bytes()); // alarm
        let mask = CA_COUNTER | CA_VALUE_TYPE | CA_VALUE | CA_TEST_TYPE | CA_DELTA | CA_EVENTS;
        body.extend_from_slice(&mask.to_le_bytes());
        body.extend_from_slice(&0x0260_0006u32.to_le_bytes()); // counter
        body.extend_from_slice(&VALUE_TYPE_RELATIVE.to_le_bytes()); // value-type
        body.extend_from_slice(&0i32.to_le_bytes()); // value hi
        body.extend_from_slice(&1u32.to_le_bytes()); // value lo = 1
        body.extend_from_slice(&TEST_POSITIVE_COMPARISON.to_le_bytes()); // test-type
        body.extend_from_slice(&0i32.to_le_bytes()); // delta hi
        body.extend_from_slice(&1u32.to_le_bytes()); // delta lo = 1
        body.push(1); // events = true
        body.extend_from_slice(&[0u8; 3]); // pad
        body
    }

    #[test]
    fn parse_alarm_attributes_decodes_muffin_create_alarm() {
        let (alarm, attrs) = parse_alarm_attributes(&muffin_create_alarm_body()).unwrap();
        assert_eq!(alarm, 0x01e0_0019);
        assert_eq!(attrs.counter, Some(0x0260_0006));
        assert_eq!(attrs.value_type, Some(VALUE_TYPE_RELATIVE));
        assert_eq!(attrs.value, Some(1));
        assert_eq!(attrs.test_type, Some(TEST_POSITIVE_COMPARISON));
        assert_eq!(attrs.delta, Some(1));
        assert_eq!(attrs.events, Some(1));
    }

    #[test]
    fn parse_alarm_attributes_respects_partial_mask() {
        // Only counter + events set: the value-list packs just those
        // two 4-byte fields, in bit order.
        let mut body = Vec::new();
        body.extend_from_slice(&0x55u32.to_le_bytes()); // alarm
        body.extend_from_slice(&(CA_COUNTER | CA_EVENTS).to_le_bytes());
        body.extend_from_slice(&0xabcdu32.to_le_bytes()); // counter
        body.push(0); // events = false
        body.extend_from_slice(&[0u8; 3]);
        let (alarm, attrs) = parse_alarm_attributes(&body).unwrap();
        assert_eq!(alarm, 0x55);
        assert_eq!(attrs.counter, Some(0xabcd));
        assert_eq!(attrs.value_type, None);
        assert_eq!(attrs.value, None);
        assert_eq!(attrs.events, Some(0));
    }

    /// Xorg's CreateAlarm / ChangeAlarm length rule: a word per mask bit,
    /// two for VALUE and DELTA; muffin's full CreateAlarm is 8 + 32 bytes.
    #[test]
    fn alarm_request_len_counts_int64_values_twice() {
        assert_eq!(alarm_request_len(0), 8);
        assert_eq!(alarm_request_len(CA_EVENTS), 12);
        assert_eq!(alarm_request_len(CA_VALUE), 16);
        assert_eq!(muffin_create_alarm_body().len(), 40);
        assert_eq!(
            alarm_request_len(
                CA_COUNTER | CA_VALUE_TYPE | CA_VALUE | CA_TEST_TYPE | CA_DELTA | CA_EVENTS
            ),
            40
        );
        assert_eq!(alarm_request_len(1 << 7), 12, "unknown bits count too");
    }

    #[test]
    fn parse_alarm_attributes_rejects_truncated_list() {
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_le_bytes()); // alarm
        body.extend_from_slice(&CA_VALUE.to_le_bytes()); // claims an INT64 value
        body.extend_from_slice(&[0u8; 4]); // only 4 bytes, INT64 needs 8
        assert!(parse_alarm_attributes(&body).is_none());
    }

    #[test]
    fn trigger_fires_semantics() {
        // PositiveComparison: fires whenever new >= wait.
        assert!(trigger_fires(TEST_POSITIVE_COMPARISON, 0, 5, 5));
        assert!(trigger_fires(TEST_POSITIVE_COMPARISON, 9, 6, 5));
        assert!(!trigger_fires(TEST_POSITIVE_COMPARISON, 0, 4, 5));
        // PositiveTransition: only on the upward crossing.
        assert!(trigger_fires(TEST_POSITIVE_TRANSITION, 4, 5, 5));
        assert!(!trigger_fires(TEST_POSITIVE_TRANSITION, 5, 6, 5)); // already past
        // NegativeComparison / NegativeTransition mirror.
        assert!(trigger_fires(TEST_NEGATIVE_COMPARISON, 0, 3, 5));
        assert!(trigger_fires(TEST_NEGATIVE_TRANSITION, 6, 5, 5));
        assert!(!trigger_fires(TEST_NEGATIVE_TRANSITION, 4, 3, 5));
    }

    #[test]
    fn alarm_notify_event_shape_matches_syncproto() {
        let evt = encode_alarm_notify_event(
            ClientByteOrder::LittleEndian,
            83, // SYNC first-event base
            SequenceNumber(0x1234),
            0x01e0_0019,
            2,           // counter value
            7,           // alarm (wait) value
            0x89ab_cdef, // time
            ALARM_STATE_ACTIVE,
        );
        assert_eq!(evt.len(), 32);
        assert_eq!(evt[0], 84, "type = SYNC first-event + AlarmNotify(1)");
        assert_eq!(evt[1], ALARM_NOTIFY_KIND, "kind byte");
        assert_eq!(u16::from_le_bytes(evt[2..4].try_into().unwrap()), 0x1234);
        assert_eq!(
            u32::from_le_bytes(evt[4..8].try_into().unwrap()),
            0x01e0_0019
        );
        assert_eq!(
            i32::from_le_bytes(evt[8..12].try_into().unwrap()),
            0,
            "counter hi"
        );
        assert_eq!(
            u32::from_le_bytes(evt[12..16].try_into().unwrap()),
            2,
            "counter lo"
        );
        assert_eq!(
            i32::from_le_bytes(evt[16..20].try_into().unwrap()),
            0,
            "alarm hi"
        );
        assert_eq!(
            u32::from_le_bytes(evt[20..24].try_into().unwrap()),
            7,
            "alarm lo"
        );
        assert_eq!(
            u32::from_le_bytes(evt[24..28].try_into().unwrap()),
            0x89ab_cdef
        );
        assert_eq!(evt[28], ALARM_STATE_ACTIVE);
        assert_eq!(&evt[29..32], &[0, 0, 0]);
    }

    #[test]
    fn create_fence_parses() {
        let mut body = vec![0u8; 12];
        body[0..4].copy_from_slice(&0x100u32.to_le_bytes());
        body[4..8].copy_from_slice(&0x500u32.to_le_bytes());
        body[8] = 1;
        let req = parse_create_fence(&body).unwrap();
        assert_eq!(req.drawable, 0x100);
        assert_eq!(req.fence, 0x500);
        assert!(req.initially_triggered);
    }

    #[test]
    fn await_fence_parses_list() {
        let mut body = vec![0u8; 12];
        body[0..4].copy_from_slice(&0x100u32.to_le_bytes());
        body[4..8].copy_from_slice(&0x200u32.to_le_bytes());
        body[8..12].copy_from_slice(&0x300u32.to_le_bytes());
        let list = parse_await_fence(ClientByteOrder::LittleEndian, &body).unwrap();
        assert_eq!(list, vec![0x100, 0x200, 0x300]);
    }

    #[test]
    fn await_fence_rejects_misaligned() {
        assert!(parse_await_fence(ClientByteOrder::LittleEndian, &[0u8; 7]).is_none());
    }

    #[test]
    fn query_fence_reply_shape() {
        let reply =
            encode_query_fence_reply(ClientByteOrder::LittleEndian, SequenceNumber(8), true);
        assert_eq!(reply.len(), 32);
        assert_eq!(reply[0], 1, "Reply opcode");
        assert_eq!(reply[8], 1, "triggered byte");
    }

    #[test]
    fn query_alarm_reply_shape() {
        let reply = encode_query_alarm_reply(
            ClientByteOrder::LittleEndian,
            SequenceNumber(2),
            7,
            0,
            TEST_NEGATIVE_COMPARISON,
            0,
            false,
            0,
        );
        assert_eq!(reply.len(), 40);
        assert_eq!(u32::from_le_bytes(reply[4..8].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(reply[8..12].try_into().unwrap()), 7);
        assert_eq!(
            u32::from_le_bytes(reply[24..28].try_into().unwrap()),
            TEST_NEGATIVE_COMPARISON,
            "test type at xSyncQueryAlarmReply offset 24"
        );
    }

    // Canonical SYNC minor-opcode values, sourced from
    // `/usr/include/X11/extensions/syncproto.h` and the xcbproto
    // `sync.xml` registry. A prior numbering bug shipped these as
    // 14/15/18/19/20/21, which made Mesa's `xcb_sync_trigger_fence`
    // route to our DESTROY_FENCE handler and hung clients in
    // `xshmfence_await`. Pin against the canonical table so any
    // future drift is caught at unit-test time.
    #[test]
    fn sync_fence_opcodes_match_canonical_table() {
        assert_eq!(CREATE_FENCE, 14, "X_SyncCreateFence");
        assert_eq!(TRIGGER_FENCE, 15, "X_SyncTriggerFence");
        assert_eq!(RESET_FENCE, 16, "X_SyncResetFence");
        assert_eq!(DESTROY_FENCE, 17, "X_SyncDestroyFence");
        assert_eq!(QUERY_FENCE, 18, "X_SyncQueryFence");
        assert_eq!(AWAIT_FENCE, 19, "X_SyncAwaitFence");
    }

    #[test]
    fn parse_await_reads_conditions_in_client_byte_order() {
        let mut body = Vec::new();
        for v in [0x0020_0000u32, 1, 0xffff_ffff, 0xffff_fffd, 3, 0, 7] {
            body.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(
            parse_await(ClientByteOrder::BigEndian, &body),
            Some(vec![WaitCondition {
                counter: 0x0020_0000,
                value_type: VALUE_TYPE_RELATIVE,
                wait_value: -3,
                test_type: TEST_NEGATIVE_COMPARISON,
                event_threshold: 7,
            }])
        );
        assert_eq!(parse_await(ClientByteOrder::BigEndian, &body[..27]), None);
        assert_eq!(
            parse_await(ClientByteOrder::BigEndian, &[]),
            Some(Vec::new())
        );
    }

    /// `xSyncCounterNotifyEvent`: type, kind, seq, counter, wait hi/lo,
    /// value hi/lo, time, count, destroyed, pad.
    #[test]
    fn counter_notify_event_layout() {
        let e = encode_counter_notify_event(
            ClientByteOrder::LittleEndian,
            83,
            SequenceNumber(9),
            0x0020_0000,
            -3,
            0x1_0000_0002,
            0x1234,
            1,
            true,
        );
        assert_eq!(e.len(), 32);
        assert_eq!(e[0], 83, "CounterNotify is first_event + 0");
        assert_eq!(e[1], COUNTER_NOTIFY_KIND);
        assert_eq!(&e[2..4], &9u16.to_le_bytes());
        assert_eq!(&e[4..8], &0x0020_0000u32.to_le_bytes());
        assert_eq!(&e[8..12], &(-1i32).to_le_bytes(), "wait hi");
        assert_eq!(&e[12..16], &0xffff_fffdu32.to_le_bytes(), "wait lo");
        assert_eq!(&e[16..20], &1u32.to_le_bytes(), "value hi");
        assert_eq!(&e[20..24], &2u32.to_le_bytes(), "value lo");
        assert_eq!(&e[24..28], &0x1234u32.to_le_bytes());
        assert_eq!(&e[28..30], &1u16.to_le_bytes());
        assert_eq!(e[30], 1);
    }
}
