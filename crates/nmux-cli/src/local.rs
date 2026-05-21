use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use nmux_core::session::Session;
use nmux_proto::{protocol, wire};

pub fn default_socket_path() -> PathBuf {
    if let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime_dir).join("nmux").join("nmuxd.sock");
    }

    PathBuf::from(format!("/tmp/nmux-{}.sock", std::process::id()))
}

pub fn bind_listener(path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    UnixListener::bind(path)
}

pub fn serve_one(
    listener: &UnixListener,
    session: &Session,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut stream, _) = listener.accept()?;
    let request = read_attach_request(&mut stream)?;
    let workspace_frame = session.workspace_tree_frame("local-client", 1);
    wire::write_default_frame(&mut stream, &workspace_frame)?;

    if request.needs_surface_snapshot(session, "pane-1") {
        let surface_frame = session.pane_surface_frame("local-client", 2);
        wire::write_default_frame(&mut stream, &surface_frame)?;
        read_input_event_from_stream(&mut stream)?;
    }
    Ok(())
}

pub fn attach(path: &Path) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    attach_with_known_surfaces(path, Vec::new())
}

pub fn attach_with_known_surfaces(
    path: &Path,
    known_surfaces: Vec<KnownSurfaceVersion>,
) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(path)?;
    write_attach_request(&mut stream, &AttachRequest { known_surfaces })?;
    let snapshot = attach_from_stream(&mut stream)?;
    if snapshot.surface.is_some() {
        send_key_input(&mut stream, "pane-1", "a")?;
    }
    Ok(snapshot)
}

pub fn attach_from_stream(
    stream: &mut UnixStream,
) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    let workspace_frame = wire::read_default_frame(stream)?;
    let workspace = workspace_summary_from_frame(&workspace_frame)?;

    let surface = match wire::read_default_frame(stream) {
        Ok(surface_frame) => Some(surface_text_from_frame(&surface_frame)?),
        Err(wire::WireError::Io(err)) if err.kind() == io::ErrorKind::UnexpectedEof => None,
        Err(err) => return Err(err.into()),
    };

    Ok(AttachSnapshot { workspace, surface })
}

pub fn send_key_input(
    stream: &mut UnixStream,
    pane_id: &str,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame =
        Session::initial().key_input_frame("local-client", 3, "local-actor", pane_id, 1, text);
    wire::write_default_frame(stream, &frame)?;
    Ok(())
}

pub fn workspace_summary_from_frame(
    frame: &[u8],
) -> Result<WorkspaceSummary, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::WorkspaceTreeSnapshot {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }

    let snapshot = envelope
        .body_as_workspace_tree_snapshot()
        .ok_or("missing workspace tree body")?;
    let tabs = snapshot.tabs().ok_or("workspace tree has no tabs")?;
    let tab = tabs.get(0);
    let pane = tab.root().ok_or("workspace tab has no root pane")?;

    Ok(WorkspaceSummary {
        session_id: snapshot.session_id().unwrap_or_default().to_owned(),
        tab_id: tab.tab_id().unwrap_or_default().to_owned(),
        pane_id: pane.pane_id().unwrap_or_default().to_owned(),
        cols: pane.cols(),
        rows: pane.rows(),
    })
}

pub fn surface_text_from_frame(frame: &[u8]) -> Result<String, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::PaneSurfaceSnapshot {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }

    let snapshot = envelope
        .body_as_pane_surface_snapshot()
        .ok_or("missing pane surface body")?;
    let rows = snapshot.rows_data().ok_or("pane surface has no rows")?;

    let mut rendered = String::new();
    for row_index in 0..rows.len() {
        if row_index > 0 {
            rendered.push('\n');
        }

        let row = rows.get(row_index);
        if let Some(runs) = row.runs() {
            for run_index in 0..runs.len() {
                if let Some(text) = runs.get(run_index).text_utf8() {
                    rendered.push_str(text);
                }
            }
        }
    }

    Ok(rendered)
}

pub fn read_input_event_from_stream(
    stream: &mut UnixStream,
) -> Result<InputSummary, Box<dyn std::error::Error>> {
    let frame = wire::read_default_frame(stream)?;
    input_summary_from_frame(&frame)
}

pub fn input_summary_from_frame(frame: &[u8]) -> Result<InputSummary, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::InputEvent {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }

    let input = envelope
        .body_as_input_event()
        .ok_or("missing input event body")?;
    if input.kind() != protocol::InputKind::Key {
        return Err(format!("unexpected input kind: {:?}", input.kind()).into());
    }

    let key = input.key().ok_or("missing key input")?;
    Ok(InputSummary {
        pane_id: input.pane_id().unwrap_or_default().to_owned(),
        actor_id: input.actor_id().unwrap_or_default().to_owned(),
        input_seq: input.input_seq(),
        text: key.text_utf8().unwrap_or_default().to_owned(),
    })
}

