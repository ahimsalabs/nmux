use std::fmt;
use std::io::{self, Read, Write};

use crate::{PROTOCOL_VERSION, protocol};

pub const DEFAULT_MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum WireError {
    Io(io::Error),
    FrameTooLarge { len: usize, max: usize },
    ProtocolVersionUnsupported { version: u32, supported: u32 },
    InvalidFlatbuffer(flatbuffers::InvalidFlatbuffer),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "wire I/O failed: {err}"),
            Self::FrameTooLarge { len, max } => {
                write!(
                    f,
                    "wire frame too large: {len} bytes exceeds {max} byte limit"
                )
            }
            Self::ProtocolVersionUnsupported { version, supported } => write!(
                f,
                "unsupported protocol version: {version}; supported version is {supported}"
            ),
            Self::InvalidFlatbuffer(err) => write!(f, "invalid flatbuffer frame: {err}"),
        }
    }
}

impl std::error::Error for WireError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::InvalidFlatbuffer(err) => Some(err),
            Self::FrameTooLarge { .. } | Self::ProtocolVersionUnsupported { .. } => None,
        }
    }
}

impl From<io::Error> for WireError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<flatbuffers::InvalidFlatbuffer> for WireError {
    fn from(err: flatbuffers::InvalidFlatbuffer) -> Self {
        Self::InvalidFlatbuffer(err)
    }
}

pub fn read_frame<R: Read>(reader: &mut R, max_len: usize) -> Result<Vec<u8>, WireError> {
    let mut prefix = [0_u8; 4];
    reader.read_exact(&mut prefix)?;

    let payload_len = u32::from_le_bytes(prefix) as usize;
    if payload_len > max_len {
        return Err(WireError::FrameTooLarge {
            len: payload_len,
            max: max_len,
        });
    }

    let mut frame = Vec::with_capacity(4 + payload_len);
    frame.extend_from_slice(&prefix);
    frame.resize(4 + payload_len, 0);
    reader.read_exact(&mut frame[4..])?;

    validate_protocol_version(&frame)?;
    Ok(frame)
}

pub fn write_frame<W: Write>(
    writer: &mut W,
    frame: &[u8],
    max_len: usize,
) -> Result<(), WireError> {
    let payload_len = frame
        .get(..4)
        .map(|prefix| u32::from_le_bytes(prefix.try_into().expect("prefix length checked")))
        .unwrap_or(0) as usize;

    if payload_len > max_len {
        return Err(WireError::FrameTooLarge {
            len: payload_len,
            max: max_len,
        });
    }

    validate_protocol_version(frame)?;
    writer.write_all(frame)?;
    Ok(())
}

pub fn read_default_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, WireError> {
    read_frame(reader, DEFAULT_MAX_FRAME_LEN)
}

pub fn write_default_frame<W: Write>(writer: &mut W, frame: &[u8]) -> Result<(), WireError> {
    write_frame(writer, frame, DEFAULT_MAX_FRAME_LEN)
}

fn validate_protocol_version(frame: &[u8]) -> Result<(), WireError> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.protocol_version() != PROTOCOL_VERSION {
        return Err(WireError::ProtocolVersionUnsupported {
            version: envelope.protocol_version(),
            supported: PROTOCOL_VERSION,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor};

    use flatbuffers::FlatBufferBuilder;

    use super::*;
    fn workspace_frame() -> Vec<u8> {
        workspace_frame_with_version(PROTOCOL_VERSION)
    }

    fn workspace_frame_with_version(protocol_version: u32) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let session_id = builder.create_string("local");
        let active_tab_id = builder.create_string("tab-1");
        let tabs = builder.create_vector::<flatbuffers::WIPOffset<protocol::TabNode>>(&[]);

        let snapshot = protocol::WorkspaceTreeSnapshot::create(
            &mut builder,
            &protocol::WorkspaceTreeSnapshotArgs {
                version: 1,
                session_id: Some(session_id),
                tabs: Some(tabs),
                active_tab_id: Some(active_tab_id),
            },
        );

        let envelope_session_id = builder.create_string("local");
        let connection_id = builder.create_string("conn-1");
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq: 1,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::WorkspaceTreeSnapshot,
                body: Some(snapshot.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    #[test]
    fn round_trips_size_prefixed_envelope_frame() {
        let frame = workspace_frame();
        let mut written = Vec::new();

        write_frame(&mut written, &frame, DEFAULT_MAX_FRAME_LEN).expect("write frame");

        let mut reader = Cursor::new(written);
        let decoded_frame = read_frame(&mut reader, DEFAULT_MAX_FRAME_LEN).expect("read frame");
        let envelope =
            protocol::size_prefixed_root_as_envelope(&decoded_frame).expect("decode envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(
            envelope.body_type(),
            protocol::EnvelopeBody::WorkspaceTreeSnapshot
        );
        assert_eq!(
            envelope
                .body_as_workspace_tree_snapshot()
                .and_then(|snapshot| snapshot.session_id()),
            Some("local")
        );
    }

    #[test]
    fn rejects_oversized_frame_before_payload_read() {
        let mut reader = Cursor::new(17_u32.to_le_bytes());
        let err = read_frame(&mut reader, 16).expect_err("oversized frame rejected");

        assert!(matches!(err, WireError::FrameTooLarge { len: 17, max: 16 }));
    }

    #[test]
    fn rejects_corrupt_payload() {
        let mut reader = Cursor::new([1, 0, 0, 0, 0]);
        let err = read_frame(&mut reader, DEFAULT_MAX_FRAME_LEN).expect_err("corrupt payload");

        assert!(matches!(err, WireError::InvalidFlatbuffer(_)));
    }

    #[test]
    fn rejects_truncated_prefix() {
        let mut reader = Cursor::new([1, 0]);
        let err = read_frame(&mut reader, DEFAULT_MAX_FRAME_LEN).expect_err("truncated prefix");

        assert!(matches!(
            err,
            WireError::Io(ref io_err) if io_err.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn rejects_truncated_payload() {
        let frame = workspace_frame();
        let truncated = &frame[..frame.len() - 1];
        let mut reader = Cursor::new(truncated);
        let err = read_frame(&mut reader, DEFAULT_MAX_FRAME_LEN).expect_err("truncated frame");

        assert!(matches!(
            err,
            WireError::Io(ref io_err) if io_err.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn rejects_protocol_version_mismatch_on_read_and_write() {
        let frame = workspace_frame_with_version(PROTOCOL_VERSION + 1);
        let mut reader = Cursor::new(frame.clone());
        let read_err =
            read_frame(&mut reader, DEFAULT_MAX_FRAME_LEN).expect_err("version mismatch rejected");
        assert!(matches!(
            read_err,
            WireError::ProtocolVersionUnsupported {
                version,
                supported
            } if version == PROTOCOL_VERSION + 1 && supported == PROTOCOL_VERSION
        ));

        let mut written = Vec::new();
        let write_err =
            write_frame(&mut written, &frame, DEFAULT_MAX_FRAME_LEN).expect_err("write rejected");
        assert!(matches!(
            write_err,
            WireError::ProtocolVersionUnsupported {
                version,
                supported
            } if version == PROTOCOL_VERSION + 1 && supported == PROTOCOL_VERSION
        ));
    }
}
