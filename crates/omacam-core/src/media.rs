//! Bounded local media-ingress framing.
//!
//! Network media is accepted only after the daemon authenticates the peer and
//! authorizes a capture generation. The daemon forwards H.264 access units to
//! the isolated output worker using this framing. Every record is bound to the
//! unpredictable media-session identifier and capture generation. `Stop` is a
//! terminal record: later data can never revive the invalidated generation.

use std::io::{self, Read, Write};

pub const MEDIA_PROTOCOL_VERSION: u8 = 1;
pub const PEER_IDENTITY_BYTES: usize = 32;
pub const CONTROL_CONNECTION_ID_BYTES: usize = 16;
pub const MEDIA_SESSION_ID_BYTES: usize = 16;
pub const MEDIA_HEADER_BYTES: usize = 104;
pub const MAX_H264_ACCESS_UNIT_BYTES: usize = 1_048_576;
pub const KEY_FRAME_FLAG: u16 = 1;

const MEDIA_MAGIC: [u8; 8] = *b"OMACAMM1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaSessionId([u8; MEDIA_SESSION_ID_BYTES]);

impl MediaSessionId {
    #[must_use]
    pub const fn new(bytes: [u8; MEDIA_SESSION_ID_BYTES]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; MEDIA_SESSION_ID_BYTES] {
        &self.0
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        encode_hex(&self.0)
    }

