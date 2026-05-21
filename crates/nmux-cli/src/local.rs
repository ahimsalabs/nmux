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

    if let Some(response) = request.surface_response(session, "pane-1") {
        let surface_frame = match response {
            SurfaceResponse::Snapshot => session.pane_surface_frame("local-client", 2),
            SurfaceResponse::Patch { base_version } => {
                session.pane_surface_patch_frame("local-client", 2, base_version)
            }
        };
        wire::write_default_frame(&mut stream, &surface_frame)?;
        read_input_event_from_stream(&mut stream)?;
        let fetch = read_scrollback_fetch_from_stream(&mut stream)?;
        let chunk =
            session.scrollback_chunk_frame("local-client", 4, fetch.start_line, fetch.line_count);
        wire::write_default_frame(&mut stream, &chunk)?;
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
        send_scrollback_fetch(&mut stream, "pane-1", 1, 2)?;
        let scrollback = read_scrollback_chunk_from_stream(&mut stream)?;
        return Ok(AttachSnapshot {
            scrollback: Some(scrollback),
            ..snapshot
        });
    }
    Ok(snapshot)
}

pub fn attach_from_stream(
    stream: &mut UnixStream,
) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    let workspace_frame = wire::read_default_frame(stream)?;
    let workspace = workspace_summary_from_frame(&workspace_frame)?;

    let surface = match wire::read_default_frame(stream) {
        Ok(surface_frame) => Some(surface_update_from_frame(&surface_frame)?),
        Err(wire::WireError::Io(err)) if err.kind() == io::ErrorKind::UnexpectedEof => None,
        Err(err) => return Err(err.into()),
    };

    Ok(AttachSnapshot {
        workspace,
        surface,
        scrollback: None,
    })
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

pub fn send_scrollback_fetch(
    stream: &mut UnixStream,
    pane_id: &str,
    start_line: u64,
    line_count: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame = Session::initial().scrollback_fetch_frame(
        "local-client",
        4,
        "local-actor",
        pane_id,
        start_line,
        line_count,
        1,
    );
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
    Ok(surface_update_from_frame(frame)?.text)
}

pub fn surface_update_from_frame(
    frame: &[u8],
) -> Result<SurfaceUpdate, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    match envelope.body_type() {
        protocol::EnvelopeBody::PaneSurfaceSnapshot => {
            let snapshot = envelope
                .body_as_pane_surface_snapshot()
                .ok_or("missing pane surface body")?;
            let rows = snapshot.rows_data().ok_or("pane surface has no rows")?;
            Ok(SurfaceUpdate {
                kind: SurfaceUpdateKind::Snapshot,
                text: render_surface_rows(rows.len(), |index| {
                    rows.get(index)
                        .runs()
                        .map(render_cell_runs)
                        .unwrap_or_default()
                }),
            })
        }
        protocol::EnvelopeBody::PaneSurfacePatch => {
            let patch = envelope
                .body_as_pane_surface_patch()
                .ok_or("missing pane surface patch body")?;
            let rows = patch
                .row_updates()
                .ok_or("pane surface patch has no rows")?;
            Ok(SurfaceUpdate {
                kind: SurfaceUpdateKind::Patch,
                text: render_surface_rows(rows.len(), |index| {
                    rows.get(index)
                        .runs()
                        .map(render_cell_runs)
                        .unwrap_or_default()
                }),
            })
        }
        other => Err(format!("unexpected envelope body: {other:?}").into()),
    }
}

fn render_surface_rows<F>(len: usize, mut row_text: F) -> String
where
    F: FnMut(usize) -> String,
{
    let mut rendered = String::new();
    for row_index in 0..len {
        if row_index > 0 {
            rendered.push('\n');
        }
        rendered.push_str(&row_text(row_index));
    }

    rendered
}

fn render_cell_runs(
    runs: flatbuffers::Vector<'_, flatbuffers::ForwardsUOffset<protocol::CellRun<'_>>>,
) -> String {
    let mut rendered = String::new();
    for run_index in 0..runs.len() {
        if let Some(text) = runs.get(run_index).text_utf8() {
            rendered.push_str(text);
        }
    }
    rendered
}

pub fn read_input_event_from_stream(
    stream: &mut UnixStream,
) -> Result<InputSummary, Box<dyn std::error::Error>> {
    let frame = wire::read_default_frame(stream)?;
    input_summary_from_frame(&frame)
}

pub fn read_scrollback_fetch_from_stream(
    stream: &mut UnixStream,
) -> Result<ScrollbackFetchSummary, Box<dyn std::error::Error>> {
    let frame = wire::read_default_frame(stream)?;
    scrollback_fetch_from_frame(&frame)
}

pub fn read_scrollback_chunk_from_stream(
    stream: &mut UnixStream,
) -> Result<ScrollbackChunkSummary, Box<dyn std::error::Error>> {
    let frame = wire::read_default_frame(stream)?;
    scrollback_chunk_from_frame(&frame)
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

pub fn scrollback_fetch_from_frame(
    frame: &[u8],
) -> Result<ScrollbackFetchSummary, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::ScrollbackFetch {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }

    let fetch = envelope
        .body_as_scrollback_fetch()
        .ok_or("missing scrollback fetch body")?;
    Ok(ScrollbackFetchSummary {
        pane_id: fetch.pane_id().unwrap_or_default().to_owned(),
        actor_id: fetch.actor_id().unwrap_or_default().to_owned(),
        start_line: fetch.start_line(),
        line_count: fetch.line_count(),
        known_scrollback_version: fetch.known_scrollback_version(),
    })
}

