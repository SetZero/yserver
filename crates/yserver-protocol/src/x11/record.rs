//! RECORD extension 1.13 wire decoding/encoding (Xorg `record/record.c`,
//! `recordproto.h`).
//!
//! Request bodies arrive in the client's byte order (RECORD has no entry in
//! the inbound swap table), so every reader here takes the byte order.

use super::{
    ClientByteOrder, SequenceNumber,
    wire::{read_u16, read_u32, write_i16, write_u16, write_u32},
};

pub const QUERY_VERSION: u8 = 0;
pub const CREATE_CONTEXT: u8 = 1;
pub const REGISTER_CLIENTS: u8 = 2;
pub const UNREGISTER_CLIENTS: u8 = 3;
pub const GET_CONTEXT: u8 = 4;
pub const ENABLE_CONTEXT: u8 = 5;
pub const DISABLE_CONTEXT: u8 = 6;
pub const FREE_CONTEXT: u8 = 7;

pub const MAJOR_VERSION: u16 = 1;
pub const MINOR_VERSION: u16 = 13;

/// Extension error offset (`XRecordBadContext`).
pub const BAD_CONTEXT: u8 = 0;

/// Element-header flags (`XRecordFromServerTime` …).
pub const FROM_SERVER_TIME: u8 = 0x01;
pub const FROM_CLIENT_TIME: u8 = 0x02;
pub const FROM_CLIENT_SEQUENCE: u8 = 0x04;

/// CLIENTSPEC constants.
pub const CURRENT_CLIENTS: u32 = 1;
pub const FUTURE_CLIENTS: u32 = 2;
pub const ALL_CLIENTS: u32 = 3;

/// EnableContext reply categories.
pub const FROM_SERVER: u8 = 0;
pub const FROM_CLIENT: u8 = 1;
pub const CLIENT_STARTED: u8 = 2;
pub const CLIENT_DIED: u8 = 3;
pub const START_OF_DATA: u8 = 4;
pub const END_OF_DATA: u8 = 5;

/// `sz_xRecordRange`.
pub const RANGE_SIZE: usize = 24;
/// Body bytes of Create/RegisterClients before the client list
/// (`sz_xRecordRegisterClientsReq` minus the request header).
pub const REGISTER_FIXED_BODY: usize = 16;
/// Body bytes of UnregisterClients before the client list.
pub const UNREGISTER_FIXED_BODY: usize = 8;

/// One `xRecordRange`, fields in wire order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    pub core_requests: (u8, u8),
    pub core_replies: (u8, u8),
    pub ext_requests_major: (u8, u8),
    pub ext_requests_minor: (u16, u16),
    pub ext_replies_major: (u8, u8),
    pub ext_replies_minor: (u16, u16),
    pub delivered_events: (u8, u8),
    pub device_events: (u8, u8),
    pub errors: (u8, u8),
    pub client_started: u8,
    pub client_died: u8,
}

impl Range {
    /// Decode the 24 bytes at the start of `bytes`.
    #[must_use]
    pub fn parse(byte_order: ClientByteOrder, bytes: &[u8]) -> Self {
        Self {
            core_requests: (bytes[0], bytes[1]),
            core_replies: (bytes[2], bytes[3]),
            ext_requests_major: (bytes[4], bytes[5]),
            ext_requests_minor: (
                read_u16(byte_order, &bytes[6..8]),
                read_u16(byte_order, &bytes[8..10]),
            ),
            ext_replies_major: (bytes[10], bytes[11]),
            ext_replies_minor: (
                read_u16(byte_order, &bytes[12..14]),
                read_u16(byte_order, &bytes[14..16]),
            ),
            delivered_events: (bytes[16], bytes[17]),
            device_events: (bytes[18], bytes[19]),
            errors: (bytes[20], bytes[21]),
            client_started: bytes[22],
            client_died: bytes[23],
        }
    }