    /// Parses the stable 32-character lowercase or uppercase hex CLI form.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::InvalidSessionId`] for any other representation.
    pub fn from_hex(value: &str) -> Result<Self, MediaError> {
        parse_hex(value)
            .map(Self)
            .ok_or(MediaError::InvalidSessionId)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerIdentity([u8; PEER_IDENTITY_BYTES]);

impl PeerIdentity {
    #[must_use]
    pub const fn new(bytes: [u8; PEER_IDENTITY_BYTES]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; PEER_IDENTITY_BYTES] {
        &self.0
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        encode_hex(&self.0)
    }

    /// Parses a SHA-256 peer identity as exactly 64 hex characters.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::InvalidPeerIdentity`] for malformed input.
    pub fn from_hex(value: &str) -> Result<Self, MediaError> {
        parse_hex(value)
            .map(Self)
            .ok_or(MediaError::InvalidPeerIdentity)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlConnectionId([u8; CONTROL_CONNECTION_ID_BYTES]);

impl ControlConnectionId {
    #[must_use]
    pub const fn new(bytes: [u8; CONTROL_CONNECTION_ID_BYTES]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; CONTROL_CONNECTION_ID_BYTES] {
        &self.0
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        encode_hex(&self.0)
    }

    /// Parses a fresh authenticated connection identifier as exactly 32 hex
    /// characters.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::InvalidConnectionId`] for malformed input.
    pub fn from_hex(value: &str) -> Result<Self, MediaError> {
        parse_hex(value)
            .map(Self)
            .ok_or(MediaError::InvalidConnectionId)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaBinding {
    pub peer_identity: PeerIdentity,
    pub control_connection_id: ControlConnectionId,
    pub media_session_id: MediaSessionId,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaRecordKind {
    H264AccessUnit,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRecord {
    pub kind: MediaRecordKind,
    pub key_frame: bool,
    pub sequence: u64,
    pub presentation_time_us: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedMediaHeader {
    kind: MediaRecordKind,
    key_frame: bool,
    sequence: u64,
    presentation_time_us: u64,
    payload_length: usize,
}

impl ValidatedMediaHeader {
    #[must_use]
    pub const fn payload_length(self) -> usize {
        self.payload_length
    }
}

#[derive(Debug)]
pub enum MediaError {
    Io(io::Error),
    TruncatedHeader,
    TruncatedPayload,
    InvalidMagic,
    UnsupportedVersion,
    InvalidKind,
    InvalidFlags,
    InvalidPeerIdentity,
    InvalidConnectionId,
    InvalidSessionId,
    WrongPeer,
    WrongConnection,
    WrongSession,
    StaleGeneration,
    InvalidSequence,
    InvalidTimestamp,
    EmptyAccessUnit,
    AccessUnitTooLarge,
    StopHasPayload,
    Invalidated,
}

impl PartialEq for MediaError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Eq for MediaError {}

impl std::fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Io(_) => "media input failed",
            Self::TruncatedHeader => "media header is truncated",
            Self::TruncatedPayload => "media payload is truncated",
            Self::InvalidMagic => "media magic is invalid",
            Self::UnsupportedVersion => "media protocol version is unsupported",
            Self::InvalidKind => "media record kind is invalid",
            Self::InvalidFlags => "media record flags are invalid",
            Self::InvalidPeerIdentity => "media peer identity is invalid",
            Self::InvalidConnectionId => "media control connection identifier is invalid",
            Self::InvalidSessionId => "media session identifier is invalid",
            Self::WrongPeer => "media record belongs to another peer",
            Self::WrongConnection => "media record belongs to another control connection",
            Self::WrongSession => "media record belongs to another session",
            Self::StaleGeneration => "media record belongs to a stale capture generation",
            Self::InvalidSequence => "media record sequence is invalid",
            Self::InvalidTimestamp => "media timestamp is invalid",
            Self::EmptyAccessUnit => "H.264 access unit is empty",
            Self::AccessUnitTooLarge => "H.264 access unit exceeds the size limit",
            Self::StopHasPayload => "media Stop record must not contain a payload",
            Self::Invalidated => "media generation was already stopped",
        })
    }
}

impl std::error::Error for MediaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for MediaError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug)]
pub struct MediaStreamValidator {
    binding: MediaBinding,
    last_sequence: Option<u64>,
    last_presentation_time_us: Option<u64>,
    invalidated: bool,
}

impl MediaStreamValidator {
    #[must_use]
    pub const fn new(binding: MediaBinding) -> Self {
        Self {
            binding,
            last_sequence: None,
            last_presentation_time_us: None,
            invalidated: false,
        }
    }

    #[must_use]
    pub const fn is_invalidated(&self) -> bool {
        self.invalidated
    }

    /// Reads and validates one complete record without allocating from an
    /// untrusted payload length before all header bounds and bindings pass.
    ///
    /// A clean EOF before another header returns `Ok(None)`. A partial header,
    /// partial payload, malformed record, wrong binding, or post-Stop record is
    /// an error and must terminate that ingress connection.
    ///
    /// # Errors
    ///
    /// Returns a typed error for malformed, oversized, out-of-order, stale, or
    /// truncated input and for underlying I/O failures.
    pub fn read_next<R: Read>(
        &mut self,
        reader: &mut R,
    ) -> Result<Option<MediaRecord>, MediaError> {
        if self.invalidated {
            return Err(MediaError::Invalidated);
        }
        let Some(header) = read_header(reader)? else {
            return Ok(None);
        };

        let pending = self.validate_header(&header)?;
        let mut payload = vec![0_u8; pending.payload_length];
        reader
            .read_exact(&mut payload)
            .map_err(|error| match error.kind() {
                io::ErrorKind::UnexpectedEof => MediaError::TruncatedPayload,
                _ => MediaError::Io(error),
            })?;
        self.finish_record(pending, payload).map(Some)
    }

    /// Validates all allocation-relevant and session-binding header fields.
    /// Callers receiving asynchronously must call this before allocating the
    /// advertised payload, then pass exactly that payload to `finish_record`.
    ///
    /// # Errors
    ///
    /// Returns a typed error for malformed, stale, replayed, or oversized input.
    pub fn validate_header(
        &self,
        header: &[u8; MEDIA_HEADER_BYTES],
    ) -> Result<ValidatedMediaHeader, MediaError> {
        if self.invalidated {
            return Err(MediaError::Invalidated);
        }
        if header[..8] != MEDIA_MAGIC {
            return Err(MediaError::InvalidMagic);
        }
        if header[8] != MEDIA_PROTOCOL_VERSION {
            return Err(MediaError::UnsupportedVersion);
        }
        let kind = match header[9] {
            1 => MediaRecordKind::H264AccessUnit,
            2 => MediaRecordKind::Stop,
            _ => return Err(MediaError::InvalidKind),
        };
        let flags = u16::from_be_bytes([header[10], header[11]]);
        if flags & !KEY_FRAME_FLAG != 0 || (kind == MediaRecordKind::Stop && flags != 0) {
            return Err(MediaError::InvalidFlags);
        }
        if header[12..44] != self.binding.peer_identity.0 {
            return Err(MediaError::WrongPeer);
        }
        if header[44..60] != self.binding.control_connection_id.0 {
            return Err(MediaError::WrongConnection);
        }
        if header[60..76] != self.binding.media_session_id.0 {
            return Err(MediaError::WrongSession);
        }
        let generation = read_u64(&header[76..84]);
        if generation != self.binding.generation {
            return Err(MediaError::StaleGeneration);
        }
        let sequence = read_u64(&header[84..92]);
        let presentation_time_us = read_u64(&header[92..100]);
        let payload_length = read_u32(&header[100..104]);
        let payload_length =
            usize::try_from(payload_length).map_err(|_| MediaError::AccessUnitTooLarge)?;

        let expected_sequence = match self.last_sequence {
            Some(value) => value.checked_add(1).ok_or(MediaError::InvalidSequence)?,
            None => 0,
        };
        if sequence != expected_sequence {
            return Err(MediaError::InvalidSequence);
        }

        match kind {
            MediaRecordKind::H264AccessUnit => {
                if payload_length == 0 {
                    return Err(MediaError::EmptyAccessUnit);
                }
                if payload_length > MAX_H264_ACCESS_UNIT_BYTES {
                    return Err(MediaError::AccessUnitTooLarge);
                }
                if self
                    .last_presentation_time_us
                    .is_some_and(|last| presentation_time_us < last)
                {
                    return Err(MediaError::InvalidTimestamp);
                }
            }
            MediaRecordKind::Stop => {
                if payload_length != 0 {
                    return Err(MediaError::StopHasPayload);
                }
                if presentation_time_us != 0 {
                    return Err(MediaError::InvalidTimestamp);
                }
            }
        }

        Ok(ValidatedMediaHeader {
            kind,
            key_frame: flags & KEY_FRAME_FLAG != 0,
            sequence,
            presentation_time_us,
            payload_length,
        })
    }

    /// Commits a previously validated header and its exact payload.
    ///
    /// # Errors
    ///
    /// Rejects a payload length mismatch or a header made stale by another
    /// committed record.
    pub fn finish_record(
        &mut self,
        pending: ValidatedMediaHeader,
        payload: Vec<u8>,
    ) -> Result<MediaRecord, MediaError> {
        if self.invalidated {
            return Err(MediaError::Invalidated);
        }
        if payload.len() != pending.payload_length {
            return Err(MediaError::TruncatedPayload);
        }
        let expected_sequence = self
            .last_sequence
            .map_or(0, |value| value.saturating_add(1));
        if pending.sequence != expected_sequence {
            return Err(MediaError::InvalidSequence);
        }
        self.last_sequence = Some(pending.sequence);
        if pending.kind == MediaRecordKind::H264AccessUnit {
            self.last_presentation_time_us = Some(pending.presentation_time_us);
        } else {
            self.invalidated = true;
        }
        Ok(MediaRecord {
            kind: pending.kind,
            key_frame: pending.key_frame,
            sequence: pending.sequence,
            presentation_time_us: pending.presentation_time_us,
            payload,
        })
    }
}

fn read_header<R: Read>(reader: &mut R) -> Result<Option<[u8; MEDIA_HEADER_BYTES]>, MediaError> {
    let mut header = [0_u8; MEDIA_HEADER_BYTES];
    let first = loop {
        match reader.read(&mut header[..1]) {
            Ok(0) => return Ok(None),
            Ok(1) => break header[0],
            Ok(_) => unreachable!("one-byte read returned more than one byte"),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(MediaError::Io(error)),
        }
    };
    header[0] = first;
    reader
        .read_exact(&mut header[1..])
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => MediaError::TruncatedHeader,
            _ => MediaError::Io(error),
        })?;
    Ok(Some(header))
}

/// Writes one record using the wire contract used by the isolated worker.
///
/// # Errors
///
/// Returns an error if the payload length cannot be represented or writing
/// fails. Callers should use [`MediaStreamValidator`] before forwarding data
/// received from an untrusted peer.
pub fn write_record<W: Write>(
    writer: &mut W,
    binding: MediaBinding,
    record: &MediaRecord,
) -> Result<(), MediaError> {
    match record.kind {
        MediaRecordKind::H264AccessUnit => {
            if record.payload.is_empty() {
                return Err(MediaError::EmptyAccessUnit);
            }
            if record.payload.len() > MAX_H264_ACCESS_UNIT_BYTES {
                return Err(MediaError::AccessUnitTooLarge);
            }
        }
        MediaRecordKind::Stop => {
            if record.key_frame {
                return Err(MediaError::InvalidFlags);
            }
            if !record.payload.is_empty() {
                return Err(MediaError::StopHasPayload);
            }
            if record.presentation_time_us != 0 {
                return Err(MediaError::InvalidTimestamp);
            }
        }
    }
    let payload_length =
        u32::try_from(record.payload.len()).map_err(|_| MediaError::AccessUnitTooLarge)?;
    let mut header = [0_u8; MEDIA_HEADER_BYTES];
    header[..8].copy_from_slice(&MEDIA_MAGIC);
    header[8] = MEDIA_PROTOCOL_VERSION;
    header[9] = match record.kind {
        MediaRecordKind::H264AccessUnit => 1,
        MediaRecordKind::Stop => 2,
    };
    let flags = if record.key_frame { KEY_FRAME_FLAG } else { 0 };
    header[10..12].copy_from_slice(&flags.to_be_bytes());
    header[12..44].copy_from_slice(&binding.peer_identity.0);
    header[44..60].copy_from_slice(&binding.control_connection_id.0);
    header[60..76].copy_from_slice(binding.media_session_id.as_bytes());
    header[76..84].copy_from_slice(&binding.generation.to_be_bytes());
    header[84..92].copy_from_slice(&record.sequence.to_be_bytes());
    header[92..100].copy_from_slice(&record.presentation_time_us.to_be_bytes());
    header[100..104].copy_from_slice(&payload_length.to_be_bytes());
    writer.write_all(&header)?;
    writer.write_all(&record.payload)?;
    Ok(())
}

fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut bytes = [0_u8; N];
    let (pairs, _) = value.as_bytes().as_chunks::<2>();
    for (index, pair) in pairs.iter().enumerate() {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        bytes[index] = (high << 4) | low;
    }
    Some(bytes)
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const BINDING: MediaBinding = MediaBinding {
        peer_identity: PeerIdentity::new([5; PEER_IDENTITY_BYTES]),
        control_connection_id: ControlConnectionId::new([6; CONTROL_CONNECTION_ID_BYTES]),
        media_session_id: MediaSessionId::new([7; MEDIA_SESSION_ID_BYTES]),
        generation: 42,
    };

