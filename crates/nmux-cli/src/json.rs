use std::path::Path;

use crate::socket::SocketPathSource;

pub fn socket_path_json(path: &Path, source: SocketPathSource) -> String {
    format!(
        "{{\"NMUX_SOCKET\":{},\"source\":{}}}",
        json_string(&path.display().to_string()),
        json_string(source.label())
    )
}

pub fn version_json(binary: &str, version: &str) -> String {
    format!(
        "{{\"binary\":{},\"version\":{}}}",
        json_string(binary),
        json_string(version)
    )
}

pub fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => {
                use std::fmt::Write as _;
                write!(&mut escaped, "\\u{:04x}", ch as u32).expect("write to string");
            }
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_json_escapes_path() {
        assert_eq!(json_string("sock\"\\\n"), "\"sock\\\"\\\\\\n\"");
        assert_eq!(
            socket_path_json(Path::new("/tmp/nmux.sock"), SocketPathSource::Explicit),
            "{\"NMUX_SOCKET\":\"/tmp/nmux.sock\",\"source\":\"--socket\"}"
        );
    }

    #[test]
    fn version_json_escapes_values() {
        assert_eq!(
            version_json("nmux\"cli", "1.2.3\n"),
            "{\"binary\":\"nmux\\\"cli\",\"version\":\"1.2.3\\n\"}"
        );
    }
}
