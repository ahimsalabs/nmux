use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use flatbuffers::FlatBufferBuilder;
use nmux_core::host::{HostError, ProcessHost, ProcessOutput};
use nmux_core::session::{Actor, AttachMode, Session};
use nmux_core::terminal::{PaneTerminalEngines, TerminalEngineKind};
use nmux_proto::{PROTOCOL_VERSION, protocol, wire};

const ATTACH_MAX_FRAME_LEN: usize = 64 * 1024;

pub trait ProcessHostOutput: ProcessHost + ProcessOutput {}

impl<T> ProcessHostOutput for T where T: ProcessHost + ProcessOutput {}

pub fn default_socket_path() -> PathBuf {
    default_socket_path_from(env::var_os("XDG_RUNTIME_DIR"), effective_uid())
}

fn default_socket_path_from(runtime_dir: Option<OsString>, uid: u32) -> PathBuf {
    match runtime_dir.map(PathBuf::from) {
        Some(runtime_dir) if !runtime_dir.as_os_str().is_empty() && runtime_dir.is_absolute() => {
            runtime_dir.join("nmux").join("nmuxd.sock")
        }
        _ => PathBuf::from(format!("/tmp/nmux-{uid}")).join("nmuxd.sock"),
    }
}

fn effective_uid() -> u32 {
    unsafe { libc::geteuid() }
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

fn connect_once(path: &Path) -> io::Result<UnixStream> {
    UnixStream::connect(path)
}

fn connect_error(path: &Path, err: io::Error) -> String {
    format!(
        "failed to connect to nmux daemon at {}: {err}",
        path.display()
    )
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

pub fn serve_one(
    listener: &UnixListener,
    session: &mut Session,
) -> Result<(), Box<dyn std::error::Error>> {
    serve_n(listener, session, 1)
}

pub fn serve_one_with_output<O: ProcessOutput>(
    listener: &UnixListener,
    session: &mut Session,
    output: &mut O,
) -> Result<(), Box<dyn std::error::Error>> {
    serve_n_with_output(listener, session, output, 1)
}

pub fn serve_one_with_host<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    serve_n_with_host(listener, session, host, 1)
}

pub fn serve_live_one_with_host<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
    cycles: usize,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    let mut engines = PaneTerminalEngines::new(TerminalEngineKind::InterimText);
    serve_live_one_with_host_and_engines(listener, session, host, &mut engines, cycles)
}

pub fn serve_live_n_with_host<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
    clients: usize,
    cycles_per_client: usize,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    serve_live_n_with_host_and_terminal_engine_kind(
        listener,
        session,
        host,
        clients,
        cycles_per_client,
        TerminalEngineKind::InterimText,
    )
}

pub fn serve_live_n_with_host_and_terminal_engine_kind<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
    clients: usize,
    cycles_per_client: usize,
    terminal_engine_kind: TerminalEngineKind,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    let mut engines = PaneTerminalEngines::new(terminal_engine_kind);
    serve_live_n_with_host_and_engines(
        listener,
        session,
        host,
        clients,
        cycles_per_client,
        &mut engines,
    )
}

pub fn serve_live_n_with_host_and_engines<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
    clients: usize,
    cycles_per_client: usize,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    for _ in 0..clients {
        serve_live_one_with_host_and_engines(listener, session, host, engines, cycles_per_client)?;
    }
    Ok(())
}

pub fn serve_n(
    listener: &UnixListener,
    session: &mut Session,
    clients: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut engines = PaneTerminalEngines::new(TerminalEngineKind::InterimText);
    for _ in 0..clients {
        serve_next(listener, session, &mut engines)?;
    }
    Ok(())
}

pub fn serve_n_with_output<O: ProcessOutput>(
    listener: &UnixListener,
    session: &mut Session,
    output: &mut O,
    clients: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut engines = PaneTerminalEngines::new(TerminalEngineKind::InterimText);
    for _ in 0..clients {
        serve_next_with_output(listener, session, Some(output), &mut engines)?;
    }
    Ok(())
}

pub fn serve_n_with_host<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
    clients: usize,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    serve_n_with_host_and_terminal_engine_kind(
        listener,
        session,
        host,
        clients,
        TerminalEngineKind::InterimText,
    )
}

pub fn serve_n_with_host_and_terminal_engine_kind<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
    clients: usize,
    terminal_engine_kind: TerminalEngineKind,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    let mut engines = PaneTerminalEngines::new(terminal_engine_kind);
    serve_n_with_host_and_engines(listener, session, host, clients, &mut engines)
}

pub fn serve_n_with_host_and_engines<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
    clients: usize,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    for _ in 0..clients {
        serve_next_with_host(listener, session, host, engines)?;
    }
    Ok(())
}

fn serve_next(
    listener: &UnixListener,
    session: &mut Session,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut stream, _) = listener.accept()?;
    let request = read_attach_request(&mut stream)?;
    serve_attached_client(&mut stream, request, session, None, engines)
}

fn serve_next_with_output(
    listener: &UnixListener,
    session: &mut Session,
    mut output: Option<&mut dyn ProcessOutput>,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut stream, _) = listener.accept()?;
    let request = read_attach_request(&mut stream)?;
    if let Some(output) = output.as_deref_mut() {
        poll_pane_output_with_engines(session, engines, output, "pane-1")?;
    }
    serve_attached_client(&mut stream, request, session, None, engines)
}

fn serve_next_with_host<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    let (mut stream, _) = listener.accept()?;
    let request = read_attach_request(&mut stream)?;
    poll_pane_output_with_engines(session, engines, host, "pane-1")?;
    serve_attached_client(&mut stream, request, session, Some(host), engines)
}

fn serve_live_one_with_host_and_engines<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
    engines: &mut PaneTerminalEngines,
    cycles: usize,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    let (mut stream, _) = listener.accept()?;
    let request = read_attach_request(&mut stream)?;
    poll_pane_output_with_engines(session, engines, host, "pane-1")?;
    serve_live_attached_client(&mut stream, request, session, host, engines, cycles)
}

fn serve_live_attached_client(
    stream: &mut UnixStream,
    request: AttachRequest,
    session: &mut Session,
    host: &mut dyn ProcessHostOutput,
    engines: &mut PaneTerminalEngines,
    cycles: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let pane_id = "pane-1";
    let mut seq = 1;
    let workspace_frame = session.workspace_tree_frame("local-client", seq);
    wire::write_default_frame(stream, &workspace_frame)?;
    seq += 1;

    let actor = request.actor();
    let presence_frame = session.presence_update_frame("local-client", seq, &actor);
    wire::write_default_frame(stream, &presence_frame)?;
    seq += 1;

    let mut known_surface_version =
        if let Some(response) = request.surface_response(session, pane_id) {
            if let Some(surface_frame) = surface_response_frame(session, pane_id, response, seq) {
                wire::write_default_frame(stream, &surface_frame)?;
                seq += 1;
            }
            session.surface_version(pane_id).unwrap_or_default()
        } else {
            session.surface_version(pane_id).unwrap_or_default()
        };

    for _ in 0..cycles {
        let input = loop {
            match read_optional_live_client_frame_from_stream(stream)? {
                LiveClientRead::Frame(LiveClientFrame::Scrollback(fetch)) => {
                    let Some(chunk) = session.scrollback_chunk_frame_for_pane(
                        "local-client",
                        seq,
                        &fetch.pane_id,
                        fetch.start_line,
                        fetch.line_count,
                    ) else {
                        continue;
                    };
                    wire::write_default_frame(stream, &chunk)?;
                    seq += 1;
                }
                LiveClientRead::Frame(LiveClientFrame::Resize(resize)) => {
                    if !Session::input_allowed(&actor) {
                        continue;
                    }
                    let policy = session
                        .pane_resize_policy(&resize.pane_id)
                        .unwrap_or(protocol::ResizePolicy::Fixed);
                    if !Session::resize_intent_allowed(policy, resize.reason) {
                        continue;
                    }
                    host.resize_pane(&resize.pane_id, resize.cols, resize.rows)?;
                    if session.commit_pane_resize_with_engine(
                        &resize.pane_id,
                        resize.cols,
                        resize.rows,
                        engines.engine_mut(&resize.pane_id),
                    ) {
                        let workspace_frame = session.workspace_tree_frame("local-client", seq);
                        wire::write_default_frame(stream, &workspace_frame)?;
                        seq += 1;
                    }
                }
                LiveClientRead::Frame(LiveClientFrame::Input(input)) => {
                    if Session::input_allowed(&actor) {
                        break Some(input);
                    }
                }
                LiveClientRead::NoFrame => break None,
                LiveClientRead::Closed => return Ok(()),
            }
        };
        if Session::input_allowed(&actor) {
            if let Some(input) = input {
                host.write_input(&input.pane_id, &input.bytes)?;
                poll_pane_output_until_quiet(session, engines, host, &input.pane_id)?;
            } else {
                poll_pane_output_until_quiet(session, engines, host, pane_id)?;
            }
        } else {
            poll_pane_output_until_quiet(session, engines, host, pane_id)?;
        }

        let current = session.surface_version(pane_id).unwrap_or_default();
        let patch_kind = session
            .surface_patch_kind(pane_id)
            .unwrap_or(protocol::PatchKind::ReplaceRows);
        if let Some(response) =
            surface_response_for_known_version(current, known_surface_version, patch_kind)
        {
            if let Some(surface_frame) = surface_response_frame(session, pane_id, response, seq) {
                wire::write_default_frame(stream, &surface_frame)?;
                seq += 1;
            }
            known_surface_version = current;
        }
    }

    Ok(())
}