    fn access_unit(sequence: u64, timestamp: u64) -> MediaRecord {
        MediaRecord {
            kind: MediaRecordKind::H264AccessUnit,
            key_frame: sequence == 0,
            sequence,
            presentation_time_us: timestamp,
            payload: vec![0, 0, 0, 1, 0x65, 0x88],
        }
    }

    fn encoded(binding: MediaBinding, record: &MediaRecord) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_record(&mut bytes, binding, record).expect("encode record");
        bytes
    }

    #[test]
    fn valid_access_units_are_bounded_and_ordered() {
        let first = access_unit(0, 1_000);
        let second = access_unit(1, 1_033);
        let mut bytes = encoded(BINDING, &first);
        bytes.extend(encoded(BINDING, &second));
        let mut validator = MediaStreamValidator::new(BINDING);
        let mut reader = Cursor::new(bytes);

        assert_eq!(validator.read_next(&mut reader).unwrap(), Some(first));
        assert_eq!(validator.read_next(&mut reader).unwrap(), Some(second));
        assert_eq!(validator.read_next(&mut reader).unwrap(), None);
    }

    #[test]
    fn wrong_session_and_generation_are_rejected_before_payload() {
        let record = access_unit(0, 1_000);
        let wrong_session = encoded(
            MediaBinding {
                media_session_id: MediaSessionId::new([8; MEDIA_SESSION_ID_BYTES]),
                ..BINDING
            },
            &record,
        );
        let wrong_generation = encoded(
            MediaBinding {
                generation: BINDING.generation - 1,
                ..BINDING
            },
            &record,
        );

        assert_eq!(
            MediaStreamValidator::new(BINDING).read_next(&mut Cursor::new(wrong_session)),
            Err(MediaError::WrongSession)
        );
        assert_eq!(
            MediaStreamValidator::new(BINDING).read_next(&mut Cursor::new(wrong_generation)),
            Err(MediaError::StaleGeneration)
        );
    }

    #[test]
    fn wrong_peer_and_control_connection_are_rejected() {
        let record = access_unit(0, 1_000);
        let wrong_peer = encoded(
            MediaBinding {
                peer_identity: PeerIdentity::new([9; PEER_IDENTITY_BYTES]),
                ..BINDING
            },
            &record,
        );
        let wrong_connection = encoded(
            MediaBinding {
                control_connection_id: ControlConnectionId::new([9; CONTROL_CONNECTION_ID_BYTES]),
                ..BINDING
            },
            &record,
        );

        assert_eq!(
            MediaStreamValidator::new(BINDING).read_next(&mut Cursor::new(wrong_peer)),
            Err(MediaError::WrongPeer)
        );
        assert_eq!(
            MediaStreamValidator::new(BINDING).read_next(&mut Cursor::new(wrong_connection)),
            Err(MediaError::WrongConnection)
        );
    }

    #[test]
    fn oversized_length_is_rejected_without_allocating_payload() {
        let record = access_unit(0, 1_000);
        let mut bytes = encoded(BINDING, &record);
        bytes[100..104].copy_from_slice(
            &(u32::try_from(MAX_H264_ACCESS_UNIT_BYTES).unwrap() + 1).to_be_bytes(),
        );

        assert_eq!(
            MediaStreamValidator::new(BINDING).read_next(&mut Cursor::new(bytes)),
            Err(MediaError::AccessUnitTooLarge)
        );
    }

    #[test]
    fn asynchronous_header_validation_precedes_payload_allocation_and_commit() {
        let record = access_unit(0, 1_000);
        let bytes = encoded(BINDING, &record);
        let header: [u8; MEDIA_HEADER_BYTES] = bytes[..MEDIA_HEADER_BYTES].try_into().unwrap();
        let mut validator = MediaStreamValidator::new(BINDING);

        let pending = validator.validate_header(&header).expect("valid header");
        assert_eq!(pending.payload_length(), record.payload.len());
        assert_eq!(
            validator.finish_record(pending, vec![0; record.payload.len() - 1]),
            Err(MediaError::TruncatedPayload)
        );
        assert!(!validator.is_invalidated());
    }

    #[test]
    fn malformed_and_truncated_headers_fail_closed() {
        let record = access_unit(0, 1_000);
        let mut malformed = encoded(BINDING, &record);
        malformed[0] ^= 0xff;

        assert_eq!(
            MediaStreamValidator::new(BINDING).read_next(&mut Cursor::new(malformed)),
            Err(MediaError::InvalidMagic)
        );
        assert_eq!(
            MediaStreamValidator::new(BINDING).read_next(&mut Cursor::new(vec![
                0;
                MEDIA_HEADER_BYTES
                    - 1
            ])),
            Err(MediaError::TruncatedHeader)
        );
    }

    #[test]
    fn truncated_payload_and_invalid_record_shapes_fail_closed() {
        let record = access_unit(0, 1_000);
        let mut truncated = encoded(BINDING, &record);
        truncated.pop();
        assert_eq!(
            MediaStreamValidator::new(BINDING).read_next(&mut Cursor::new(truncated)),
            Err(MediaError::TruncatedPayload)
        );

        let empty = MediaRecord {
            payload: Vec::new(),
            ..record.clone()
        };
        assert_eq!(
            write_record(&mut Vec::new(), BINDING, &empty),
            Err(MediaError::EmptyAccessUnit)
        );
        let invalid_stop = MediaRecord {
            kind: MediaRecordKind::Stop,
            key_frame: false,
            sequence: 0,
            presentation_time_us: 0,
            payload: vec![1],
        };
        assert_eq!(
            write_record(&mut Vec::new(), BINDING, &invalid_stop),
            Err(MediaError::StopHasPayload)
        );
    }

    #[test]
    fn gaps_replays_and_timestamp_regressions_are_rejected() {
        let mut validator = MediaStreamValidator::new(BINDING);
        validator
            .read_next(&mut Cursor::new(encoded(BINDING, &access_unit(0, 2_000))))
            .unwrap();

        assert_eq!(
            validator.read_next(&mut Cursor::new(encoded(BINDING, &access_unit(2, 2_033)))),
            Err(MediaError::InvalidSequence)
        );

        let mut validator = MediaStreamValidator::new(BINDING);
        validator
            .read_next(&mut Cursor::new(encoded(BINDING, &access_unit(0, 2_000))))
            .unwrap();
        assert_eq!(
            validator.read_next(&mut Cursor::new(encoded(BINDING, &access_unit(1, 1_999)))),
            Err(MediaError::InvalidTimestamp)
        );
    }

    #[test]
    fn stop_invalidates_generation_before_late_frame() {
        let stop = MediaRecord {
            kind: MediaRecordKind::Stop,
            key_frame: false,
            sequence: 0,
            presentation_time_us: 0,
            payload: Vec::new(),
        };
        let mut validator = MediaStreamValidator::new(BINDING);

        assert_eq!(
            validator
                .read_next(&mut Cursor::new(encoded(BINDING, &stop)))
                .unwrap(),
            Some(stop)
        );
        assert!(validator.is_invalidated());
        assert_eq!(
            validator.read_next(&mut Cursor::new(encoded(BINDING, &access_unit(1, 1_000)))),
            Err(MediaError::Invalidated)
        );
    }

    #[test]
    fn session_hex_parser_is_strict() {
        assert_eq!(
            MediaSessionId::from_hex("000102030405060708090a0b0c0d0e0f")
                .unwrap()
                .as_bytes(),
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        );
        assert_eq!(
            MediaSessionId::from_hex("not-a-session"),
            Err(MediaError::InvalidSessionId)
        );
        assert_eq!(
            PeerIdentity::from_hex("too-short"),
            Err(MediaError::InvalidPeerIdentity)
        );
        assert_eq!(
            ControlConnectionId::from_hex("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
            Err(MediaError::InvalidConnectionId)
        );
    }
}