pub fn scrollback_chunk_from_frame(
    frame: &[u8],
) -> Result<ScrollbackChunkSummary, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::ScrollbackChunk {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }

    let chunk = envelope
        .body_as_scrollback_chunk()
        .ok_or("missing scrollback chunk body")?;
    let rows = chunk.rows().ok_or("scrollback chunk has no rows")?;
    let mut lines = Vec::with_capacity(rows.len());
    for index in 0..rows.len() {
        let row = rows.get(index);
        lines.push(ScrollbackLine {
            line: row.line(),
            text: row.runs().map(render_cell_runs).unwrap_or_default(),
        });
    }

    Ok(ScrollbackChunkSummary {
        pane_id: chunk.pane_id().unwrap_or_default().to_owned(),
        scrollback_version: chunk.scrollback_version(),
        start_line: chunk.start_line(),
        total_lines: chunk.total_lines(),
        lines,
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

    fn surface_response(&self, session: &Session, pane_id: &str) -> Option<SurfaceResponse> {
        let Some(current) = session.surface_version(pane_id) else {
            return None;
        };

        let Some(known) = self
            .known_surfaces
            .iter()
            .find(|known| known.pane_id == pane_id)
        else {
            return Some(SurfaceResponse::Snapshot);
        };

        if known.version == current {
            None
        } else if known.version.checked_add(1) == Some(current) {
            Some(SurfaceResponse::Patch {
                base_version: known.version,
            })
        } else {
            Some(SurfaceResponse::Snapshot)
        }
    }
}

enum SurfaceResponse {
    Snapshot,
    Patch { base_version: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownSurfaceVersion {
    pub pane_id: String,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachSnapshot {
    pub workspace: WorkspaceSummary,
    pub surface: Option<SurfaceUpdate>,
    pub scrollback: Option<ScrollbackChunkSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceUpdate {
    pub kind: SurfaceUpdateKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceUpdateKind {
    Snapshot,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSummary {
    pub pane_id: String,
    pub actor_id: String,
    pub input_seq: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollbackFetchSummary {
    pub pane_id: String,
    pub actor_id: String,
    pub start_line: u64,
    pub line_count: u32,
    pub known_scrollback_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollbackChunkSummary {
    pub pane_id: String,
    pub scrollback_version: u64,
    pub start_line: u64,
    pub total_lines: u64,
    pub lines: Vec<ScrollbackLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollbackLine {
    pub line: u64,
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
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
            snapshot.surface,
            Some(SurfaceUpdate {
                kind: SurfaceUpdateKind::Snapshot,
                text: "nmux pane-1\nserver-owned terminal state".to_owned(),
            })
        );
        assert_eq!(
            snapshot.scrollback,
            Some(ScrollbackChunkSummary {
                pane_id: "pane-1".to_owned(),
                scrollback_version: 1,
                start_line: 1,
                total_lines: 3,
                lines: vec![
                    ScrollbackLine {
                        line: 1,
                        text: "nmux pane-1".to_owned(),
                    },
                    ScrollbackLine {
                        line: 2,
                        text: "server-owned terminal state".to_owned(),
                    },
                ],
            })
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
                version: 2,
            }],
        )
        .expect("attach snapshot");
        server.join().expect("server thread");

        assert_eq!(snapshot.workspace.pane_id, "pane-1");
        assert_eq!(snapshot.surface, None);
        assert_eq!(snapshot.scrollback, None);

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serves_patch_when_client_surface_version_is_patchable() {
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

        assert_eq!(
            snapshot.surface,
            Some(SurfaceUpdate {
                kind: SurfaceUpdateKind::Patch,
                text: "nmux pane-1\nserver-owned terminal state".to_owned(),
            })
        );

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
            snapshot.surface,
            Some(SurfaceUpdate {
                kind: SurfaceUpdateKind::Snapshot,
                text: "nmux pane-1\nserver-owned terminal state".to_owned(),
            })
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

    #[test]
    fn decodes_scrollback_fetch_from_client_frame() {
        let frame = Session::initial().scrollback_fetch_frame(
            "local-client",
            4,
            "actor-1",
            "pane-1",
            1,
            2,
            1,
        );
        let fetch = scrollback_fetch_from_frame(&frame).expect("scrollback fetch");

        assert_eq!(
            fetch,
            ScrollbackFetchSummary {
                pane_id: "pane-1".to_owned(),
                actor_id: "actor-1".to_owned(),
                start_line: 1,
                line_count: 2,
                known_scrollback_version: 1,
            }
        );
    }

    #[test]
    fn decodes_scrollback_chunk_from_server_frame() {
        let frame = Session::initial().scrollback_chunk_frame("local-client", 4, 1, 2);
        let chunk = scrollback_chunk_from_frame(&frame).expect("scrollback chunk");

        assert_eq!(chunk.pane_id, "pane-1");
        assert_eq!(chunk.scrollback_version, 1);
        assert_eq!(chunk.start_line, 1);
        assert_eq!(chunk.total_lines, 3);
        assert_eq!(
            chunk.lines,
            vec![
                ScrollbackLine {
                    line: 1,
                    text: "nmux pane-1".to_owned(),
                },
                ScrollbackLine {
                    line: 2,
                    text: "server-owned terminal state".to_owned(),
                },
            ]
        );
    }
}
