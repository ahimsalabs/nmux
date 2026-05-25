use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SocketIdentity {
    pub(crate) dev: u64,
    pub(crate) ino: u64,
    pub(crate) ctime: i64,
    pub(crate) ctime_nsec: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketPathSource {
    Explicit,
    NmuxSocket,
    XdgRuntimeDir,
    TempFallback,
}

impl SocketPathSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Explicit => "--socket",
            Self::NmuxSocket => "NMUX_SOCKET",
            Self::XdgRuntimeDir => "XDG_RUNTIME_DIR",
            Self::TempFallback => "fallback",
        }
    }
}

pub fn default_socket_path() -> PathBuf {
    default_socket_path_and_source().0
}

pub fn default_socket_path_and_source() -> (PathBuf, SocketPathSource) {
    default_socket_path_and_source_from(
        env::var_os("NMUX_SOCKET"),
        env::var_os("XDG_RUNTIME_DIR"),
        effective_uid(),
    )
}

pub(crate) fn default_socket_path_and_source_from(
    socket_path: Option<OsString>,
    runtime_dir: Option<OsString>,
    uid: u32,
) -> (PathBuf, SocketPathSource) {
    if let Some(socket_path) = socket_path.map(PathBuf::from)
        && !socket_path.as_os_str().is_empty()
        && socket_path.is_absolute()
    {
        return (socket_path, SocketPathSource::NmuxSocket);
    }

    match runtime_dir.map(PathBuf::from) {
        Some(runtime_dir) if !runtime_dir.as_os_str().is_empty() && runtime_dir.is_absolute() => (
            runtime_dir.join("nmux").join("nmux.sock"),
            SocketPathSource::XdgRuntimeDir,
        ),
        _ => (
            PathBuf::from(format!("/tmp/nmux-{uid}")).join("nmux.sock"),
            SocketPathSource::TempFallback,
        ),
    }
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid is always safe to call; it has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

pub fn socket_identity(path: &Path) -> io::Result<SocketIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(SocketIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        ctime: metadata.ctime(),
        ctime_nsec: metadata.ctime_nsec(),
    })
}

pub fn bind_listener(path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            io::Error::new(
                err.kind(),
                format!(
                    "failed to create socket directory {}: {err}",
                    parent.display()
                ),
            )
        })?;
    }

    match fs::symlink_metadata(path) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!(
                    "socket path already exists: {}; remove stale sockets deliberately or pass --socket PATH",
                    path.display()
                ),
            ));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    UnixListener::bind(path).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!(
                "failed to bind nmux daemon socket at {}: {err}",
                path.display()
            ),
        )
    })
}

pub fn connect_to_daemon(path: &Path) -> Result<UnixStream, Box<dyn std::error::Error>> {
    connect_once(path).map_err(|err| connect_error(path, err).into())
}

pub fn connect_to_daemon_with_timeout(
    path: &Path,
    timeout: Duration,
) -> Result<UnixStream, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + timeout;

    let err = loop {
        match connect_once(path) {
            Ok(stream) => return Ok(stream),
            Err(err) if connect_error_is_retryable(&err) && Instant::now() < deadline => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                thread::sleep(remaining.min(Duration::from_millis(25)));
            }
            Err(err) => break err,
        }
    };

    Err(format!(
        "failed to connect to nmux daemon at {} within {}ms: {err}",
        path.display(),
        timeout.as_millis()
    )
    .into())
}

pub fn bind_tcp_listener(addr: &str) -> io::Result<TcpListener> {
    TcpListener::bind(addr).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("failed to bind nmux TCP listener at {addr}: {err}"),
        )
    })
}

pub fn accept_authenticated_tcp_client(
    listener: &TcpListener,
    token: &str,
) -> Result<UnixStream, Box<dyn std::error::Error>> {
    let (stream, _) = listener.accept()?;
    authenticate_tcp_server(stream, token).map_err(Into::into)
}

pub fn connect_to_tcp_daemon(
    addr: &str,
    token: &str,
) -> Result<UnixStream, Box<dyn std::error::Error>> {
    let stream = connect_tcp_once(addr).map_err(|err| tcp_connect_error(addr, err))?;
    authenticate_tcp_client(stream, token).map_err(Into::into)
}