    pub fn encode(&self, byte_order: ClientByteOrder, out: &mut Vec<u8>) {
        out.extend_from_slice(&[
            self.core_requests.0,
            self.core_requests.1,
            self.core_replies.0,
            self.core_replies.1,
            self.ext_requests_major.0,
            self.ext_requests_major.1,
        ]);
        write_u16(byte_order, out, self.ext_requests_minor.0);
        write_u16(byte_order, out, self.ext_requests_minor.1);
        out.extend_from_slice(&[self.ext_replies_major.0, self.ext_replies_major.1]);
        write_u16(byte_order, out, self.ext_replies_minor.0);
        write_u16(byte_order, out, self.ext_replies_minor.1);
        out.extend_from_slice(&[
            self.delivered_events.0,
            self.delivered_events.1,
            self.device_events.0,
            self.device_events.1,
            self.errors.0,
            self.errors.1,
            self.client_started,
            self.client_died,
        ]);
    }
}

/// Fixed part of a CreateContext / RegisterClients body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterHeader {
    pub context: u32,
    pub element_header: u8,
    pub n_clients: u32,
    pub n_ranges: u32,
}

/// `None` when the body is shorter than the fixed part (the caller's
/// `REQUEST_AT_LEAST_SIZE` BadLength).
#[must_use]
pub fn parse_register_header(byte_order: ClientByteOrder, body: &[u8]) -> Option<RegisterHeader> {
    if body.len() < REGISTER_FIXED_BODY {
        return None;
    }
    Some(RegisterHeader {
        context: read_u32(byte_order, &body[0..4]),
        element_header: body[4],
        n_clients: read_u32(byte_order, &body[8..12]),
        n_ranges: read_u32(byte_order, &body[12..16]),
    })
}

/// The `index`th CARD32 of a list starting at body offset `from`.
#[must_use]
pub fn read_list_u32(byte_order: ClientByteOrder, body: &[u8], from: usize, index: usize) -> u32 {
    let at = from + 4 * index;
    read_u32(byte_order, &body[at..at + 4])
}

/// A CARD32 at body offset `at`.
#[must_use]
pub fn read_body_u32(byte_order: ClientByteOrder, body: &[u8], at: usize) -> u32 {
    read_u32(byte_order, &body[at..at + 4])
}

#[must_use]
pub fn encode_query_version_reply(
    byte_order: ClientByteOrder,
    sequence: SequenceNumber,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&[1, 0]);
    write_u16(byte_order, &mut out, sequence.0);
    write_u32(byte_order, &mut out, 0);
    write_u16(byte_order, &mut out, MAJOR_VERSION);
    write_u16(byte_order, &mut out, MINOR_VERSION);
    out.resize(32, 0);
    out
}

/// GetContext reply: one `xRecordClientInfo` + its ranges per entry of
/// `clients` (`(client_resource, ranges)`).
#[must_use]
pub fn encode_get_context_reply(
    byte_order: ClientByteOrder,
    sequence: SequenceNumber,
    enabled: bool,
    element_header: u8,
    clients: &[(u32, &[Range])],
) -> Vec<u8> {
    let words: usize = clients
        .iter()
        .map(|(_, ranges)| 2 + ranges.len() * (RANGE_SIZE / 4))
        .sum();
    let mut out = Vec::with_capacity(32 + 4 * words);
    out.extend_from_slice(&[1, u8::from(enabled)]);
    write_u16(byte_order, &mut out, sequence.0);
    write_u32(
        byte_order,
        &mut out,
        u32::try_from(words).unwrap_or(u32::MAX),
    );
    out.extend_from_slice(&[element_header, 0, 0, 0]);
    write_u32(
        byte_order,
        &mut out,
        u32::try_from(clients.len()).unwrap_or(u32::MAX),
    );
    out.resize(32, 0);
    for (resource, ranges) in clients {
        write_u32(byte_order, &mut out, *resource);
        write_u32(
            byte_order,
            &mut out,
            u32::try_from(ranges.len()).unwrap_or(u32::MAX),
        );
        for range in *ranges {
            range.encode(byte_order, &mut out);
        }
    }
    out
}