fn read_optional_live_client_frame_from_stream(
    stream: &mut UnixStream,
) -> Result<LiveClientRead, Box<dyn std::error::Error>> {
    let previous_timeout = match stream.read_timeout() {
        Ok(timeout) => timeout,
        Err(err) if socket_closed_error(&err) => return Ok(LiveClientRead::Closed),
        Err(err) => return Err(err.into()),
    };
    if let Err(err) = stream.set_read_timeout(Some(Duration::from_millis(20))) {
        if socket_closed_error(&err) {
            return Ok(LiveClientRead::Closed);
        }
        return Err(err.into());
    }
    let read_result = match wire::read_default_frame(stream) {
        Ok(frame) => {
            let envelope = protocol::size_prefixed_root_as_envelope(&frame)?;
            match envelope.body_type() {
                protocol::EnvelopeBody::ResizeIntent => Ok(LiveClientRead::Frame(
                    LiveClientFrame::Resize(resize_intent_from_frame(&frame)?),
                )),
                protocol::EnvelopeBody::ScrollbackFetch => Ok(LiveClientRead::Frame(
                    LiveClientFrame::Scrollback(scrollback_fetch_from_frame(&frame)?),
                )),
                protocol::EnvelopeBody::InputEvent => Ok(LiveClientRead::Frame(
                    LiveClientFrame::Input(input_summary_from_frame(&frame)?),
                )),
                other => Err(format!("unexpected live client frame: {other:?}").into()),
            }
        }
        Err(wire::WireError::Io(err))
            if matches!(
                err.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            Ok(LiveClientRead::NoFrame)
        }
        Err(wire::WireError::Io(err))
            if matches!(
                err.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::BrokenPipe
            ) =>
        {
            Ok(LiveClientRead::Closed)
        }
        Err(err) => Err(err.into()),
    };
    match (read_result, stream.set_read_timeout(previous_timeout)) {
        (Err(err), _) => Err(err),
        (Ok(read), Ok(())) => Ok(read),
        (Ok(read), Err(err)) if socket_closed_error(&err) => Ok(read),
        (Ok(_), Err(err)) => Err(err.into()),
    }
}

fn socket_closed_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::InvalidInput
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::BrokenPipe
    ) || err.raw_os_error() == Some(22)
}

enum LiveClientRead {
    Frame(LiveClientFrame),
    NoFrame,
    Closed,
}

enum LiveClientFrame {
    Resize(ResizeIntentSummary),
    Scrollback(ScrollbackFetchSummary),
    Input(InputSummary),
}