pub fn connect_to_tcp_daemon_with_timeout(
    addr: &str,
    token: &str,
    timeout: Duration,
) -> Result<UnixStream, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + timeout;
    let err = loop {
        match connect_tcp_once(addr) {
            Ok(stream) => return authenticate_tcp_client(stream, token).map_err(Into::into),
            Err(err) if connect_error_is_retryable(&err) && Instant::now() < deadline => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                thread::sleep(remaining.min(Duration::from_millis(25)));
            }
            Err(err) => break err,
        }
    };

    Err(format!(
        "failed to connect to nmux daemon at tcp://{addr} within {}ms: {err}",
        timeout.as_millis()
    )
    .into())
}

fn connect_once(path: &Path) -> io::Result<UnixStream> {
    UnixStream::connect(path)
}

fn connect_tcp_once(addr: &str) -> io::Result<TcpStream> {
    let mut addrs = addr.to_socket_addrs()?;
    let Some(addr) = addrs.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("TCP address resolved to no endpoints: {addr}"),
        ));
    };
    TcpStream::connect(addr)
}

fn connect_error(path: &Path, err: io::Error) -> String {
    format!(
        "failed to connect to nmux daemon at {}: {err}",
        path.display()
    )
}

fn tcp_connect_error(addr: &str, err: io::Error) -> String {
    format!("failed to connect to nmux daemon at tcp://{addr}: {err}")
}

fn connect_error_is_retryable(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::NotFound
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::TimedOut
            | io::ErrorKind::WouldBlock
    )
}

fn authenticate_tcp_client(mut tcp: TcpStream, token: &str) -> io::Result<UnixStream> {
    validate_tcp_token(token)?;
    tcp.write_all(format!("NMUX-TCP/1 {token}\n").as_bytes())?;
    tcp.flush()?;
    let response = read_tcp_auth_line(&mut tcp)?;
    if response != "NMUX-TCP/1 OK" {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("TCP authentication rejected: {response}"),
        ));
    }
    bridge_tcp_stream(tcp)
}

fn authenticate_tcp_server(mut tcp: TcpStream, token: &str) -> io::Result<UnixStream> {
    validate_tcp_token(token)?;
    let request = read_tcp_auth_line(&mut tcp)?;
    if request != format!("NMUX-TCP/1 {token}") {
        let _ = tcp.write_all(b"NMUX-TCP/1 ERR\n");
        let _ = tcp.flush();
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "TCP authentication failed",
        ));
    }
    tcp.write_all(b"NMUX-TCP/1 OK\n")?;
    tcp.flush()?;
    bridge_tcp_stream(tcp)
}

fn validate_tcp_token(token: &str) -> io::Result<()> {
    if token.is_empty()
        || token
            .bytes()
            .any(|byte| matches!(byte, b'\n' | b'\r' | b' ' | b'\t'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "TCP token must be non-empty and must not contain whitespace",
        ));
    }
    Ok(())
}

fn read_tcp_auth_line(stream: &mut TcpStream) -> io::Result<String> {
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    while bytes.len() < 4096 {
        stream.read_exact(&mut byte)?;
        if byte[0] == b'\n' {
            let line = String::from_utf8(bytes).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid TCP auth line: {err}"),
                )
            })?;
            return Ok(line.trim_end_matches('\r').to_owned());
        }
        bytes.push(byte[0]);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "TCP auth line exceeded 4096 bytes",
    ))
}