/// The 32-byte header of one EnableContext reply; `length` counts the
/// data words that follow it.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn encode_enable_context_header(
    byte_order: ClientByteOrder,
    sequence: SequenceNumber,
    category: u8,
    length: u32,
    element_header: u8,
    client_swapped: bool,
    id_base: u32,
    server_time: u32,
    recorded_sequence: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&[1, category]);
    write_u16(byte_order, &mut out, sequence.0);
    write_u32(byte_order, &mut out, length);
    out.extend_from_slice(&[element_header, u8::from(client_swapped), 0, 0]);
    write_u32(byte_order, &mut out, id_base);
    write_u32(byte_order, &mut out, server_time);
    write_u32(byte_order, &mut out, recorded_sequence);
    out.resize(32, 0);
    out
}

/// A recorded core device event as Xorg's `EventToCore` builds it: no
/// event/child window, zero event coordinates and `same_screen`, and the
/// autorepeat flag in the sequence field.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn encode_core_device_event(
    byte_order: ClientByteOrder,
    event_type: u8,
    detail: u8,
    repeat: bool,
    time: u32,
    root: u32,
    root_x: i16,
    root_y: i16,
    state: u16,
) -> [u8; 32] {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&[event_type, detail]);
    write_u16(byte_order, &mut out, u16::from(repeat));
    write_u32(byte_order, &mut out, time);
    write_u32(byte_order, &mut out, root);
    write_u32(byte_order, &mut out, 0);
    write_u32(byte_order, &mut out, 0);
    write_i16(byte_order, &mut out, root_x);
    write_i16(byte_order, &mut out, root_y);
    write_u32(byte_order, &mut out, 0);
    write_u16(byte_order, &mut out, state);
    out.resize(32, 0);
    out.try_into().expect("32-byte event")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A range from the survey capture (tools/record-probe.c `basic` on Xvfb
    /// 21.1.24): device events 2..6, clientStarted and clientDied set:
    /// `00000000 00000000 00000000 00000000 00000206 00000101`.
    const DEVICE_RANGE: [u8; 24] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 6, 0, 0, 1, 1,
    ];

    #[test]
    fn range_round_trips() {
        let range = Range::parse(ClientByteOrder::LittleEndian, &DEVICE_RANGE);
        assert_eq!(range.device_events, (2, 6));
        assert_eq!((range.client_started, range.client_died), (1, 1));
        let mut out = Vec::new();
        range.encode(ClientByteOrder::LittleEndian, &mut out);
        assert_eq!(out, DEVICE_RANGE);
    }

    #[test]
    fn range_minor_fields_follow_byte_order() {
        let mut bytes = [0u8; 24];
        bytes[4] = 150;
        bytes[5] = 150;
        bytes[6..8].copy_from_slice(&[0x01, 0x02]);
        bytes[8..10].copy_from_slice(&[0x03, 0x04]);
        let range = Range::parse(ClientByteOrder::BigEndian, &bytes);
        assert_eq!(range.ext_requests_minor, (0x0102, 0x0304));
        let mut out = Vec::new();
        range.encode(ClientByteOrder::BigEndian, &mut out);
        assert_eq!(out, bytes);
    }

    #[test]
    fn query_version_reply_is_1_13() {
        // Xvfb 21.1.24 (tools/record-probe.c): 01000200 00000000 01000d00 …
        let reply = encode_query_version_reply(ClientByteOrder::LittleEndian, SequenceNumber(2));
        assert_eq!(&reply[..12], &[1, 0, 2, 0, 0, 0, 0, 0, 1, 0, 0x0d, 0]);
        assert!(reply[12..].iter().all(|b| *b == 0));
    }
}