fn serve_attached_client(
    stream: &mut UnixStream,
    request: AttachRequest,
    session: &mut Session,
    mut host: Option<&mut dyn ProcessHostOutput>,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>> {
    let workspace_frame = session.workspace_tree_frame("local-client", 1);
    wire::write_default_frame(stream, &workspace_frame)?;

    let actor = request.actor();
    let presence_frame = session.presence_update_frame("local-client", 2, &actor);
    wire::write_default_frame(stream, &presence_frame)?;

    if let Some(response) = request.surface_response(session, "pane-1") {
        if let Some(surface_frame) = surface_response_frame(session, "pane-1", response, 3) {
            wire::write_default_frame(stream, &surface_frame)?;
        }
        if Session::input_allowed(&actor) {
            let input = read_input_event_from_stream(stream)?;
            if let Some(host) = host.as_deref_mut() {
                host.write_input(&input.pane_id, &input.bytes)?;
                poll_pane_output_with_engines(session, engines, host, &input.pane_id)?;
            }
        }
        let fetch = read_scrollback_fetch_from_stream(stream)?;
        if let Some(chunk) = session.scrollback_chunk_frame_for_pane(
            "local-client",
            5,
            &fetch.pane_id,
            fetch.start_line,
            fetch.line_count,
        ) {
            wire::write_default_frame(stream, &chunk)?;
        }
    }
    Ok(())
}

fn surface_response_frame(
    session: &Session,
    pane_id: &str,
    response: SurfaceResponse,
    seq: u64,
) -> Option<Vec<u8>> {
    match response {
        SurfaceResponse::Snapshot => {
            session.pane_surface_frame_for_pane("local-client", seq, pane_id)
        }
        SurfaceResponse::Patch { base_version } => {
            session.pane_surface_patch_frame_for_pane("local-client", seq, pane_id, base_version)
        }
    }
}

pub fn poll_pane_output(
    session: &mut Session,
    output: &mut dyn ProcessOutput,
    pane_id: &str,
) -> Result<bool, HostError> {
    let mut engines = PaneTerminalEngines::new(TerminalEngineKind::InterimText);
    poll_pane_output_with_engines(session, &mut engines, output, pane_id)
}

pub fn poll_pane_output_with_engines(
    session: &mut Session,
    engines: &mut PaneTerminalEngines,
    output: &mut dyn ProcessOutput,
    pane_id: &str,
) -> Result<bool, HostError> {
    let mut buffer = [0_u8; 4096];
    let mut pumped = Vec::new();
    loop {
        let count = output.try_read_output(pane_id, &mut buffer)?;
        if count == 0 {
            break;
        }
        pumped.extend_from_slice(&buffer[..count]);
    }

    if pumped.is_empty() {
        return Ok(false);
    }

    Ok(session.apply_pane_output_with_engine(pane_id, &pumped, engines.engine_mut(pane_id)))
}

fn poll_pane_output_until_quiet(
    session: &mut Session,
    engines: &mut PaneTerminalEngines,
    output: &mut dyn ProcessOutput,
    pane_id: &str,
) -> Result<bool, HostError> {
    let deadline = Instant::now() + Duration::from_millis(120);
    let mut quiet_since = None;
    let mut changed = false;

    loop {
        if poll_pane_output_with_engines(session, engines, output, pane_id)? {
            changed = true;
            quiet_since = None;
        } else if changed {
            let quiet_start = quiet_since.get_or_insert_with(Instant::now);
            if quiet_start.elapsed() >= Duration::from_millis(20) {
                return Ok(true);
            }
        } else if Instant::now() >= deadline {
            return Ok(false);
        }

        if Instant::now() >= deadline {
            return Ok(changed);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub fn attach(path: &Path) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    attach_with_known_surfaces(path, Vec::new())
}

pub fn attach_with_known_surfaces(
    path: &Path,
    known_surfaces: Vec<KnownSurfaceVersion>,
) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    attach_with_options(
        path,
        AttachRequest {
            actor_id: "local-actor".to_owned(),
            user_id: "local-user".to_owned(),
            display_name: "local".to_owned(),
            mode: AttachMode::ReadWrite,
            focused_pane_id: Some("pane-1".to_owned()),
            known_surfaces,
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachOptions {
    pub request: AttachRequest,
    pub input_text: Option<String>,
    pub scrollback_start_line: u64,
    pub scrollback_line_count: u32,
    pub connect_timeout: Option<Duration>,
}

impl Default for AttachOptions {
    fn default() -> Self {
        Self {
            request: AttachRequest {
                actor_id: "local-actor".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
            input_text: Some("a".to_owned()),
            scrollback_start_line: 1,
            scrollback_line_count: 2,
            connect_timeout: None,
        }
    }
}

pub fn attach_with_options(
    path: &Path,
    request: AttachRequest,
) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    attach_with_client_options(
        path,
        AttachOptions {
            request,
            ..AttachOptions::default()
        },
    )
}

pub fn attach_with_client_options(
    path: &Path,
    options: AttachOptions,
) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    let mut stream = match options.connect_timeout {
        Some(timeout) => connect_to_daemon_with_timeout(path, timeout)?,
        None => connect_to_daemon(path)?,
    };
    let mode = options.request.mode;
    write_attach_request(&mut stream, &options.request)?;
    let snapshot = attach_from_stream(&mut stream)?;
    if snapshot.surface.is_some() {
        if mode == AttachMode::ReadWrite {
            if let Some(input_text) = options.input_text.as_deref() {
                send_key_input(&mut stream, "pane-1", input_text)?;
            }
        }
        send_scrollback_fetch(
            &mut stream,
            "pane-1",
            options.scrollback_start_line,
            options.scrollback_line_count,
        )?;
        let scrollback = read_scrollback_chunk_from_stream(&mut stream)?;
        return Ok(AttachSnapshot {
            scrollback: Some(scrollback),
            ..snapshot
        });
    }
    Ok(snapshot)
}

pub fn attach_render_once(
    path: &Path,
    mut options: AttachOptions,
    client_state: &mut ClientAttachState,
) -> Result<RenderedAttach, Box<dyn std::error::Error>> {
    options.request.known_surfaces = client_state.known_surfaces();
    let snapshot = attach_with_client_options(path, options)?;
    client_state.render_attach(snapshot)
}

pub fn attach_from_stream(
    stream: &mut UnixStream,
) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    let workspace_frame = wire::read_default_frame(stream)?;
    let workspace = workspace_summary_from_frame(&workspace_frame)?;

    let presence_frame = wire::read_default_frame(stream)?;
    let presence = presence_from_frame(&presence_frame)?;

    let surface = match wire::read_default_frame(stream) {
        Ok(surface_frame) => Some(surface_update_from_frame(&surface_frame)?),
        Err(wire::WireError::Io(err))
            if matches!(
                err.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            None
        }
        Err(err) => return Err(err.into()),
    };

    Ok(AttachSnapshot {
        workspace,
        presence,
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

pub fn send_raw_input(
    stream: &mut UnixStream,
    pane_id: &str,
    bytes: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let frame =
        Session::initial().raw_input_frame("local-client", 3, "local-actor", pane_id, 1, bytes);
    wire::write_default_frame(stream, &frame)?;
    Ok(())
}

pub fn send_resize_intent(
    stream: &mut UnixStream,
    pane_id: &str,
    cols: u32,
    rows: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame = Session::initial().resize_intent_frame(
        "local-client",
        3,
        "local-actor",
        pane_id,
        cols,
        rows,
        protocol::ResizeReason::FrontendViewport,
    );
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
        resize_policy: pane.resize_policy(),
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
            let row_updates = decoded_surface_rows(rows.len(), |index| {
                let row = rows.get(index);
                decoded_surface_row(row.row(), row.runs(), row.dirty_hash())
            });
            let text = render_decoded_rows(&row_updates);
            Ok(SurfaceUpdate {
                kind: SurfaceUpdateKind::Snapshot,
                pane_id: snapshot.pane_id().unwrap_or_default().to_owned(),
                version: snapshot.version(),
                base_version: None,
                patch_kind: None,
                cols: Some(snapshot.cols()),
                rows: Some(snapshot.rows()),
                surface: Some(snapshot.surface()),
                cursor: snapshot.cursor().map(CursorSummary::from_protocol),
                row_updates,
                text,
            })
        }
        protocol::EnvelopeBody::PaneSurfacePatch => {
            let patch = envelope
                .body_as_pane_surface_patch()
                .ok_or("missing pane surface patch body")?;
            let rows = patch
                .row_updates()
                .ok_or("pane surface patch has no rows")?;
            let row_updates = decoded_surface_rows(rows.len(), |index| {
                let row = rows.get(index);
                decoded_surface_row(row.row(), row.runs(), row.dirty_hash())
            });
            let text = render_decoded_rows(&row_updates);
            Ok(SurfaceUpdate {
                kind: SurfaceUpdateKind::Patch,
                pane_id: patch.pane_id().unwrap_or_default().to_owned(),
                version: patch.version(),
                base_version: Some(patch.base_version()),
                patch_kind: Some(patch.kind()),
                cols: None,
                rows: None,
                surface: None,
                cursor: patch.cursor().map(CursorSummary::from_protocol),
                row_updates,
                text,
            })
        }
        other => Err(format!("unexpected envelope body: {other:?}").into()),
    }
}

fn decoded_surface_rows<F>(len: usize, mut row: F) -> Vec<SurfaceRowUpdate>
where
    F: FnMut(usize) -> SurfaceRowUpdate,
{
    (0..len).map(&mut row).collect()
}

fn decoded_surface_row(
    row: u32,
    runs: Option<flatbuffers::Vector<'_, flatbuffers::ForwardsUOffset<protocol::CellRun<'_>>>>,
    dirty_hash: u64,
) -> SurfaceRowUpdate {
    let runs = runs.map(decoded_cell_runs).unwrap_or_default();
    SurfaceRowUpdate {
        row,
        text: render_run_summaries(&runs),
        runs,
        dirty_hash,
    }
}

fn render_cell_runs(
    runs: flatbuffers::Vector<'_, flatbuffers::ForwardsUOffset<protocol::CellRun<'_>>>,
) -> String {
    render_run_summaries(&decoded_cell_runs(runs))
}

fn decoded_cell_runs(
    runs: flatbuffers::Vector<'_, flatbuffers::ForwardsUOffset<protocol::CellRun<'_>>>,
) -> Vec<CellRunSummary> {
    let mut decoded = Vec::with_capacity(runs.len());
    for run_index in 0..runs.len() {
        let run = runs.get(run_index);
        let cell_widths = run
            .cell_widths()
            .map(|widths| (0..widths.len()).map(|index| widths.get(index)).collect())
            .unwrap_or_default();
        decoded.push(CellRunSummary {
            text: run.text_utf8().unwrap_or_default().to_owned(),
            cell_widths,
            style_id: run.style_id(),
            flags: run.flags(),
            hyperlink_id: run.hyperlink_id(),
        });
    }
    decoded
}

fn render_decoded_rows(rows: &[SurfaceRowUpdate]) -> String {
    let mut rendered = String::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        rendered.push_str(&row.text);
    }
    rendered
}

fn render_run_summaries(runs: &[CellRunSummary]) -> String {
    let mut rendered = String::new();
    for run in runs {
        rendered.push_str(&run.text);
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

pub fn read_surface_update_from_stream(
    stream: &mut UnixStream,
) -> Result<SurfaceUpdate, Box<dyn std::error::Error>> {
    let frame = wire::read_default_frame(stream)?;
    surface_update_from_frame(&frame)
}

pub fn read_optional_surface_update_from_stream(
    stream: &mut UnixStream,
) -> Result<Option<SurfaceUpdate>, Box<dyn std::error::Error>> {
    match wire::read_default_frame(stream) {
        Ok(frame) => Ok(Some(surface_update_from_frame(&frame)?)),
        Err(wire::WireError::Io(err))
            if matches!(
                err.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

pub fn read_live_surface_update_from_stream(
    stream: &mut UnixStream,
) -> Result<LiveSurfaceRead, Box<dyn std::error::Error>> {
    match wire::read_default_frame(stream) {
        Ok(frame) => {
            let envelope = protocol::size_prefixed_root_as_envelope(&frame)?;
            match envelope.body_type() {
                protocol::EnvelopeBody::WorkspaceTreeSnapshot => Ok(LiveSurfaceRead::Workspace(
                    workspace_summary_from_frame(&frame)?,
                )),
                protocol::EnvelopeBody::PaneSurfaceSnapshot
                | protocol::EnvelopeBody::PaneSurfacePatch => {
                    Ok(LiveSurfaceRead::Update(surface_update_from_frame(&frame)?))
                }
                other => Err(format!("unexpected live server frame: {other:?}").into()),
            }
        }
        Err(wire::WireError::Io(err))
            if matches!(
                err.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            Ok(LiveSurfaceRead::NoFrame)
        }
        Err(wire::WireError::Io(err))
            if matches!(
                err.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::BrokenPipe
            ) =>
        {
            Ok(LiveSurfaceRead::Closed)
        }
        Err(err) => Err(err.into()),
    }
}

pub fn input_summary_from_frame(frame: &[u8]) -> Result<InputSummary, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::InputEvent {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }

    let input = envelope
        .body_as_input_event()
        .ok_or("missing input event body")?;
    let bytes = match input.kind() {
        protocol::InputKind::Key => input
            .key()
            .and_then(|key| key.text_utf8())
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
        protocol::InputKind::RawBytes => input
            .raw()
            .and_then(|raw| raw.bytes())
            .map(|bytes| bytes.iter().collect())
            .unwrap_or_default(),
        other => return Err(format!("unexpected input kind: {other:?}").into()),
    };
    Ok(InputSummary {
        pane_id: input.pane_id().unwrap_or_default().to_owned(),
        actor_id: input.actor_id().unwrap_or_default().to_owned(),
        input_seq: input.input_seq(),
        text: String::from_utf8_lossy(&bytes).into_owned(),
        bytes,
    })
}

pub fn resize_intent_from_frame(
    frame: &[u8],
) -> Result<ResizeIntentSummary, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::ResizeIntent {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }

    let resize = envelope
        .body_as_resize_intent()
        .ok_or("missing resize intent body")?;
    Ok(ResizeIntentSummary {
        pane_id: resize.pane_id().unwrap_or_default().to_owned(),
        actor_id: resize.actor_id().unwrap_or_default().to_owned(),
        cols: resize.desired_cols(),
        rows: resize.desired_rows(),
        reason: resize.reason(),
    })
}

pub fn presence_from_frame(frame: &[u8]) -> Result<PresenceSummary, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::PresenceUpdate {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }

    let presence = envelope
        .body_as_presence_update()
        .ok_or("missing presence update body")?;
    Ok(PresenceSummary {
        actor_id: presence.actor_id().unwrap_or_default().to_owned(),
        user_id: presence.user_id().unwrap_or_default().to_owned(),
        display_name: presence.display_name().unwrap_or_default().to_owned(),
        mode: attach_mode_from_protocol(presence.mode()),
        focused_pane_id: presence.focused_pane_id().map(ToOwned::to_owned),
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
    let frame = request.frame();
    wire::write_frame(writer, &frame, ATTACH_MAX_FRAME_LEN).map_err(wire_error_to_io)
}

pub fn read_attach_request<R: Read>(reader: &mut R) -> io::Result<AttachRequest> {
    let frame = wire::read_frame(reader, ATTACH_MAX_FRAME_LEN).map_err(wire_error_to_io)?;
    attach_request_from_frame(&frame)
}

fn attach_request_from_frame(frame: &[u8]) -> io::Result<AttachRequest> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid attach frame: {err}"),
        )
    })?;
    if envelope.body_type() != protocol::EnvelopeBody::AttachRequest {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unexpected attach envelope body: {:?}",
                envelope.body_type()
            ),
        ));
    }

    let request = envelope
        .body_as_attach_request()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing attach body"))?;
    let known = request.known_surfaces();
    let mut known_surfaces = Vec::with_capacity(known.map(|known| known.len()).unwrap_or(0));
    if let Some(known) = known {
        for index in 0..known.len() {
            let surface = known.get(index);
            known_surfaces.push(KnownSurfaceVersion {
                pane_id: surface.pane_id().unwrap_or_default().to_owned(),
                version: surface.version(),
            });
        }
    }

    Ok(AttachRequest {
        actor_id: request.actor_id().unwrap_or("local-actor").to_owned(),
        user_id: request.user_id().unwrap_or("local-user").to_owned(),
        display_name: request.display_name().unwrap_or("local").to_owned(),
        mode: attach_mode_from_protocol(request.mode()),
        focused_pane_id: request.focused_pane_id().map(ToOwned::to_owned),
        known_surfaces,
    })
}