pub fn write_attach_request<W: Write>(writer: &mut W, request: &AttachRequest) -> io::Result<()> {
    let payload = request.encode();
    let len = u32::try_from(payload.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "local attach request exceeds u32 length",
        )
    })?;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(payload.as_bytes())
}

pub fn read_attach_request<R: Read>(reader: &mut R) -> io::Result<AttachRequest> {
    let mut len = [0_u8; 4];
    reader.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "local attach request exceeds 4096 byte limit",
        ));
    }

    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload)?;
    let payload = String::from_utf8(payload)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    AttachRequest::decode(&payload)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRequest {
    pub known_surfaces: Vec<KnownSurfaceVersion>,
}

impl AttachRequest {
    fn encode(&self) -> String {
        let mut payload = String::from("NMUX_LOCAL_ATTACH 1\n");
        for surface in &self.known_surfaces {
            payload.push_str("surface ");
            payload.push_str(&surface.pane_id);
            payload.push(' ');
            payload.push_str(&surface.version.to_string());
            payload.push('\n');
        }
        payload
    }

    fn decode(payload: &str) -> io::Result<Self> {
        let mut lines = payload.lines();
        if lines.next() != Some("NMUX_LOCAL_ATTACH 1") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid local attach request header",
            ));
        }

        let mut known_surfaces = Vec::new();
        for line in lines {
            let mut parts = line.split(' ');
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some("surface"), Some(pane_id), Some(version), None) => {
                    let version = version
                        .parse::<u64>()
                        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    known_surfaces.push(KnownSurfaceVersion {
                        pane_id: pane_id.to_owned(),
                        version,
                    });
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid local attach request line",
                    ));
                }
            }
        }

        Ok(Self { known_surfaces })
    }

    fn needs_surface_snapshot(&self, session: &Session, pane_id: &str) -> bool {
        let Some(current) = session.surface_version(pane_id) else {
            return false;
        };

        self.known_surfaces
            .iter()
            .find(|known| known.pane_id == pane_id)
            .is_none_or(|known| known.version != current)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownSurfaceVersion {
    pub pane_id: String,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachSnapshot {
    pub workspace: WorkspaceSummary,
    pub surface: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSummary {
    pub pane_id: String,
    pub actor_id: String,
    pub input_seq: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSummary {
    pub session_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub cols: u32,
    pub rows: u32,
}

impl WorkspaceSummary {
    pub fn display_line(&self) -> String {
        format!(
            "session={} tab={} pane={} size={}x{}",
            self.session_id, self.tab_id, self.pane_id, self.cols, self.rows
        )
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn test_socket_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        PathBuf::from(format!("/tmp/nmux-{}-{nanos}.sock", std::process::id()))
    }

    #[test]
    fn serves_initial_attach_snapshot_over_unix_socket() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &session).expect("serve one"));
        let snapshot = attach(&socket_path).expect("attach snapshot");
        server.join().expect("server thread");

        assert_eq!(
            snapshot.workspace,
            WorkspaceSummary {
                session_id: "local".to_owned(),
                tab_id: "tab-1".to_owned(),
                pane_id: "pane-1".to_owned(),
                cols: 80,
                rows: 24,
            }
        );
        assert_eq!(
            snapshot.workspace.display_line(),
            "session=local tab=tab-1 pane=pane-1 size=80x24"
        );
        assert_eq!(
            snapshot.surface.as_deref(),
            Some("nmux pane-1\nserver-owned terminal state")
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serves_no_surface_when_client_has_current_surface_version() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &session).expect("serve one"));
        let snapshot = attach_with_known_surfaces(
            &socket_path,
            vec![KnownSurfaceVersion {
                pane_id: "pane-1".to_owned(),
                version: 1,
            }],
        )
        .expect("attach snapshot");
        server.join().expect("server thread");

        assert_eq!(snapshot.workspace.pane_id, "pane-1");
        assert_eq!(snapshot.surface, None);

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serves_full_surface_snapshot_when_client_surface_version_is_stale() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &session).expect("serve one"));
        let snapshot = attach_with_known_surfaces(
            &socket_path,
            vec![KnownSurfaceVersion {
                pane_id: "pane-1".to_owned(),
                version: 0,
            }],
        )
        .expect("attach snapshot");
        server.join().expect("server thread");

        assert_eq!(
            snapshot.surface.as_deref(),
            Some("nmux pane-1\nserver-owned terminal state")
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn decodes_key_input_from_client_frame() {
        let frame =
            Session::initial().key_input_frame("local-client", 3, "actor-1", "pane-1", 2, "x");
        let input = input_summary_from_frame(&frame).expect("input summary");

        assert_eq!(
            input,
            InputSummary {
                pane_id: "pane-1".to_owned(),
                actor_id: "actor-1".to_owned(),
                input_seq: 2,
                text: "x".to_owned(),
            }
        );
    }
}
