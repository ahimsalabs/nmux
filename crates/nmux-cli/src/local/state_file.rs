use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nmux_proto::protocol;

pub(super) fn state_save_tmp_path(path: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(format!(".tmp-{}-{nanos}", std::process::id()));
    PathBuf::from(tmp)
}

pub(super) fn parse_state_u64(value: &str) -> io::Result<u64> {
    value
        .parse()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub(super) fn parse_state_i64(value: &str) -> io::Result<i64> {
    value
        .parse()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub(super) fn parse_state_u32(value: &str) -> io::Result<u32> {
    value
        .parse()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn parse_state_i8(value: &str) -> io::Result<i8> {
    value
        .parse()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub(super) fn parse_state_surface_kind(value: &str) -> io::Result<protocol::SurfaceKind> {
    let surface = protocol::SurfaceKind(parse_state_i8(value)?);
    if surface.variant_name().is_some() {
        Ok(surface)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid surface kind in client state",
        ))
    }
}

pub(super) fn parse_state_cursor_shape(value: &str) -> io::Result<protocol::CursorShape> {
    let shape = protocol::CursorShape(parse_state_i8(value)?);
    if shape.variant_name().is_some() {
        Ok(shape)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid cursor shape in client state",
        ))
    }
}

pub(super) fn parse_state_row_semantic_prompt(
    value: &str,
) -> io::Result<protocol::RowSemanticPrompt> {
    let prompt = protocol::RowSemanticPrompt(parse_state_i8(value)?);
    if prompt.variant_name().is_some() {
        Ok(prompt)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid row semantic prompt in client state",
        ))
    }
}

pub(super) fn parse_state_cell_semantic_content(
    value: &str,
) -> io::Result<protocol::CellSemanticContent> {
    let content = protocol::CellSemanticContent(parse_state_i8(value)?);
    if content.variant_name().is_some() {
        Ok(content)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid cell semantic content in client state",
        ))
    }
}

pub(super) fn parse_state_mouse_tracking_mode(
    value: &str,
) -> io::Result<protocol::MouseTrackingMode> {
    let mode = protocol::MouseTrackingMode(parse_state_i8(value)?);
    if mode.variant_name().is_some() {
        Ok(mode)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid mouse tracking mode in client state",
        ))
    }
}

pub(super) fn parse_state_mouse_format(value: &str) -> io::Result<protocol::MouseFormat> {
    let format = protocol::MouseFormat(parse_state_i8(value)?);
    if format.variant_name().is_some() {
        Ok(format)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid mouse format in client state",
        ))
    }
}

pub(super) fn parse_state_bool(value: &str) -> io::Result<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid boolean in client state",
        )),
    }
}

pub(super) fn parse_state_usize(value: &str) -> io::Result<usize> {
    value
        .parse()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub(super) fn decode_palette_rgba(encoded: &str) -> io::Result<Vec<u32>> {
    let bytes = hex_decode(encoded)?;
    if !bytes.len().is_multiple_of(4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "palette color data length is not divisible by four",
        ));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

pub(super) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

pub(super) fn state_hex_field(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        "-".to_owned()
    } else {
        hex_encode(bytes)
    }
}

pub(super) fn decode_state_hex_field(encoded: &str) -> io::Result<Vec<u8>> {
    if encoded == "-" {
        Ok(Vec::new())
    } else {
        hex_decode(encoded)
    }
}

pub(super) fn hex_decode(encoded: &str) -> io::Result<Vec<u8>> {
    let bytes = encoded.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "hex string has odd length",
        ));
    }

    let mut decoded = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let high = hex_value(pair[0])?;
        let low = hex_value(pair[1])?;
        decoded.push((high << 4) | low);
    }
    Ok(decoded)
}

fn hex_value(byte: u8) -> io::Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid hex digit",
        )),
    }
}