fn wire_error_to_io(err: wire::WireError) -> io::Error {
    match err {
        wire::WireError::Io(err) => err,
        err => io::Error::new(io::ErrorKind::InvalidData, err),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRequest {
    pub actor_id: String,
    pub user_id: String,
    pub display_name: String,
    pub mode: AttachMode,
    pub focused_pane_id: Option<String>,
    pub known_surfaces: Vec<KnownSurfaceVersion>,
}

impl AttachRequest {
    fn frame(&self) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let mut known_surface_offsets = Vec::with_capacity(self.known_surfaces.len());
        for surface in &self.known_surfaces {
            let pane_id = builder.create_string(&surface.pane_id);
            let known_surface = protocol::KnownPaneSurfaceVersion::create(
                &mut builder,
                &protocol::KnownPaneSurfaceVersionArgs {
                    pane_id: Some(pane_id),
                    version: surface.version,
                },
            );
            known_surface_offsets.push(known_surface);
        }
        let known_surfaces = builder.create_vector(&known_surface_offsets);

        let actor_id = builder.create_string(&self.actor_id);
        let user_id = builder.create_string(&self.user_id);
        let display_name = builder.create_string(&self.display_name);
        let focused_pane_id = self
            .focused_pane_id
            .as_ref()
            .map(|focused_pane_id| builder.create_string(focused_pane_id));
        let request = protocol::AttachRequest::create(
            &mut builder,
            &protocol::AttachRequestArgs {
                actor_id: Some(actor_id),
                user_id: Some(user_id),
                display_name: Some(display_name),
                mode: attach_mode_as_protocol(self.mode),
                focused_pane_id,
                known_surfaces: Some(known_surfaces),
            },
        );

        let session_id = builder.create_string("local");
        let connection_id = builder.create_string("local-client");
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(session_id),
                connection_id: Some(connection_id),
                seq: 0,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::AttachRequest,
                body: Some(request.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    fn actor(&self) -> Actor {
        Actor {
            id: self.actor_id.clone(),
            user_id: self.user_id.clone(),
            display_name: self.display_name.clone(),
            mode: self.mode,
            focused_pane_id: self.focused_pane_id.clone(),
        }
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

        let patch_kind = session
            .surface_patch_kind(pane_id)
            .unwrap_or(protocol::PatchKind::ReplaceRows);
        surface_response_for_known_version(current, known.version, patch_kind)
    }
}

fn surface_response_for_known_version(
    current: u64,
    known: u64,
    patch_kind: protocol::PatchKind,
) -> Option<SurfaceResponse> {
    if known == current {
        None
    } else if patch_kind == protocol::PatchKind::FullRefreshRequired {
        Some(SurfaceResponse::Snapshot)
    } else if known.checked_add(1) == Some(current) {
        Some(SurfaceResponse::Patch {
            base_version: known,
        })
    } else {
        Some(SurfaceResponse::Snapshot)
    }
}

fn attach_mode_as_protocol(mode: AttachMode) -> protocol::AttachMode {
    match mode {
        AttachMode::ReadOnly => protocol::AttachMode::ReadOnly,
        AttachMode::ReadWrite => protocol::AttachMode::ReadWrite,
    }
}

fn attach_mode_from_protocol(mode: protocol::AttachMode) -> AttachMode {
    if mode == protocol::AttachMode::ReadWrite {
        AttachMode::ReadWrite
    } else {
        AttachMode::ReadOnly
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub presence: PresenceSummary,
    pub surface: Option<SurfaceUpdate>,
    pub scrollback: Option<ScrollbackChunkSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedAttach {
    pub workspace: WorkspaceSummary,
    pub surface_text: Option<String>,
    pub scrollback: Option<ScrollbackChunkSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceUpdate {
    pub kind: SurfaceUpdateKind,
    pub pane_id: String,
    pub version: u64,
    pub base_version: Option<u64>,
    pub patch_kind: Option<protocol::PatchKind>,
    pub cols: Option<u32>,
    pub rows: Option<u32>,
    pub surface: Option<protocol::SurfaceKind>,
    pub cursor: Option<CursorSummary>,
    pub row_updates: Vec<SurfaceRowUpdate>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveSurfaceRead {
    Workspace(WorkspaceSummary),
    Update(SurfaceUpdate),
    NoFrame,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceUpdateKind {
    Snapshot,
    Patch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorSummary {
    pub row: u32,
    pub col: u32,
    pub visible: bool,
    pub shape: protocol::CursorShape,
}

impl CursorSummary {
    fn from_protocol(cursor: protocol::CursorState<'_>) -> Self {
        Self {
            row: cursor.row(),
            col: cursor.col(),
            visible: cursor.visible(),
            shape: cursor.shape(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceRowUpdate {
    pub row: u32,
    pub text: String,
    pub runs: Vec<CellRunSummary>,
    pub dirty_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRunSummary {
    pub text: String,
    pub cell_widths: Vec<u8>,
    pub style_id: u32,
    pub flags: u32,
    pub hyperlink_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPaneSurface {
    pub pane_id: String,
    pub version: u64,
    pub cols: u32,
    pub rows: u32,
    pub surface: protocol::SurfaceKind,
    pub cursor: Option<CursorSummary>,
    row_text: Vec<String>,
}

impl ClientPaneSurface {
    pub fn from_snapshot(update: &SurfaceUpdate) -> Result<Self, Box<dyn std::error::Error>> {
        let mut surface = Self {
            pane_id: update.pane_id.clone(),
            version: update.version,
            cols: update.cols.ok_or("surface snapshot missing cols")?,
            rows: update.rows.ok_or("surface snapshot missing rows")?,
            surface: update.surface.unwrap_or(protocol::SurfaceKind::Main),
            cursor: update.cursor,
            row_text: Vec::new(),
        };
        let row_count =
            usize::try_from(surface.rows).map_err(|_| "surface row count does not fit in usize")?;
        surface.row_text.resize(row_count, String::new());
        surface.apply_rows(&update.row_updates)?;
        Ok(surface)
    }

    pub fn apply_update(
        &mut self,
        update: &SurfaceUpdate,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match update.kind {
            SurfaceUpdateKind::Snapshot => {
                *self = Self::from_snapshot(update)?;
                Ok(())
            }
            SurfaceUpdateKind::Patch => self.apply_patch(update),
        }
    }

    pub fn apply_patch(
        &mut self,
        update: &SurfaceUpdate,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if update.pane_id != self.pane_id {
            return Err(format!(
                "surface patch pane mismatch: expected {}, got {}",
                self.pane_id, update.pane_id
            )
            .into());
        }
        if update.base_version != Some(self.version) {
            return Err(format!(
                "surface patch base version mismatch: expected {}, got {:?}",
                self.version, update.base_version
            )
            .into());
        }
        if update.patch_kind == Some(protocol::PatchKind::FullRefreshRequired) {
            return Err("surface patch requires full refresh".into());
        }
        if update.patch_kind == Some(protocol::PatchKind::CursorOnly) {
            self.cursor = update.cursor;
            self.version = update.version;
            return Ok(());
        }
        if update.patch_kind != Some(protocol::PatchKind::ReplaceRows) {
            return Err(format!("unsupported surface patch kind: {:?}", update.patch_kind).into());
        }
        self.apply_rows(&update.row_updates)?;
        self.cursor = update.cursor;
        self.version = update.version;
        Ok(())
    }

    pub fn render_text(&self) -> String {
        let visible_rows = self
            .row_text
            .iter()
            .rposition(|row| !row.is_empty())
            .map(|index| index + 1)
            .unwrap_or(0);
        self.row_text[..visible_rows].join("\n")
    }

    fn apply_rows(&mut self, rows: &[SurfaceRowUpdate]) -> Result<(), Box<dyn std::error::Error>> {
        for row in rows {
            let index =
                usize::try_from(row.row).map_err(|_| "surface row index does not fit in usize")?;
            let Some(target) = self.row_text.get_mut(index) else {
                return Err(format!(
                    "surface row {} is outside {} row surface",
                    row.row, self.rows
                )
                .into());
            };
            *target = row.text.clone();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientAttachState {
    surfaces: Vec<ClientPaneSurface>,
}

impl ClientAttachState {
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(encoded) => Self::decode(&encoded),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, self.encode())
    }

    pub fn known_surfaces(&self) -> Vec<KnownSurfaceVersion> {
        self.surfaces
            .iter()
            .map(|surface| KnownSurfaceVersion {
                pane_id: surface.pane_id.clone(),
                version: surface.version,
            })
            .collect()
    }

    pub fn render_attach(
        &mut self,
        snapshot: AttachSnapshot,
    ) -> Result<RenderedAttach, Box<dyn std::error::Error>> {
        let surface_text = match snapshot.surface.as_ref() {
            Some(update) => Some(self.apply_surface_update(update)?),
            None => None,
        };

        Ok(RenderedAttach {
            workspace: snapshot.workspace,
            surface_text,
            scrollback: snapshot.scrollback,
        })
    }

    pub fn render_surface_update(
        &mut self,
        update: &SurfaceUpdate,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.apply_surface_update(update)
    }

    pub fn cached_surface_text(&self, pane_id: &str) -> Option<String> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(ClientPaneSurface::render_text)
    }

    fn apply_surface_update(
        &mut self,
        update: &SurfaceUpdate,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if let Some(surface) = self
            .surfaces
            .iter_mut()
            .find(|surface| surface.pane_id == update.pane_id)
        {
            surface.apply_update(update)?;
            return Ok(surface.render_text());
        }

        if update.kind != SurfaceUpdateKind::Snapshot {
            return Err(format!(
                "cannot apply pane {} patch without a cached snapshot",
                update.pane_id
            )
            .into());
        }

        let surface = ClientPaneSurface::from_snapshot(update)?;
        let rendered = surface.render_text();
        self.surfaces.push(surface);
        Ok(rendered)
    }

    fn encode(&self) -> String {
        let mut encoded = String::from("NMUX_CLIENT_STATE 1\n");
        for surface in &self.surfaces {
            encoded.push_str("surface ");
            encoded.push_str(&hex_encode(surface.pane_id.as_bytes()));
            encoded.push(' ');
            encoded.push_str(&surface.version.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.cols.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.rows.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.surface.0.to_string());
            encoded.push('\n');
            match surface.cursor {
                Some(cursor) => {
                    encoded.push_str("cursor ");
                    encoded.push_str(&cursor.row.to_string());
                    encoded.push(' ');
                    encoded.push_str(&cursor.col.to_string());
                    encoded.push(' ');
                    encoded.push_str(if cursor.visible { "1" } else { "0" });
                    encoded.push(' ');
                    encoded.push_str(&cursor.shape.0.to_string());
                    encoded.push('\n');
                }
                None => encoded.push_str("cursor none\n"),
            }
            for (index, row) in surface.row_text.iter().enumerate() {
                encoded.push_str("row ");
                encoded.push_str(&index.to_string());
                encoded.push(' ');
                encoded.push_str(&hex_encode(row.as_bytes()));
                encoded.push('\n');
            }
            encoded.push_str("end\n");
        }
        encoded
    }

    fn decode(encoded: &str) -> io::Result<Self> {
        let mut lines = encoded.lines();
        if lines.next() != Some("NMUX_CLIENT_STATE 1") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid nmux client state header",
            ));
        }

        let mut surfaces = Vec::new();
        while let Some(line) = lines.next() {
            let mut parts = line.split(' ');
            let (
                Some("surface"),
                Some(pane_id),
                Some(version),
                Some(cols),
                Some(rows),
                surface,
                None,
            ) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            )
            else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid nmux client state surface line",
                ));
            };

            let pane_id = String::from_utf8(hex_decode(pane_id)?)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            let version = parse_state_u64(version)?;
            let cols = parse_state_u32(cols)?;
            let rows = parse_state_u32(rows)?;
            let surface = surface
                .map(parse_state_i8)
                .transpose()?
                .map(protocol::SurfaceKind)
                .unwrap_or(protocol::SurfaceKind::Main);
            let row_count = usize::try_from(rows).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "surface rows too large")
            })?;
            let mut cursor = None;
            let mut row_text = vec![String::new(); row_count];

            loop {
                let Some(line) = lines.next() else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unterminated nmux client state surface",
                    ));
                };
                if line == "end" {
                    break;
                }

                let mut parts = line.split(' ');
                match (
                    parts.next(),
                    parts.next(),
                    parts.next(),
                    parts.next(),
                    parts.next(),
                    parts.next(),
                ) {
                    (Some("cursor"), Some("none"), None, None, None, None) => cursor = None,
                    (Some("cursor"), Some(row), Some(col), Some(visible), Some(shape), None) => {
                        cursor = Some(CursorSummary {
                            row: parse_state_u32(row)?,
                            col: parse_state_u32(col)?,
                            visible: parse_state_bool(visible)?,
                            shape: protocol::CursorShape(parse_state_i8(shape)?),
                        });
                    }
                    (Some("row"), Some(row), Some(text), None, None, None) => {
                        let row = parse_state_usize(row)?;
                        let Some(target) = row_text.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row index outside surface",
                            ));
                        };
                        *target = String::from_utf8(hex_decode(text)?)
                            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid nmux client state surface body line",
                        ));
                    }
                }
            }

            surfaces.push(ClientPaneSurface {
                pane_id,
                version,
                cols,
                rows,
                surface,
                cursor,
                row_text,
            });
        }

        Ok(Self { surfaces })
    }
}