fn bridge_tcp_stream(tcp: TcpStream) -> io::Result<UnixStream> {
    let (client, server) = UnixStream::pair()?;
    let tcp_reader = tcp.try_clone()?;
    let tcp_writer = tcp;
    let unix_reader = client.try_clone()?;
    let unix_writer = client;
    thread::spawn(move || {
        let mut reader = tcp_reader;
        let mut writer = unix_writer;
        let _ = io::copy(&mut reader, &mut writer);
        let _ = writer.shutdown(std::net::Shutdown::Write);
    });
    thread::spawn(move || {
        let mut reader = unix_reader;
        let mut writer = tcp_writer;
        let _ = io::copy(&mut reader, &mut writer);
        let _ = writer.shutdown(std::net::Shutdown::Write);
    });
    Ok(server)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(0);

    fn test_socket_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
        PathBuf::from(format!(
            "/tmp/nmux-{}-{nanos}-{id}.sock",
            std::process::id()
        ))
    }

    #[test]
    fn default_socket_path_uses_runtime_dir_when_available() {
        assert_eq!(
            default_socket_path_and_source_from(None, Some(OsString::from("/run/user/1000")), 1000),
            (
                PathBuf::from("/run/user/1000")
                    .join("nmux")
                    .join("nmux.sock"),
                SocketPathSource::XdgRuntimeDir,
            )
        );
    }

    #[test]
    fn default_socket_path_uses_valid_env_socket_before_runtime_dir() {
        assert_eq!(
            default_socket_path_and_source_from(
                Some(OsString::from("/tmp/project-nmux.sock")),
                Some(OsString::from("/run/user/1000")),
                1000,
            ),
            (
                PathBuf::from("/tmp/project-nmux.sock"),
                SocketPathSource::NmuxSocket,
            )
        );
    }

    #[test]
    fn default_socket_path_ignores_invalid_env_socket() {
        assert_eq!(
            default_socket_path_and_source_from(
                Some(OsString::from("relative.sock")),
                Some(OsString::from("/run/user/1000")),
                1000,
            ),
            (
                PathBuf::from("/run/user/1000")
                    .join("nmux")
                    .join("nmux.sock"),
                SocketPathSource::XdgRuntimeDir,
            )
        );
        assert_eq!(
            default_socket_path_and_source_from(
                Some(OsString::from("")),
                Some(OsString::from("/run/user/1000")),
                1000,
            ),
            (
                PathBuf::from("/run/user/1000")
                    .join("nmux")
                    .join("nmux.sock"),
                SocketPathSource::XdgRuntimeDir,
            )
        );
    }

    #[test]
    fn default_socket_path_fallback_is_stable_for_user() {
        let first = default_socket_path_and_source_from(None, None, 501);
        let second = default_socket_path_and_source_from(None, None, 501);

        assert_eq!(first, second);
        assert_eq!(
            first,
            (
                PathBuf::from("/tmp/nmux-501").join("nmux.sock"),
                SocketPathSource::TempFallback,
            )
        );
    }

    #[test]
    fn default_socket_path_falls_back_for_invalid_runtime_dir() {
        assert_eq!(
            default_socket_path_and_source_from(None, Some(OsString::from("")), 501),
            (
                PathBuf::from("/tmp/nmux-501").join("nmux.sock"),
                SocketPathSource::TempFallback,
            )
        );
        assert_eq!(
            default_socket_path_and_source_from(
                None,
                Some(OsString::from("relative-runtime")),
                501
            ),
            (
                PathBuf::from("/tmp/nmux-501").join("nmux.sock"),
                SocketPathSource::TempFallback,
            )
        );
        assert_eq!(
            default_socket_path_and_source_from(None, Some(OsString::from("/run/user/501")), 501),
            (
                PathBuf::from("/run/user/501")
                    .join("nmux")
                    .join("nmux.sock"),
                SocketPathSource::XdgRuntimeDir,
            )
        );
        assert_eq!(
            default_socket_path_and_source_from(None, Some(OsString::from("relative")), 501),
            (
                PathBuf::from("/tmp/nmux-501").join("nmux.sock"),
                SocketPathSource::TempFallback,
            )
        );
    }

    #[test]
    fn bind_listener_rejects_existing_socket_path() {
        let socket_path = test_socket_path();
        let _listener = bind_listener(&socket_path).expect("bind listener");
        let err = bind_listener(&socket_path).expect_err("existing socket should fail");

        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        assert!(
            err.to_string().contains("socket path already exists"),
            "missing existing path context: {err}"
        );
        assert!(
            err.to_string().contains("pass --socket PATH"),
            "missing recovery hint: {err}"
        );
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn bind_listener_includes_path_in_bind_errors() {
        let long_name = format!("nmux-{}.sock", "x".repeat(160));
        let socket_path = std::env::temp_dir().join(long_name);
        let err = bind_listener(&socket_path).expect_err("overlong socket should fail");

        assert!(
            err.to_string()
                .contains("failed to bind nmux daemon socket at"),
            "missing bind context: {err}"
        );
        assert!(
            err.to_string()
                .contains(socket_path.to_str().expect("socket path")),
            "missing socket path: {err}"
        );
    }
}