fn parse_state_u64(value: &str) -> io::Result<u64> {
    value
        .parse()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn parse_state_u32(value: &str) -> io::Result<u32> {
    value
        .parse()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn parse_state_i8(value: &str) -> io::Result<i8> {
    value
        .parse()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn parse_state_bool(value: &str) -> io::Result<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid boolean in client state",
        )),
    }
}

fn parse_state_usize(value: &str) -> io::Result<usize> {
    value
        .parse()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn hex_decode(encoded: &str) -> io::Result<Vec<u8>> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSummary {
    pub pane_id: String,
    pub actor_id: String,
    pub input_seq: u64,
    pub text: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResizeIntentSummary {
    pub pane_id: String,
    pub actor_id: String,
    pub cols: u32,
    pub rows: u32,
    pub reason: protocol::ResizeReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceSummary {
    pub actor_id: String,
    pub user_id: String,
    pub display_name: String,
    pub mode: AttachMode,
    pub focused_pane_id: Option<String>,
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
    pub resize_policy: protocol::ResizePolicy,
}

impl WorkspaceSummary {
    pub fn display_line(&self) -> String {
        format!(
            "session={} tab={} pane={} size={}x{} resize={}",
            self.session_id,
            self.tab_id,
            self.pane_id,
            self.cols,
            self.rows,
            resize_policy_label(self.resize_policy)
        )
    }
}

fn resize_policy_label(policy: protocol::ResizePolicy) -> &'static str {
    match policy {
        protocol::ResizePolicy::Fixed => "fixed",
        protocol::ResizePolicy::Leader => "leader",
        protocol::ResizePolicy::ActiveClient => "active-client",
        protocol::ResizePolicy::Manual => "manual",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use nmux_core::host::{
        HostError, HostEvent, HostSpec, PaneProcess, PlanningHost, ProcessHost, ProcessOutput,
        ProcessStatus, RecordingOutput,
    };
    use nmux_core::terminal::{TerminalEngine, TerminalInput, TerminalUpdate};

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
            default_socket_path_from(Some(OsString::from("/run/user/1000")), 1000),
            PathBuf::from("/run/user/1000")
                .join("nmux")
                .join("nmuxd.sock")
        );
    }

    #[test]
    fn default_socket_path_fallback_is_stable_for_user() {
        let first = default_socket_path_from(None, 501);
        let second = default_socket_path_from(None, 501);

        assert_eq!(first, second);
        assert_eq!(first, PathBuf::from("/tmp/nmux-501").join("nmuxd.sock"));
    }

    #[test]
    fn default_socket_path_falls_back_for_invalid_runtime_dir() {
        assert_eq!(
            default_socket_path_from(Some(OsString::from("")), 501),
            PathBuf::from("/tmp/nmux-501").join("nmuxd.sock")
        );
        assert_eq!(
            default_socket_path_from(Some(OsString::from("relative-runtime")), 501),
            PathBuf::from("/tmp/nmux-501").join("nmuxd.sock")
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

    fn surface_update(
        kind: SurfaceUpdateKind,
        version: u64,
        base_version: Option<u64>,
        rows: Vec<SurfaceRowUpdate>,
    ) -> SurfaceUpdate {
        SurfaceUpdate {
            kind,
            pane_id: "pane-1".to_owned(),
            version,
            base_version,
            patch_kind: match kind {
                SurfaceUpdateKind::Snapshot => None,
                SurfaceUpdateKind::Patch => Some(protocol::PatchKind::ReplaceRows),
            },
            cols: match kind {
                SurfaceUpdateKind::Snapshot => Some(80),
                SurfaceUpdateKind::Patch => None,
            },
            rows: match kind {
                SurfaceUpdateKind::Snapshot => Some(3),
                SurfaceUpdateKind::Patch => None,
            },
            surface: match kind {
                SurfaceUpdateKind::Snapshot => Some(protocol::SurfaceKind::Main),
                SurfaceUpdateKind::Patch => None,
            },
            cursor: None,
            text: render_decoded_rows(&rows),
            row_updates: rows,
        }
    }

    fn surface_row(row: u32, text: &str) -> SurfaceRowUpdate {
        SurfaceRowUpdate {
            row,
            text: text.to_owned(),
            runs: vec![CellRunSummary {
                text: text.to_owned(),
                cell_widths: text.chars().map(|_| 1).collect(),
                style_id: 0,
                flags: 0,
                hyperlink_id: 0,
            }],
            dirty_hash: u64::from(row),
        }
    }

    fn presence_summary(mode: AttachMode) -> PresenceSummary {
        PresenceSummary {
            actor_id: "local-actor".to_owned(),
            user_id: "local-user".to_owned(),
            display_name: "local".to_owned(),
            mode,
            focused_pane_id: Some("pane-1".to_owned()),
        }
    }

    fn read_only_attach_options() -> AttachOptions {
        AttachOptions {
            request: AttachRequest {
                actor_id: "local-actor".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadOnly,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
            input_text: None,
            scrollback_start_line: 1,
            scrollback_line_count: 2,
            connect_timeout: None,
        }
    }

    #[test]
    fn serves_initial_attach_snapshot_over_unix_socket() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
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
                resize_policy: protocol::ResizePolicy::Fixed,
            }
        );
        assert_eq!(
            snapshot.workspace.display_line(),
            "session=local tab=tab-1 pane=pane-1 size=80x24 resize=fixed"
        );
        assert_eq!(
            snapshot.presence,
            PresenceSummary {
                actor_id: "local-actor".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
            }
        );
        let surface = snapshot.surface.as_ref().expect("surface update");
        assert_eq!(surface.kind, SurfaceUpdateKind::Snapshot);
        assert_eq!(surface.pane_id, "pane-1");
        assert_eq!(surface.version, 2);
        assert_eq!(surface.cols, Some(80));
        assert_eq!(surface.rows, Some(24));
        assert_eq!(surface.surface, Some(protocol::SurfaceKind::Main));
        assert_eq!(surface.text, "nmux pane-1\nserver-owned terminal state");
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
    fn client_surface_applies_patch_by_row_index() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![
                surface_row(0, "top"),
                surface_row(1, "middle"),
                surface_row(2, "bottom"),
            ],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");

        let patch = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row(2, "new bottom"), surface_row(0, "new top")],
        );
        surface.apply_patch(&patch).expect("apply patch");

        assert_eq!(surface.version, 2);
        assert_eq!(surface.render_text(), "new top\nmiddle\nnew bottom");
    }

    #[test]
    fn client_surface_applies_cursor_only_patch_without_rows() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top"), surface_row(1, "bottom")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut patch = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        patch.patch_kind = Some(protocol::PatchKind::CursorOnly);
        patch.cursor = Some(CursorSummary {
            row: 1,
            col: 6,
            visible: true,
            shape: protocol::CursorShape::Beam,
        });

        surface.apply_patch(&patch).expect("apply patch");

        assert_eq!(surface.version, 2);
        assert_eq!(surface.render_text(), "top\nbottom");
        assert_eq!(surface.cursor, patch.cursor);
    }

    #[test]
    fn client_surface_rejects_mode_only_patch_without_mode_fields() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut patch = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        patch.patch_kind = Some(protocol::PatchKind::ModeOnly);

        let err = surface
            .apply_patch(&patch)
            .expect_err("unsupported mode patch");

        assert!(err.to_string().contains("unsupported surface patch kind"));
        assert_eq!(surface.version, 1);
        assert_eq!(surface.render_text(), "top");
    }

    #[test]
    fn client_surface_rejects_patch_base_mismatch() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            3,
            None,
            vec![surface_row(0, "current")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");

        let patch = surface_update(
            SurfaceUpdateKind::Patch,
            4,
            Some(2),
            vec![surface_row(0, "stale")],
        );

        let err = surface.apply_patch(&patch).expect_err("base mismatch");
        assert!(err.to_string().contains("base version mismatch"));
        assert_eq!(surface.version, 3);
        assert_eq!(surface.render_text(), "current");
    }

    #[test]
    fn full_refresh_required_surface_response_uses_snapshot() {
        assert_eq!(
            surface_response_for_known_version(2, 1, protocol::PatchKind::FullRefreshRequired),
            Some(SurfaceResponse::Snapshot)
        );
    }

    #[test]
    fn client_attach_state_tracks_known_surface_and_renders_patch() {
        let mut state = ClientAttachState::default();
        let mut session = Session::initial();
        let snapshot_update =
            surface_update_from_frame(&session.pane_surface_frame("local-client", 3))
                .expect("snapshot update");
        let rendered = state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                surface: Some(snapshot_update),
                scrollback: None,
            })
            .expect("render snapshot");

        assert_eq!(
            rendered.surface_text.as_deref(),
            Some("nmux pane-1\nserver-owned terminal state")
        );
        assert_eq!(
            state.known_surfaces(),
            vec![KnownSurfaceVersion {
                pane_id: "pane-1".to_owned(),
                version: 2,
            }]
        );

        session.apply_pane_output("pane-1", b"new output\n");
        let patch_update =
            surface_update_from_frame(&session.pane_surface_patch_frame("local-client", 4, 2))
                .expect("patch update");
        let rendered = state
            .render_attach(AttachSnapshot {
                workspace: rendered.workspace,
                presence: presence_summary(AttachMode::ReadWrite),
                surface: Some(patch_update),
                scrollback: None,
            })
            .expect("render patch");

        assert_eq!(
            rendered.surface_text.as_deref(),
            Some("booting nmux workspace\nnmux pane-1\nserver-owned terminal state\nnew output")
        );
        assert_eq!(
            state.known_surfaces(),
            vec![KnownSurfaceVersion {
                pane_id: "pane-1".to_owned(),
                version: 3,
            }]
        );
    }

    #[test]
    fn client_attach_state_round_trips_cached_surface() {
        let mut state = ClientAttachState::default();
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            7,
            None,
            vec![surface_row(0, "cached"), surface_row(2, "tail")],
        );
        state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                surface: Some(snapshot),
                scrollback: None,
            })
            .expect("render snapshot");

        let decoded = ClientAttachState::decode(&state.encode()).expect("decode state");
        assert_eq!(decoded.known_surfaces(), state.known_surfaces());
        assert_eq!(decoded.surfaces[0].surface, protocol::SurfaceKind::Main);
        assert_eq!(decoded.surfaces[0].render_text(), "cached\n\ntail");
    }

    #[test]
    fn client_attach_state_decodes_cached_surface_without_surface_kind() {
        let decoded = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 1\nsurface 70616e652d31 7 80 24\ncursor none\nrow 0 636163686564\nend\n",
        )
        .expect("decode old state");

        assert_eq!(decoded.surfaces[0].surface, protocol::SurfaceKind::Main);
        assert_eq!(decoded.surfaces[0].render_text(), "cached");
    }

    #[test]
    fn serves_process_derived_surface_over_unix_socket() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::from_pane_output(b"real process output\n");

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
        let snapshot = attach(&socket_path).expect("attach snapshot");
        server.join().expect("server thread");

        let surface = snapshot.surface.as_ref().expect("surface update");
        assert_eq!(surface.kind, SurfaceUpdateKind::Snapshot);
        assert_eq!(surface.pane_id, "pane-1");
        assert_eq!(surface.version, 3);
        assert_eq!(surface.text, "real process output");
        assert_eq!(
            snapshot.scrollback,
            Some(ScrollbackChunkSummary {
                pane_id: "pane-1".to_owned(),
                scrollback_version: 2,
                start_line: 1,
                total_lines: 1,
                lines: Vec::new(),
            })
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serve_one_with_output_polls_before_attach_response() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut output = RecordingOutput::default();
        output.push_output("pane-1", b"real output\n");

        let server = thread::spawn(move || {
            serve_one_with_output(&listener, &mut session, &mut output).expect("serve one")
        });
        let snapshot = attach(&socket_path).expect("attach snapshot");
        server.join().expect("server thread");

        let surface = snapshot.surface.as_ref().expect("surface update");
        assert_eq!(surface.kind, SurfaceUpdateKind::Snapshot);
        assert_eq!(
            surface.text,
            "booting nmux workspace\nnmux pane-1\nserver-owned terminal state\nreal output"
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serve_one_with_output_polls_before_reconnect_decision() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut output = RecordingOutput::default();
        output.push_output("pane-1", b"new output\n");

        let server = thread::spawn(move || {
            serve_one_with_output(&listener, &mut session, &mut output).expect("serve one")
        });
        let snapshot = attach_with_known_surfaces(
            &socket_path,
            vec![KnownSurfaceVersion {
                pane_id: "pane-1".to_owned(),
                version: 2,
            }],
        )
        .expect("attach snapshot");
        server.join().expect("server thread");

        assert_eq!(
            snapshot.surface.as_ref().map(|surface| surface.kind),
            Some(SurfaceUpdateKind::Patch)
        );
        assert_eq!(
            snapshot.surface.map(|surface| surface.text),
            Some(
                "booting nmux workspace\nnmux pane-1\nserver-owned terminal state\nnew output"
                    .to_owned()
            )
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serve_one_with_host_forwards_read_write_input() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host).expect("serve one");
            host
        });
        let snapshot = attach(&socket_path).expect("attach snapshot");
        let host = server.join().expect("server thread");

        assert!(snapshot.surface.is_some());
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"a".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serve_one_with_host_does_not_forward_read_only_input() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host).expect("serve one");
            host
        });
        let snapshot = attach_with_options(
            &socket_path,
            AttachRequest {
                actor_id: "spectator".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadOnly,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("attach snapshot");
        let host = server.join().expect("server thread");

        assert_eq!(snapshot.presence.mode, AttachMode::ReadOnly);
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serve_one_with_host_polls_after_forwarded_input() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = EchoHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start echo pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host).expect("serve one");
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "local-actor".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("write attach request");
        let snapshot = attach_from_stream(&mut stream).expect("attach snapshot");
        send_key_input(&mut stream, "pane-1", "z").expect("send key input");
        send_scrollback_fetch(&mut stream, "pane-1", 3, 1).expect("send scrollback fetch");
        let scrollback = read_scrollback_chunk_from_stream(&mut stream).expect("scrollback chunk");
        server.join().expect("server thread");

        assert!(snapshot.surface.is_some());
        assert_eq!(
            scrollback,
            ScrollbackChunkSummary {
                pane_id: "pane-1".to_owned(),
                scrollback_version: 2,
                start_line: 3,
                total_lines: 4,
                lines: vec![ScrollbackLine {
                    line: 3,
                    text: "z".to_owned(),
                }],
            }
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_with_client_options_controls_input_and_scrollback_range() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = EchoHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start echo pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host).expect("serve one");
        });
        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                input_text: Some("custom".to_owned()),
                scrollback_start_line: 3,
                scrollback_line_count: 1,
                ..AttachOptions::default()
            },
        )
        .expect("attach snapshot");
        server.join().expect("server thread");

        assert_eq!(
            snapshot.scrollback,
            Some(ScrollbackChunkSummary {
                pane_id: "pane-1".to_owned(),
                scrollback_version: 2,
                start_line: 3,
                total_lines: 4,
                lines: vec![ScrollbackLine {
                    line: 3,
                    text: "custom".to_owned(),
                }],
            })
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_forwards_repeated_input_and_streams_surface_updates() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = EchoHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start echo pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 2).expect("serve live");
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(
            initial.surface.as_ref().map(|surface| surface.version),
            Some(2)
        );

        send_key_input(&mut stream, "pane-1", "first").expect("send first input");
        let first_update = read_surface_update_from_stream(&mut stream).expect("first update");
        assert_eq!(first_update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(first_update.base_version, Some(2));
        assert_eq!(first_update.version, 3);
        assert!(first_update.text.ends_with("first"));

        send_key_input(&mut stream, "pane-1", "second").expect("send second input");
        let second_update = read_surface_update_from_stream(&mut stream).expect("second update");
        assert_eq!(second_update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(second_update.base_version, Some(3));
        assert_eq!(second_update.version, 4);
        assert!(second_update.text.ends_with("second"));

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_read_write_attach_observes_output_without_input() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = ScriptedOutputHost::new(vec![Vec::new(), b"idle update\n".to_vec()]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1)
                .expect("serve read-write live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);
        assert_eq!(
            initial.surface.as_ref().map(|surface| surface.version),
            Some(2)
        );

        let update = read_surface_update_from_stream(&mut stream).expect("idle output update");
        assert_eq!(update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(update.base_version, Some(2));
        assert_eq!(update.version, 3);
        assert!(update.text.ends_with("idle update"));

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_sends_no_surface_update_when_version_is_current() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1).expect("serve live")
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");
        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(
            initial.surface.as_ref().map(|surface| surface.version),
            Some(2)
        );

        send_key_input(&mut stream, "pane-1", "silent").expect("send silent input");
        let update =
            read_optional_surface_update_from_stream(&mut stream).expect("optional surface update");
        assert_eq!(update, None);

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_forwards_resize_intent_before_input() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1).expect("serve live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");
        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert!(initial.surface.is_some());

        send_resize_intent(&mut stream, "pane-1", 100, 30).expect("send resize intent");
        send_key_input(&mut stream, "pane-1", "after-resize").expect("send input");
        let workspace =
            read_live_surface_update_from_stream(&mut stream).expect("live workspace update");
        assert_eq!(
            workspace,
            LiveSurfaceRead::Workspace(WorkspaceSummary {
                session_id: "local".to_owned(),
                tab_id: "tab-1".to_owned(),
                pane_id: "pane-1".to_owned(),
                cols: 100,
                rows: 30,
                resize_policy: protocol::ResizePolicy::Fixed,
            })
        );
        let update =
            read_optional_surface_update_from_stream(&mut stream).expect("optional surface update");
        let update = update.expect("resize surface update");
        assert_eq!(update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(update.version, 3);
        assert_eq!(update.base_version, Some(2));
        assert_eq!(
            update.cursor,
            Some(CursorSummary {
                row: 2,
                col: 0,
                visible: true,
                shape: protocol::CursorShape::Block,
            })
        );
        assert_eq!(
            update.text,
            "booting nmux workspace\nnmux pane-1\nserver-owned terminal state"
        );

        let host = server.join().expect("server thread");
        assert!(host.events().contains(&HostEvent::Resized {
            pane_id: "pane-1".to_owned(),
            cols: 100,
            rows: 30,
        }));
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"after-resize".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_ignores_frontend_resize_when_policy_is_manual() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        assert!(session.set_pane_resize_policy("pane-1", protocol::ResizePolicy::Manual));
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1).expect("serve live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");
        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(
            initial.workspace.resize_policy,
            protocol::ResizePolicy::Manual
        );

        send_resize_intent(&mut stream, "pane-1", 100, 30).expect("send resize intent");
        send_key_input(&mut stream, "pane-1", "after-resize").expect("send input");
        let update = read_live_surface_update_from_stream(&mut stream).expect("live update");
        assert!(matches!(
            update,
            LiveSurfaceRead::NoFrame | LiveSurfaceRead::Closed
        ));

        let host = server.join().expect("server thread");
        assert!(!host.events().iter().any(|event| matches!(
            event,
            HostEvent::Resized {
                pane_id,
                cols: 100,
                rows: 30,
            } if pane_id == "pane-1"
        )));
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"after-resize".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_read_only_attach_observes_output_without_forwarding_input() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = ScriptedOutputHost::new(vec![Vec::new(), b"observer update\n".to_vec()]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1)
                .expect("serve read-only live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "spectator".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadOnly,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadOnly);
        assert_eq!(
            initial.surface.as_ref().map(|surface| surface.version),
            Some(2)
        );

        let update = read_surface_update_from_stream(&mut stream).expect("observer update");
        assert_eq!(update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(update.base_version, Some(2));
        assert_eq!(update.version, 3);
        assert!(update.text.ends_with("observer update"));

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_serves_initial_scrollback_fetch() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = ScriptedOutputHost::new(vec![Vec::new()]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1)
                .expect("serve live scrollback");
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "spectator".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadOnly,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadOnly);
        send_scrollback_fetch(&mut stream, "pane-1", 1, 2).expect("send scrollback fetch");
        let scrollback = read_scrollback_chunk_from_stream(&mut stream).expect("scrollback chunk");

        assert_eq!(
            scrollback,
            ScrollbackChunkSummary {
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
            }
        );

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serves_no_surface_when_client_has_current_surface_version() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
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
        assert_eq!(snapshot.presence.mode, AttachMode::ReadWrite);
        assert_eq!(snapshot.surface, None);
        assert_eq!(snapshot.scrollback, None);

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_request_round_trips_read_only_mode() {
        let request = AttachRequest {
            actor_id: "spectator".to_owned(),
            user_id: "user-2".to_owned(),
            display_name: "Spectator".to_owned(),
            mode: AttachMode::ReadOnly,
            focused_pane_id: Some("pane-1".to_owned()),
            known_surfaces: vec![KnownSurfaceVersion {
                pane_id: "pane-1".to_owned(),
                version: 2,
            }],
        };

        let mut buffer = Vec::new();
        write_attach_request(&mut buffer, &request).expect("write attach request");
        let envelope = protocol::size_prefixed_root_as_envelope(&buffer).expect("attach envelope");
        assert_eq!(envelope.body_type(), protocol::EnvelopeBody::AttachRequest);
        let body = envelope.body_as_attach_request().expect("attach body");
        assert_eq!(body.actor_id(), Some("spectator"));
        assert_eq!(body.user_id(), Some("user-2"));
        assert_eq!(body.display_name(), Some("Spectator"));
        assert_eq!(body.mode(), protocol::AttachMode::ReadOnly);
        assert_eq!(body.focused_pane_id(), Some("pane-1"));
        let known_surfaces = body.known_surfaces().expect("known surfaces");
        assert_eq!(known_surfaces.len(), 1);
        assert_eq!(known_surfaces.get(0).pane_id(), Some("pane-1"));
        assert_eq!(known_surfaces.get(0).version(), 2);
        let decoded = read_attach_request(&mut buffer.as_slice()).expect("read attach request");

        assert_eq!(decoded, request);
    }

    #[test]
    fn read_only_attach_receives_state_without_sending_input() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
        let snapshot = attach_with_options(
            &socket_path,
            AttachRequest {
                actor_id: "spectator".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadOnly,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("attach snapshot");
        server.join().expect("server thread");

        assert_eq!(snapshot.presence.actor_id, "spectator");
        assert_eq!(snapshot.presence.mode, AttachMode::ReadOnly);
        assert!(snapshot.surface.is_some());
        assert!(snapshot.scrollback.is_some());

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn two_clients_can_attach_to_one_session_sequentially() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();

        let server = thread::spawn(move || serve_n(&listener, &mut session, 2).expect("serve two"));
        let first = attach(&socket_path).expect("first attach");
        let second = attach_with_options(
            &socket_path,
            AttachRequest {
                actor_id: "spectator".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadOnly,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("second attach");
        server.join().expect("server thread");

        assert_eq!(first.presence.actor_id, "local-actor");
        assert_eq!(first.presence.mode, AttachMode::ReadWrite);
        assert_eq!(second.presence.actor_id, "spectator");
        assert_eq!(second.presence.mode, AttachMode::ReadOnly);
        assert_eq!(first.workspace.session_id, second.workspace.session_id);
        assert!(first.surface.is_some());
        assert!(second.surface.is_some());

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn reconnect_loop_renders_snapshot_patch_then_no_update() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let output = Arc::new(Mutex::new(RecordingOutput::default()));
        let mut server_output = SharedOutput {
            inner: Arc::clone(&output),
        };

        let server = thread::spawn(move || {
            serve_n_with_output(&listener, &mut session, &mut server_output, 3)
                .expect("serve three")
        });
        let mut state = ClientAttachState::default();
        let first = attach_render_once(&socket_path, read_only_attach_options(), &mut state)
            .expect("first render");
        output
            .lock()
            .expect("output lock")
            .push_output("pane-1", b"loop update\n");
        let second = attach_render_once(&socket_path, read_only_attach_options(), &mut state)
            .expect("second render");
        let third = attach_render_once(&socket_path, read_only_attach_options(), &mut state)
            .expect("third render");
        server.join().expect("server thread");

        assert_eq!(
            first.surface_text.as_deref(),
            Some("nmux pane-1\nserver-owned terminal state")
        );
        assert_eq!(
            second.surface_text.as_deref(),
            Some("booting nmux workspace\nnmux pane-1\nserver-owned terminal state\nloop update")
        );
        assert_eq!(third.surface_text, None);
        assert_eq!(
            state.known_surfaces(),
            vec![KnownSurfaceVersion {
                pane_id: "pane-1".to_owned(),
                version: 3,
            }]
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn decodes_presence_update_from_server_frame() {
        let actor = Session::initial_actor(AttachMode::ReadOnly);
        let frame = Session::initial().presence_update_frame("local-client", 2, &actor);
        let presence = presence_from_frame(&frame).expect("presence");

        assert_eq!(
            presence,
            PresenceSummary {
                actor_id: "local-actor".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadOnly,
                focused_pane_id: Some("pane-1".to_owned()),
            }
        );
    }

    #[test]
    fn serves_patch_when_client_surface_version_is_patchable() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
        let snapshot = attach_with_known_surfaces(
            &socket_path,
            vec![KnownSurfaceVersion {
                pane_id: "pane-1".to_owned(),
                version: 1,
            }],
        )
        .expect("attach snapshot");
        server.join().expect("server thread");

        let surface = snapshot.surface.as_ref().expect("surface update");
        assert_eq!(surface.kind, SurfaceUpdateKind::Patch);
        assert_eq!(surface.pane_id, "pane-1");
        assert_eq!(surface.version, 2);
        assert_eq!(surface.base_version, Some(1));
        assert_eq!(surface.patch_kind, Some(protocol::PatchKind::ReplaceRows));
        assert_eq!(surface.text, "nmux pane-1\nserver-owned terminal state");

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serves_full_surface_snapshot_when_client_surface_version_is_stale() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
        let snapshot = attach_with_known_surfaces(
            &socket_path,
            vec![KnownSurfaceVersion {
                pane_id: "pane-1".to_owned(),
                version: 0,
            }],
        )
        .expect("attach snapshot");
        server.join().expect("server thread");

        let surface = snapshot.surface.as_ref().expect("surface update");
        assert_eq!(surface.kind, SurfaceUpdateKind::Snapshot);
        assert_eq!(surface.pane_id, "pane-1");
        assert_eq!(surface.version, 2);
        assert_eq!(surface.text, "nmux pane-1\nserver-owned terminal state");

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn surface_kind_transition_requires_full_snapshot_response() {
        struct AlternateScreenEngine;

        impl TerminalEngine for AlternateScreenEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"alternate");
                Some(TerminalUpdate::plain(
                    protocol::PatchKind::CursorOnly,
                    protocol::SurfaceKind::Alternate,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    input.scrollback_lines.to_vec(),
                ))
            }

            fn resize(
                &mut self,
                _input: TerminalInput<'_>,
                _cols: u32,
                _rows: u32,
            ) -> Option<TerminalUpdate> {
                panic!("resize is not used by this test")
            }
        }

        let mut session = Session::initial();
        let mut engine = AlternateScreenEngine;
        assert!(session.apply_pane_output_with_engine("pane-1", b"alternate", &mut engine));

        let request = AttachRequest {
            known_surfaces: vec![KnownSurfaceVersion {
                pane_id: "pane-1".to_owned(),
                version: 2,
            }],
            ..AttachOptions::default().request
        };

        assert_eq!(
            request.surface_response(&session, "pane-1"),
            Some(SurfaceResponse::Snapshot)
        );

        let frame = surface_response_frame(&session, "pane-1", SurfaceResponse::Snapshot, 9)
            .expect("surface response");
        let update = surface_update_from_frame(&frame).expect("surface update");
        assert_eq!(update.kind, SurfaceUpdateKind::Snapshot);
        assert_eq!(update.version, 3);
        assert_eq!(update.surface, Some(protocol::SurfaceKind::Alternate));
    }

    #[test]
    fn surface_response_frame_is_pane_scoped() {
        let mut session = Session::initial();
        let mut second = session.tabs[0].clone();
        second.id = "tab-2".to_owned();
        second.active_pane_id = "pane-2".to_owned();
        second.root.id = "pane-2".to_owned();
        second.root.surface_version = 5;
        second.root.surface_lines = vec!["pane two".to_owned()];
        session.tabs.push(second);

        assert!(
            surface_response_frame(&session, "missing", SurfaceResponse::Snapshot, 9).is_none()
        );

        let frame = surface_response_frame(&session, "pane-2", SurfaceResponse::Snapshot, 9)
            .expect("surface response");
        let update = surface_update_from_frame(&frame).expect("surface update");
        assert_eq!(update.kind, SurfaceUpdateKind::Snapshot);
        assert_eq!(update.pane_id, "pane-2");
        assert_eq!(update.version, 5);
        assert_eq!(update.text, "pane two");
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
                bytes: b"x".to_vec(),
            }
        );
    }

    #[test]
    fn decodes_raw_input_from_client_frame() {
        let frame = Session::initial().raw_input_frame(
            "local-client",
            3,
            "actor-1",
            "pane-1",
            2,
            &[0, b'x', 255],
        );
        let input = input_summary_from_frame(&frame).expect("input summary");

        assert_eq!(input.pane_id, "pane-1");
        assert_eq!(input.actor_id, "actor-1");
        assert_eq!(input.input_seq, 2);
        assert_eq!(input.bytes, vec![0, b'x', 255]);
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

    #[derive(Debug, Default)]
    struct EchoHost {
        running: bool,
        output: VecDeque<u8>,
    }

    impl ProcessHost for EchoHost {
        fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
            self.running = true;
            Ok(PaneProcess {
                pane_id: pane_id.to_owned(),
                host_id: spec.id.clone(),
                status: ProcessStatus::Running,
            })
        }

        fn write_input(&mut self, pane_id: &str, bytes: &[u8]) -> Result<(), HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
            self.output.extend(bytes);
            self.output.push_back(b'\n');
            Ok(())
        }

        fn resize_pane(&mut self, pane_id: &str, _cols: u32, _rows: u32) -> Result<(), HostError> {
            if self.running {
                Ok(())
            } else {
                Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                })
            }
        }

        fn stop_pane(&mut self, pane_id: &str) -> Result<PaneProcess, HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
            self.running = false;
            Ok(PaneProcess {
                pane_id: pane_id.to_owned(),
                host_id: "echo".to_owned(),
                status: ProcessStatus::Exited,
            })
        }
    }

    impl ProcessOutput for EchoHost {
        fn try_read_output(&mut self, pane_id: &str, bytes: &mut [u8]) -> Result<usize, HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }

            let count = bytes.len().min(self.output.len());
            for byte in &mut bytes[..count] {
                *byte = self.output.pop_front().expect("queued echo output");
            }
            Ok(count)
        }
    }

    #[derive(Debug, Default)]
    struct ScriptedOutputHost {
        running: bool,
        events: Vec<HostEvent>,
        output: VecDeque<Vec<u8>>,
        pending: VecDeque<u8>,
    }

    impl ScriptedOutputHost {
        fn new(output: Vec<Vec<u8>>) -> Self {
            Self {
                output: output.into(),
                ..Self::default()
            }
        }
    }

    impl ProcessHost for ScriptedOutputHost {
        fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
            self.running = true;
            self.events.push(HostEvent::Started {
                pane_id: pane_id.to_owned(),
                host_id: spec.id.clone(),
                kind: spec.kind.clone(),
            });
            Ok(PaneProcess {
                pane_id: pane_id.to_owned(),
                host_id: spec.id.clone(),
                status: ProcessStatus::Running,
            })
        }

        fn write_input(&mut self, pane_id: &str, bytes: &[u8]) -> Result<(), HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
            self.events.push(HostEvent::Input {
                pane_id: pane_id.to_owned(),
                bytes: bytes.to_vec(),
            });
            Ok(())
        }

        fn resize_pane(&mut self, pane_id: &str, cols: u32, rows: u32) -> Result<(), HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
            self.events.push(HostEvent::Resized {
                pane_id: pane_id.to_owned(),
                cols,
                rows,
            });
            Ok(())
        }

        fn stop_pane(&mut self, pane_id: &str) -> Result<PaneProcess, HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
            self.running = false;
            self.events.push(HostEvent::Stopped {
                pane_id: pane_id.to_owned(),
            });
            Ok(PaneProcess {
                pane_id: pane_id.to_owned(),
                host_id: "scripted".to_owned(),
                status: ProcessStatus::Exited,
            })
        }
    }

    impl ProcessOutput for ScriptedOutputHost {
        fn try_read_output(&mut self, pane_id: &str, bytes: &mut [u8]) -> Result<usize, HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
            if self.pending.is_empty() {
                if let Some(next) = self.output.pop_front() {
                    self.pending.extend(next);
                }
            }

            let count = bytes.len().min(self.pending.len());
            for byte in &mut bytes[..count] {
                *byte = self.pending.pop_front().expect("queued scripted output");
            }
            Ok(count)
        }
    }

    struct SharedOutput {
        inner: Arc<Mutex<RecordingOutput>>,
    }

    impl ProcessOutput for SharedOutput {
        fn try_read_output(&mut self, pane_id: &str, bytes: &mut [u8]) -> Result<usize, HostError> {
            self.inner
                .lock()
                .expect("shared output lock")
                .try_read_output(pane_id, bytes)
        }
    }
}
