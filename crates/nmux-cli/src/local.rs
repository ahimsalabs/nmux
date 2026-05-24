use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use flatbuffers::FlatBufferBuilder;
use nmux_core::host::{HostError, ProcessHost, ProcessOutput};
use nmux_core::session::{
    Actor, AttachMode, ErrorRetryability, FocusInputSpec, InputFrameContext, MouseInputSpec,
    PasteInputSpec, ScrollbackFetchSpec, ScrollbackRange, Session,
};
use nmux_core::terminal::{
    KeyTerminalInput, MouseAction, MouseButton, MouseTerminalInput, PaneTerminalEngines,
    TerminalEngineKind, named_key_bytes,
};
use nmux_proto::{PROTOCOL_VERSION, protocol, wire};

pub use crate::json::{json_string, socket_path_json, version_json};
pub use crate::socket::{
    SocketIdentity, SocketPathSource, accept_authenticated_tcp_client, bind_listener,
    bind_tcp_listener, connect_to_daemon, connect_to_daemon_with_timeout, connect_to_tcp_daemon,
    connect_to_tcp_daemon_with_timeout, default_socket_path, default_socket_path_and_source,
    socket_identity,
};
pub use crate::speculative_echo::{
    SpeculativeEchoOverlay, SpeculativeEchoPrediction, SpeculativeEchoReconcile,
};

mod control;
mod state_file;
use control::serve_control_command;
use state_file::{
    decode_palette_rgba, decode_state_hex_field, hex_decode, hex_encode, parse_state_bool,
    parse_state_cell_semantic_content, parse_state_cursor_shape, parse_state_i64,
    parse_state_mouse_format, parse_state_mouse_tracking_mode, parse_state_row_semantic_prompt,
    parse_state_surface_kind, parse_state_u32, parse_state_u64, parse_state_usize, state_hex_field,
    state_save_tmp_path,
};

const ATTACH_MAX_FRAME_LEN: usize = 64 * 1024;
const INPUT_MODIFIER_MASK: u32 = 0x0f;
const LIVE_IDLE_POLL_TIMEOUT: Duration = Duration::from_millis(20);
const LIVE_BACKGROUND_OUTPUT_QUIET_TIMEOUT: Duration = Duration::from_millis(20);
const LIVE_POST_INPUT_POLL_TIMEOUT: Duration = Duration::from_millis(3);

pub trait ProcessHostOutput: ProcessHost + ProcessOutput {}

impl<T> ProcessHostOutput for T where T: ProcessHost + ProcessOutput {}

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
    if clients > 1 && clients != usize::MAX {
        return serve_live_concurrent_n_with_host_and_engines(
            listener,
            session,
            host,
            clients,
            cycles_per_client,
            engines,
        );
    }
    for _ in 0..clients {
        serve_live_one_with_host_and_engines(listener, session, host, engines, cycles_per_client)?;
    }
    Ok(())
}

fn serve_live_concurrent_n_with_host_and_engines<H>(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut H,
    max_clients: usize,
    cycles_per_client: usize,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    listener.set_nonblocking(true)?;
    let mut accepted_clients = 0_usize;
    let mut clients = Vec::new();

    while accepted_clients < max_clients || !clients.is_empty() {
        let readiness = poll_live_concurrent_sources(
            listener,
            &clients,
            host.notify_fd(),
            LIVE_IDLE_POLL_TIMEOUT,
        )?;
        if let Some(notify_fd) = host.notify_fd()
            && readiness.host_output
        {
            drain_notify_fd(notify_fd)?;
        }

        if readiness.listener && accepted_clients < max_clients {
            loop {
                match accept_live_client(listener, session, host, engines) {
                    Ok(Some(LiveClientAccept::Attached(mut client))) => {
                        accepted_clients += 1;
                        let existing_actors = clients
                            .iter()
                            .map(|client| client.actor.clone())
                            .collect::<Vec<_>>();
                        for existing in &mut clients {
                            let _ = write_presence_to_live_client(existing, session, &client.actor);
                        }
                        for actor in existing_actors {
                            let _ = write_presence_frame(
                                &mut client.stream,
                                session,
                                &mut client.seq,
                                &actor,
                            );
                        }
                        clients.push(client);
                        if accepted_clients >= max_clients {
                            break;
                        }
                    }
                    Ok(Some(LiveClientAccept::Command)) => {
                        accepted_clients += 1;
                        if accepted_clients >= max_clients {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(err) => {
                        listener.set_nonblocking(false)?;
                        return Err(err);
                    }
                }
            }
        }

        let mut had_input = false;
        let mut changed_workspace = false;
        let mut closed_clients = Vec::new();
        for client_index in readiness.client_indices {
            let Some(client) = clients.get_mut(client_index) else {
                continue;
            };
            match drain_live_client_frames(client, session, host, engines) {
                Ok(ClientDrainStatus::Open {
                    had_input: client_had_input,
                    changed_workspace: client_changed_workspace,
                }) => {
                    had_input |= client_had_input;
                    changed_workspace |= client_changed_workspace;
                }
                Ok(ClientDrainStatus::Closed) => closed_clients.push(client_index),
                Err(err) if boxed_socket_closed_error(err.as_ref()) => {
                    closed_clients.push(client_index);
                }
                Err(err) => {
                    listener.set_nonblocking(false)?;
                    return Err(err);
                }
            }
        }

        if had_input
            && !readiness.host_output
            && let Some(notify_fd) = host.notify_fd()
        {
            let readiness = poll_live_concurrent_sources(
                listener,
                &clients,
                Some(notify_fd),
                LIVE_POST_INPUT_POLL_TIMEOUT,
            )?;
            if readiness.host_output {
                drain_notify_fd(notify_fd)?;
            }
        }

        let leaf_pane_ids = session.leaf_pane_ids();
        let quiet_timeout = if had_input {
            LIVE_POST_INPUT_POLL_TIMEOUT
        } else {
            LIVE_BACKGROUND_OUTPUT_QUIET_TIMEOUT
        };
        let output_result = if host.notify_fd().is_some() {
            poll_panes_output_with_host_until_poll_quiet(
                session,
                engines,
                host,
                &leaf_pane_ids,
                quiet_timeout,
            )
        } else {
            poll_panes_output_with_host_until_quiet(session, engines, host, &leaf_pane_ids)
        };
        if let Err(err) = output_result {
            let error_pane_id = host_error_pane_id(&err).to_owned();
            for client in &mut clients {
                let _ = write_host_output_error(
                    &mut client.stream,
                    session,
                    &mut client.seq,
                    &error_pane_id,
                    err.clone(),
                );
            }
            listener.set_nonblocking(false)?;
            return Ok(());
        }

        for (index, client) in clients.iter_mut().enumerate() {
            if changed_workspace {
                let workspace_frame = session.workspace_tree_frame("local-client", client.seq);
                if let Err(err) = wire::write_default_frame(&mut client.stream, &workspace_frame) {
                    if socket_closed_error_from_wire(&err) {
                        closed_clients.push(index);
                        continue;
                    }
                    listener.set_nonblocking(false)?;
                    return Err(err.into());
                }
                client.seq += 1;
            }
            if let Err(err) = write_changed_surface_frames(
                &mut client.stream,
                session,
                &mut client.seq,
                &leaf_pane_ids,
                &mut client.known_surface_versions,
            ) {
                if boxed_socket_closed_error(err.as_ref()) {
                    closed_clients.push(index);
                    continue;
                }
                listener.set_nonblocking(false)?;
                return Err(err);
            }
            if had_input {
                client.completed_cycles = client.completed_cycles.saturating_add(1);
            }
            if client.completed_cycles >= cycles_per_client {
                closed_clients.push(index);
            }
        }

        closed_clients.sort_unstable();
        closed_clients.dedup();
        for index in closed_clients.into_iter().rev() {
            if index < clients.len() {
                clients.remove(index);
            }
        }
    }

    listener.set_nonblocking(false)?;
    Ok(())
}

struct LiveAttachedClient {
    stream: UnixStream,
    actor: Actor,
    seq: u64,
    known_surface_versions: BTreeMap<String, u64>,
    completed_cycles: usize,
}

enum LiveClientAccept {
    Attached(LiveAttachedClient),
    Command,
}

struct LiveConcurrentReadiness {
    listener: bool,
    host_output: bool,
    client_indices: Vec<usize>,
}

enum ClientDrainStatus {
    Open {
        had_input: bool,
        changed_workspace: bool,
    },
    Closed,
}

fn accept_live_client(
    listener: &UnixListener,
    session: &mut Session,
    host: &mut dyn ProcessHostOutput,
    engines: &mut PaneTerminalEngines,
) -> Result<Option<LiveClientAccept>, Box<dyn std::error::Error>> {
    let (mut stream, _) = match listener.accept() {
        Ok(accepted) => accepted,
        Err(err) if matches!(err.kind(), io::ErrorKind::WouldBlock) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    stream.set_nonblocking(false)?;
    let request = match read_client_initial_frame(&mut stream)? {
        ClientInitialFrame::Attach(request) => request,
        ClientInitialFrame::Control(command) => {
            serve_control_command(&mut stream, command, session, Some(host))?;
            return Ok(Some(LiveClientAccept::Command));
        }
    };
    let leaf_pane_ids = session.leaf_pane_ids();
    if let Some(pane_id) = attach_target_pane_id(session, &request) {
        let actor = request_actor_for_pane(&request, &pane_id);
        if let Err(err) =
            poll_panes_output_with_host_until_quiet(session, engines, host, &leaf_pane_ids)
        {
            let mut seq = 1;
            let error_pane_id = host_error_pane_id(&err).to_owned();
            write_host_output_error(&mut stream, session, &mut seq, &error_pane_id, err)?;
            return Ok(Some(LiveClientAccept::Attached(LiveAttachedClient {
                stream,
                actor,
                seq,
                known_surface_versions: BTreeMap::new(),
                completed_cycles: usize::MAX,
            })));
        }
        let mut seq = 1;
        write_live_attach_initial(&mut stream, session, &request, &pane_id, &mut seq)?;
        let mut known_surface_versions = known_surface_versions_from_request(&request);
        if let Some(current) = session.surface_version(&pane_id) {
            known_surface_versions.insert(pane_id, current);
        }
        return Ok(Some(LiveClientAccept::Attached(LiveAttachedClient {
            stream,
            actor,
            seq,
            known_surface_versions,
            completed_cycles: 0,
        })));
    }

    let mut seq = 1;
    write_attach_target_not_found_error(&mut stream, session, &mut seq, &request)?;
    Ok(Some(LiveClientAccept::Attached(LiveAttachedClient {
        stream,
        actor: request.actor(),
        seq,
        known_surface_versions: BTreeMap::new(),
        completed_cycles: usize::MAX,
    })))
}

fn write_live_attach_initial(
    stream: &mut UnixStream,
    session: &Session,
    request: &AttachRequest,
    pane_id: &str,
    seq: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let workspace_frame = session.workspace_tree_frame("local-client", *seq);
    wire::write_default_frame(stream, &workspace_frame)?;
    *seq += 1;

    let actor = request_actor_for_pane(request, pane_id);
    let presence_frame = session.presence_update_frame("local-client", *seq, &actor);
    wire::write_default_frame(stream, &presence_frame)?;
    *seq += 1;

    let response = request.surface_response(session, pane_id);
    let surface_frame = response
        .as_ref()
        .and_then(|response| surface_response_frame(session, pane_id, *response, *seq + 1));
    let surface_state = if surface_frame.is_some() {
        attach_surface_state(response)
    } else {
        protocol::AttachSurfaceState::Current
    };
    let status_frame = session.attach_status_frame("local-client", *seq, pane_id, surface_state);
    wire::write_default_frame(stream, &status_frame)?;
    *seq += 1;
    if let Some(surface_frame) = surface_frame {
        wire::write_default_frame(stream, &surface_frame)?;
        *seq += 1;
    }
    Ok(())
}

fn request_actor_for_pane(request: &AttachRequest, pane_id: &str) -> Actor {
    let mut actor = request.actor();
    if actor.focused_pane_id.is_none() {
        actor.focused_pane_id = Some(pane_id.to_owned());
    }
    actor
}

fn write_presence_to_live_client(
    client: &mut LiveAttachedClient,
    session: &Session,
    actor: &Actor,
) -> Result<(), Box<dyn std::error::Error>> {
    write_presence_frame(&mut client.stream, session, &mut client.seq, actor)
}

fn write_presence_frame(
    stream: &mut UnixStream,
    session: &Session,
    seq: &mut u64,
    actor: &Actor,
) -> Result<(), Box<dyn std::error::Error>> {
    let presence_frame = session.presence_update_frame("local-client", *seq, actor);
    wire::write_default_frame(stream, &presence_frame)?;
    *seq += 1;
    Ok(())
}

fn drain_live_client_frames(
    client: &mut LiveAttachedClient,
    session: &mut Session,
    host: &mut dyn ProcessHostOutput,
    engines: &mut PaneTerminalEngines,
) -> Result<ClientDrainStatus, Box<dyn std::error::Error>> {
    let mut had_input = false;
    let mut changed_workspace = false;
    loop {
        match read_live_client_frame_from_stream(&mut client.stream)? {
            LiveClientRead::Frame(LiveClientFrame::Scrollback(fetch)) => {
                if let Some(error) = scrollback_fetch_error_code(session, &fetch) {
                    write_scrollback_fetch_error(
                        &mut client.stream,
                        session,
                        &mut client.seq,
                        &fetch,
                        error,
                    )?;
                    if error == protocol::ErrorCode::StaleVersion {
                        continue;
                    }
                    return Ok(ClientDrainStatus::Closed);
                }
                let Some(chunk) = session.scrollback_chunk_frame_for_pane(
                    "local-client",
                    client.seq,
                    &fetch.pane_id,
                    fetch.start_line,
                    fetch.line_count,
                ) else {
                    write_pane_not_found_error(
                        &mut client.stream,
                        session,
                        &mut client.seq,
                        &fetch.pane_id,
                    )?;
                    return Ok(ClientDrainStatus::Closed);
                };
                wire::write_default_frame(&mut client.stream, &chunk)?;
                client.seq += 1;
            }
            LiveClientRead::Frame(LiveClientFrame::Resize(resize)) => {
                if !Session::input_allowed(&client.actor) {
                    write_protocol_error(
                        &mut client.stream,
                        session,
                        &mut client.seq,
                        protocol::ErrorCode::PermissionDenied,
                        "resize rejected: actor is read-only",
                        Some(&resize.pane_id),
                        0,
                    )?;
                    return Ok(ClientDrainStatus::Closed);
                }
                if session.surface_version(&resize.pane_id).is_none() {
                    write_pane_not_found_error(
                        &mut client.stream,
                        session,
                        &mut client.seq,
                        &resize.pane_id,
                    )?;
                    return Ok(ClientDrainStatus::Closed);
                }
                let policy = session
                    .pane_resize_policy(&resize.pane_id)
                    .unwrap_or(protocol::ResizePolicy::Fixed);
                if Session::resize_intent_allowed(policy, resize.reason) {
                    if let Err(err) = host.resize_pane(&resize.pane_id, resize.cols, resize.rows) {
                        write_protocol_error(
                            &mut client.stream,
                            session,
                            &mut client.seq,
                            protocol::ErrorCode::Unknown,
                            &format!("resize failed: {err}"),
                            Some(&resize.pane_id),
                            0,
                        )?;
                        return Ok(ClientDrainStatus::Closed);
                    }
                    if session.commit_pane_resize_with_engine(
                        &resize.pane_id,
                        resize.cols,
                        resize.rows,
                        engines.engine_mut(&resize.pane_id),
                    ) {
                        changed_workspace = true;
                    }
                }
            }
            LiveClientRead::Frame(LiveClientFrame::Input(input)) => {
                if !Session::input_allowed(&client.actor) {
                    write_protocol_error(
                        &mut client.stream,
                        session,
                        &mut client.seq,
                        protocol::ErrorCode::PermissionDenied,
                        "input rejected: actor is read-only",
                        Some(&input.pane_id),
                        input.input_seq,
                    )?;
                    return Ok(ClientDrainStatus::Closed);
                }
                forward_live_input(
                    &mut client.stream,
                    session,
                    host,
                    engines,
                    &mut client.seq,
                    input,
                )?;
                had_input = true;
            }
            LiveClientRead::NoFrame => break,
            LiveClientRead::Closed => return Ok(ClientDrainStatus::Closed),
        }

        if !stream_readable_within(&client.stream, Duration::ZERO)? {
            break;
        }
    }
    Ok(ClientDrainStatus::Open {
        had_input,
        changed_workspace,
    })
}

fn forward_live_input(
    stream: &mut UnixStream,
    session: &mut Session,
    host: &mut dyn ProcessHostOutput,
    engines: &mut PaneTerminalEngines,
    seq: &mut u64,
    input: InputSummary,
) -> Result<(), Box<dyn std::error::Error>> {
    if session.surface_version(&input.pane_id).is_none() {
        write_pane_not_found_error_with_input_seq(
            stream,
            session,
            seq,
            &input.pane_id,
            input.input_seq,
        )?;
        return Ok(());
    }
    if let Some(rejection) = input.forwarding_rejection(session) {
        write_protocol_error(
            stream,
            session,
            seq,
            protocol::ErrorCode::PermissionDenied,
            rejection.message(),
            Some(&input.pane_id),
            input.input_seq,
        )?;
        return Ok(());
    }
    let bytes = match input.forwarded_bytes(session, engines) {
        Ok(bytes) => bytes,
        Err(err) => {
            write_protocol_error(
                stream,
                session,
                seq,
                protocol::ErrorCode::Unknown,
                &err.to_string(),
                Some(&input.pane_id),
                input.input_seq,
            )?;
            return Ok(());
        }
    };
    if let Err(err) = host.write_input(&input.pane_id, &bytes) {
        write_protocol_error(
            stream,
            session,
            seq,
            protocol::ErrorCode::Unknown,
            &format!("input forwarding failed: {err}"),
            Some(&input.pane_id),
            input.input_seq,
        )?;
    }
    Ok(())
}

fn poll_live_concurrent_sources(
    listener: &UnixListener,
    clients: &[LiveAttachedClient],
    notify_fd: Option<std::os::fd::RawFd>,
    timeout: Duration,
) -> io::Result<LiveConcurrentReadiness> {
    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    let mut fds = Vec::with_capacity(1 + clients.len() + usize::from(notify_fd.is_some()));
    fds.push(libc::pollfd {
        fd: listener.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    });
    for client in clients {
        fds.push(libc::pollfd {
            fd: client.stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
    }
    if let Some(fd) = notify_fd {
        fds.push(libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        });
    }

    loop {
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if result >= 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }

    let listener = fds
        .first()
        .is_some_and(|fd| fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0);
    let mut client_indices = Vec::new();
    for index in 0..clients.len() {
        if fds
            .get(index + 1)
            .is_some_and(|fd| fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0)
        {
            client_indices.push(index);
        }
    }
    let host_output = notify_fd.is_some_and(|_| {
        fds.last()
            .is_some_and(|fd| fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0)
    });
    Ok(LiveConcurrentReadiness {
        listener,
        host_output,
        client_indices,
    })
}

fn socket_closed_error_from_wire(err: &wire::WireError) -> bool {
    matches!(err, wire::WireError::Io(err) if socket_closed_error(err))
}

fn boxed_socket_closed_error(err: &(dyn std::error::Error + 'static)) -> bool {
    err.downcast_ref::<io::Error>()
        .is_some_and(socket_closed_error)
        || err
            .downcast_ref::<wire::WireError>()
            .is_some_and(socket_closed_error_from_wire)
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

pub fn serve_stream_with_host_and_engines<H>(
    mut stream: UnixStream,
    session: &mut Session,
    host: &mut H,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    let request = match read_client_initial_frame(&mut stream)? {
        ClientInitialFrame::Attach(request) => request,
        ClientInitialFrame::Control(command) => {
            return serve_control_command(&mut stream, command, session, Some(host));
        }
    };
    let Some(pane_id) = attach_target_pane_id(session, &request) else {
        let mut seq = 1;
        write_attach_target_not_found_error(&mut stream, session, &mut seq, &request)?;
        return Ok(());
    };
    if let Err(err) = poll_pane_output_with_host_and_engines(session, engines, host, &pane_id) {
        let mut seq = 1;
        write_host_output_error(&mut stream, session, &mut seq, &pane_id, err)?;
        return Ok(());
    }
    serve_attached_client(&mut stream, request, session, Some(host), engines)
}

fn serve_next(
    listener: &UnixListener,
    session: &mut Session,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut stream, _) = listener.accept()?;
    match read_client_initial_frame(&mut stream)? {
        ClientInitialFrame::Attach(request) => {
            serve_attached_client(&mut stream, request, session, None, engines)
        }
        ClientInitialFrame::Control(command) => {
            serve_control_command(&mut stream, command, session, None)
        }
    }
}

fn serve_next_with_output(
    listener: &UnixListener,
    session: &mut Session,
    mut output: Option<&mut dyn ProcessOutput>,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut stream, _) = listener.accept()?;
    let request = match read_client_initial_frame(&mut stream)? {
        ClientInitialFrame::Attach(request) => request,
        ClientInitialFrame::Control(command) => {
            return serve_control_command(&mut stream, command, session, None);
        }
    };
    if let Some(output) = output.as_deref_mut() {
        let Some(pane_id) = attach_target_pane_id(session, &request) else {
            let mut seq = 1;
            write_attach_target_not_found_error(&mut stream, session, &mut seq, &request)?;
            return Ok(());
        };
        if let Err(err) = poll_pane_output_with_engines(session, engines, output, &pane_id) {
            let mut seq = 1;
            write_host_output_error(&mut stream, session, &mut seq, &pane_id, err)?;
            return Ok(());
        }
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
    let (stream, _) = listener.accept()?;
    serve_stream_with_host_and_engines(stream, session, host, engines)
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
    let (stream, _) = listener.accept()?;
    serve_live_stream_with_host_and_engines(stream, session, host, engines, cycles)
}

pub fn serve_live_stream_with_host_and_engines<H>(
    mut stream: UnixStream,
    session: &mut Session,
    host: &mut H,
    engines: &mut PaneTerminalEngines,
    cycles: usize,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: ProcessHost + ProcessOutput,
{
    let request = match read_client_initial_frame(&mut stream)? {
        ClientInitialFrame::Attach(request) => request,
        ClientInitialFrame::Control(command) => {
            return serve_control_command(&mut stream, command, session, Some(host));
        }
    };
    let Some(pane_id) = attach_target_pane_id(session, &request) else {
        let mut seq = 1;
        write_attach_target_not_found_error(&mut stream, session, &mut seq, &request)?;
        return Ok(());
    };
    if let Err(err) = poll_pane_output_with_host_and_engines(session, engines, host, &pane_id) {
        let mut seq = 1;
        write_host_output_error(&mut stream, session, &mut seq, &pane_id, err)?;
        return Ok(());
    }
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
    let mut seq = 1;
    let Some(pane_id) = attach_target_pane_id(session, &request) else {
        write_attach_target_not_found_error(stream, session, &mut seq, &request)?;
        return Ok(());
    };
    let leaf_pane_ids = session.leaf_pane_ids();
    let workspace_frame = session.workspace_tree_frame("local-client", seq);
    wire::write_default_frame(stream, &workspace_frame)?;
    seq += 1;

    let actor = request_actor_for_pane(&request, &pane_id);
    let presence_frame = session.presence_update_frame("local-client", seq, &actor);
    wire::write_default_frame(stream, &presence_frame)?;
    seq += 1;

    let response = request.surface_response(session, &pane_id);
    let status_frame = session.attach_status_frame(
        "local-client",
        seq,
        &pane_id,
        attach_surface_state(response),
    );
    wire::write_default_frame(stream, &status_frame)?;
    seq += 1;
    if let Some(response) = response {
        if let Some(surface_frame) = surface_response_frame(session, &pane_id, response, seq) {
            wire::write_default_frame(stream, &surface_frame)?;
            seq += 1;
        }
    }
    let mut known_surface_versions = known_surface_versions_from_request(&request);
    if let Some(current) = session.surface_version(&pane_id) {
        known_surface_versions.insert(pane_id.clone(), current);
    }

    let mut completed_cycles = 0;
    while completed_cycles < cycles {
        let mut count_cycle = true;
        let mut readiness =
            poll_live_client_sources(stream, host.notify_fd(), LIVE_IDLE_POLL_TIMEOUT)?;
        if let Some(notify_fd) = host.notify_fd()
            && readiness.host_output
        {
            drain_notify_fd(notify_fd)?;
        }
        if readiness.host_output && !readiness.client_input {
            let client_readiness =
                poll_live_client_sources(stream, None, LIVE_POST_INPUT_POLL_TIMEOUT)?;
            readiness.client_input = client_readiness.client_input;
        }

        let mut inputs = Vec::new();
        if readiness.client_input {
            loop {
                match read_live_client_frame_from_stream(stream)? {
                    LiveClientRead::Frame(LiveClientFrame::Scrollback(fetch)) => {
                        count_cycle = false;
                        if let Some(error) = scrollback_fetch_error_code(session, &fetch) {
                            write_scrollback_fetch_error(stream, session, &mut seq, &fetch, error)?;
                            if error == protocol::ErrorCode::StaleVersion {
                                continue;
                            }
                            return Ok(());
                        }
                        let Some(chunk) = session.scrollback_chunk_frame_for_pane(
                            "local-client",
                            seq,
                            &fetch.pane_id,
                            fetch.start_line,
                            fetch.line_count,
                        ) else {
                            write_pane_not_found_error(stream, session, &mut seq, &fetch.pane_id)?;
                            return Ok(());
                        };
                        wire::write_default_frame(stream, &chunk)?;
                        seq += 1;
                    }
                    LiveClientRead::Frame(LiveClientFrame::Resize(resize)) => {
                        count_cycle = false;
                        if !Session::input_allowed(&actor) {
                            write_protocol_error(
                                stream,
                                session,
                                &mut seq,
                                protocol::ErrorCode::PermissionDenied,
                                "resize rejected: actor is read-only",
                                Some(&resize.pane_id),
                                0,
                            )?;
                            return Ok(());
                        }
                        if session.surface_version(&resize.pane_id).is_none() {
                            write_pane_not_found_error(stream, session, &mut seq, &resize.pane_id)?;
                            return Ok(());
                        }
                        let policy = session
                            .pane_resize_policy(&resize.pane_id)
                            .unwrap_or(protocol::ResizePolicy::Fixed);
                        if !Session::resize_intent_allowed(policy, resize.reason) {
                            continue;
                        }
                        if let Err(err) =
                            host.resize_pane(&resize.pane_id, resize.cols, resize.rows)
                        {
                            write_protocol_error(
                                stream,
                                session,
                                &mut seq,
                                protocol::ErrorCode::Unknown,
                                &format!("resize failed: {err}"),
                                Some(&resize.pane_id),
                                0,
                            )?;
                            return Ok(());
                        }
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
                        count_cycle = true;
                        if Session::input_allowed(&actor) {
                            inputs.push(input);
                        } else {
                            write_protocol_error(
                                stream,
                                session,
                                &mut seq,
                                protocol::ErrorCode::PermissionDenied,
                                "input rejected: actor is read-only",
                                Some(&input.pane_id),
                                input.input_seq,
                            )?;
                            return Ok(());
                        }
                    }
                    LiveClientRead::NoFrame => break,
                    LiveClientRead::Closed => return Ok(()),
                }

                if !stream_readable_within(stream, Duration::ZERO)? {
                    break;
                }
            }
        }

        let mut input_pane_id = None;
        for input in inputs {
            input_pane_id = Some(input.pane_id.clone());
            if session.surface_version(&input.pane_id).is_none() {
                write_pane_not_found_error_with_input_seq(
                    stream,
                    session,
                    &mut seq,
                    &input.pane_id,
                    input.input_seq,
                )?;
                return Ok(());
            }
            if let Some(rejection) = input.forwarding_rejection(session) {
                write_protocol_error(
                    stream,
                    session,
                    &mut seq,
                    protocol::ErrorCode::PermissionDenied,
                    rejection.message(),
                    Some(&input.pane_id),
                    input.input_seq,
                )?;
                return Ok(());
            }
            let bytes = match input.forwarded_bytes(session, engines) {
                Ok(bytes) => bytes,
                Err(err) => {
                    write_protocol_error(
                        stream,
                        session,
                        &mut seq,
                        protocol::ErrorCode::Unknown,
                        &err.to_string(),
                        Some(&input.pane_id),
                        input.input_seq,
                    )?;
                    return Ok(());
                }
            };
            if let Err(err) = host.write_input(&input.pane_id, &bytes) {
                write_protocol_error(
                    stream,
                    session,
                    &mut seq,
                    protocol::ErrorCode::Unknown,
                    &format!("input forwarding failed: {err}"),
                    Some(&input.pane_id),
                    input.input_seq,
                )?;
                return Ok(());
            }
        }

        if input_pane_id.is_some()
            && !readiness.host_output
            && let Some(notify_fd) = host.notify_fd()
        {
            let readiness =
                poll_live_client_sources(stream, Some(notify_fd), LIVE_POST_INPUT_POLL_TIMEOUT)?;
            if readiness.host_output {
                drain_notify_fd(notify_fd)?;
            }
        }

        let quiet_timeout = if input_pane_id.is_some() {
            LIVE_POST_INPUT_POLL_TIMEOUT
        } else {
            LIVE_BACKGROUND_OUTPUT_QUIET_TIMEOUT
        };
        let output_result = if host.notify_fd().is_some() {
            poll_panes_output_with_host_until_poll_quiet(
                session,
                engines,
                host,
                &leaf_pane_ids,
                quiet_timeout,
            )
        } else {
            poll_panes_output_with_host_until_quiet(session, engines, host, &leaf_pane_ids)
        };
        if let Err(err) = output_result {
            let error_pane_id = host_error_pane_id(&err).to_owned();
            write_host_output_error(stream, session, &mut seq, &error_pane_id, err)?;
            return Ok(());
        }

        write_changed_surface_frames(
            stream,
            session,
            &mut seq,
            &leaf_pane_ids,
            &mut known_surface_versions,
        )?;
        if count_cycle {
            completed_cycles += 1;
        }
    }

    Ok(())
}

fn read_live_client_frame_from_stream(
    stream: &mut UnixStream,
) -> Result<LiveClientRead, Box<dyn std::error::Error>> {
    match wire::read_default_frame(stream) {
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
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct LiveReadiness {
    client_input: bool,
    host_output: bool,
}

fn poll_live_client_sources(
    stream: &UnixStream,
    notify_fd: Option<std::os::fd::RawFd>,
    timeout: Duration,
) -> io::Result<LiveReadiness> {
    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    let mut fds = vec![libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }];
    if let Some(fd) = notify_fd {
        fds.push(libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        });
    }

    loop {
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if result >= 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }

    Ok(LiveReadiness {
        client_input: fds
            .first()
            .is_some_and(|fd| fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0),
        host_output: fds
            .get(1)
            .is_some_and(|fd| fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0),
    })
}

fn drain_notify_fd(fd: std::os::fd::RawFd) -> io::Result<()> {
    let mut buffer = [0_u8; 1024];
    loop {
        let count = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if count > 0 {
            continue;
        }
        if count == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
        ) {
            return Ok(());
        }
        return Err(error);
    }
}

fn poll_notify_fd(fd: std::os::fd::RawFd, timeout: Duration) -> io::Result<bool> {
    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if result >= 0 {
            return Ok(result > 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientInitialFrame {
    Attach(AttachRequest),
    Control(ControlCommandSummary),
}

fn read_client_initial_frame<R: Read>(reader: &mut R) -> io::Result<ClientInitialFrame> {
    let frame = wire::read_frame(reader, ATTACH_MAX_FRAME_LEN).map_err(wire_error_to_io)?;
    let envelope = protocol::size_prefixed_root_as_envelope(&frame).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid client frame: {err}"),
        )
    })?;
    match envelope.body_type() {
        protocol::EnvelopeBody::AttachRequest => {
            attach_request_from_frame(&frame).map(ClientInitialFrame::Attach)
        }
        protocol::EnvelopeBody::ControlCommand => {
            control_command_from_frame(&frame).map(ClientInitialFrame::Control)
        }
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected initial envelope body: {other:?}"),
        )),
    }
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

enum AttachedClientFrame {
    Scrollback(ScrollbackFetchSummary),
    Input(InputSummary),
}

enum AttachedClientRead {
    Frame(AttachedClientFrame),
    NoFrame,
    Closed,
}

fn serve_attached_client(
    stream: &mut UnixStream,
    request: AttachRequest,
    session: &mut Session,
    mut host: Option<&mut dyn ProcessHostOutput>,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut seq = 1;
    let Some(pane_id) = attach_target_pane_id(session, &request) else {
        write_attach_target_not_found_error(stream, session, &mut seq, &request)?;
        return Ok(());
    };

    let workspace_frame = session.workspace_tree_frame("local-client", seq);
    wire::write_default_frame(stream, &workspace_frame)?;
    seq += 1;

    let actor = request_actor_for_pane(&request, &pane_id);
    let presence_frame = session.presence_update_frame("local-client", seq, &actor);
    wire::write_default_frame(stream, &presence_frame)?;
    seq += 1;

    let response = request.surface_response(session, &pane_id);
    let status_frame = session.attach_status_frame(
        "local-client",
        seq,
        &pane_id,
        attach_surface_state(response),
    );
    wire::write_default_frame(stream, &status_frame)?;
    seq += 1;
    if let Some(response) = response {
        if let Some(surface_frame) = surface_response_frame(session, &pane_id, response, seq) {
            wire::write_default_frame(stream, &surface_frame)?;
            seq += 1;
        }
    }
    let mut wait_for_more = true;
    loop {
        let read = if wait_for_more {
            read_attached_client_frame_from_stream(stream)?
        } else {
            read_optional_attached_client_frame_from_stream(stream, Duration::from_millis(250))?
        };
        let frame = match read {
            AttachedClientRead::Frame(frame) => frame,
            AttachedClientRead::NoFrame => return Ok(()),
            AttachedClientRead::Closed => return Ok(()),
        };
        match frame {
            AttachedClientFrame::Input(input) => {
                wait_for_more = true;
                if !Session::input_allowed(&actor) {
                    write_protocol_error(
                        stream,
                        session,
                        &mut seq,
                        protocol::ErrorCode::PermissionDenied,
                        "input rejected: attach is read-only",
                        Some(&input.pane_id),
                        input.input_seq,
                    )?;
                    return Ok(());
                }
                if let Some(host) = host.as_deref_mut() {
                    if !process_one_shot_input(stream, session, engines, host, input)? {
                        return Ok(());
                    }
                }
            }
            AttachedClientFrame::Scrollback(fetch) => {
                if let Some(error) = scrollback_fetch_error_code(session, &fetch) {
                    write_scrollback_fetch_error(stream, session, &mut seq, &fetch, error)?;
                    wait_for_more = true;
                    continue;
                }
                if let Some(chunk) = session.scrollback_chunk_frame_for_pane(
                    "local-client",
                    seq,
                    &fetch.pane_id,
                    fetch.start_line,
                    fetch.line_count,
                ) {
                    wire::write_default_frame(stream, &chunk)?;
                    seq += 1;
                    wait_for_more = false;
                } else {
                    write_pane_not_found_error(stream, session, &mut seq, &fetch.pane_id)?;
                    return Ok(());
                }
            }
        }
    }
}

fn active_pane_id(session: &Session) -> Option<&str> {
    session.active_pane_id()
}

fn attach_target_pane_id(session: &mut Session, request: &AttachRequest) -> Option<String> {
    match request.focused_pane_id.as_deref() {
        Some(target_id) => {
            if session.surface_version(target_id).is_some() {
                return Some(target_id.to_owned());
            }
            let pane_id = session
                .tabs
                .iter()
                .find(|tab| tab.id == target_id)
                .and_then(|tab| {
                    session
                        .surface_version(&tab.active_pane_id)
                        .map(|_| tab.active_pane_id.clone())
                })?;
            if session.active_tab_id != target_id {
                let _ = session.switch_tab(target_id);
            }
            Some(pane_id)
        }
        None => active_pane_id(session).map(ToOwned::to_owned),
    }
}

fn write_attach_target_not_found_error(
    stream: &mut UnixStream,
    session: &Session,
    seq: &mut u64,
    request: &AttachRequest,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(pane_id) = request.focused_pane_id.as_deref() {
        write_pane_not_found_error(stream, session, seq, pane_id)
    } else {
        write_active_pane_not_found_error(stream, session, seq)
    }
}

fn active_tab<'a>(session: &'a Session) -> Option<&'a nmux_core::session::Tab> {
    session
        .tabs
        .iter()
        .find(|tab| tab.id == session.active_tab_id)
}

fn process_one_shot_input(
    stream: &mut UnixStream,
    session: &mut Session,
    engines: &mut PaneTerminalEngines,
    host: &mut dyn ProcessHostOutput,
    input: InputSummary,
) -> Result<bool, Box<dyn std::error::Error>> {
    if session.surface_version(&input.pane_id).is_none() {
        let mut seq = 4;
        write_pane_not_found_error_with_input_seq(
            stream,
            session,
            &mut seq,
            &input.pane_id,
            input.input_seq,
        )?;
        return Ok(false);
    }
    if let Some(rejection) = input.forwarding_rejection(session) {
        let mut seq = 4;
        write_protocol_error(
            stream,
            session,
            &mut seq,
            protocol::ErrorCode::PermissionDenied,
            rejection.message(),
            Some(&input.pane_id),
            input.input_seq,
        )?;
        return Ok(false);
    } else {
        let bytes = match input.forwarded_bytes(session, engines) {
            Ok(bytes) => bytes,
            Err(err) => {
                let mut seq = 4;
                write_protocol_error(
                    stream,
                    session,
                    &mut seq,
                    protocol::ErrorCode::Unknown,
                    &err.to_string(),
                    Some(&input.pane_id),
                    input.input_seq,
                )?;
                return Ok(false);
            }
        };
        if let Err(err) = host.write_input(&input.pane_id, &bytes) {
            let mut seq = 4;
            write_protocol_error(
                stream,
                session,
                &mut seq,
                protocol::ErrorCode::Unknown,
                &format!("input forwarding failed: {err}"),
                Some(&input.pane_id),
                input.input_seq,
            )?;
            return Ok(false);
        }
    }
    if let Err(err) = poll_pane_output_with_host_and_engines(session, engines, host, &input.pane_id)
    {
        let mut seq = 4;
        write_host_output_error(stream, session, &mut seq, &input.pane_id, err)?;
        return Ok(false);
    }
    Ok(true)
}

fn scrollback_fetch_error_code(
    session: &Session,
    fetch: &ScrollbackFetchSummary,
) -> Option<protocol::ErrorCode> {
    let current = session.scrollback_version(&fetch.pane_id)?;
    (fetch.known_scrollback_version != 0 && fetch.known_scrollback_version != current)
        .then_some(protocol::ErrorCode::StaleVersion)
}

fn write_scrollback_fetch_error(
    stream: &mut UnixStream,
    session: &Session,
    seq: &mut u64,
    fetch: &ScrollbackFetchSummary,
    code: protocol::ErrorCode,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(current) = session.scrollback_version(&fetch.pane_id) else {
        write_pane_not_found_error(stream, session, seq, &fetch.pane_id)?;
        return Ok(());
    };
    write_protocol_error(
        stream,
        session,
        seq,
        code,
        &format!(
            "stale scrollback version for {}: client={} server={current}",
            fetch.pane_id, fetch.known_scrollback_version
        ),
        Some(&fetch.pane_id),
        0,
    )
}

fn write_pane_not_found_error(
    stream: &mut UnixStream,
    session: &Session,
    seq: &mut u64,
    pane_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    write_pane_not_found_error_with_input_seq(stream, session, seq, pane_id, 0)
}

fn write_active_pane_not_found_error(
    stream: &mut UnixStream,
    session: &Session,
    seq: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(tab) = active_tab(session) else {
        return write_protocol_error(
            stream,
            session,
            seq,
            protocol::ErrorCode::PaneNotFound,
            &format!("active tab not found: {}", session.active_tab_id),
            None,
            0,
        );
    };
    let pane_id = tab.active_pane_id.as_str();
    write_protocol_error(
        stream,
        session,
        seq,
        protocol::ErrorCode::PaneNotFound,
        &format!("active pane not found: {pane_id}"),
        Some(pane_id),
        0,
    )
}

fn write_pane_not_found_error_with_input_seq(
    stream: &mut UnixStream,
    session: &Session,
    seq: &mut u64,
    pane_id: &str,
    input_seq: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    write_protocol_error(
        stream,
        session,
        seq,
        protocol::ErrorCode::PaneNotFound,
        &format!("pane not found: {pane_id}"),
        Some(pane_id),
        input_seq,
    )
}

fn write_host_output_error(
    stream: &mut UnixStream,
    session: &Session,
    seq: &mut u64,
    pane_id: &str,
    error: HostError,
) -> Result<(), Box<dyn std::error::Error>> {
    write_protocol_error(
        stream,
        session,
        seq,
        protocol::ErrorCode::Unknown,
        &format!("output polling failed: {error}"),
        Some(pane_id),
        0,
    )
}

fn write_protocol_error(
    stream: &mut UnixStream,
    session: &Session,
    seq: &mut u64,
    code: protocol::ErrorCode,
    message: &str,
    pane_id: Option<&str>,
    input_seq: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let error = session.error_frame_with_context(
        "local-client",
        *seq,
        code,
        message,
        ErrorRetryability::NotRetryable,
        pane_id,
        input_seq,
    );
    wire::write_default_frame(stream, &error)?;
    *seq += 1;
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

fn known_surface_versions_from_request(request: &AttachRequest) -> BTreeMap<String, u64> {
    request
        .known_surfaces
        .iter()
        .map(|known| (known.pane_id.clone(), known.version))
        .collect()
}

fn write_changed_surface_frames(
    stream: &mut UnixStream,
    session: &Session,
    seq: &mut u64,
    pane_ids: &[String],
    known_versions: &mut BTreeMap<String, u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    for pane_id in pane_ids {
        let Some(current) = session.surface_version(pane_id) else {
            continue;
        };
        let response = match known_versions.get(pane_id).copied() {
            Some(known) => {
                let patch_kind = session
                    .surface_patch_kind(pane_id)
                    .unwrap_or(protocol::PatchKind::ReplaceRows);
                surface_response_for_known_version(current, known, patch_kind)
            }
            None => Some(SurfaceResponse::Snapshot),
        };
        if let Some(response) = response {
            if let Some(surface_frame) = surface_response_frame(session, pane_id, response, *seq) {
                wire::write_default_frame(stream, &surface_frame)?;
                *seq += 1;
            }
        }
        known_versions.insert(pane_id.clone(), current);
    }
    Ok(())
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

pub fn poll_pane_output_with_host_and_engines(
    session: &mut Session,
    engines: &mut PaneTerminalEngines,
    host: &mut dyn ProcessHostOutput,
    pane_id: &str,
) -> Result<bool, HostError> {
    let changed = poll_pane_output_with_engines(session, engines, host, pane_id)?;
    for bytes in engines.engine_mut(pane_id).drain_pty_writes() {
        host.write_input(pane_id, &bytes)?;
    }
    Ok(changed)
}

fn poll_panes_output_with_host_until_quiet(
    session: &mut Session,
    engines: &mut PaneTerminalEngines,
    host: &mut dyn ProcessHostOutput,
    pane_ids: &[String],
) -> Result<bool, HostError> {
    let deadline = Instant::now() + Duration::from_millis(120);
    let mut quiet_since = None;
    let mut changed = false;

    loop {
        let mut cycle_changed = false;
        for pane_id in pane_ids {
            cycle_changed |=
                poll_pane_output_with_host_and_engines(session, engines, host, pane_id)?;
        }
        if cycle_changed {
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

fn poll_panes_output_with_host_until_poll_quiet(
    session: &mut Session,
    engines: &mut PaneTerminalEngines,
    host: &mut dyn ProcessHostOutput,
    pane_ids: &[String],
    quiet_timeout: Duration,
) -> Result<bool, HostError> {
    let Some(notify_fd) = host.notify_fd() else {
        return poll_panes_output_with_host_until_quiet(session, engines, host, pane_ids);
    };
    let deadline = Instant::now() + Duration::from_millis(120);
    let mut quiet_since = None;
    let mut changed = false;

    loop {
        if let Err(error) = drain_notify_fd(notify_fd) {
            let pane_id = pane_ids.first().map(String::as_str).unwrap_or("unknown");
            return Err(HostError::Io {
                pane_id: pane_id.to_owned(),
                operation: "drain_notify_fd".to_owned(),
                message: error.to_string(),
            });
        }
        let mut cycle_changed = false;
        for pane_id in pane_ids {
            cycle_changed |=
                poll_pane_output_with_host_and_engines(session, engines, host, pane_id)?;
        }
        if cycle_changed {
            changed = true;
            quiet_since = None;
        } else if changed {
            let quiet_start = quiet_since.get_or_insert_with(Instant::now);
            if quiet_start.elapsed() >= quiet_timeout {
                return Ok(true);
            }
        } else if Instant::now() >= deadline {
            return Ok(false);
        }

        if Instant::now() >= deadline {
            return Ok(changed);
        }

        let timeout = if changed {
            quiet_timeout
        } else {
            deadline
                .saturating_duration_since(Instant::now())
                .min(LIVE_IDLE_POLL_TIMEOUT)
        };
        if let Err(error) = poll_notify_fd(notify_fd, timeout) {
            let pane_id = pane_ids.first().map(String::as_str).unwrap_or("unknown");
            return Err(HostError::Io {
                pane_id: pane_id.to_owned(),
                operation: "poll_notify_fd".to_owned(),
                message: error.to_string(),
            });
        }
    }
}

fn host_error_pane_id(error: &HostError) -> &str {
    match error {
        HostError::UnsupportedHostKind { host_id, .. } => host_id,
        HostError::UnsupportedOperation { pane_id, .. }
        | HostError::AlreadyRunning { pane_id }
        | HostError::NotRunning { pane_id }
        | HostError::Io { pane_id, .. } => pane_id,
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
            focused_pane_id: None,
            known_surfaces,
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachOptions {
    pub request: AttachRequest,
    pub input_text: Option<String>,
    pub key_name: Option<String>,
    pub key_names: Vec<String>,
    pub key_modifiers: u32,
    pub paste_text: Option<String>,
    pub focus: Option<bool>,
    pub mouse: Option<AttachMouseInput>,
    pub scrollback_start_line: u64,
    pub scrollback_line_count: u32,
    pub scrollback_tail_count: Option<u32>,
    pub fetch_scrollback: bool,
    pub known_scrollback_version: u64,
    pub known_scrollback_versions: Vec<KnownScrollbackVersion>,
    pub connect_timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownScrollbackVersion {
    pub pane_id: String,
    pub start_line: u64,
    pub line_count: u32,
    pub version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachMouseInput {
    pub row: u32,
    pub col: u32,
    pub pixel_x: Option<u32>,
    pub pixel_y: Option<u32>,
    pub button: protocol::MouseButton,
    pub action: protocol::MouseAction,
    pub modifiers: u32,
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
            key_name: None,
            key_names: Vec::new(),
            key_modifiers: 0,
            paste_text: None,
            focus: None,
            mouse: None,
            scrollback_start_line: 1,
            scrollback_line_count: 2,
            scrollback_tail_count: None,
            fetch_scrollback: true,
            known_scrollback_version: 0,
            known_scrollback_versions: Vec::new(),
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
    let stream = match options.connect_timeout {
        Some(timeout) => connect_to_daemon_with_timeout(path, timeout)?,
        None => connect_to_daemon(path)?,
    };
    attach_with_client_options_from_stream(stream, options)
}

pub fn attach_with_client_options_from_stream(
    mut stream: UnixStream,
    options: AttachOptions,
) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    let mode = options.request.mode;
    write_attach_request(&mut stream, &options.request)?;
    let mut sequence = ClientFrameSequence::default();
    let snapshot = attach_from_stream(&mut stream)?;
    let attached_pane_id = snapshot.status.pane_id.clone();
    if mode == AttachMode::ReadWrite {
        let mut sent_input = false;
        let key_names = options.named_key_names();
        if !key_names.is_empty() {
            for key_name in key_names {
                send_named_key_input_with_modifiers_and_sequence(
                    &mut stream,
                    &mut sequence,
                    &attached_pane_id,
                    key_name,
                    options.key_modifiers,
                )?;
                read_optional_server_error_from_stream(&mut stream)?;
            }
            sent_input = true;
        } else if let Some(mouse) = options.mouse {
            send_mouse_input_with_sequence(&mut stream, &mut sequence, &attached_pane_id, mouse)?;
            sent_input = true;
        } else if let Some(focused) = options.focus {
            send_focus_input_with_sequence(&mut stream, &mut sequence, &attached_pane_id, focused)?;
            sent_input = true;
        } else if let Some(paste_text) = options.paste_text.as_deref() {
            send_paste_input_with_sequence(
                &mut stream,
                &mut sequence,
                &attached_pane_id,
                paste_text,
            )?;
            sent_input = true;
        } else if let Some(input_text) = options.input_text.as_deref() {
            send_key_input_with_sequence(
                &mut stream,
                &mut sequence,
                &attached_pane_id,
                input_text,
            )?;
            sent_input = true;
        }
        if sent_input && options.named_key_names().is_empty() {
            read_optional_server_error_from_stream(&mut stream)?;
        }
    }
    let scrollback = if options.fetch_scrollback {
        Some(fetch_scrollback_chunk_with_selection(
            &mut stream,
            &mut sequence,
            &attached_pane_id,
            options.scrollback_start_line,
            options.scrollback_line_count,
            options.scrollback_tail_count,
            |start_line, line_count| {
                options.known_scrollback_version_for(&attached_pane_id, start_line, line_count)
            },
        )?)
    } else {
        None
    };
    Ok(AttachSnapshot {
        scrollback,
        ..snapshot
    })
}

pub fn attach_render_once(
    path: &Path,
    mut options: AttachOptions,
    client_state: &mut ClientAttachState,
) -> Result<RenderedAttach, Box<dyn std::error::Error>> {
    let scope = socket_identity(path).ok();
    options.request.known_surfaces = client_state.known_surfaces_for_scope(scope);
    options.known_scrollback_versions = client_state.known_scrollback_versions_for_scope(scope);
    let snapshot = attach_with_client_options(path, options)?;
    client_state.apply_scope(socket_identity(path).ok());
    client_state.render_attach(snapshot)
}

pub fn attach_render_once_from_stream(
    stream: UnixStream,
    mut options: AttachOptions,
    client_state: &mut ClientAttachState,
) -> Result<RenderedAttach, Box<dyn std::error::Error>> {
    options.request.known_surfaces = client_state.known_surfaces_for_scope(None);
    options.known_scrollback_versions = client_state.known_scrollback_versions_for_scope(None);
    let snapshot = attach_with_client_options_from_stream(stream, options)?;
    client_state.apply_scope(None);
    client_state.render_attach(snapshot)
}

pub fn run_control_command(
    path: &Path,
    connect_timeout: Option<Duration>,
    command: ControlCommandSummary,
) -> Result<WorkspaceSummary, Box<dyn std::error::Error>> {
    let stream = match connect_timeout {
        Some(timeout) => connect_to_daemon_with_timeout(path, timeout)?,
        None => connect_to_daemon(path)?,
    };
    run_control_command_on_stream(stream, command)
}

pub fn run_control_command_on_stream(
    mut stream: UnixStream,
    command: ControlCommandSummary,
) -> Result<WorkspaceSummary, Box<dyn std::error::Error>> {
    write_control_command(&mut stream, &command)?;
    read_control_command_response(&mut stream)
}

impl AttachOptions {
    fn named_key_names(&self) -> Vec<&str> {
        if self.key_names.is_empty() {
            self.key_name.iter().map(String::as_str).collect()
        } else {
            self.key_names.iter().map(String::as_str).collect()
        }
    }

    fn known_scrollback_version_for(&self, pane_id: &str, start_line: u64, line_count: u32) -> u64 {
        self.known_scrollback_versions
            .iter()
            .find(|known| {
                known.pane_id == pane_id
                    && known.start_line == start_line
                    && known.line_count == line_count
            })
            .map_or(self.known_scrollback_version, |known| known.version)
    }
}

pub fn attach_from_stream(
    stream: &mut UnixStream,
) -> Result<AttachSnapshot, Box<dyn std::error::Error>> {
    let workspace_frame = wire::read_default_frame(stream)?;
    if protocol::size_prefixed_root_as_envelope(&workspace_frame)?.body_type()
        == protocol::EnvelopeBody::Error
    {
        let error = error_summary_from_frame(&workspace_frame)?;
        return Err(server_error(error));
    }
    let workspace = workspace_summary_from_frame(&workspace_frame)?;

    let presence_frame = wire::read_default_frame(stream)?;
    let presence = presence_from_frame(&presence_frame)?;

    let status_frame = wire::read_default_frame(stream)?;
    let status = attach_status_from_frame(&status_frame)?;
    let surface = match status.surface_state {
        protocol::AttachSurfaceState::Current => None,
        protocol::AttachSurfaceState::Snapshot | protocol::AttachSurfaceState::Patch => {
            let surface_frame = wire::read_default_frame(stream)?;
            let update = surface_update_from_frame(&surface_frame)?;
            validate_attach_surface_update(&status, &update)?;
            Some(update)
        }
        other => return Err(format!("unsupported attach surface state: {other:?}").into()),
    };

    Ok(AttachSnapshot {
        workspace,
        presence,
        status,
        surface,
        scrollback: None,
    })
}

pub fn send_key_input(
    stream: &mut UnixStream,
    pane_id: &str,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sequence = ClientFrameSequence::default();
    send_key_input_with_sequence(stream, &mut sequence, pane_id, text).map(|_| ())
}

pub fn send_key_input_with_sequence(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    text: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    let input_seq = sequence.next_input_seq();
    let frame = Session::initial().key_input_frame(
        "local-client",
        sequence.next_envelope_seq(),
        "local-actor",
        pane_id,
        input_seq,
        text,
    );
    wire::write_default_frame(stream, &frame)?;
    Ok(input_seq)
}

pub fn send_named_key_input(
    stream: &mut UnixStream,
    pane_id: &str,
    key_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    send_named_key_input_with_modifiers(stream, pane_id, key_name, 0)
}

pub fn send_named_key_input_with_modifiers(
    stream: &mut UnixStream,
    pane_id: &str,
    key_name: &str,
    modifiers: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sequence = ClientFrameSequence::default();
    send_named_key_input_with_modifiers_and_sequence(
        stream,
        &mut sequence,
        pane_id,
        key_name,
        modifiers,
    )
}

pub fn send_named_key_input_with_modifiers_and_sequence(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    key_name: &str,
    modifiers: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame = Session::initial().named_key_input_frame_with_modifiers(
        "local-client",
        sequence.next_envelope_seq(),
        "local-actor",
        pane_id,
        sequence.next_input_seq(),
        key_name,
        modifiers,
    );
    wire::write_default_frame(stream, &frame)?;
    Ok(())
}

pub fn send_raw_input(
    stream: &mut UnixStream,
    pane_id: &str,
    bytes: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sequence = ClientFrameSequence::default();
    send_raw_input_with_sequence(stream, &mut sequence, pane_id, bytes).map(|_| ())
}

pub fn send_raw_input_with_sequence(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    bytes: &[u8],
) -> Result<u64, Box<dyn std::error::Error>> {
    let input_seq = sequence.next_input_seq();
    let frame = Session::initial().raw_input_frame(
        "local-client",
        sequence.next_envelope_seq(),
        "local-actor",
        pane_id,
        input_seq,
        bytes,
    );
    wire::write_default_frame(stream, &frame)?;
    Ok(input_seq)
}

pub fn send_paste_input(
    stream: &mut UnixStream,
    pane_id: &str,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sequence = ClientFrameSequence::default();
    send_paste_input_with_sequence(stream, &mut sequence, pane_id, text)
}

pub fn send_paste_input_with_sequence(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame = Session::initial().paste_input_frame(
        InputFrameContext {
            connection_id: "local-client",
            seq: sequence.next_envelope_seq(),
            actor_id: "local-actor",
            pane_id,
            input_seq: sequence.next_input_seq(),
        },
        PasteInputSpec {
            text,
            bracketed: false,
        },
    );
    wire::write_default_frame(stream, &frame)?;
    Ok(())
}

pub fn send_focus_input(
    stream: &mut UnixStream,
    pane_id: &str,
    focused: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sequence = ClientFrameSequence::default();
    send_focus_input_with_sequence(stream, &mut sequence, pane_id, focused)
}

pub fn send_focus_input_with_sequence(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    focused: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame = Session::initial().focus_input_frame(
        InputFrameContext {
            connection_id: "local-client",
            seq: sequence.next_envelope_seq(),
            actor_id: "local-actor",
            pane_id,
            input_seq: sequence.next_input_seq(),
        },
        FocusInputSpec { focused },
    );
    wire::write_default_frame(stream, &frame)?;
    Ok(())
}

pub fn send_mouse_input(
    stream: &mut UnixStream,
    pane_id: &str,
    mouse: AttachMouseInput,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sequence = ClientFrameSequence::default();
    send_mouse_input_with_sequence(stream, &mut sequence, pane_id, mouse)
}

pub fn send_mouse_input_with_sequence(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    mouse: AttachMouseInput,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame = Session::initial().mouse_input_frame(
        InputFrameContext {
            connection_id: "local-client",
            seq: sequence.next_envelope_seq(),
            actor_id: "local-actor",
            pane_id,
            input_seq: sequence.next_input_seq(),
        },
        MouseInputSpec::from(mouse),
    );
    wire::write_default_frame(stream, &frame)?;
    Ok(())
}

impl From<AttachMouseInput> for MouseInputSpec {
    fn from(mouse: AttachMouseInput) -> Self {
        Self {
            row: mouse.row,
            col: mouse.col,
            pixel_x: mouse.pixel_x,
            pixel_y: mouse.pixel_y,
            button: mouse.button,
            action: mouse.action,
            modifiers: mouse.modifiers,
        }
    }
}

pub fn send_resize_intent(
    stream: &mut UnixStream,
    pane_id: &str,
    cols: u32,
    rows: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sequence = ClientFrameSequence::default();
    send_resize_intent_with_sequence(stream, &mut sequence, pane_id, cols, rows)
}

pub fn send_resize_intent_with_sequence(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    cols: u32,
    rows: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    send_resize_intent_with_reason_and_sequence(
        stream,
        sequence,
        pane_id,
        cols,
        rows,
        protocol::ResizeReason::FrontendViewport,
    )
}

pub fn send_resize_intent_with_reason_and_sequence(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    cols: u32,
    rows: u32,
    reason: protocol::ResizeReason,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame = Session::initial().resize_intent_frame(
        "local-client",
        sequence.next_envelope_seq(),
        "local-actor",
        pane_id,
        cols,
        rows,
        reason,
    );
    wire::write_default_frame(stream, &frame)?;
    Ok(())
}

pub fn send_scrollback_fetch(
    stream: &mut UnixStream,
    pane_id: &str,
    range: ScrollbackRange,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sequence = ClientFrameSequence::default();
    send_scrollback_fetch_with_sequence(stream, &mut sequence, pane_id, range)
}

pub fn send_scrollback_fetch_with_sequence(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    range: ScrollbackRange,
) -> Result<(), Box<dyn std::error::Error>> {
    send_scrollback_fetch_with_known_version(
        stream,
        sequence,
        pane_id,
        ScrollbackFetchSpec {
            range,
            known_scrollback_version: 0,
        },
    )
}

pub fn send_scrollback_fetch_with_known_version(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    fetch_spec: ScrollbackFetchSpec,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame = Session::initial().scrollback_fetch_frame(
        InputFrameContext {
            connection_id: "local-client",
            seq: sequence.next_envelope_seq(),
            actor_id: "local-actor",
            pane_id,
            input_seq: 0,
        },
        fetch_spec,
    );
    wire::write_default_frame(stream, &frame)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientFrameSequence {
    next_envelope_seq: u64,
    next_input_seq: u64,
}

impl Default for ClientFrameSequence {
    fn default() -> Self {
        Self {
            next_envelope_seq: 1,
            next_input_seq: 1,
        }
    }
}

impl ClientFrameSequence {
    fn next_envelope_seq(&mut self) -> u64 {
        let seq = self.next_envelope_seq;
        self.next_envelope_seq += 1;
        seq
    }

    fn next_input_seq(&mut self) -> u64 {
        let seq = self.next_input_seq;
        self.next_input_seq += 1;
        seq
    }
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
    required_string(snapshot.session_id(), "workspace session_id")?;
    required_string(snapshot.active_tab_id(), "workspace active_tab_id")?;
    for index in 0..tabs.len() {
        let tab = tabs.get(index);
        required_string(tab.tab_id(), "workspace tab_id")?;
        required_string(tab.active_pane_id(), "workspace active_pane_id")?;
        let root = tab.root().ok_or("workspace tab has no root pane")?;
        validate_pane_node(root)?;
    }
    let active_tab_id = required_string(snapshot.active_tab_id(), "workspace active_tab_id")?;
    let tab = (0..tabs.len())
        .map(|index| tabs.get(index))
        .find(|tab| tab.tab_id() == Some(active_tab_id.as_str()))
        .ok_or_else(|| format!("workspace active tab not found: {active_tab_id}"))?;
    let active_pane_id = required_string(tab.active_pane_id(), "workspace active_pane_id")?;
    let root = tab.root().ok_or("workspace tab has no root pane")?;
    let pane = find_pane_node(root, &active_pane_id)
        .ok_or_else(|| format!("workspace active pane not found: {active_pane_id}"))?;
    let pane_tree = workspace_pane_summary_from_node(root)?;

    Ok(WorkspaceSummary {
        session_id: required_string(snapshot.session_id(), "workspace session_id")?,
        tab_id: required_string(tab.tab_id(), "workspace tab_id")?,
        pane_id: required_string(pane.pane_id(), "workspace pane_id")?,
        cols: pane.cols(),
        rows: pane.rows(),
        resize_policy: validate_resize_policy(pane.resize_policy())?,
        pane_tree: Some(pane_tree),
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
            validate_surface_kind(snapshot.surface())?;
            let rows = snapshot.rows_data().ok_or("pane surface has no rows")?;
            let styles = snapshot
                .styles()
                .map(decoded_styles)
                .unwrap_or_else(default_style_summaries);
            let hyperlinks = snapshot
                .hyperlinks()
                .map(decoded_hyperlinks)
                .transpose()?
                .unwrap_or_default();
            let row_updates = decoded_surface_rows(rows.len(), |index| {
                let row = rows.get(index);
                decoded_surface_row(
                    row.row(),
                    row.runs(),
                    row.dirty_hash(),
                    row.row_state_hash(),
                    row.semantic_prompt(),
                    row.dirty(),
                    row.kitty_virtual_placeholder(),
                )
            });
            validate_row_update_terminal_enums(&row_updates)?;
            validate_row_update_style_ids(&row_updates, &styles)?;
            validate_row_update_hyperlink_ids(&row_updates, &hyperlinks)?;
            let cursor = snapshot.cursor().map(CursorSummary::from_protocol);
            validate_cursor_summary(cursor)?;
            let modes = snapshot
                .modes()
                .map(TerminalModeSummary::from_protocol)
                .unwrap_or_default();
            validate_terminal_mode_summary(modes)?;
            let text = render_decoded_rows(&row_updates);
            let colors = decoded_terminal_colors(snapshot.colors());
            validate_palette_diff_scope(SurfaceUpdateKind::Snapshot, None, colors.as_ref())?;
            Ok(SurfaceUpdate {
                kind: SurfaceUpdateKind::Snapshot,
                pane_id: required_string(snapshot.pane_id(), "surface snapshot pane_id")?,
                version: snapshot.version(),
                base_version: None,
                patch_kind: None,
                cols: Some(snapshot.cols()),
                rows: Some(snapshot.rows()),
                surface: Some(snapshot.surface()),
                cursor,
                modes,
                title: snapshot
                    .metadata()
                    .and_then(|metadata| metadata.title())
                    .unwrap_or_default()
                    .to_owned(),
                working_directory: snapshot
                    .metadata()
                    .and_then(|metadata| metadata.working_directory())
                    .unwrap_or_default()
                    .to_owned(),
                colors,
                row_updates,
                styles,
                hyperlinks,
                text,
            })
        }
        protocol::EnvelopeBody::PaneSurfacePatch => {
            let patch = envelope
                .body_as_pane_surface_patch()
                .ok_or("missing pane surface patch body")?;
            validate_patch_kind(patch.kind())?;
            let rows = patch
                .row_updates()
                .ok_or("pane surface patch has no rows")?;
            let row_updates = decoded_surface_rows(rows.len(), |index| {
                let row = rows.get(index);
                decoded_surface_row(
                    row.row(),
                    row.runs(),
                    row.dirty_hash(),
                    row.row_state_hash(),
                    row.semantic_prompt(),
                    row.dirty(),
                    row.kitty_virtual_placeholder(),
                )
            });
            validate_row_update_terminal_enums(&row_updates)?;
            validate_no_row_patch_payload(patch.kind(), &row_updates)?;
            let cursor = patch.cursor().map(CursorSummary::from_protocol);
            validate_cursor_summary(cursor)?;
            let modes = patch
                .modes()
                .map(TerminalModeSummary::from_protocol)
                .unwrap_or_default();
            validate_terminal_mode_summary(modes)?;
            let text = render_decoded_rows(&row_updates);
            let colors = decoded_terminal_colors(patch.colors());
            validate_palette_diff_scope(
                SurfaceUpdateKind::Patch,
                Some(patch.kind()),
                colors.as_ref(),
            )?;
            Ok(SurfaceUpdate {
                kind: SurfaceUpdateKind::Patch,
                pane_id: required_string(patch.pane_id(), "surface patch pane_id")?,
                version: patch.version(),
                base_version: Some(patch.base_version()),
                patch_kind: Some(patch.kind()),
                cols: None,
                rows: None,
                surface: None,
                cursor,
                modes,
                title: patch
                    .metadata()
                    .and_then(|metadata| metadata.title())
                    .unwrap_or_default()
                    .to_owned(),
                working_directory: patch
                    .metadata()
                    .and_then(|metadata| metadata.working_directory())
                    .unwrap_or_default()
                    .to_owned(),
                colors,
                row_updates,
                styles: Vec::new(),
                hyperlinks: Vec::new(),
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
    row_state_hash: u64,
    semantic_prompt: protocol::RowSemanticPrompt,
    dirty: bool,
    kitty_virtual_placeholder: bool,
) -> SurfaceRowUpdate {
    let runs = runs.map(decoded_cell_runs).unwrap_or_default();
    SurfaceRowUpdate {
        row,
        text: render_run_summaries(&runs),
        runs,
        dirty_hash,
        row_state_hash,
        semantic_prompt,
        dirty,
        kitty_virtual_placeholder,
    }
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
            semantic_content: run.semantic_content(),
        });
    }
    decoded
}

fn decoded_styles(
    styles: flatbuffers::Vector<'_, flatbuffers::ForwardsUOffset<protocol::Style<'_>>>,
) -> Vec<StyleSummary> {
    let mut decoded = Vec::with_capacity(styles.len());
    for style_index in 0..styles.len() {
        let style = styles.get(style_index);
        decoded.push(StyleSummary {
            fg_rgba: style.fg_rgba(),
            bg_rgba: style.bg_rgba(),
            underline_rgba: style.underline_rgba(),
            flags: style.flags(),
        });
    }
    decoded
}

fn decoded_hyperlinks(
    hyperlinks: flatbuffers::Vector<'_, flatbuffers::ForwardsUOffset<protocol::Hyperlink<'_>>>,
) -> Result<Vec<HyperlinkSummary>, Box<dyn std::error::Error>> {
    let mut decoded = Vec::with_capacity(hyperlinks.len());
    for hyperlink_index in 0..hyperlinks.len() {
        let hyperlink = hyperlinks.get(hyperlink_index);
        let uri = hyperlink.uri().unwrap_or_default().to_owned();
        decoded.push(HyperlinkSummary {
            id: hyperlink.id(),
            uri,
            osc8_id: hyperlink.osc8_id().unwrap_or_default().to_owned(),
            params: hyperlink.params().unwrap_or_default().to_owned(),
        });
    }
    validate_hyperlink_table(&decoded)?;
    Ok(decoded)
}

fn decoded_terminal_colors(
    colors: Option<protocol::TerminalColorState<'_>>,
) -> Option<TerminalColorSummary> {
    let Some(colors) = colors else {
        return None;
    };
    let palette_diff_rgba: Vec<u32> = colors
        .palette_diff_rgba()
        .map(|palette| (0..palette.len()).map(|index| palette.get(index)).collect())
        .unwrap_or_default();
    let palette_diff_start = if palette_diff_rgba.is_empty() {
        None
    } else {
        Some(colors.palette_diff_start())
    };
    Some(TerminalColorSummary {
        default_fg_rgba: colors.default_fg_rgba(),
        default_bg_rgba: colors.default_bg_rgba(),
        cursor_rgba: colors.cursor_rgba(),
        cursor_rgba_set: colors.cursor_rgba_set(),
        palette_rgba: colors
            .palette_rgba()
            .map(|palette| (0..palette.len()).map(|index| palette.get(index)).collect())
            .unwrap_or_default(),
        palette_diff_start,
        palette_diff_rgba,
    })
}

fn validate_palette_diff_scope(
    kind: SurfaceUpdateKind,
    patch_kind: Option<protocol::PatchKind>,
    colors: Option<&TerminalColorSummary>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(colors) = colors else {
        return Ok(());
    };
    if !terminal_colors_have_palette_diff(colors) {
        return Ok(());
    }
    match kind {
        SurfaceUpdateKind::Snapshot => Err("surface snapshot cannot carry palette diff".into()),
        SurfaceUpdateKind::Patch if patch_kind == Some(protocol::PatchKind::ColorOnly) => Ok(()),
        SurfaceUpdateKind::Patch => Err("non-color surface patch cannot carry palette diff".into()),
    }
}

fn terminal_colors_have_palette_diff(colors: &TerminalColorSummary) -> bool {
    colors.palette_diff_start.is_some() || !colors.palette_diff_rgba.is_empty()
}

fn validate_attach_surface_update(
    status: &AttachStatusSummary,
    update: &SurfaceUpdate,
) -> Result<(), Box<dyn std::error::Error>> {
    if update.pane_id != status.pane_id {
        return Err(format!(
            "attach surface pane_id {} does not match attach status pane_id {}",
            update.pane_id, status.pane_id
        )
        .into());
    }

    match (status.surface_state, update.kind) {
        (protocol::AttachSurfaceState::Snapshot, SurfaceUpdateKind::Snapshot)
        | (protocol::AttachSurfaceState::Patch, SurfaceUpdateKind::Patch) => Ok(()),
        (protocol::AttachSurfaceState::Current, _) => {
            Err("attach status Current cannot include a surface frame".into())
        }
        (state, kind) => {
            Err(format!("attach status {state:?} does not match surface update {kind:?}").into())
        }
    }
}

fn required_string(value: Option<&str>, field: &str) -> Result<String, Box<dyn std::error::Error>> {
    let value = value.ok_or_else(|| format!("missing {field}"))?;
    if value.is_empty() {
        return Err(format!("empty {field}").into());
    }
    Ok(value.to_owned())
}

fn default_style_summaries() -> Vec<StyleSummary> {
    vec![StyleSummary {
        fg_rgba: 0,
        bg_rgba: 0,
        underline_rgba: 0,
        flags: 0,
    }]
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

/// Convert a structured style into an ANSI SGR escape sequence.
/// Style flags: bit 0=bold, 1=italic, 2=faint, 3=blink, 4=inverse,
/// 5=invisible, 6=strikethrough, 7=overline, 8..12=underline variants.
/// RGBA colors are packed as `[R, G, B, 0xFF]` in big-endian u32.
fn style_to_sgr(style: &StyleSummary) -> String {
    let mut params = Vec::new();
    let flags = style.flags;
    if flags & (1 << 0) != 0 {
        params.push("1".to_owned());
    }
    if flags & (1 << 2) != 0 {
        params.push("2".to_owned());
    }
    if flags & (1 << 1) != 0 {
        params.push("3".to_owned());
    }
    // Underline variants: bit 8=single, 9=double, 10=curly, 11=dotted, 12=dashed
    if flags & (1 << 8) != 0 {
        params.push("4".to_owned());
    } else if flags & (1 << 9) != 0 {
        params.push("21".to_owned());
    } else if flags & (0x1f << 10) != 0 {
        // Curly/dotted/dashed — use SGR 4:3/4:4/4:5 if terminal supports it,
        // fall back to single underline for broad compatibility.
        params.push("4".to_owned());
    }
    if flags & (1 << 3) != 0 {
        params.push("5".to_owned());
    }
    if flags & (1 << 4) != 0 {
        params.push("7".to_owned());
    }
    if flags & (1 << 5) != 0 {
        params.push("8".to_owned());
    }
    if flags & (1 << 6) != 0 {
        params.push("9".to_owned());
    }
    if flags & (1 << 7) != 0 {
        params.push("53".to_owned());
    }
    if style.fg_rgba != 0 {
        let [r, g, b, _] = style.fg_rgba.to_be_bytes();
        params.push(format!("38;2;{r};{g};{b}"));
    }
    if style.bg_rgba != 0 {
        let [r, g, b, _] = style.bg_rgba.to_be_bytes();
        params.push(format!("48;2;{r};{g};{b}"));
    }
    if params.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", params.join(";"))
    }
}

pub fn read_input_event_from_stream(
    stream: &mut UnixStream,
) -> Result<InputSummary, Box<dyn std::error::Error>> {
    let frame = wire::read_default_frame(stream)?;
    input_summary_from_frame(&frame)
}

fn read_optional_attached_client_frame_from_stream(
    stream: &mut UnixStream,
    timeout: Duration,
) -> Result<AttachedClientRead, Box<dyn std::error::Error>> {
    if !stream_readable_within(stream, timeout)? {
        return Ok(AttachedClientRead::NoFrame);
    }
    read_attached_client_frame_from_stream(stream)
}

fn stream_readable_within(stream: &UnixStream, timeout: Duration) -> io::Result<bool> {
    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    let mut pollfd = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(result > 0)
}

fn read_attached_client_frame_from_stream(
    stream: &mut UnixStream,
) -> Result<AttachedClientRead, Box<dyn std::error::Error>> {
    let frame = match wire::read_default_frame(stream) {
        Ok(frame) => frame,
        Err(wire::WireError::Io(err))
            if matches!(
                err.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::BrokenPipe
            ) =>
        {
            return Ok(AttachedClientRead::Closed);
        }
        Err(err) => return Err(err.into()),
    };
    let envelope = protocol::size_prefixed_root_as_envelope(&frame)?;
    match envelope.body_type() {
        protocol::EnvelopeBody::InputEvent => Ok(AttachedClientRead::Frame(
            AttachedClientFrame::Input(input_summary_from_frame(&frame)?),
        )),
        protocol::EnvelopeBody::ScrollbackFetch => Ok(AttachedClientRead::Frame(
            AttachedClientFrame::Scrollback(scrollback_fetch_from_frame(&frame)?),
        )),
        other => Err(format!("unexpected attached client frame: {other:?}").into()),
    }
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
    match read_scrollback_response_from_stream(stream)? {
        ScrollbackRead::Chunk(chunk) => Ok(chunk),
        ScrollbackRead::Error(error) => Err(server_error(error)),
    }
}

pub fn read_scrollback_chunk_with_stale_retry(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    start_line: u64,
    line_count: u32,
) -> Result<ScrollbackChunkSummary, Box<dyn std::error::Error>> {
    match read_scrollback_response_from_stream(stream)? {
        ScrollbackRead::Chunk(chunk) => Ok(chunk),
        ScrollbackRead::Error(error) if error.code == protocol::ErrorCode::StaleVersion => {
            send_scrollback_fetch_with_known_version(
                stream,
                sequence,
                pane_id,
                ScrollbackFetchSpec {
                    range: ScrollbackRange {
                        start_line,
                        line_count,
                    },
                    known_scrollback_version: 0,
                },
            )?;
            match read_scrollback_response_from_stream(stream)? {
                ScrollbackRead::Chunk(chunk) => Ok(chunk),
                ScrollbackRead::Error(error) => Err(server_error(error)),
            }
        }
        ScrollbackRead::Error(error) => Err(server_error(error)),
    }
}

pub fn fetch_scrollback_chunk_with_selection(
    stream: &mut UnixStream,
    sequence: &mut ClientFrameSequence,
    pane_id: &str,
    start_line: u64,
    line_count: u32,
    tail_count: Option<u32>,
    known_version_for: impl Fn(u64, u32) -> u64,
) -> Result<ScrollbackChunkSummary, Box<dyn std::error::Error>> {
    let (start_line, line_count) = if let Some(tail_count) = tail_count {
        send_scrollback_fetch_with_known_version(
            stream,
            sequence,
            pane_id,
            ScrollbackFetchSpec {
                range: ScrollbackRange {
                    start_line: 1,
                    line_count: 1,
                },
                known_scrollback_version: 0,
            },
        )?;
        let probe = read_scrollback_chunk_with_stale_retry(stream, sequence, pane_id, 1, 1)?;
        let tail_count_u64 = u64::from(tail_count);
        let start_line = if probe.total_lines > tail_count_u64 {
            probe.total_lines - tail_count_u64 + 1
        } else {
            1
        };
        (start_line, tail_count)
    } else {
        (start_line, line_count)
    };
    send_scrollback_fetch_with_known_version(
        stream,
        sequence,
        pane_id,
        ScrollbackFetchSpec {
            range: ScrollbackRange {
                start_line,
                line_count,
            },
            known_scrollback_version: known_version_for(start_line, line_count),
        },
    )?;
    read_scrollback_chunk_with_stale_retry(stream, sequence, pane_id, start_line, line_count)
}

fn read_scrollback_response_from_stream(
    stream: &mut UnixStream,
) -> Result<ScrollbackRead, Box<dyn std::error::Error>> {
    loop {
        let frame = wire::read_default_frame(stream)?;
        let envelope = protocol::size_prefixed_root_as_envelope(&frame)?;
        match envelope.body_type() {
            protocol::EnvelopeBody::ScrollbackChunk => {
                return Ok(ScrollbackRead::Chunk(scrollback_chunk_from_frame(&frame)?));
            }
            protocol::EnvelopeBody::Error => {
                let error = error_summary_from_frame(&frame)?;
                return Ok(ScrollbackRead::Error(error));
            }
            protocol::EnvelopeBody::PresenceUpdate => {}
            other => return Err(format!("unexpected envelope body: {other:?}").into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScrollbackRead {
    Chunk(ScrollbackChunkSummary),
    Error(ErrorSummary),
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

fn read_optional_server_error_from_stream(
    stream: &mut UnixStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let previous_timeout = match stream.read_timeout() {
        Ok(timeout) => timeout,
        Err(err) if socket_closed_error(&err) => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    if let Err(err) = stream.set_read_timeout(Some(Duration::from_millis(20))) {
        if socket_closed_error(&err) {
            return Ok(());
        }
        return Err(err.into());
    }
    let read_result = match wire::read_default_frame(stream) {
        Ok(frame) => {
            let envelope = protocol::size_prefixed_root_as_envelope(&frame)?;
            match envelope.body_type() {
                protocol::EnvelopeBody::Error => {
                    let error = error_summary_from_frame(&frame)?;
                    Err(server_error(error))
                }
                protocol::EnvelopeBody::PresenceUpdate
                | protocol::EnvelopeBody::WorkspaceTreeSnapshot
                | protocol::EnvelopeBody::PaneSurfaceSnapshot
                | protocol::EnvelopeBody::PaneSurfacePatch => Ok(()),
                other => Err(
                    format!("unexpected server frame before scrollback fetch: {other:?}").into(),
                ),
            }
        }
        Err(wire::WireError::Io(err))
            if matches!(
                err.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            Ok(())
        }
        Err(wire::WireError::Io(err))
            if matches!(
                err.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::BrokenPipe
            ) =>
        {
            Ok(())
        }
        Err(err) => Err(err.into()),
    };
    match (read_result, stream.set_read_timeout(previous_timeout)) {
        (Err(err), _) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(err)) if socket_closed_error(&err) => Ok(()),
        (Ok(()), Err(err)) => Err(err.into()),
    }
}

fn read_control_command_response(
    stream: &mut UnixStream,
) -> Result<WorkspaceSummary, Box<dyn std::error::Error>> {
    loop {
        let frame = wire::read_default_frame(stream)?;
        let envelope = protocol::size_prefixed_root_as_envelope(&frame)?;
        match envelope.body_type() {
            protocol::EnvelopeBody::WorkspaceTreeSnapshot => {
                return workspace_summary_from_frame(&frame);
            }
            protocol::EnvelopeBody::Error => {
                return Err(server_error(error_summary_from_frame(&frame)?));
            }
            protocol::EnvelopeBody::PresenceUpdate => {}
            other => return Err(format!("unexpected control response: {other:?}").into()),
        }
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
                protocol::EnvelopeBody::Error => {
                    Ok(LiveSurfaceRead::Error(error_summary_from_frame(&frame)?))
                }
                protocol::EnvelopeBody::PresenceUpdate => {
                    Ok(LiveSurfaceRead::Presence(presence_from_frame(&frame)?))
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

pub fn error_summary_from_frame(frame: &[u8]) -> Result<ErrorSummary, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::Error {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }
    let error = envelope.body_as_error().ok_or("missing error body")?;
    let pane_id = error
        .pane_id()
        .map(|pane_id| required_string(Some(pane_id), "error pane_id"))
        .transpose()?;
    Ok(ErrorSummary {
        code: validate_error_code(error.code())?,
        message: required_string(error.message(), "error message")?,
        retryable: error.retryable(),
        pane_id,
        input_seq: error.input_seq(),
    })
}

pub fn input_summary_from_frame(frame: &[u8]) -> Result<InputSummary, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::InputEvent {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }

    let input = envelope
        .body_as_input_event()
        .ok_or("missing input event body")?;
    let parts = match input.kind() {
        protocol::InputKind::Key => {
            let key = input.key().ok_or("missing key input")?;
            let modifiers = validate_input_modifiers(key.modifiers())?;
            InputSummaryParts {
                bytes: key.text_utf8().unwrap_or_default().as_bytes().to_vec(),
                key_name: key.key_name().map(ToOwned::to_owned),
                key_modifiers: modifiers,
                ..InputSummaryParts::default()
            }
        }
        protocol::InputKind::RawBytes => {
            let raw = input.raw().ok_or("missing raw input")?;
            InputSummaryParts {
                bytes: raw
                    .bytes()
                    .map(|bytes| bytes.iter().collect())
                    .unwrap_or_default(),
                ..InputSummaryParts::default()
            }
        }
        protocol::InputKind::Paste => {
            let paste = input.paste().ok_or("missing paste input")?;
            let paste_text = paste.text_utf8().unwrap_or_default().to_owned();
            InputSummaryParts {
                bytes: paste_text.as_bytes().to_vec(),
                paste_text: Some(paste_text),
                ..InputSummaryParts::default()
            }
        }
        protocol::InputKind::Focus => {
            let focus = input.focus().ok_or("missing focus input")?;
            InputSummaryParts {
                bytes: if focus.focused() {
                    b"\x1b[I".to_vec()
                } else {
                    b"\x1b[O".to_vec()
                },
                requires_focus_reporting: true,
                ..InputSummaryParts::default()
            }
        }
        protocol::InputKind::Mouse => {
            let mouse = input.mouse().ok_or("missing mouse input")?;
            let button = mouse_button_from_protocol(mouse.button())?;
            let action = mouse_action_from_protocol(mouse.action())?;
            let modifiers = validate_input_modifiers(mouse.modifiers())?;
            InputSummaryParts {
                mouse: Some(MouseSummary {
                    row: mouse.row(),
                    col: mouse.col(),
                    pixel_x: mouse.has_pixels().then_some(mouse.pixel_x()),
                    pixel_y: mouse.has_pixels().then_some(mouse.pixel_y()),
                    button,
                    action,
                    modifiers,
                }),
                requires_mouse_tracking: true,
                ..InputSummaryParts::default()
            }
        }
        other => return Err(format!("unexpected input kind: {other:?}").into()),
    };
    Ok(InputSummary {
        pane_id: required_string(input.pane_id(), "input pane_id")?,
        actor_id: required_string(input.actor_id(), "input actor_id")?,
        input_seq: input.input_seq(),
        text: String::from_utf8_lossy(&parts.bytes).into_owned(),
        bytes: parts.bytes,
        paste_text: parts.paste_text,
        key_name: parts.key_name,
        key_modifiers: parts.key_modifiers,
        mouse: parts.mouse,
        requires_focus_reporting: parts.requires_focus_reporting,
        requires_mouse_tracking: parts.requires_mouse_tracking,
    })
}

#[derive(Debug, Default)]
struct InputSummaryParts {
    bytes: Vec<u8>,
    paste_text: Option<String>,
    key_name: Option<String>,
    key_modifiers: u32,
    mouse: Option<MouseSummary>,
    requires_focus_reporting: bool,
    requires_mouse_tracking: bool,
}

fn validate_input_modifiers(modifiers: u32) -> Result<u32, Box<dyn std::error::Error>> {
    if modifiers & !INPUT_MODIFIER_MASK != 0 {
        return Err(format!("unsupported input modifier bits {modifiers:#x}").into());
    }
    Ok(modifiers)
}

fn mouse_action_from_protocol(
    action: protocol::MouseAction,
) -> Result<MouseAction, Box<dyn std::error::Error>> {
    if action.variant_name().is_none() {
        return Err(format!("unknown mouse action {}", action.0).into());
    }
    Ok(match action {
        protocol::MouseAction::Press => MouseAction::Press,
        protocol::MouseAction::Release => MouseAction::Release,
        protocol::MouseAction::Motion => MouseAction::Motion,
        _ => unreachable!("validated mouse action enum"),
    })
}

fn mouse_button_from_protocol(
    button: protocol::MouseButton,
) -> Result<MouseButton, Box<dyn std::error::Error>> {
    if button.variant_name().is_none() {
        return Err(format!("unknown mouse button {}", button.0).into());
    }
    Ok(match button {
        protocol::MouseButton::None => MouseButton::None,
        protocol::MouseButton::Left => MouseButton::Left,
        protocol::MouseButton::Middle => MouseButton::Middle,
        protocol::MouseButton::Right => MouseButton::Right,
        protocol::MouseButton::WheelUp => MouseButton::WheelUp,
        protocol::MouseButton::WheelDown => MouseButton::WheelDown,
        _ => unreachable!("validated mouse button enum"),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BracketedPasteMode {
    Disabled,
    Enabled,
}

impl BracketedPasteMode {
    fn from_enabled(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

fn paste_input_bytes(
    text: &str,
    mode: BracketedPasteMode,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if text.contains("\x1b[201~") {
        return Err("paste input contains a bracketed paste terminator".into());
    }
    if mode == BracketedPasteMode::Disabled {
        return Ok(text.as_bytes().to_vec());
    }

    let mut bytes = Vec::with_capacity("\x1b[200~".len() + text.len() + "\x1b[201~".len());
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    Ok(bytes)
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
        pane_id: required_string(resize.pane_id(), "resize pane_id")?,
        actor_id: required_string(resize.actor_id(), "resize actor_id")?,
        cols: resize.desired_cols(),
        rows: resize.desired_rows(),
        reason: validate_resize_reason(resize.reason())?,
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
    validate_presence_kind(presence.kind())?;
    let focused_pane_id = presence
        .focused_pane_id()
        .map(|pane_id| required_string(Some(pane_id), "presence focused_pane_id"))
        .transpose()?;
    Ok(PresenceSummary {
        actor_id: required_string(presence.actor_id(), "presence actor_id")?,
        user_id: required_string(presence.user_id(), "presence user_id")?,
        display_name: required_string(presence.display_name(), "presence display_name")?,
        mode: attach_mode_from_protocol(presence.mode())?,
        focused_pane_id,
    })
}

pub fn attach_status_from_frame(
    frame: &[u8],
) -> Result<AttachStatusSummary, Box<dyn std::error::Error>> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)?;
    if envelope.body_type() != protocol::EnvelopeBody::AttachStatus {
        return Err(format!("unexpected envelope body: {:?}", envelope.body_type()).into());
    }

    let status = envelope
        .body_as_attach_status()
        .ok_or("missing attach status body")?;
    Ok(AttachStatusSummary {
        pane_id: required_string(status.pane_id(), "attach status pane_id")?,
        surface_version: status.surface_version(),
        surface_state: validate_attach_surface_state(status.surface_state())?,
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
    validate_scrollback_fetch_range(fetch.start_line(), fetch.line_count())?;
    Ok(ScrollbackFetchSummary {
        pane_id: required_string(fetch.pane_id(), "scrollback fetch pane_id")?,
        actor_id: required_string(fetch.actor_id(), "scrollback fetch actor_id")?,
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
    let styles = chunk.styles().map(decoded_styles).unwrap_or_default();
    let hyperlinks = chunk
        .hyperlinks()
        .map(decoded_hyperlinks)
        .transpose()?
        .unwrap_or_default();
    let colors = decoded_terminal_colors(chunk.colors()).unwrap_or_default();
    if terminal_colors_have_palette_diff(&colors) {
        return Err("scrollback chunk cannot carry palette diff".into());
    }
    let rows = chunk.rows().ok_or("scrollback chunk has no rows")?;
    validate_scrollback_chunk_start_line(chunk.start_line())?;
    let mut lines = Vec::with_capacity(rows.len());
    for index in 0..rows.len() {
        let row = rows.get(index);
        let runs = row.runs().map(decoded_cell_runs).unwrap_or_default();
        validate_row_semantic_prompt(row.semantic_prompt())?;
        validate_cell_run_semantic_content(&runs)?;
        validate_cell_run_style_ids(&runs, &styles)?;
        validate_cell_run_hyperlink_ids(&runs, &hyperlinks)?;
        validate_scrollback_row_line(row.line())?;
        validate_scrollback_row_public_range(
            chunk.start_line(),
            chunk.total_lines(),
            index,
            row.line(),
        )?;
        lines.push(ScrollbackLine {
            line: row.line(),
            text: render_run_summaries(&runs),
            runs,
            dirty_hash: row.dirty_hash(),
            row_state_hash: row.row_state_hash(),
            semantic_prompt: row.semantic_prompt(),
            dirty: row.dirty(),
            kitty_virtual_placeholder: row.kitty_virtual_placeholder(),
        });
    }

    Ok(ScrollbackChunkSummary {
        pane_id: required_string(chunk.pane_id(), "scrollback chunk pane_id")?,
        scrollback_version: chunk.scrollback_version(),
        start_line: chunk.start_line(),
        total_lines: chunk.total_lines(),
        styles,
        hyperlinks,
        colors,
        lines,
    })
}

pub fn write_attach_request<W: Write>(writer: &mut W, request: &AttachRequest) -> io::Result<()> {
    let frame = request.frame();
    wire::write_frame(writer, &frame, ATTACH_MAX_FRAME_LEN).map_err(wire_error_to_io)
}

pub fn write_control_command<W: Write>(
    writer: &mut W,
    command: &ControlCommandSummary,
) -> io::Result<()> {
    let frame = command.frame();
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
                pane_id: required_io_string(surface.pane_id(), "known surface pane_id")?,
                version: surface.version(),
            });
        }
    }

    let focused_pane_id = request
        .focused_pane_id()
        .map(|pane_id| required_io_string(Some(pane_id), "focused pane_id"))
        .transpose()?;

    Ok(AttachRequest {
        actor_id: required_io_string(request.actor_id(), "attach actor_id")?,
        user_id: required_io_string(request.user_id(), "attach user_id")?,
        display_name: required_io_string(request.display_name(), "attach display_name")?,
        mode: attach_mode_from_protocol(request.mode()).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid attach mode: {err}"),
            )
        })?,
        focused_pane_id,
        known_surfaces,
    })
}

fn control_command_from_frame(frame: &[u8]) -> io::Result<ControlCommandSummary> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid control command frame: {err}"),
        )
    })?;
    if envelope.body_type() != protocol::EnvelopeBody::ControlCommand {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unexpected control command envelope body: {:?}",
                envelope.body_type()
            ),
        ));
    }
    let command = envelope.body_as_control_command().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing control command body")
    })?;
    Ok(ControlCommandSummary {
        actor_id: required_io_string(command.actor_id(), "control actor_id")?,
        command_seq: command.command_seq(),
        kind: validate_control_command_kind_io(command.kind())?,
        pane_id: optional_io_string(command.pane_id(), "control pane_id")?,
        tab_id: optional_io_string(command.tab_id(), "control tab_id")?,
        split_axis: validate_split_axis_io(command.split_axis())?,
        title: optional_io_string(command.title(), "control title")?,
    })
}

fn required_io_string(value: Option<&str>, field: &str) -> io::Result<String> {
    let value = value
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("missing {field}")))?;
    if value.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("empty {field}"),
        ));
    }
    Ok(value.to_owned())
}

fn optional_io_string(value: Option<&str>, field: &str) -> io::Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("empty {field}"),
        ));
    }
    Ok(Some(value.to_owned()))
}

fn validate_control_command_kind_io(
    kind: protocol::ControlCommandKind,
) -> io::Result<protocol::ControlCommandKind> {
    if kind.variant_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown control command kind {}", kind.0),
        ));
    }
    Ok(kind)
}

fn validate_split_axis_io(axis: protocol::SplitAxis) -> io::Result<protocol::SplitAxis> {
    if axis.variant_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown split axis {}", axis.0),
        ));
    }
    Ok(axis)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlCommandSummary {
    pub actor_id: String,
    pub command_seq: u64,
    pub kind: protocol::ControlCommandKind,
    pub pane_id: Option<String>,
    pub tab_id: Option<String>,
    pub split_axis: protocol::SplitAxis,
    pub title: Option<String>,
}

impl ControlCommandSummary {
    fn frame(&self) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let actor_id = builder.create_string(&self.actor_id);
        let pane_id = self
            .pane_id
            .as_ref()
            .map(|pane_id| builder.create_string(pane_id));
        let tab_id = self
            .tab_id
            .as_ref()
            .map(|tab_id| builder.create_string(tab_id));
        let title = self
            .title
            .as_ref()
            .map(|title| builder.create_string(title));
        let command = protocol::ControlCommand::create(
            &mut builder,
            &protocol::ControlCommandArgs {
                actor_id: Some(actor_id),
                command_seq: self.command_seq,
                kind: self.kind,
                pane_id,
                tab_id,
                split_axis: self.split_axis,
                title,
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
                seq: self.command_seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::ControlCommand,
                body: Some(command.as_union_value()),
            },
        );
        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }
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

fn attach_surface_state(response: Option<SurfaceResponse>) -> protocol::AttachSurfaceState {
    match response {
        Some(SurfaceResponse::Snapshot) => protocol::AttachSurfaceState::Snapshot,
        Some(SurfaceResponse::Patch { .. }) => protocol::AttachSurfaceState::Patch,
        None => protocol::AttachSurfaceState::Current,
    }
}

fn attach_mode_as_protocol(mode: AttachMode) -> protocol::AttachMode {
    match mode {
        AttachMode::ReadOnly => protocol::AttachMode::ReadOnly,
        AttachMode::ReadWrite => protocol::AttachMode::ReadWrite,
    }
}

fn validate_resize_policy(
    policy: protocol::ResizePolicy,
) -> Result<protocol::ResizePolicy, Box<dyn std::error::Error>> {
    if policy.variant_name().is_none() {
        return Err(format!("unknown resize policy {}", policy.0).into());
    }
    Ok(policy)
}

fn validate_pane_kind(
    kind: protocol::PaneKind,
) -> Result<protocol::PaneKind, Box<dyn std::error::Error>> {
    if kind.variant_name().is_none() {
        return Err(format!("unknown pane kind {}", kind.0).into());
    }
    Ok(kind)
}

fn validate_split_axis(
    axis: protocol::SplitAxis,
) -> Result<protocol::SplitAxis, Box<dyn std::error::Error>> {
    if axis.variant_name().is_none() {
        return Err(format!("unknown split axis {}", axis.0).into());
    }
    Ok(axis)
}

fn validate_pane_node(pane: protocol::PaneNode<'_>) -> Result<(), Box<dyn std::error::Error>> {
    required_string(pane.pane_id(), "workspace pane_id")?;
    validate_pane_kind(pane.kind())?;
    validate_split_axis(pane.split_axis())?;
    validate_resize_policy(pane.resize_policy())?;
    if let Some(children) = pane.children() {
        for index in 0..children.len() {
            validate_pane_node(children.get(index))?;
        }
    }
    Ok(())
}

fn find_pane_node<'a>(
    pane: protocol::PaneNode<'a>,
    pane_id: &str,
) -> Option<protocol::PaneNode<'a>> {
    if pane.pane_id() == Some(pane_id) {
        return Some(pane);
    }
    let children = pane.children()?;
    for index in 0..children.len() {
        if let Some(found) = find_pane_node(children.get(index), pane_id) {
            return Some(found);
        }
    }
    None
}

fn workspace_pane_summary_from_node(
    pane: protocol::PaneNode<'_>,
) -> Result<WorkspacePaneSummary, Box<dyn std::error::Error>> {
    let children = if let Some(children) = pane.children() {
        let mut summaries = Vec::with_capacity(children.len());
        for index in 0..children.len() {
            summaries.push(workspace_pane_summary_from_node(children.get(index))?);
        }
        summaries
    } else {
        Vec::new()
    };

    Ok(WorkspacePaneSummary {
        pane_id: required_string(pane.pane_id(), "workspace pane_id")?,
        cols: pane.cols(),
        rows: pane.rows(),
        resize_policy: validate_resize_policy(pane.resize_policy())?,
        split_axis: validate_split_axis(pane.split_axis())?,
        children,
    })
}

fn validate_resize_reason(
    reason: protocol::ResizeReason,
) -> Result<protocol::ResizeReason, Box<dyn std::error::Error>> {
    if reason.variant_name().is_none() {
        return Err(format!("unknown resize reason {}", reason.0).into());
    }
    Ok(reason)
}

fn validate_error_code(
    code: protocol::ErrorCode,
) -> Result<protocol::ErrorCode, Box<dyn std::error::Error>> {
    if code.variant_name().is_none() {
        return Err(format!("unknown error code {}", code.0).into());
    }
    Ok(code)
}

fn validate_attach_surface_state(
    state: protocol::AttachSurfaceState,
) -> Result<protocol::AttachSurfaceState, Box<dyn std::error::Error>> {
    if state.variant_name().is_none() {
        return Err(format!("unknown attach surface state {}", state.0).into());
    }
    Ok(state)
}

fn validate_presence_kind(
    kind: protocol::PresenceKind,
) -> Result<protocol::PresenceKind, Box<dyn std::error::Error>> {
    if kind.variant_name().is_none() {
        return Err(format!("unknown presence kind {}", kind.0).into());
    }
    Ok(kind)
}

fn attach_mode_from_protocol(
    mode: protocol::AttachMode,
) -> Result<AttachMode, Box<dyn std::error::Error>> {
    if mode.variant_name().is_none() {
        return Err(format!("unknown attach mode {}", mode.0).into());
    }
    Ok(match mode {
        protocol::AttachMode::ReadOnly => AttachMode::ReadOnly,
        protocol::AttachMode::ReadWrite => AttachMode::ReadWrite,
        _ => unreachable!("validated attach mode enum"),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub status: AttachStatusSummary,
    pub surface: Option<SurfaceUpdate>,
    pub scrollback: Option<ScrollbackChunkSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedAttach {
    pub workspace: WorkspaceSummary,
    pub status: AttachStatusSummary,
    pub surface_metadata: TerminalMetadataSummary,
    pub surface_kind: protocol::SurfaceKind,
    pub cursor: Option<CursorSummary>,
    pub modes: TerminalModeSummary,
    pub surface: RenderedSurfaceSummary,
    pub surface_text: Option<String>,
    pub scrollback: Option<ScrollbackChunkSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedSurfaceSummary {
    pub pane_id: String,
    pub version: u64,
    pub cols: u32,
    pub rows: u32,
    pub colors: TerminalColorSummary,
    pub styles: Vec<StyleSummary>,
    pub hyperlinks: Vec<HyperlinkSummary>,
    pub row_updates: Vec<SurfaceRowUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachStatusSummary {
    pub pane_id: String,
    pub surface_version: u64,
    pub surface_state: protocol::AttachSurfaceState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalMetadataSummary {
    pub title: String,
    pub working_directory: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedSurfaceSummary {
    pub surface_kind: protocol::SurfaceKind,
    pub cursor: Option<CursorSummary>,
    pub modes: TerminalModeSummary,
}

impl TerminalMetadataSummary {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.working_directory.is_empty()
    }

    pub fn display_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.title.is_empty() {
            lines.push(format!("title={}", sanitized_metadata_value(&self.title)));
        }
        if !self.working_directory.is_empty() {
            lines.push(format!(
                "working-directory={}",
                sanitized_metadata_value(&self.working_directory)
            ));
        }
        lines
    }

    fn from_surface(surface: &ClientPaneSurface) -> Self {
        Self {
            title: surface.title.clone(),
            working_directory: surface.working_directory.clone(),
        }
    }
}

fn sanitized_metadata_value(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch == '\t' || !ch.is_control() {
                ch
            } else {
                '\u{fffd}'
            }
        })
        .collect()
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
    pub modes: TerminalModeSummary,
    pub title: String,
    pub working_directory: String,
    pub colors: Option<TerminalColorSummary>,
    pub row_updates: Vec<SurfaceRowUpdate>,
    pub styles: Vec<StyleSummary>,
    pub hyperlinks: Vec<HyperlinkSummary>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveSurfaceRead {
    Workspace(WorkspaceSummary),
    Presence(PresenceSummary),
    Update(SurfaceUpdate),
    Error(ErrorSummary),
    NoFrame,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorSummary {
    pub code: protocol::ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub pane_id: Option<String>,
    pub input_seq: u64,
}

impl fmt::Display for ErrorSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} (code={:?}", self.message, self.code)?;
        if let Some(pane_id) = &self.pane_id {
            write!(formatter, ", pane_id={pane_id}")?;
        }
        if self.input_seq != 0 {
            write!(formatter, ", input_seq={}", self.input_seq)?;
        }
        if self.retryable {
            write!(formatter, ", retryable=true")?;
        }
        write!(formatter, ")")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerError {
    pub error: ErrorSummary,
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "server error: {}", self.error)
    }
}

impl std::error::Error for ServerError {}

fn server_error(error: ErrorSummary) -> Box<dyn std::error::Error> {
    Box::new(ServerError { error })
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
    pub blinking: bool,
}

impl CursorSummary {
    fn from_protocol(cursor: protocol::CursorState<'_>) -> Self {
        Self {
            row: cursor.row(),
            col: cursor.col(),
            visible: cursor.visible(),
            shape: cursor.shape(),
            blinking: cursor.blinking(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalModeSummary {
    pub bracketed_paste: bool,
    pub mouse_tracking: bool,
    pub focus_reporting: bool,
    pub application_keypad: bool,
    pub application_cursor: bool,
    pub origin: bool,
    pub wraparound: bool,
    pub mouse_tracking_mode: protocol::MouseTrackingMode,
    pub mouse_format: protocol::MouseFormat,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalColorSummary {
    pub default_fg_rgba: u32,
    pub default_bg_rgba: u32,
    pub cursor_rgba: u32,
    pub cursor_rgba_set: bool,
    pub palette_rgba: Vec<u32>,
    pub palette_diff_start: Option<u32>,
    pub palette_diff_rgba: Vec<u32>,
}

impl TerminalColorSummary {
    fn materialize(&self, base: &Self) -> Result<Self, Box<dyn std::error::Error>> {
        let mut colors = self.clone();
        if let Some(start) = self.palette_diff_start {
            let start = usize::try_from(start)?;
            if start > base.palette_rgba.len() {
                return Err(format!(
                    "palette diff start {start} exceeds cached palette length {}",
                    base.palette_rgba.len()
                )
                .into());
            }
            colors.palette_rgba = base.palette_rgba[..start].to_vec();
            colors
                .palette_rgba
                .extend(self.palette_diff_rgba.iter().copied());
        } else if self.palette_rgba.is_empty() {
            colors.palette_rgba = base.palette_rgba.clone();
        }
        colors.palette_diff_start = None;
        colors.palette_diff_rgba.clear();
        Ok(colors)
    }
}

impl Default for TerminalModeSummary {
    fn default() -> Self {
        Self {
            bracketed_paste: false,
            mouse_tracking: false,
            focus_reporting: false,
            application_keypad: false,
            application_cursor: false,
            origin: false,
            wraparound: true,
            mouse_tracking_mode: protocol::MouseTrackingMode::None,
            mouse_format: protocol::MouseFormat::X10,
        }
    }
}

impl TerminalModeSummary {
    fn from_protocol(modes: protocol::TerminalModeState<'_>) -> Self {
        Self {
            bracketed_paste: modes.bracketed_paste(),
            mouse_tracking: modes.mouse_tracking(),
            focus_reporting: modes.focus_reporting(),
            application_keypad: modes.application_keypad(),
            application_cursor: modes.application_cursor(),
            origin: modes.origin(),
            wraparound: modes.wraparound(),
            mouse_tracking_mode: modes.mouse_tracking_mode(),
            mouse_format: modes.mouse_format(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceRowUpdate {
    pub row: u32,
    pub text: String,
    pub runs: Vec<CellRunSummary>,
    pub dirty_hash: u64,
    pub row_state_hash: u64,
    pub semantic_prompt: protocol::RowSemanticPrompt,
    pub dirty: bool,
    pub kitty_virtual_placeholder: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRunSummary {
    pub text: String,
    pub cell_widths: Vec<u8>,
    pub style_id: u32,
    pub flags: u32,
    pub hyperlink_id: u32,
    pub semantic_content: protocol::CellSemanticContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleSummary {
    pub fg_rgba: u32,
    pub bg_rgba: u32,
    pub underline_rgba: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperlinkSummary {
    pub id: u32,
    pub uri: String,
    pub osc8_id: String,
    pub params: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPaneSurface {
    pub pane_id: String,
    pub version: u64,
    pub cols: u32,
    pub rows: u32,
    pub surface: protocol::SurfaceKind,
    pub cursor: Option<CursorSummary>,
    pub modes: TerminalModeSummary,
    pub title: String,
    pub working_directory: String,
    pub colors: TerminalColorSummary,
    styles: Vec<StyleSummary>,
    hyperlinks: Vec<HyperlinkSummary>,
    row_text: Vec<String>,
    row_runs: Vec<Vec<CellRunSummary>>,
    row_dirty_hashes: Vec<u64>,
    row_semantic_prompts: Vec<protocol::RowSemanticPrompt>,
    row_dirty: Vec<bool>,
    row_kitty_placeholders: Vec<bool>,
    row_state_hashes: Vec<u64>,
}

impl ClientPaneSurface {
    pub fn from_snapshot(update: &SurfaceUpdate) -> Result<Self, Box<dyn std::error::Error>> {
        validate_palette_diff_scope(SurfaceUpdateKind::Snapshot, None, update.colors.as_ref())?;
        let mut surface = Self {
            pane_id: update.pane_id.clone(),
            version: update.version,
            cols: update.cols.ok_or("surface snapshot missing cols")?,
            rows: update.rows.ok_or("surface snapshot missing rows")?,
            surface: update.surface.unwrap_or(protocol::SurfaceKind::Main),
            cursor: update.cursor,
            modes: update.modes,
            title: update.title.clone(),
            working_directory: update.working_directory.clone(),
            colors: update.colors.clone().unwrap_or_default(),
            styles: if update.styles.is_empty() {
                default_style_summaries()
            } else {
                update.styles.clone()
            },
            hyperlinks: update.hyperlinks.clone(),
            row_text: Vec::new(),
            row_runs: Vec::new(),
            row_dirty_hashes: Vec::new(),
            row_semantic_prompts: Vec::new(),
            row_dirty: Vec::new(),
            row_kitty_placeholders: Vec::new(),
            row_state_hashes: Vec::new(),
        };
        validate_surface_kind(surface.surface)?;
        validate_cursor_summary(surface.cursor)?;
        validate_terminal_mode_summary(surface.modes)?;
        validate_hyperlink_table(&surface.hyperlinks)?;
        let row_count =
            usize::try_from(surface.rows).map_err(|_| "surface row count does not fit in usize")?;
        surface.row_text.resize(row_count, String::new());
        surface.row_runs.resize(row_count, Vec::new());
        surface.row_dirty_hashes.resize(row_count, 0);
        surface
            .row_semantic_prompts
            .resize(row_count, protocol::RowSemanticPrompt::None);
        surface.row_dirty.resize(row_count, false);
        surface.row_kitty_placeholders.resize(row_count, false);
        surface.row_state_hashes.resize(row_count, 0);
        validate_row_update_indices(&update.row_updates, row_count)?;
        validate_row_update_style_ids(&update.row_updates, &surface.styles)?;
        validate_row_update_hyperlink_ids(&update.row_updates, &surface.hyperlinks)?;
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
        if let Some(patch_kind) = update.patch_kind {
            validate_patch_kind(patch_kind)?;
        }
        validate_palette_diff_scope(update.kind, update.patch_kind, update.colors.as_ref())?;
        validate_cursor_summary(update.cursor)?;
        validate_terminal_mode_summary(update.modes)?;
        if !update.hyperlinks.is_empty() {
            return Err("surface patch cannot change hyperlink table".into());
        }
        if let Some(patch_kind) = update.patch_kind {
            validate_no_row_patch_payload(patch_kind, &update.row_updates)?;
        }
        if update.patch_kind == Some(protocol::PatchKind::FullRefreshRequired) {
            return Err("surface patch requires full refresh".into());
        }
        if update.patch_kind == Some(protocol::PatchKind::CursorOnly) {
            if let Some(colors) = update.colors.as_ref()
                && colors != &self.colors
            {
                return Err("cursor-only patch changes terminal colors".into());
            }
            self.cursor = update.cursor;
            self.title = update.title.clone();
            self.working_directory = update.working_directory.clone();
            self.version = update.version;
            return Ok(());
        }
        if update.patch_kind == Some(protocol::PatchKind::ModeOnly) {
            if let Some(colors) = update.colors.as_ref()
                && colors != &self.colors
            {
                return Err("mode-only patch changes terminal colors".into());
            }
            self.cursor = update.cursor;
            self.modes = update.modes;
            self.title = update.title.clone();
            self.working_directory = update.working_directory.clone();
            self.version = update.version;
            return Ok(());
        }
        if update.patch_kind == Some(protocol::PatchKind::ColorOnly) {
            let Some(colors) = update.colors.as_ref() else {
                return Err("color-only patch is missing terminal colors".into());
            };
            let colors = colors.materialize(&self.colors)?;
            self.cursor = update.cursor;
            self.colors = colors;
            self.modes = update.modes;
            self.title = update.title.clone();
            self.working_directory = update.working_directory.clone();
            self.version = update.version;
            return Ok(());
        }
        if update.patch_kind != Some(protocol::PatchKind::ReplaceRows) {
            return Err(format!("unsupported surface patch kind: {:?}", update.patch_kind).into());
        }
        if let Some(colors) = update.colors.as_ref()
            && colors != &self.colors
        {
            return Err("replace-rows patch changes terminal colors".into());
        }
        validate_row_update_terminal_enums(&update.row_updates)?;
        validate_row_update_indices(&update.row_updates, self.row_text.len())?;
        validate_row_update_style_ids(&update.row_updates, &self.styles)?;
        validate_row_update_hyperlink_ids(&update.row_updates, &self.hyperlinks)?;
        self.apply_rows(&update.row_updates)?;
        self.cursor = update.cursor;
        self.modes = update.modes;
        self.title = update.title.clone();
        self.working_directory = update.working_directory.clone();
        self.version = update.version;
        Ok(())
    }

    pub fn render_text(&self) -> String {
        let visible_rows = self.visible_row_count();
        self.row_text[..visible_rows].join("\n")
    }

    /// Render visible rows with ANSI SGR styling derived from structured cell
    /// runs and the style table. The client never parses raw VT bytes; it
    /// reconstructs styled output from the protocol's structured style objects.
    pub fn render_styled_text(&self) -> String {
        let visible_rows = self.visible_row_count();
        let mut output = String::new();
        for row_index in 0..visible_rows {
            if row_index > 0 {
                output.push('\n');
            }
            output.push_str(&self.render_styled_row(row_index));
        }
        output
    }

    pub(crate) fn render_styled_row(&self, row_index: usize) -> String {
        let runs = &self.row_runs[row_index];
        if runs.is_empty() {
            return self.row_text[row_index].clone();
        }
        let mut output = String::new();
        for run in runs {
            let style = self.styles.get(run.style_id as usize);
            let needs_sgr = style.is_some_and(|s| s.fg_rgba != 0 || s.bg_rgba != 0 || s.flags != 0);
            if needs_sgr && let Some(style) = style {
                output.push_str(&style_to_sgr(style));
            }
            output.push_str(&run.text);
            if needs_sgr {
                output.push_str("\x1b[0m");
            }
        }
        output
    }

    /// Returns true when the style table contains any non-default styles,
    /// indicating that the terminal engine produced structured style data.
    pub fn has_styled_runs(&self) -> bool {
        self.styles.len() > 1
            || self
                .styles
                .first()
                .is_some_and(|s| s.fg_rgba != 0 || s.bg_rgba != 0 || s.flags != 0)
    }

    pub(crate) fn visible_row_count(&self) -> usize {
        self.row_text
            .iter()
            .rposition(|row| !row.is_empty())
            .map(|index| index + 1)
            .unwrap_or(0)
    }

    pub(crate) fn row_text(&self) -> &[String] {
        &self.row_text
    }

    fn summary(&self) -> RenderedSurfaceSummary {
        let row_updates = (0..self.visible_row_count())
            .map(|index| SurfaceRowUpdate {
                row: u32::try_from(index).expect("surface row index fits in u32"),
                text: self.row_text[index].clone(),
                runs: self.row_runs[index].clone(),
                dirty_hash: self.row_dirty_hashes[index],
                row_state_hash: self.row_state_hashes[index],
                semantic_prompt: self.row_semantic_prompts[index],
                dirty: self.row_dirty[index],
                kitty_virtual_placeholder: self.row_kitty_placeholders[index],
            })
            .collect();
        RenderedSurfaceSummary {
            pane_id: self.pane_id.clone(),
            version: self.version,
            cols: self.cols,
            rows: self.rows,
            colors: self.colors.clone(),
            styles: self.styles.clone(),
            hyperlinks: self.hyperlinks.clone(),
            row_updates,
        }
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
            self.row_runs[index] = if row.runs.is_empty() {
                vec![CellRunSummary::plain(&row.text)]
            } else {
                row.runs.clone()
            };
            self.row_dirty_hashes[index] = row.dirty_hash;
            self.row_semantic_prompts[index] = row.semantic_prompt;
            self.row_dirty[index] = row.dirty;
            self.row_kitty_placeholders[index] = row.kitty_virtual_placeholder;
            self.row_state_hashes[index] = row.row_state_hash;
        }
        Ok(())
    }
}

impl CellRunSummary {
    fn plain(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            cell_widths: text.chars().map(|_| 1).collect(),
            style_id: 0,
            flags: 0,
            hyperlink_id: 0,
            semantic_content: protocol::CellSemanticContent::Output,
        }
    }
}

fn row_runs_for_text(
    row_text: &[String],
    mut row_runs: Vec<Vec<CellRunSummary>>,
) -> Vec<Vec<CellRunSummary>> {
    for (index, text) in row_text.iter().enumerate() {
        if row_runs[index].is_empty() || render_run_summaries(&row_runs[index]) != *text {
            row_runs[index] = vec![CellRunSummary::plain(text)];
        }
    }
    row_runs
}

fn validate_row_update_hyperlink_ids(
    rows: &[SurfaceRowUpdate],
    hyperlinks: &[HyperlinkSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for row in rows {
        validate_cell_run_hyperlink_ids(&row.runs, hyperlinks)?;
    }
    Ok(())
}

fn validate_no_row_patch_payload(
    patch_kind: protocol::PatchKind,
    rows: &[SurfaceRowUpdate],
) -> Result<(), Box<dyn std::error::Error>> {
    if !rows.is_empty()
        && matches!(
            patch_kind,
            protocol::PatchKind::CursorOnly
                | protocol::PatchKind::ModeOnly
                | protocol::PatchKind::ColorOnly
                | protocol::PatchKind::FullRefreshRequired
        )
    {
        return Err(format!("{patch_kind:?} patch cannot carry row updates").into());
    }
    Ok(())
}

fn validate_scrollback_fetch_range(
    start_line: u64,
    line_count: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    if start_line == 0 {
        return Err("scrollback fetch start_line must be 1-based".into());
    }
    if line_count == 0 {
        return Err("scrollback fetch line_count must be nonzero".into());
    }
    Ok(())
}

fn validate_scrollback_chunk_start_line(start_line: u64) -> Result<(), Box<dyn std::error::Error>> {
    if start_line == 0 {
        return Err("scrollback chunk start_line must be 1-based".into());
    }
    Ok(())
}

fn validate_scrollback_row_line(line: u64) -> Result<(), Box<dyn std::error::Error>> {
    if line == 0 {
        return Err("scrollback row line must be 1-based".into());
    }
    Ok(())
}

fn validate_scrollback_row_public_range(
    start_line: u64,
    total_lines: u64,
    index: usize,
    row_line: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let offset = u64::try_from(index).map_err(|_| "scrollback row index does not fit in u64")?;
    let expected = start_line
        .checked_add(offset)
        .ok_or("scrollback row line overflow")?;
    if row_line != expected {
        return Err(format!(
            "scrollback row line {row_line} does not match expected public line {expected}"
        )
        .into());
    }
    if row_line > total_lines {
        return Err(
            format!("scrollback row line {row_line} exceeds total_lines {total_lines}").into(),
        );
    }
    Ok(())
}

fn validate_patch_kind(patch_kind: protocol::PatchKind) -> Result<(), Box<dyn std::error::Error>> {
    if patch_kind.variant_name().is_some() {
        Ok(())
    } else {
        Err(format!("unknown surface patch kind {}", patch_kind.0).into())
    }
}

fn validate_surface_kind(surface: protocol::SurfaceKind) -> Result<(), Box<dyn std::error::Error>> {
    if surface.variant_name().is_some() {
        Ok(())
    } else {
        Err(format!("unknown surface kind {}", surface.0).into())
    }
}

fn validate_cursor_summary(
    cursor: Option<CursorSummary>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(cursor) = cursor
        && cursor.shape.variant_name().is_none()
    {
        return Err(format!("unknown cursor shape {}", cursor.shape.0).into());
    }
    Ok(())
}

fn validate_terminal_mode_summary(
    modes: TerminalModeSummary,
) -> Result<(), Box<dyn std::error::Error>> {
    if modes.mouse_tracking_mode.variant_name().is_none() {
        return Err(format!(
            "unknown mouse tracking mode {}",
            modes.mouse_tracking_mode.0
        )
        .into());
    }
    if modes.mouse_format.variant_name().is_none() {
        return Err(format!("unknown mouse format {}", modes.mouse_format.0).into());
    }
    Ok(())
}

fn validate_row_update_indices(
    rows: &[SurfaceRowUpdate],
    row_count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut seen = vec![false; row_count];
    for row in rows {
        let index =
            usize::try_from(row.row).map_err(|_| "surface row index does not fit in usize")?;
        let Some(seen_row) = seen.get_mut(index) else {
            return Err(
                format!("surface row {} is outside {row_count} row surface", row.row).into(),
            );
        };
        if *seen_row {
            return Err(format!("surface row {} is repeated in one update", row.row).into());
        }
        *seen_row = true;
    }
    Ok(())
}

fn validate_row_update_terminal_enums(
    rows: &[SurfaceRowUpdate],
) -> Result<(), Box<dyn std::error::Error>> {
    for row in rows {
        validate_row_semantic_prompt(row.semantic_prompt)?;
        validate_cell_run_semantic_content(&row.runs)?;
    }
    Ok(())
}

fn validate_row_semantic_prompt(
    prompt: protocol::RowSemanticPrompt,
) -> Result<(), Box<dyn std::error::Error>> {
    if prompt.variant_name().is_some() {
        Ok(())
    } else {
        Err(format!("unknown row semantic prompt {}", prompt.0).into())
    }
}

fn validate_cell_run_semantic_content(
    runs: &[CellRunSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for run in runs {
        if run.semantic_content.variant_name().is_none() {
            return Err(format!(
                "cell run references unknown semantic content {}",
                run.semantic_content.0
            )
            .into());
        }
    }
    Ok(())
}

fn validate_row_update_style_ids(
    rows: &[SurfaceRowUpdate],
    styles: &[StyleSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for row in rows {
        validate_cell_run_style_ids(&row.runs, styles)?;
    }
    Ok(())
}

fn validate_cell_run_style_ids(
    runs: &[CellRunSummary],
    styles: &[StyleSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    let style_count = styles.len().max(1);
    for run in runs {
        let style_id = usize::try_from(run.style_id)
            .map_err(|_| format!("cell run style_id {} is too large", run.style_id))?;
        if style_id >= style_count {
            return Err(format!("cell run references unknown style_id {}", run.style_id).into());
        }
    }
    Ok(())
}

fn validate_cell_run_hyperlink_ids(
    runs: &[CellRunSummary],
    hyperlinks: &[HyperlinkSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for run in runs {
        if run.hyperlink_id != 0
            && !hyperlinks
                .iter()
                .any(|hyperlink| hyperlink.id == run.hyperlink_id)
        {
            return Err(format!(
                "cell run references unknown hyperlink_id {}",
                run.hyperlink_id
            )
            .into());
        }
    }
    Ok(())
}

fn validate_hyperlink_table(
    hyperlinks: &[HyperlinkSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for (index, hyperlink) in hyperlinks.iter().enumerate() {
        if hyperlink.id == 0 {
            return Err("hyperlink table entry uses reserved id 0".into());
        }
        if hyperlink.uri.is_empty() {
            return Err(format!("hyperlink {} is missing target uri", hyperlink.id).into());
        }
        if hyperlinks[..index]
            .iter()
            .any(|existing| existing.id == hyperlink.id)
        {
            return Err(format!("duplicate hyperlink id {}", hyperlink.id).into());
        }
    }
    Ok(())
}

fn validate_cached_row_hyperlink_ids(
    row_runs: &[Vec<CellRunSummary>],
    hyperlinks: &[HyperlinkSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for runs in row_runs {
        validate_cell_run_hyperlink_ids(runs, hyperlinks)?;
    }
    Ok(())
}

fn validate_cached_row_style_ids(
    row_runs: &[Vec<CellRunSummary>],
    styles: &[StyleSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for runs in row_runs {
        validate_cell_run_style_ids(runs, styles)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientAttachState {
    scope: Option<SocketIdentity>,
    surfaces: Vec<ClientPaneSurface>,
    scrollbacks: Vec<ClientPaneScrollback>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPaneScrollback {
    pub pane_id: String,
    pub version: u64,
    pub start_line: u64,
    pub line_count: u32,
    pub total_lines: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientStateSummary {
    pub scope: Option<SocketIdentitySummary>,
    pub surfaces: Vec<ClientStateSurfaceSummary>,
    pub scrollbacks: Vec<ClientPaneScrollback>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketIdentitySummary {
    pub dev: u64,
    pub ino: u64,
    pub ctime: i64,
    pub ctime_nsec: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientStateSurfaceSummary {
    pub pane_id: String,
    pub version: u64,
    pub cols: u32,
    pub rows: u32,
    pub surface_kind: protocol::SurfaceKind,
    pub title: String,
    pub working_directory: String,
    pub cursor: Option<CursorSummary>,
    pub modes: TerminalModeSummary,
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
        let tmp_path = state_save_tmp_path(path);
        fs::write(&tmp_path, self.encode())?;
        match fs::rename(&tmp_path, path) {
            Ok(()) => Ok(()),
            Err(err) => {
                let _ = fs::remove_file(&tmp_path);
                Err(err)
            }
        }
    }

    pub fn summary(&self) -> ClientStateSummary {
        ClientStateSummary {
            scope: self.scope.map(SocketIdentitySummary::from),
            surfaces: self
                .surfaces
                .iter()
                .map(|surface| ClientStateSurfaceSummary {
                    pane_id: surface.pane_id.clone(),
                    version: surface.version,
                    cols: surface.cols,
                    rows: surface.rows,
                    surface_kind: surface.surface,
                    title: surface.title.clone(),
                    working_directory: surface.working_directory.clone(),
                    cursor: surface.cursor,
                    modes: surface.modes,
                })
                .collect(),
            scrollbacks: self.scrollbacks.clone(),
        }
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

    pub fn known_surfaces_for_scope(
        &self,
        scope: Option<SocketIdentity>,
    ) -> Vec<KnownSurfaceVersion> {
        if self.scope == scope {
            self.known_surfaces()
        } else {
            Vec::new()
        }
    }

    pub fn cached_scrollback_version_for_scope(
        &self,
        scope: Option<SocketIdentity>,
        pane_id: &str,
        start_line: u64,
        line_count: u32,
    ) -> Option<u64> {
        if self.scope != scope {
            return None;
        }
        self.cached_scrollback_version(pane_id, start_line, line_count)
    }

    pub fn known_scrollback_versions_for_scope(
        &self,
        scope: Option<SocketIdentity>,
    ) -> Vec<KnownScrollbackVersion> {
        if self.scope != scope {
            return Vec::new();
        }
        self.scrollbacks
            .iter()
            .map(|scrollback| KnownScrollbackVersion {
                pane_id: scrollback.pane_id.clone(),
                start_line: scrollback.start_line,
                line_count: scrollback.line_count,
                version: scrollback.version,
            })
            .collect()
    }

    pub fn apply_scope(&mut self, scope: Option<SocketIdentity>) {
        if self.scope != scope {
            self.surfaces.clear();
            self.scrollbacks.clear();
        }
        self.scope = scope;
    }

    pub fn render_attach(
        &mut self,
        snapshot: AttachSnapshot,
    ) -> Result<RenderedAttach, Box<dyn std::error::Error>> {
        let pane_id = snapshot.status.pane_id.clone();
        let surface_text = match snapshot.surface.as_ref() {
            Some(update) => {
                validate_attach_surface_update(&snapshot.status, update)?;
                Some(self.apply_surface_update(update)?)
            }
            None => {
                if snapshot.status.surface_state != protocol::AttachSurfaceState::Current {
                    return Err(format!(
                        "attach status {:?} requires a matching surface frame",
                        snapshot.status.surface_state
                    )
                    .into());
                }
                let surface =
                    self.cached_current_surface(&pane_id, snapshot.status.surface_version)?;
                Some(surface.render_text())
            }
        };
        let surface = self.cached_current_surface(&pane_id, snapshot.status.surface_version)?;
        let surface_metadata = TerminalMetadataSummary::from_surface(surface);
        let surface_kind = surface.surface;
        let cursor = surface.cursor;
        let modes = surface.modes;
        let surface_summary = surface.summary();

        if let Some(scrollback) = snapshot.scrollback.as_ref() {
            self.cache_scrollback_chunk(scrollback);
        }

        Ok(RenderedAttach {
            workspace: snapshot.workspace,
            status: snapshot.status,
            surface_metadata,
            surface_kind,
            cursor,
            modes,
            surface: surface_summary,
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

    pub fn render_speculative_echo(
        &self,
        overlay: &mut SpeculativeEchoOverlay,
        pane_id: &str,
        input_seq: u64,
        text: &str,
        styled: bool,
    ) -> Option<String> {
        let surface = self
            .surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)?;
        overlay.predict_printable_key(surface, input_seq, text)?;
        if styled {
            overlay.render_underlined_styled(surface)
        } else {
            overlay.render_underlined(surface)
        }
    }

    /// Render a surface update with optional ANSI SGR styling from structured
    /// cell runs. When `styled` is true and the engine produced style data,
    /// the output contains SGR escape sequences reconstructed from protocol
    /// objects — the client never parses raw VT bytes.
    pub fn render_surface_update_styled(
        &mut self,
        update: &SurfaceUpdate,
        styled: bool,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.apply_surface_update_styled(update, styled)
    }

    pub fn cached_surface_text(&self, pane_id: &str) -> Option<String> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(ClientPaneSurface::render_text)
    }

    /// Like `cached_surface_text` but with optional ANSI SGR styling.
    pub fn cached_surface_text_styled(&self, pane_id: &str, styled: bool) -> Option<String> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(|surface| {
                if styled && surface.has_styled_runs() {
                    surface.render_styled_text()
                } else {
                    surface.render_text()
                }
            })
    }

    pub fn cached_surface_metadata(&self, pane_id: &str) -> Option<TerminalMetadataSummary> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(TerminalMetadataSummary::from_surface)
    }

    pub fn cached_surface_modes(&self, pane_id: &str) -> Option<TerminalModeSummary> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(|surface| surface.modes)
    }

    pub fn cached_surface_summary(&self, pane_id: &str) -> Option<CachedSurfaceSummary> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(|surface| CachedSurfaceSummary {
                surface_kind: surface.surface,
                cursor: surface.cursor,
                modes: surface.modes,
            })
    }

    fn cached_current_surface(
        &self,
        pane_id: &str,
        version: u64,
    ) -> Result<&ClientPaneSurface, Box<dyn std::error::Error>> {
        let Some(surface) = self
            .surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
        else {
            return Err(format!("current attach has no cached surface for pane {pane_id}").into());
        };
        if surface.version != version {
            return Err(format!(
                "current attach surface version mismatch for pane {pane_id}: cached {}, status {}",
                surface.version, version
            )
            .into());
        }
        Ok(surface)
    }

    pub fn cached_scrollback_version(
        &self,
        pane_id: &str,
        start_line: u64,
        line_count: u32,
    ) -> Option<u64> {
        self.scrollbacks
            .iter()
            .find(|scrollback| {
                scrollback.pane_id == pane_id
                    && scrollback.start_line == start_line
                    && scrollback.line_count == line_count
            })
            .map(|scrollback| scrollback.version)
    }

    fn apply_surface_update(
        &mut self,
        update: &SurfaceUpdate,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.apply_surface_update_styled(update, false)
    }

    fn apply_surface_update_styled(
        &mut self,
        update: &SurfaceUpdate,
        styled: bool,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if let Some(surface) = self
            .surfaces
            .iter_mut()
            .find(|surface| surface.pane_id == update.pane_id)
        {
            surface.apply_update(update)?;
            return Ok(if styled && surface.has_styled_runs() {
                surface.render_styled_text()
            } else {
                surface.render_text()
            });
        }

        if update.kind != SurfaceUpdateKind::Snapshot {
            return Err(format!(
                "cannot apply pane {} patch without a cached snapshot",
                update.pane_id
            )
            .into());
        }

        let surface = ClientPaneSurface::from_snapshot(update)?;
        let rendered = if styled && surface.has_styled_runs() {
            surface.render_styled_text()
        } else {
            surface.render_text()
        };
        self.surfaces.push(surface);
        Ok(rendered)
    }

    pub fn cache_scrollback_chunk(&mut self, chunk: &ScrollbackChunkSummary) {
        if chunk.lines.is_empty() {
            return;
        }
        let line_count = u32::try_from(chunk.lines.len()).unwrap_or(u32::MAX);
        let cached = ClientPaneScrollback {
            pane_id: chunk.pane_id.clone(),
            version: chunk.scrollback_version,
            start_line: chunk.start_line,
            line_count,
            total_lines: chunk.total_lines,
        };
        if let Some(existing) = self.scrollbacks.iter_mut().find(|scrollback| {
            scrollback.pane_id == cached.pane_id
                && scrollback.start_line == cached.start_line
                && scrollback.line_count == cached.line_count
        }) {
            *existing = cached;
        } else {
            self.scrollbacks.push(cached);
        }
    }

    fn encode(&self) -> String {
        let mut encoded = String::from("NMUX_CLIENT_STATE 8\n");
        if let Some(scope) = self.scope {
            encoded.push_str("scope socket ");
            encoded.push_str(&scope.dev.to_string());
            encoded.push(' ');
            encoded.push_str(&scope.ino.to_string());
            encoded.push(' ');
            encoded.push_str(&scope.ctime.to_string());
            encoded.push(' ');
            encoded.push_str(&scope.ctime_nsec.to_string());
            encoded.push('\n');
        }
        for scrollback in &self.scrollbacks {
            encoded.push_str("scrollback ");
            encoded.push_str(&hex_encode(scrollback.pane_id.as_bytes()));
            encoded.push(' ');
            encoded.push_str(&scrollback.version.to_string());
            encoded.push(' ');
            encoded.push_str(&scrollback.start_line.to_string());
            encoded.push(' ');
            encoded.push_str(&scrollback.line_count.to_string());
            encoded.push(' ');
            encoded.push_str(&scrollback.total_lines.to_string());
            encoded.push('\n');
        }
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
                    encoded.push(' ');
                    encoded.push_str(if cursor.blinking { "1" } else { "0" });
                    encoded.push('\n');
                }
                None => encoded.push_str("cursor none\n"),
            }
            encoded.push_str("modes ");
            encoded.push_str(if surface.modes.bracketed_paste {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(if surface.modes.mouse_tracking {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(if surface.modes.focus_reporting {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(if surface.modes.application_keypad {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(if surface.modes.application_cursor {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(if surface.modes.origin { "1" } else { "0" });
            encoded.push(' ');
            encoded.push_str(if surface.modes.wraparound { "1" } else { "0" });
            encoded.push(' ');
            encoded.push_str(&surface.modes.mouse_tracking_mode.0.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.modes.mouse_format.0.to_string());
            encoded.push('\n');
            encoded.push_str("title ");
            encoded.push_str(&hex_encode(surface.title.as_bytes()));
            encoded.push('\n');
            encoded.push_str("pwd ");
            encoded.push_str(&hex_encode(surface.working_directory.as_bytes()));
            encoded.push('\n');
            encoded.push_str("colors ");
            encoded.push_str(&surface.colors.default_fg_rgba.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.colors.default_bg_rgba.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.colors.cursor_rgba.to_string());
            encoded.push(' ');
            encoded.push_str(if surface.colors.cursor_rgba_set {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(&hex_encode(
                &surface
                    .colors
                    .palette_rgba
                    .iter()
                    .flat_map(|color| color.to_be_bytes())
                    .collect::<Vec<_>>(),
            ));
            encoded.push('\n');
            for style in &surface.styles {
                encoded.push_str("style ");
                encoded.push_str(&style.fg_rgba.to_string());
                encoded.push(' ');
                encoded.push_str(&style.bg_rgba.to_string());
                encoded.push(' ');
                encoded.push_str(&style.underline_rgba.to_string());
                encoded.push(' ');
                encoded.push_str(&style.flags.to_string());
                encoded.push('\n');
            }
            for hyperlink in &surface.hyperlinks {
                encoded.push_str("hyperlink ");
                encoded.push_str(&hyperlink.id.to_string());
                encoded.push(' ');
                encoded.push_str(&hex_encode(hyperlink.uri.as_bytes()));
                encoded.push(' ');
                encoded.push_str(&state_hex_field(hyperlink.osc8_id.as_bytes()));
                encoded.push(' ');
                encoded.push_str(&state_hex_field(hyperlink.params.as_bytes()));
                encoded.push('\n');
            }
            for (index, row) in surface.row_text.iter().enumerate() {
                encoded.push_str("row ");
                encoded.push_str(&index.to_string());
                encoded.push(' ');
                encoded.push_str(&hex_encode(row.as_bytes()));
                encoded.push('\n');
                encoded.push_str("rowmeta ");
                encoded.push_str(&index.to_string());
                encoded.push(' ');
                encoded.push_str(&surface.row_semantic_prompts[index].0.to_string());
                encoded.push(' ');
                encoded.push_str(if surface.row_dirty[index] { "1" } else { "0" });
                encoded.push(' ');
                encoded.push_str(if surface.row_kitty_placeholders[index] {
                    "1"
                } else {
                    "0"
                });
                encoded.push(' ');
                encoded.push_str(&surface.row_state_hashes[index].to_string());
                encoded.push(' ');
                encoded.push_str(&surface.row_dirty_hashes[index].to_string());
                encoded.push('\n');
                for run in &surface.row_runs[index] {
                    encoded.push_str("run ");
                    encoded.push_str(&index.to_string());
                    encoded.push(' ');
                    encoded.push_str(&hex_encode(run.text.as_bytes()));
                    encoded.push(' ');
                    encoded.push_str(&hex_encode(&run.cell_widths));
                    encoded.push(' ');
                    encoded.push_str(&run.style_id.to_string());
                    encoded.push(' ');
                    encoded.push_str(&run.flags.to_string());
                    encoded.push(' ');
                    encoded.push_str(&run.hyperlink_id.to_string());
                    encoded.push(' ');
                    encoded.push_str(&run.semantic_content.0.to_string());
                    encoded.push('\n');
                }
            }
            encoded.push_str("end\n");
        }
        encoded
    }

    fn decode(encoded: &str) -> io::Result<Self> {
        let mut lines = encoded.lines();
        let Some(header) = lines.next() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid nmux client state header",
            ));
        };
        if header != "NMUX_CLIENT_STATE 1"
            && header != "NMUX_CLIENT_STATE 2"
            && header != "NMUX_CLIENT_STATE 3"
            && header != "NMUX_CLIENT_STATE 4"
            && header != "NMUX_CLIENT_STATE 5"
            && header != "NMUX_CLIENT_STATE 6"
            && header != "NMUX_CLIENT_STATE 7"
            && header != "NMUX_CLIENT_STATE 8"
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid nmux client state header",
            ));
        }

        let mut scope = None;
        let mut surfaces = Vec::new();
        let mut scrollbacks = Vec::new();
        while let Some(line) = lines.next() {
            let scope_parts = line.split(' ').collect::<Vec<_>>();
            if let ["scope", "socket", dev, ino] = scope_parts.as_slice() {
                scope = Some(SocketIdentity {
                    dev: parse_state_u64(dev)?,
                    ino: parse_state_u64(ino)?,
                    ctime: 0,
                    ctime_nsec: 0,
                });
                continue;
            }
            if let ["scope", "socket", dev, ino, ctime, ctime_nsec] = scope_parts.as_slice() {
                scope = Some(SocketIdentity {
                    dev: parse_state_u64(dev)?,
                    ino: parse_state_u64(ino)?,
                    ctime: parse_state_i64(ctime)?,
                    ctime_nsec: parse_state_i64(ctime_nsec)?,
                });
                continue;
            }
            if let [
                "scrollback",
                pane_id,
                version,
                start_line,
                line_count,
                total_lines,
            ] = scope_parts.as_slice()
            {
                let start_line = parse_state_u64(start_line)?;
                let line_count = parse_state_u32(line_count)?;
                if start_line == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "cached scrollback start_line must be 1-based",
                    ));
                }
                if line_count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "cached scrollback line_count must be nonzero",
                    ));
                }
                scrollbacks.push(ClientPaneScrollback {
                    pane_id: String::from_utf8(hex_decode(pane_id)?)
                        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                    version: parse_state_u64(version)?,
                    start_line,
                    line_count,
                    total_lines: parse_state_u64(total_lines)?,
                });
                continue;
            }

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
                .map(parse_state_surface_kind)
                .transpose()?
                .unwrap_or(protocol::SurfaceKind::Main);
            let row_count = usize::try_from(rows).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "surface rows too large")
            })?;
            let mut cursor = None;
            let mut modes = TerminalModeSummary::default();
            let mut title = String::new();
            let mut working_directory = String::new();
            let mut colors = TerminalColorSummary::default();
            let mut styles = Vec::new();
            let mut hyperlinks = Vec::new();
            let mut row_text = vec![String::new(); row_count];
            let mut row_runs = vec![Vec::new(); row_count];
            let mut row_dirty_hashes = vec![0; row_count];
            let mut row_semantic_prompts = vec![protocol::RowSemanticPrompt::None; row_count];
            let mut row_dirty = vec![false; row_count];
            let mut row_kitty_placeholders = vec![false; row_count];
            let mut row_state_hashes = vec![0; row_count];

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

                let parts = line.split(' ').collect::<Vec<_>>();
                match parts.as_slice() {
                    ["cursor", "none"] => cursor = None,
                    ["cursor", row, col, visible, shape] => {
                        cursor = Some(CursorSummary {
                            row: parse_state_u32(row)?,
                            col: parse_state_u32(col)?,
                            visible: parse_state_bool(visible)?,
                            shape: parse_state_cursor_shape(shape)?,
                            blinking: true,
                        });
                    }
                    ["cursor", row, col, visible, shape, blinking] => {
                        cursor = Some(CursorSummary {
                            row: parse_state_u32(row)?,
                            col: parse_state_u32(col)?,
                            visible: parse_state_bool(visible)?,
                            shape: parse_state_cursor_shape(shape)?,
                            blinking: parse_state_bool(blinking)?,
                        });
                    }
                    [
                        "modes",
                        bracketed_paste,
                        mouse_tracking,
                        focus_reporting,
                        application_keypad,
                        application_cursor,
                        origin,
                        wraparound,
                    ] => {
                        modes = TerminalModeSummary {
                            bracketed_paste: parse_state_bool(bracketed_paste)?,
                            mouse_tracking: parse_state_bool(mouse_tracking)?,
                            focus_reporting: parse_state_bool(focus_reporting)?,
                            application_keypad: parse_state_bool(application_keypad)?,
                            application_cursor: parse_state_bool(application_cursor)?,
                            origin: parse_state_bool(origin)?,
                            wraparound: parse_state_bool(wraparound)?,
                            ..TerminalModeSummary::default()
                        };
                    }
                    [
                        "modes",
                        bracketed_paste,
                        mouse_tracking,
                        focus_reporting,
                        application_keypad,
                        application_cursor,
                        origin,
                        wraparound,
                        mouse_tracking_mode,
                        mouse_format,
                    ] => {
                        modes = TerminalModeSummary {
                            bracketed_paste: parse_state_bool(bracketed_paste)?,
                            mouse_tracking: parse_state_bool(mouse_tracking)?,
                            focus_reporting: parse_state_bool(focus_reporting)?,
                            application_keypad: parse_state_bool(application_keypad)?,
                            application_cursor: parse_state_bool(application_cursor)?,
                            origin: parse_state_bool(origin)?,
                            wraparound: parse_state_bool(wraparound)?,
                            mouse_tracking_mode: parse_state_mouse_tracking_mode(
                                mouse_tracking_mode,
                            )?,
                            mouse_format: parse_state_mouse_format(mouse_format)?,
                        };
                    }
                    ["title", value] => {
                        title = String::from_utf8(hex_decode(value)?)
                            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    }
                    ["pwd", value] => {
                        working_directory = String::from_utf8(hex_decode(value)?)
                            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    }
                    [
                        "colors",
                        default_fg_rgba,
                        default_bg_rgba,
                        cursor_rgba,
                        palette_rgba,
                    ] => {
                        let cursor_rgba = parse_state_u32(cursor_rgba)?;
                        colors = TerminalColorSummary {
                            default_fg_rgba: parse_state_u32(default_fg_rgba)?,
                            default_bg_rgba: parse_state_u32(default_bg_rgba)?,
                            cursor_rgba,
                            cursor_rgba_set: cursor_rgba != 0,
                            palette_rgba: decode_palette_rgba(palette_rgba)?,
                            palette_diff_start: None,
                            palette_diff_rgba: Vec::new(),
                        };
                    }
                    [
                        "colors",
                        default_fg_rgba,
                        default_bg_rgba,
                        cursor_rgba,
                        cursor_rgba_set,
                        palette_rgba,
                    ] => {
                        colors = TerminalColorSummary {
                            default_fg_rgba: parse_state_u32(default_fg_rgba)?,
                            default_bg_rgba: parse_state_u32(default_bg_rgba)?,
                            cursor_rgba: parse_state_u32(cursor_rgba)?,
                            cursor_rgba_set: parse_state_bool(cursor_rgba_set)?,
                            palette_rgba: decode_palette_rgba(palette_rgba)?,
                            palette_diff_start: None,
                            palette_diff_rgba: Vec::new(),
                        };
                    }
                    ["style", fg_rgba, bg_rgba, underline_rgba, flags] => {
                        styles.push(StyleSummary {
                            fg_rgba: parse_state_u32(fg_rgba)?,
                            bg_rgba: parse_state_u32(bg_rgba)?,
                            underline_rgba: parse_state_u32(underline_rgba)?,
                            flags: parse_state_u32(flags)?,
                        });
                    }
                    ["hyperlink", id, uri, osc8_id, params] => {
                        hyperlinks.push(HyperlinkSummary {
                            id: parse_state_u32(id)?,
                            uri: String::from_utf8(hex_decode(uri)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                            osc8_id: String::from_utf8(decode_state_hex_field(osc8_id)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                            params: String::from_utf8(decode_state_hex_field(params)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                        });
                    }
                    ["row", row, text] => {
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
                    ["rowmeta", row, semantic_prompt] => {
                        let row = parse_state_usize(row)?;
                        let Some(target) = row_semantic_prompts.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        *target = parse_state_row_semantic_prompt(semantic_prompt)?;
                    }
                    ["rowmeta", row, semantic_prompt, dirty] => {
                        let row = parse_state_usize(row)?;
                        let Some(semantic_target) = row_semantic_prompts.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(dirty_target) = row_dirty.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        *semantic_target = parse_state_row_semantic_prompt(semantic_prompt)?;
                        *dirty_target = parse_state_bool(dirty)?;
                    }
                    ["rowmeta", row, semantic_prompt, dirty, kitty_placeholder] => {
                        let row = parse_state_usize(row)?;
                        let Some(semantic_target) = row_semantic_prompts.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(dirty_target) = row_dirty.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(kitty_target) = row_kitty_placeholders.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        *semantic_target = parse_state_row_semantic_prompt(semantic_prompt)?;
                        *dirty_target = parse_state_bool(dirty)?;
                        *kitty_target = parse_state_bool(kitty_placeholder)?;
                    }
                    [
                        "rowmeta",
                        row,
                        semantic_prompt,
                        dirty,
                        kitty_placeholder,
                        row_state_hash,
                    ] => {
                        let row = parse_state_usize(row)?;
                        let Some(semantic_target) = row_semantic_prompts.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(dirty_target) = row_dirty.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(kitty_target) = row_kitty_placeholders.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(hash_target) = row_state_hashes.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        *semantic_target = parse_state_row_semantic_prompt(semantic_prompt)?;
                        *dirty_target = parse_state_bool(dirty)?;
                        *kitty_target = parse_state_bool(kitty_placeholder)?;
                        *hash_target = parse_state_u64(row_state_hash)?;
                    }
                    [
                        "rowmeta",
                        row,
                        semantic_prompt,
                        dirty,
                        kitty_placeholder,
                        row_state_hash,
                        dirty_hash,
                    ] => {
                        let row = parse_state_usize(row)?;
                        let Some(semantic_target) = row_semantic_prompts.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(dirty_target) = row_dirty.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(kitty_target) = row_kitty_placeholders.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(hash_target) = row_state_hashes.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(dirty_hash_target) = row_dirty_hashes.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        *semantic_target = parse_state_row_semantic_prompt(semantic_prompt)?;
                        *dirty_target = parse_state_bool(dirty)?;
                        *kitty_target = parse_state_bool(kitty_placeholder)?;
                        *hash_target = parse_state_u64(row_state_hash)?;
                        *dirty_hash_target = parse_state_u64(dirty_hash)?;
                    }
                    ["run", row, text, cell_widths, style_id, flags] => {
                        let row = parse_state_usize(row)?;
                        let Some(target) = row_runs.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state run row index outside surface",
                            ));
                        };
                        target.push(CellRunSummary {
                            text: String::from_utf8(hex_decode(text)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                            cell_widths: hex_decode(cell_widths)?,
                            style_id: parse_state_u32(style_id)?,
                            flags: parse_state_u32(flags)?,
                            hyperlink_id: 0,
                            semantic_content: protocol::CellSemanticContent::Output,
                        });
                    }
                    ["run", row, text, cell_widths, style_id, flags, hyperlink_id] => {
                        let row = parse_state_usize(row)?;
                        let Some(target) = row_runs.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state run row index outside surface",
                            ));
                        };
                        target.push(CellRunSummary {
                            text: String::from_utf8(hex_decode(text)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                            cell_widths: hex_decode(cell_widths)?,
                            style_id: parse_state_u32(style_id)?,
                            flags: parse_state_u32(flags)?,
                            hyperlink_id: parse_state_u32(hyperlink_id)?,
                            semantic_content: protocol::CellSemanticContent::Output,
                        });
                    }
                    [
                        "run",
                        row,
                        text,
                        cell_widths,
                        style_id,
                        flags,
                        hyperlink_id,
                        semantic_content,
                    ] => {
                        let row = parse_state_usize(row)?;
                        let Some(target) = row_runs.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state run row index outside surface",
                            ));
                        };
                        target.push(CellRunSummary {
                            text: String::from_utf8(hex_decode(text)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                            cell_widths: hex_decode(cell_widths)?,
                            style_id: parse_state_u32(style_id)?,
                            flags: parse_state_u32(flags)?,
                            hyperlink_id: parse_state_u32(hyperlink_id)?,
                            semantic_content: parse_state_cell_semantic_content(semantic_content)?,
                        });
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid nmux client state surface body line",
                        ));
                    }
                }
            }

            validate_hyperlink_table(&hyperlinks)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
            let row_runs = row_runs_for_text(&row_text, row_runs);
            let styles = if styles.is_empty() {
                default_style_summaries()
            } else {
                styles
            };
            validate_cached_row_style_ids(&row_runs, &styles)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
            validate_cached_row_hyperlink_ids(&row_runs, &hyperlinks)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

            surfaces.push(ClientPaneSurface {
                pane_id,
                version,
                cols,
                rows,
                surface,
                cursor,
                modes,
                title,
                working_directory,
                colors,
                styles,
                hyperlinks,
                row_runs,
                row_dirty_hashes,
                row_semantic_prompts,
                row_dirty,
                row_kitty_placeholders,
                row_state_hashes,
                row_text,
            });
        }

        Ok(Self {
            scope,
            surfaces,
            scrollbacks,
        })
    }
}

impl From<SocketIdentity> for SocketIdentitySummary {
    fn from(identity: SocketIdentity) -> Self {
        Self {
            dev: identity.dev,
            ino: identity.ino,
            ctime: identity.ctime,
            ctime_nsec: identity.ctime_nsec,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSummary {
    pub pane_id: String,
    pub actor_id: String,
    pub input_seq: u64,
    pub text: String,
    pub bytes: Vec<u8>,
    pub paste_text: Option<String>,
    pub key_name: Option<String>,
    pub key_modifiers: u32,
    pub mouse: Option<MouseSummary>,
    pub requires_focus_reporting: bool,
    pub requires_mouse_tracking: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseSummary {
    pub row: u32,
    pub col: u32,
    pub pixel_x: Option<u32>,
    pub pixel_y: Option<u32>,
    pub button: MouseButton,
    pub action: MouseAction,
    pub modifiers: u32,
}

impl InputSummary {
    fn forwarding_rejection(&self, session: &Session) -> Option<InputRejection> {
        if self.requires_focus_reporting && !session.pane_focus_reporting(&self.pane_id) {
            return Some(InputRejection::FocusReportingDisabled);
        }
        if self.requires_mouse_tracking {
            return self.mouse_forwarding_rejection(session);
        }
        None
    }

    fn mouse_forwarding_rejection(&self, session: &Session) -> Option<InputRejection> {
        let Some(mouse) = self.mouse else {
            return (!session.pane_mouse_tracking(&self.pane_id))
                .then_some(InputRejection::MouseTrackingDisabled);
        };
        let Some(mode) = session.pane_mouse_tracking_mode(&self.pane_id) else {
            return Some(InputRejection::MouseTrackingDisabled);
        };
        if mode == protocol::MouseTrackingMode::None {
            return Some(InputRejection::MouseTrackingDisabled);
        }
        let Some((cols, rows)) = session.pane_size(&self.pane_id) else {
            return None;
        };
        if mouse.row >= rows || mouse.col >= cols {
            return Some(InputRejection::MouseCoordinatesOutOfBounds);
        }
        (!mouse_input_allowed(mode, mouse)).then_some(InputRejection::MouseActionRejected(mode))
    }

    fn forwarded_bytes(
        &self,
        session: &Session,
        engines: &mut PaneTerminalEngines,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        if let Some(key_name) = self.key_name.as_deref() {
            let application_keypad = session.pane_application_keypad(&self.pane_id);
            let application_cursor = session.pane_application_cursor(&self.pane_id);
            if let Some(bytes) =
                engines
                    .engine_mut(&self.pane_id)
                    .encode_key_input(KeyTerminalInput {
                        key_name,
                        modifiers: self.key_modifiers,
                        application_keypad,
                        application_cursor,
                    })
            {
                return Ok(bytes);
            }
            if self.key_modifiers != 0 {
                return Err(
                    format!("terminal engine cannot encode modified key name: {key_name}").into(),
                );
            }
            return named_key_bytes(key_name, application_keypad, application_cursor)
                .ok_or_else(|| format!("unsupported key name: {key_name}").into());
        }
        if let Some(mouse) = self.mouse {
            let (cols, rows) = session
                .pane_size(&self.pane_id)
                .ok_or("mouse input pane is missing")?;
            return engines
                .engine_mut(&self.pane_id)
                .encode_mouse_input(MouseTerminalInput {
                    row: mouse.row,
                    col: mouse.col,
                    pixel_x: mouse.pixel_x,
                    pixel_y: mouse.pixel_y,
                    button: mouse.button,
                    action: mouse.action,
                    modifiers: mouse.modifiers,
                    mouse_format: session
                        .pane_mouse_format(&self.pane_id)
                        .ok_or("mouse input pane is missing")?,
                    cols,
                    rows,
                })
                .ok_or_else(|| "terminal engine cannot encode mouse input".into());
        }
        if let Some(paste_text) = self.paste_text.as_deref() {
            return paste_input_bytes(
                paste_text,
                BracketedPasteMode::from_enabled(session.pane_bracketed_paste(&self.pane_id)),
            );
        }
        Ok(self.bytes.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputRejection {
    FocusReportingDisabled,
    MouseTrackingDisabled,
    MouseCoordinatesOutOfBounds,
    MouseActionRejected(protocol::MouseTrackingMode),
}

impl InputRejection {
    fn message(self) -> &'static str {
        match self {
            Self::FocusReportingDisabled => "input rejected: focus reporting is disabled",
            Self::MouseTrackingDisabled => "input rejected: mouse tracking is disabled",
            Self::MouseCoordinatesOutOfBounds => {
                "input rejected: mouse coordinates are outside pane bounds"
            }
            Self::MouseActionRejected(protocol::MouseTrackingMode::X10) => {
                "input rejected: X10 mouse tracking accepts press events only"
            }
            Self::MouseActionRejected(protocol::MouseTrackingMode::Normal) => {
                "input rejected: normal mouse tracking accepts press and release events only"
            }
            Self::MouseActionRejected(protocol::MouseTrackingMode::Button) => {
                "input rejected: button mouse tracking requires a pressed button for motion"
            }
            Self::MouseActionRejected(protocol::MouseTrackingMode::Any) => {
                "input rejected: mouse action is not accepted by current tracking mode"
            }
            Self::MouseActionRejected(protocol::MouseTrackingMode::None) => {
                "input rejected: mouse tracking is disabled"
            }
            Self::MouseActionRejected(_) => {
                "input rejected: mouse action is not accepted by current tracking mode"
            }
        }
    }
}

fn mouse_input_allowed(mode: protocol::MouseTrackingMode, mouse: MouseSummary) -> bool {
    match mode {
        protocol::MouseTrackingMode::None => false,
        protocol::MouseTrackingMode::X10 => mouse.action == MouseAction::Press,
        protocol::MouseTrackingMode::Normal => {
            matches!(mouse.action, MouseAction::Press | MouseAction::Release)
        }
        protocol::MouseTrackingMode::Button => match mouse.action {
            MouseAction::Press | MouseAction::Release => true,
            MouseAction::Motion => mouse.button != MouseButton::None,
        },
        protocol::MouseTrackingMode::Any => true,
        _ => false,
    }
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
    pub styles: Vec<StyleSummary>,
    pub hyperlinks: Vec<HyperlinkSummary>,
    pub colors: TerminalColorSummary,
    pub lines: Vec<ScrollbackLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollbackLine {
    pub line: u64,
    pub text: String,
    pub runs: Vec<CellRunSummary>,
    pub dirty_hash: u64,
    pub row_state_hash: u64,
    pub semantic_prompt: protocol::RowSemanticPrompt,
    pub dirty: bool,
    pub kitty_virtual_placeholder: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSummary {
    pub session_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub cols: u32,
    pub rows: u32,
    pub resize_policy: protocol::ResizePolicy,
    pub pane_tree: Option<WorkspacePaneSummary>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePaneSummary {
    pub pane_id: String,
    pub cols: u32,
    pub rows: u32,
    pub resize_policy: protocol::ResizePolicy,
    pub split_axis: protocol::SplitAxis,
    pub children: Vec<WorkspacePaneSummary>,
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
    use std::hash::{Hash, Hasher};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use nmux_core::host::{
        HostError, HostEvent, HostSpec, PaneProcess, PlanningHost, ProcessHost, ProcessOutput,
        ProcessStatus, RecordingOutput,
    };
    use nmux_core::terminal::{
        CELL_RUN_FLAG_HYPERLINK_PRESENT, CellRun, PaneStyle, TerminalEngine, TerminalInput,
        TerminalUpdate,
    };

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

    fn test_socket_identity(dev: u64, ino: u64) -> SocketIdentity {
        test_socket_identity_with_ctime(dev, ino, 100, 200)
    }

    fn scrollback_range(start_line: u64, line_count: u32) -> ScrollbackRange {
        ScrollbackRange {
            start_line,
            line_count,
        }
    }

    fn scrollback_fetch_spec(
        start_line: u64,
        line_count: u32,
        known_scrollback_version: u64,
    ) -> ScrollbackFetchSpec {
        ScrollbackFetchSpec {
            range: scrollback_range(start_line, line_count),
            known_scrollback_version,
        }
    }

    fn test_socket_identity_with_ctime(
        dev: u64,
        ino: u64,
        ctime: i64,
        ctime_nsec: i64,
    ) -> SocketIdentity {
        SocketIdentity {
            dev,
            ino,
            ctime,
            ctime_nsec,
        }
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
            modes: TerminalModeSummary::default(),
            title: String::new(),
            working_directory: String::new(),
            colors: None,
            styles: if kind == SurfaceUpdateKind::Snapshot {
                default_style_summaries()
            } else {
                Vec::new()
            },
            hyperlinks: Vec::new(),
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
                semantic_content: protocol::CellSemanticContent::Output,
            }],
            dirty_hash: u64::from(row),
            row_state_hash: u64::from(row),
            semantic_prompt: protocol::RowSemanticPrompt::None,
            dirty: false,
            kitty_virtual_placeholder: false,
        }
    }

    fn flatbuffer_terminal_colors<'a>(
        builder: &mut FlatBufferBuilder<'a>,
    ) -> flatbuffers::WIPOffset<protocol::TerminalColorState<'a>> {
        let palette = builder.create_vector::<u32>(&[]);
        let palette_diff = builder.create_vector::<u32>(&[]);
        protocol::TerminalColorState::create(
            builder,
            &protocol::TerminalColorStateArgs {
                default_fg_rgba: 0,
                default_bg_rgba: 0,
                cursor_rgba: 0,
                cursor_rgba_set: false,
                palette_rgba: Some(palette),
                palette_diff_start: 0,
                palette_diff_rgba: Some(palette_diff),
            },
        )
    }

    fn flatbuffer_terminal_colors_with_palette_diff<'a>(
        builder: &mut FlatBufferBuilder<'a>,
    ) -> flatbuffers::WIPOffset<protocol::TerminalColorState<'a>> {
        let palette = builder.create_vector::<u32>(&[]);
        let palette_diff = builder.create_vector(&[0x1122_33ff_u32]);
        protocol::TerminalColorState::create(
            builder,
            &protocol::TerminalColorStateArgs {
                default_fg_rgba: 0,
                default_bg_rgba: 0,
                cursor_rgba: 0,
                cursor_rgba_set: false,
                palette_rgba: Some(palette),
                palette_diff_start: 0,
                palette_diff_rgba: Some(palette_diff),
            },
        )
    }

    fn flatbuffer_hyperlink<'a>(
        builder: &mut FlatBufferBuilder<'a>,
    ) -> flatbuffers::WIPOffset<protocol::Hyperlink<'a>> {
        let uri = builder.create_string("https://example.test/link");
        let osc8_id = builder.create_string("link-id");
        let params = builder.create_string("id=link-id");
        protocol::Hyperlink::create(
            builder,
            &protocol::HyperlinkArgs {
                id: 7,
                uri: Some(uri),
                osc8_id: Some(osc8_id),
                params: Some(params),
            },
        )
    }

    #[derive(Debug, Clone, Copy)]
    struct RunMetadataFixture {
        style_id: u32,
        flags: u32,
        hyperlink_id: u32,
        semantic_content: protocol::CellSemanticContent,
    }

    impl Default for RunMetadataFixture {
        fn default() -> Self {
            Self {
                style_id: 0,
                flags: 0,
                hyperlink_id: 0,
                semantic_content: protocol::CellSemanticContent::Output,
            }
        }
    }

    impl RunMetadataFixture {
        fn hyperlink() -> Self {
            Self {
                flags: CELL_RUN_FLAG_HYPERLINK_PRESENT,
                hyperlink_id: 7,
                ..Self::default()
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct RowMetadataFixture {
        semantic_prompt: protocol::RowSemanticPrompt,
        dirty: bool,
        kitty_virtual_placeholder: bool,
    }

    impl Default for RowMetadataFixture {
        fn default() -> Self {
            Self {
                semantic_prompt: protocol::RowSemanticPrompt::None,
                dirty: false,
                kitty_virtual_placeholder: false,
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct ScrollbackPublicLinesFixture {
        start_line: u64,
        row_line: u64,
        total_lines: u64,
    }

    impl Default for ScrollbackPublicLinesFixture {
        fn default() -> Self {
            Self {
                start_line: 1,
                row_line: 1,
                total_lines: 1,
            }
        }
    }

    fn flatbuffer_run_with_metadata<'a>(
        builder: &mut FlatBufferBuilder<'a>,
        metadata: RunMetadataFixture,
    ) -> flatbuffers::WIPOffset<protocol::CellRun<'a>> {
        let text = builder.create_string("linked");
        let widths = builder.create_vector(&[1u8, 1, 1, 1, 1, 1]);
        protocol::CellRun::create(
            builder,
            &protocol::CellRunArgs {
                text_utf8: Some(text),
                cell_widths: Some(widths),
                style_id: metadata.style_id,
                flags: metadata.flags,
                hyperlink_id: metadata.hyperlink_id,
                semantic_content: metadata.semantic_content,
            },
        )
    }

    fn flatbuffer_cursor<'a>(
        builder: &mut FlatBufferBuilder<'a>,
        shape: protocol::CursorShape,
    ) -> flatbuffers::WIPOffset<protocol::CursorState<'a>> {
        protocol::CursorState::create(
            builder,
            &protocol::CursorStateArgs {
                row: 0,
                col: 0,
                visible: true,
                shape,
                blinking: true,
            },
        )
    }

    fn flatbuffer_modes<'a>(
        builder: &mut FlatBufferBuilder<'a>,
        mouse_tracking_mode: protocol::MouseTrackingMode,
        mouse_format: protocol::MouseFormat,
    ) -> flatbuffers::WIPOffset<protocol::TerminalModeState<'a>> {
        protocol::TerminalModeState::create(
            builder,
            &protocol::TerminalModeStateArgs {
                bracketed_paste: false,
                mouse_tracking: mouse_tracking_mode != protocol::MouseTrackingMode::None,
                focus_reporting: false,
                application_keypad: false,
                application_cursor: false,
                origin: false,
                wraparound: true,
                mouse_tracking_mode,
                mouse_format,
            },
        )
    }

    fn envelope_frame<'a>(
        builder: &mut FlatBufferBuilder<'a>,
        body_type: protocol::EnvelopeBody,
        body: flatbuffers::WIPOffset<flatbuffers::UnionWIPOffset>,
    ) -> Vec<u8> {
        let session_id = builder.create_string("local");
        let connection_id = builder.create_string("local-client");
        let envelope = protocol::Envelope::create(
            builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(session_id),
                connection_id: Some(connection_id),
                seq: 0,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type,
                body: Some(body),
            },
        );
        protocol::finish_size_prefixed_envelope_buffer(builder, envelope);
        builder.finished_data().to_vec()
    }

    fn presence_update_frame_with_mode(mode: protocol::AttachMode) -> Vec<u8> {
        presence_update_frame_with_mode_and_kind(mode, protocol::PresenceKind::Joined)
    }

    fn presence_update_frame_with_mode_and_kind(
        mode: protocol::AttachMode,
        kind: protocol::PresenceKind,
    ) -> Vec<u8> {
        presence_update_frame_with_fields(
            Some("local-actor"),
            Some("local-user"),
            Some("local"),
            mode,
            kind,
            Some("pane-1"),
        )
    }

    fn presence_update_frame_with_fields(
        actor_id: Option<&str>,
        user_id: Option<&str>,
        display_name: Option<&str>,
        mode: protocol::AttachMode,
        kind: protocol::PresenceKind,
        focused_pane_id: Option<&str>,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let actor_id = actor_id.map(|actor_id| builder.create_string(actor_id));
        let user_id = user_id.map(|user_id| builder.create_string(user_id));
        let display_name = display_name.map(|display_name| builder.create_string(display_name));
        let focused_pane_id =
            focused_pane_id.map(|focused_pane_id| builder.create_string(focused_pane_id));
        let presence = protocol::PresenceUpdate::create(
            &mut builder,
            &protocol::PresenceUpdateArgs {
                actor_id,
                user_id,
                display_name,
                mode,
                kind,
                focused_pane_id,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::PresenceUpdate,
            presence.as_union_value(),
        )
    }

    fn attach_request_frame_with_mode(mode: protocol::AttachMode) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let actor_id = builder.create_string("local-actor");
        let user_id = builder.create_string("local-user");
        let display_name = builder.create_string("local");
        let focused_pane_id = builder.create_string("pane-1");
        let request = protocol::AttachRequest::create(
            &mut builder,
            &protocol::AttachRequestArgs {
                actor_id: Some(actor_id),
                user_id: Some(user_id),
                display_name: Some(display_name),
                mode,
                focused_pane_id: Some(focused_pane_id),
                known_surfaces: None,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::AttachRequest,
            request.as_union_value(),
        )
    }

    fn attach_request_frame_with_fields(
        actor_id: Option<&str>,
        user_id: Option<&str>,
        display_name: Option<&str>,
        focused_pane_id: Option<&str>,
        known_surface_pane_id: Option<Option<&str>>,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let actor_id = actor_id.map(|actor_id| builder.create_string(actor_id));
        let user_id = user_id.map(|user_id| builder.create_string(user_id));
        let display_name = display_name.map(|display_name| builder.create_string(display_name));
        let focused_pane_id =
            focused_pane_id.map(|focused_pane_id| builder.create_string(focused_pane_id));
        let known_surfaces = known_surface_pane_id.map(|pane_id| {
            let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
            let known_surface = protocol::KnownPaneSurfaceVersion::create(
                &mut builder,
                &protocol::KnownPaneSurfaceVersionArgs {
                    pane_id,
                    version: 1,
                },
            );
            builder.create_vector(&[known_surface])
        });
        let request = protocol::AttachRequest::create(
            &mut builder,
            &protocol::AttachRequestArgs {
                actor_id,
                user_id,
                display_name,
                mode: protocol::AttachMode::ReadWrite,
                focused_pane_id,
                known_surfaces,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::AttachRequest,
            request.as_union_value(),
        )
    }

    fn input_frame_without_payload(kind: protocol::InputKind) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let actor_id = builder.create_string("actor-1");
        let pane_id = builder.create_string("pane-1");
        let input = protocol::InputEvent::create(
            &mut builder,
            &protocol::InputEventArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                input_seq: 2,
                kind,
                key: None,
                mouse: None,
                paste: None,
                raw: None,
                focus: None,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::InputEvent,
            input.as_union_value(),
        )
    }

    fn input_frame_with_ids(pane_id: Option<&str>, actor_id: Option<&str>) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
        let actor_id = actor_id.map(|actor_id| builder.create_string(actor_id));
        let text = builder.create_string("x");
        let key = protocol::KeyInput::create(
            &mut builder,
            &protocol::KeyInputArgs {
                text_utf8: Some(text),
                key_name: None,
                modifiers: 0,
            },
        );
        let input = protocol::InputEvent::create(
            &mut builder,
            &protocol::InputEventArgs {
                pane_id,
                actor_id,
                input_seq: 2,
                kind: protocol::InputKind::Key,
                key: Some(key),
                mouse: None,
                paste: None,
                raw: None,
                focus: None,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::InputEvent,
            input.as_union_value(),
        )
    }

    fn resize_intent_frame_with_ids(pane_id: Option<&str>, actor_id: Option<&str>) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
        let actor_id = actor_id.map(|actor_id| builder.create_string(actor_id));
        let resize = protocol::ResizeIntent::create(
            &mut builder,
            &protocol::ResizeIntentArgs {
                pane_id,
                actor_id,
                desired_cols: 80,
                desired_rows: 24,
                reason: protocol::ResizeReason::UserCommand,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::ResizeIntent,
            resize.as_union_value(),
        )
    }

    fn scrollback_fetch_frame_with_ids(pane_id: Option<&str>, actor_id: Option<&str>) -> Vec<u8> {
        scrollback_fetch_frame_with_ids_and_range(pane_id, actor_id, 1, 2)
    }

    fn scrollback_fetch_frame_with_ids_and_range(
        pane_id: Option<&str>,
        actor_id: Option<&str>,
        start_line: u64,
        line_count: u32,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
        let actor_id = actor_id.map(|actor_id| builder.create_string(actor_id));
        let fetch = protocol::ScrollbackFetch::create(
            &mut builder,
            &protocol::ScrollbackFetchArgs {
                pane_id,
                actor_id,
                start_line,
                line_count,
                known_scrollback_version: 0,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::ScrollbackFetch,
            fetch.as_union_value(),
        )
    }

    fn workspace_tree_frame_with_resize_policy(policy: protocol::ResizePolicy) -> Vec<u8> {
        workspace_tree_frame_with_pane_enums(
            protocol::PaneKind::Pty,
            protocol::SplitAxis::None,
            policy,
            None,
        )
    }

    fn workspace_tree_frame_with_pane_enums(
        kind: protocol::PaneKind,
        split_axis: protocol::SplitAxis,
        resize_policy: protocol::ResizePolicy,
        child: Option<(
            protocol::PaneKind,
            protocol::SplitAxis,
            protocol::ResizePolicy,
        )>,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let children = child.map(|(child_kind, child_split_axis, child_resize_policy)| {
            let child_pane_id = builder.create_string("pane-2");
            let child_pane = protocol::PaneNode::create(
                &mut builder,
                &protocol::PaneNodeArgs {
                    pane_id: Some(child_pane_id),
                    kind: child_kind,
                    split_axis: child_split_axis,
                    children: None,
                    surface_version: 1,
                    cols: 80,
                    rows: 24,
                    resize_policy: child_resize_policy,
                },
            );
            builder.create_vector(&[child_pane])
        });
        let pane_id = builder.create_string("pane-1");
        let pane = protocol::PaneNode::create(
            &mut builder,
            &protocol::PaneNodeArgs {
                pane_id: Some(pane_id),
                kind,
                split_axis,
                children,
                surface_version: 1,
                cols: 80,
                rows: 24,
                resize_policy,
            },
        );
        let tab_id = builder.create_string("tab-1");
        let title = builder.create_string("main");
        let active_pane_id = builder.create_string("pane-1");
        let tab = protocol::TabNode::create(
            &mut builder,
            &protocol::TabNodeArgs {
                tab_id: Some(tab_id),
                title: Some(title),
                root: Some(pane),
                active_pane_id: Some(active_pane_id),
            },
        );
        let tabs = builder.create_vector(&[tab]);
        let session_id = builder.create_string("local");
        let active_tab_id = builder.create_string("tab-1");
        let snapshot = protocol::WorkspaceTreeSnapshot::create(
            &mut builder,
            &protocol::WorkspaceTreeSnapshotArgs {
                version: 1,
                session_id: Some(session_id),
                tabs: Some(tabs),
                active_tab_id: Some(active_tab_id),
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::WorkspaceTreeSnapshot,
            snapshot.as_union_value(),
        )
    }

    fn workspace_tree_frame_with_ids(
        session_id: Option<&str>,
        active_tab_id: Option<&str>,
        tab_id: Option<&str>,
        active_pane_id: Option<&str>,
        pane_id: Option<&str>,
        child_pane_id: Option<Option<&str>>,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let children = child_pane_id.map(|child_pane_id| {
            let child_pane_id = child_pane_id.map(|pane_id| builder.create_string(pane_id));
            let child_pane = protocol::PaneNode::create(
                &mut builder,
                &protocol::PaneNodeArgs {
                    pane_id: child_pane_id,
                    kind: protocol::PaneKind::Pty,
                    split_axis: protocol::SplitAxis::None,
                    children: None,
                    surface_version: 1,
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                },
            );
            builder.create_vector(&[child_pane])
        });
        let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
        let pane = protocol::PaneNode::create(
            &mut builder,
            &protocol::PaneNodeArgs {
                pane_id,
                kind: protocol::PaneKind::Pty,
                split_axis: protocol::SplitAxis::Horizontal,
                children,
                surface_version: 1,
                cols: 80,
                rows: 24,
                resize_policy: protocol::ResizePolicy::Fixed,
            },
        );
        let tab_id = tab_id.map(|tab_id| builder.create_string(tab_id));
        let title = builder.create_string("main");
        let active_pane_id = active_pane_id.map(|pane_id| builder.create_string(pane_id));
        let tab = protocol::TabNode::create(
            &mut builder,
            &protocol::TabNodeArgs {
                tab_id,
                title: Some(title),
                root: Some(pane),
                active_pane_id,
            },
        );
        let tabs = builder.create_vector(&[tab]);
        let session_id = session_id.map(|session_id| builder.create_string(session_id));
        let active_tab_id = active_tab_id.map(|tab_id| builder.create_string(tab_id));
        let snapshot = protocol::WorkspaceTreeSnapshot::create(
            &mut builder,
            &protocol::WorkspaceTreeSnapshotArgs {
                version: 1,
                session_id,
                tabs: Some(tabs),
                active_tab_id,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::WorkspaceTreeSnapshot,
            snapshot.as_union_value(),
        )
    }

    fn resize_intent_frame_with_reason(reason: protocol::ResizeReason) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let pane_id = builder.create_string("pane-1");
        let actor_id = builder.create_string("local-actor");
        let resize = protocol::ResizeIntent::create(
            &mut builder,
            &protocol::ResizeIntentArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                desired_cols: 80,
                desired_rows: 24,
                reason,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::ResizeIntent,
            resize.as_union_value(),
        )
    }

    fn error_frame_with_code(code: protocol::ErrorCode) -> Vec<u8> {
        error_frame_with_fields(code, Some("unsupported input"), None)
    }

    fn error_frame_with_fields(
        code: protocol::ErrorCode,
        message: Option<&str>,
        pane_id: Option<&str>,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let message = message.map(|message| builder.create_string(message));
        let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
        let error = protocol::Error::create(
            &mut builder,
            &protocol::ErrorArgs {
                code,
                message,
                retryable: false,
                pane_id,
                input_seq: 0,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::Error,
            error.as_union_value(),
        )
    }

    fn attach_status_frame_with_surface_state(state: protocol::AttachSurfaceState) -> Vec<u8> {
        attach_status_frame_with_pane_id_and_surface_state(Some("pane-1"), state)
    }

    fn attach_status_frame_with_pane_id_and_surface_state(
        pane_id: Option<&str>,
        state: protocol::AttachSurfaceState,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
        let status = protocol::AttachStatus::create(
            &mut builder,
            &protocol::AttachStatusArgs {
                pane_id,
                surface_version: 2,
                surface_state: state,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::AttachStatus,
            status.as_union_value(),
        )
    }

    fn pane_surface_snapshot_with_hyperlink_frame() -> Vec<u8> {
        pane_surface_snapshot_with_run_refs_frame(RunMetadataFixture::hyperlink())
    }

    fn pane_surface_snapshot_with_run_refs_frame(run_metadata: RunMetadataFixture) -> Vec<u8> {
        pane_surface_snapshot_with_run_metadata_frame(run_metadata, RowMetadataFixture::default())
    }

    fn pane_surface_snapshot_with_run_metadata_frame(
        run_metadata: RunMetadataFixture,
        row_metadata: RowMetadataFixture,
    ) -> Vec<u8> {
        pane_surface_snapshot_with_kind_and_run_metadata_frame(
            protocol::SurfaceKind::Main,
            run_metadata,
            row_metadata,
        )
    }

    fn pane_surface_snapshot_with_kind_and_run_metadata_frame(
        surface: protocol::SurfaceKind,
        run_metadata: RunMetadataFixture,
        row_metadata: RowMetadataFixture,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let run = flatbuffer_run_with_metadata(&mut builder, run_metadata);
        let runs = builder.create_vector(&[run]);
        let row = protocol::SurfaceRow::create(
            &mut builder,
            &protocol::SurfaceRowArgs {
                row: 0,
                runs: Some(runs),
                dirty_hash: 1,
                semantic_prompt: row_metadata.semantic_prompt,
                dirty: row_metadata.dirty,
                kitty_virtual_placeholder: row_metadata.kitty_virtual_placeholder,
                row_state_hash: 1,
            },
        );
        let rows = builder.create_vector(&[row]);
        let styles = builder.create_vector::<flatbuffers::WIPOffset<protocol::Style>>(&[]);
        let hyperlink = flatbuffer_hyperlink(&mut builder);
        let hyperlinks = builder.create_vector(&[hyperlink]);
        let colors = flatbuffer_terminal_colors(&mut builder);
        let pane_id = builder.create_string("pane-1");
        let snapshot = protocol::PaneSurfaceSnapshot::create(
            &mut builder,
            &protocol::PaneSurfaceSnapshotArgs {
                pane_id: Some(pane_id),
                version: 1,
                surface,
                cols: 80,
                rows: 1,
                cursor: None,
                modes: None,
                metadata: None,
                styles: Some(styles),
                rows_data: Some(rows),
                colors: Some(colors),
                hyperlinks: Some(hyperlinks),
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::PaneSurfaceSnapshot,
            snapshot.as_union_value(),
        )
    }

    fn pane_surface_snapshot_with_cursor_and_modes_frame(
        cursor_shape: protocol::CursorShape,
        mouse_tracking_mode: protocol::MouseTrackingMode,
        mouse_format: protocol::MouseFormat,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let run = flatbuffer_run_with_metadata(&mut builder, RunMetadataFixture::default());
        let runs = builder.create_vector(&[run]);
        let row = protocol::SurfaceRow::create(
            &mut builder,
            &protocol::SurfaceRowArgs {
                row: 0,
                runs: Some(runs),
                dirty_hash: 1,
                semantic_prompt: protocol::RowSemanticPrompt::None,
                dirty: false,
                kitty_virtual_placeholder: false,
                row_state_hash: 1,
            },
        );
        let rows = builder.create_vector(&[row]);
        let styles = builder.create_vector::<flatbuffers::WIPOffset<protocol::Style>>(&[]);
        let hyperlinks = builder.create_vector::<flatbuffers::WIPOffset<protocol::Hyperlink>>(&[]);
        let colors = flatbuffer_terminal_colors(&mut builder);
        let cursor = flatbuffer_cursor(&mut builder, cursor_shape);
        let modes = flatbuffer_modes(&mut builder, mouse_tracking_mode, mouse_format);
        let pane_id = builder.create_string("pane-1");
        let snapshot = protocol::PaneSurfaceSnapshot::create(
            &mut builder,
            &protocol::PaneSurfaceSnapshotArgs {
                pane_id: Some(pane_id),
                version: 1,
                surface: protocol::SurfaceKind::Main,
                cols: 80,
                rows: 1,
                cursor: Some(cursor),
                modes: Some(modes),
                metadata: None,
                styles: Some(styles),
                rows_data: Some(rows),
                colors: Some(colors),
                hyperlinks: Some(hyperlinks),
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::PaneSurfaceSnapshot,
            snapshot.as_union_value(),
        )
    }

    fn pane_surface_snapshot_with_palette_diff_frame() -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let run = flatbuffer_run_with_metadata(&mut builder, RunMetadataFixture::default());
        let runs = builder.create_vector(&[run]);
        let row = protocol::SurfaceRow::create(
            &mut builder,
            &protocol::SurfaceRowArgs {
                row: 0,
                runs: Some(runs),
                dirty_hash: 1,
                semantic_prompt: protocol::RowSemanticPrompt::None,
                dirty: false,
                kitty_virtual_placeholder: false,
                row_state_hash: 1,
            },
        );
        let rows = builder.create_vector(&[row]);
        let styles = builder.create_vector::<flatbuffers::WIPOffset<protocol::Style>>(&[]);
        let colors = flatbuffer_terminal_colors_with_palette_diff(&mut builder);
        let pane_id = builder.create_string("pane-1");
        let snapshot = protocol::PaneSurfaceSnapshot::create(
            &mut builder,
            &protocol::PaneSurfaceSnapshotArgs {
                pane_id: Some(pane_id),
                version: 1,
                surface: protocol::SurfaceKind::Main,
                cols: 80,
                rows: 1,
                cursor: None,
                modes: None,
                metadata: None,
                styles: Some(styles),
                rows_data: Some(rows),
                colors: Some(colors),
                hyperlinks: None,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::PaneSurfaceSnapshot,
            snapshot.as_union_value(),
        )
    }

    fn pane_surface_snapshot_with_pane_id(pane_id: Option<&str>) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let run = flatbuffer_run_with_metadata(&mut builder, RunMetadataFixture::default());
        let runs = builder.create_vector(&[run]);
        let row = protocol::SurfaceRow::create(
            &mut builder,
            &protocol::SurfaceRowArgs {
                row: 0,
                runs: Some(runs),
                dirty_hash: 1,
                semantic_prompt: protocol::RowSemanticPrompt::None,
                dirty: false,
                kitty_virtual_placeholder: false,
                row_state_hash: 1,
            },
        );
        let rows = builder.create_vector(&[row]);
        let styles = builder.create_vector::<flatbuffers::WIPOffset<protocol::Style>>(&[]);
        let colors = flatbuffer_terminal_colors(&mut builder);
        let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
        let snapshot = protocol::PaneSurfaceSnapshot::create(
            &mut builder,
            &protocol::PaneSurfaceSnapshotArgs {
                pane_id,
                version: 1,
                surface: protocol::SurfaceKind::Main,
                cols: 80,
                rows: 1,
                cursor: None,
                modes: None,
                metadata: None,
                styles: Some(styles),
                rows_data: Some(rows),
                colors: Some(colors),
                hyperlinks: None,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::PaneSurfaceSnapshot,
            snapshot.as_union_value(),
        )
    }

    fn scrollback_chunk_with_hyperlink_frame() -> Vec<u8> {
        scrollback_chunk_with_run_metadata_frame(
            RowMetadataFixture::default(),
            RunMetadataFixture::hyperlink(),
        )
    }

    fn scrollback_chunk_with_pane_id(pane_id: Option<&str>) -> Vec<u8> {
        scrollback_chunk_with_pane_id_and_run_metadata(
            pane_id,
            RowMetadataFixture::default(),
            RunMetadataFixture::hyperlink(),
        )
    }

    fn scrollback_chunk_with_run_metadata_frame(
        row_metadata: RowMetadataFixture,
        run_metadata: RunMetadataFixture,
    ) -> Vec<u8> {
        scrollback_chunk_with_pane_id_and_run_metadata(Some("pane-1"), row_metadata, run_metadata)
    }

    fn scrollback_chunk_with_pane_id_and_run_metadata(
        pane_id: Option<&str>,
        row_metadata: RowMetadataFixture,
        run_metadata: RunMetadataFixture,
    ) -> Vec<u8> {
        scrollback_chunk_with_public_lines(
            pane_id,
            row_metadata,
            run_metadata,
            ScrollbackPublicLinesFixture::default(),
        )
    }

    fn scrollback_chunk_with_public_lines(
        pane_id: Option<&str>,
        row_metadata: RowMetadataFixture,
        run_metadata: RunMetadataFixture,
        public_lines: ScrollbackPublicLinesFixture,
    ) -> Vec<u8> {
        scrollback_chunk_with_public_lines_and_total(
            pane_id,
            row_metadata,
            run_metadata,
            public_lines,
        )
    }

    fn scrollback_chunk_with_public_lines_and_total(
        pane_id: Option<&str>,
        row_metadata: RowMetadataFixture,
        run_metadata: RunMetadataFixture,
        public_lines: ScrollbackPublicLinesFixture,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let run = flatbuffer_run_with_metadata(&mut builder, run_metadata);
        let runs = builder.create_vector(&[run]);
        let row = protocol::ScrollbackRow::create(
            &mut builder,
            &protocol::ScrollbackRowArgs {
                line: public_lines.row_line,
                runs: Some(runs),
                dirty_hash: 1,
                semantic_prompt: row_metadata.semantic_prompt,
                dirty: row_metadata.dirty,
                kitty_virtual_placeholder: row_metadata.kitty_virtual_placeholder,
                row_state_hash: 1,
            },
        );
        let rows = builder.create_vector(&[row]);
        let styles = builder.create_vector::<flatbuffers::WIPOffset<protocol::Style>>(&[]);
        let hyperlink = flatbuffer_hyperlink(&mut builder);
        let hyperlinks = builder.create_vector(&[hyperlink]);
        let colors = flatbuffer_terminal_colors(&mut builder);
        let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
        let chunk = protocol::ScrollbackChunk::create(
            &mut builder,
            &protocol::ScrollbackChunkArgs {
                pane_id,
                scrollback_version: 1,
                start_line: public_lines.start_line,
                total_lines: public_lines.total_lines,
                rows: Some(rows),
                styles: Some(styles),
                colors: Some(colors),
                hyperlinks: Some(hyperlinks),
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::ScrollbackChunk,
            chunk.as_union_value(),
        )
    }

    fn scrollback_chunk_with_palette_diff_frame() -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let run = flatbuffer_run_with_metadata(&mut builder, RunMetadataFixture::default());
        let runs = builder.create_vector(&[run]);
        let row = protocol::ScrollbackRow::create(
            &mut builder,
            &protocol::ScrollbackRowArgs {
                line: 1,
                runs: Some(runs),
                dirty_hash: 1,
                semantic_prompt: protocol::RowSemanticPrompt::None,
                dirty: false,
                kitty_virtual_placeholder: false,
                row_state_hash: 1,
            },
        );
        let rows = builder.create_vector(&[row]);
        let styles = builder.create_vector::<flatbuffers::WIPOffset<protocol::Style>>(&[]);
        let colors = flatbuffer_terminal_colors_with_palette_diff(&mut builder);
        let pane_id = builder.create_string("pane-1");
        let chunk = protocol::ScrollbackChunk::create(
            &mut builder,
            &protocol::ScrollbackChunkArgs {
                pane_id: Some(pane_id),
                scrollback_version: 1,
                start_line: 1,
                total_lines: 1,
                rows: Some(rows),
                styles: Some(styles),
                colors: Some(colors),
                hyperlinks: None,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::ScrollbackChunk,
            chunk.as_union_value(),
        )
    }

    fn pane_surface_patch_with_cursor_and_modes_frame(
        cursor_shape: protocol::CursorShape,
        mouse_tracking_mode: protocol::MouseTrackingMode,
        mouse_format: protocol::MouseFormat,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let rows = builder.create_vector::<flatbuffers::WIPOffset<protocol::RowUpdate>>(&[]);
        let cursor = flatbuffer_cursor(&mut builder, cursor_shape);
        let modes = flatbuffer_modes(&mut builder, mouse_tracking_mode, mouse_format);
        let pane_id = builder.create_string("pane-1");
        let patch = protocol::PaneSurfacePatch::create(
            &mut builder,
            &protocol::PaneSurfacePatchArgs {
                pane_id: Some(pane_id),
                base_version: 1,
                version: 2,
                kind: protocol::PatchKind::CursorOnly,
                row_updates: Some(rows),
                cursor: Some(cursor),
                modes: Some(modes),
                metadata: None,
                colors: None,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::PaneSurfacePatch,
            patch.as_union_value(),
        )
    }

    fn pane_surface_patch_with_row_frame(kind: protocol::PatchKind) -> Vec<u8> {
        pane_surface_patch_with_run_metadata_frame(
            kind,
            RowMetadataFixture::default(),
            RunMetadataFixture::default(),
        )
    }

    fn pane_surface_patch_with_run_metadata_frame(
        kind: protocol::PatchKind,
        row_metadata: RowMetadataFixture,
        run_metadata: RunMetadataFixture,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let run = flatbuffer_run_with_metadata(&mut builder, run_metadata);
        let runs = builder.create_vector(&[run]);
        let row = protocol::RowUpdate::create(
            &mut builder,
            &protocol::RowUpdateArgs {
                row: 0,
                runs: Some(runs),
                dirty_hash: 1,
                semantic_prompt: row_metadata.semantic_prompt,
                dirty: row_metadata.dirty,
                kitty_virtual_placeholder: row_metadata.kitty_virtual_placeholder,
                row_state_hash: 1,
            },
        );
        let rows = builder.create_vector(&[row]);
        let pane_id = builder.create_string("pane-1");
        let patch = protocol::PaneSurfacePatch::create(
            &mut builder,
            &protocol::PaneSurfacePatchArgs {
                pane_id: Some(pane_id),
                base_version: 1,
                version: 2,
                kind,
                row_updates: Some(rows),
                cursor: None,
                modes: None,
                metadata: None,
                colors: None,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::PaneSurfacePatch,
            patch.as_union_value(),
        )
    }

    fn pane_surface_patch_with_palette_diff_frame(kind: protocol::PatchKind) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let rows = builder.create_vector::<flatbuffers::WIPOffset<protocol::RowUpdate>>(&[]);
        let colors = flatbuffer_terminal_colors_with_palette_diff(&mut builder);
        let pane_id = builder.create_string("pane-1");
        let patch = protocol::PaneSurfacePatch::create(
            &mut builder,
            &protocol::PaneSurfacePatchArgs {
                pane_id: Some(pane_id),
                base_version: 1,
                version: 2,
                kind,
                row_updates: Some(rows),
                cursor: None,
                modes: None,
                metadata: None,
                colors: Some(colors),
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::PaneSurfacePatch,
            patch.as_union_value(),
        )
    }

    fn pane_surface_patch_with_pane_id(pane_id: Option<&str>) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let rows = builder.create_vector::<flatbuffers::WIPOffset<protocol::RowUpdate>>(&[]);
        let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
        let patch = protocol::PaneSurfacePatch::create(
            &mut builder,
            &protocol::PaneSurfacePatchArgs {
                pane_id,
                base_version: 1,
                version: 2,
                kind: protocol::PatchKind::CursorOnly,
                row_updates: Some(rows),
                cursor: None,
                modes: None,
                metadata: None,
                colors: None,
            },
        );
        envelope_frame(
            &mut builder,
            protocol::EnvelopeBody::PaneSurfacePatch,
            patch.as_union_value(),
        )
    }

    fn rename_initial_pane(session: &mut Session, pane_id: &str) {
        session.tabs[0].active_pane_id = pane_id.to_owned();
        session.tabs[0].root.id = pane_id.to_owned();
        session.tabs[0].root.surface_lines = vec![format!("nmux {pane_id}")];
        session.tabs[0].root.surface_row_runs =
            vec![vec![CellRun::plain(format!("nmux {pane_id}"))]];
        session.tabs[0].root.surface_semantic_prompts = vec![protocol::RowSemanticPrompt::None];
        session.tabs[0].root.surface_dirty_rows = vec![false];
        session.tabs[0].root.surface_kitty_placeholders = vec![false];
        session.tabs[0].root.scrollback_lines = vec![format!("booting {pane_id}")];
        session.tabs[0].root.scrollback_row_runs =
            vec![vec![CellRun::plain(format!("booting {pane_id}"))]];
        session.tabs[0].root.scrollback_semantic_prompts = vec![protocol::RowSemanticPrompt::None];
        session.tabs[0].root.scrollback_dirty_rows = vec![false];
        session.tabs[0].root.scrollback_kitty_placeholders = vec![false];
    }

    fn surface_row_with_runs(row: u32, runs: Vec<CellRunSummary>) -> SurfaceRowUpdate {
        SurfaceRowUpdate {
            row,
            text: render_run_summaries(&runs),
            runs,
            dirty_hash: u64::from(row),
            row_state_hash: u64::from(row) + 100,
            semantic_prompt: protocol::RowSemanticPrompt::None,
            dirty: false,
            kitty_virtual_placeholder: false,
        }
    }

    fn scrollback_line(line: u64, text: &str) -> ScrollbackLine {
        let runs = vec![CellRunSummary::plain(text)];
        ScrollbackLine {
            line,
            text: text.to_owned(),
            runs,
            dirty_hash: stable_test_row_hash(text),
            row_state_hash: test_row_state_hash(
                &[CellRunSummary::plain(text)],
                TestRowStateMetadata {
                    semantic_prompt: protocol::RowSemanticPrompt::None,
                    dirty: false,
                    kitty_virtual_placeholder: false,
                },
            ),
            semantic_prompt: protocol::RowSemanticPrompt::None,
            dirty: false,
            kitty_virtual_placeholder: false,
        }
    }

    fn stable_test_row_hash(line: &str) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in line.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestRowStateMetadata {
        semantic_prompt: protocol::RowSemanticPrompt,
        dirty: bool,
        kitty_virtual_placeholder: bool,
    }

    fn test_row_state_hash(runs: &[CellRunSummary], metadata: TestRowStateMetadata) -> u64 {
        let mut hasher = TestStableHasher::new();
        for run in runs {
            run.text.hash(&mut hasher);
            run.cell_widths.hash(&mut hasher);
            run.style_id.hash(&mut hasher);
            run.flags.hash(&mut hasher);
            run.hyperlink_id.hash(&mut hasher);
            run.semantic_content.0.hash(&mut hasher);
        }
        metadata.semantic_prompt.0.hash(&mut hasher);
        metadata.dirty.hash(&mut hasher);
        metadata.kitty_virtual_placeholder.hash(&mut hasher);
        hasher.finish()
    }

    struct TestStableHasher(u64);

    impl TestStableHasher {
        fn new() -> Self {
            Self(0xcbf2_9ce4_8422_2325_u64)
        }
    }

    impl Hasher for TestStableHasher {
        fn finish(&self) -> u64 {
            self.0
        }

        fn write(&mut self, bytes: &[u8]) {
            for byte in bytes {
                self.0 ^= u64::from(*byte);
                self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }

    fn styled_run(text: &str, style_id: u32, widths: Vec<u8>) -> CellRunSummary {
        CellRunSummary {
            text: text.to_owned(),
            cell_widths: widths,
            style_id,
            flags: 0,
            hyperlink_id: 0,
            semantic_content: protocol::CellSemanticContent::Output,
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

    fn attach_status_summary(pane_id: &str, surface_version: u64) -> AttachStatusSummary {
        attach_status_summary_with_state(
            pane_id,
            surface_version,
            protocol::AttachSurfaceState::Snapshot,
        )
    }

    fn attach_status_summary_with_state(
        pane_id: &str,
        surface_version: u64,
        surface_state: protocol::AttachSurfaceState,
    ) -> AttachStatusSummary {
        AttachStatusSummary {
            pane_id: pane_id.to_owned(),
            surface_version,
            surface_state,
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
            key_name: None,
            key_names: Vec::new(),
            key_modifiers: 0,
            paste_text: None,
            focus: None,
            mouse: None,
            scrollback_start_line: 1,
            scrollback_line_count: 2,
            scrollback_tail_count: None,
            fetch_scrollback: true,
            known_scrollback_version: 0,
            known_scrollback_versions: Vec::new(),
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
                pane_tree: Some(WorkspacePaneSummary {
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    split_axis: protocol::SplitAxis::None,
                    children: Vec::new(),
                }),
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
                styles: default_style_summaries(),
                hyperlinks: Vec::new(),
                colors: TerminalColorSummary::default(),
                lines: vec![
                    scrollback_line(1, "booting nmux workspace"),
                    scrollback_line(2, "nmux pane-1"),
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

        let mut patch = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row(2, "new bottom"), surface_row(0, "new top")],
        );
        patch.title = "patched title".to_owned();
        patch.working_directory = "file://localhost/tmp/patched".to_owned();
        surface.apply_patch(&patch).expect("apply patch");

        assert_eq!(surface.version, 2);
        assert_eq!(surface.title, "patched title");
        assert_eq!(surface.working_directory, "file://localhost/tmp/patched");
        assert_eq!(surface.render_text(), "new top\nmiddle\nnew bottom");
    }

    #[test]
    fn decodes_surface_snapshot_hyperlink_table_from_frame() {
        let frame = pane_surface_snapshot_with_hyperlink_frame();
        let update = surface_update_from_frame(&frame).expect("surface update");

        assert_eq!(
            update.hyperlinks,
            vec![HyperlinkSummary {
                id: 7,
                uri: "https://example.test/link".to_owned(),
                osc8_id: "link-id".to_owned(),
                params: "id=link-id".to_owned(),
            }]
        );
        assert_eq!(update.row_updates[0].runs[0].hyperlink_id, 7);

        let surface = ClientPaneSurface::from_snapshot(&update).expect("client surface");
        assert_eq!(surface.row_runs[0][0].hyperlink_id, 7);
        assert_eq!(surface.render_text(), "linked");
    }

    #[test]
    fn rejects_surface_snapshot_with_unknown_style_id() {
        let frame = pane_surface_snapshot_with_run_refs_frame(RunMetadataFixture {
            style_id: 1,
            ..RunMetadataFixture::default()
        });
        let err = surface_update_from_frame(&frame)
            .expect_err("surface snapshot with unknown style id should be rejected");

        assert!(err.to_string().contains("unknown style_id"));
    }

    #[test]
    fn rejects_surface_snapshot_with_unknown_hyperlink_id() {
        let frame = pane_surface_snapshot_with_run_refs_frame(RunMetadataFixture {
            flags: CELL_RUN_FLAG_HYPERLINK_PRESENT,
            hyperlink_id: 8,
            ..RunMetadataFixture::default()
        });
        let err = surface_update_from_frame(&frame)
            .expect_err("surface snapshot with unknown hyperlink id should be rejected");

        assert!(err.to_string().contains("unknown hyperlink_id"));
    }

    #[test]
    fn rejects_surface_snapshot_with_unknown_row_semantic_prompt() {
        let frame = pane_surface_snapshot_with_run_metadata_frame(
            RunMetadataFixture::default(),
            RowMetadataFixture {
                semantic_prompt: protocol::RowSemanticPrompt(99),
                ..RowMetadataFixture::default()
            },
        );
        let err = surface_update_from_frame(&frame)
            .expect_err("surface snapshot with unknown row semantic prompt should be rejected");

        assert!(err.to_string().contains("unknown row semantic prompt"));
    }

    #[test]
    fn rejects_surface_snapshot_with_unknown_cell_semantic_content() {
        let frame = pane_surface_snapshot_with_run_metadata_frame(
            RunMetadataFixture {
                semantic_content: protocol::CellSemanticContent(99),
                ..RunMetadataFixture::default()
            },
            RowMetadataFixture::default(),
        );
        let err = surface_update_from_frame(&frame)
            .expect_err("surface snapshot with unknown cell semantic content should be rejected");

        assert!(err.to_string().contains("unknown semantic content"));
    }

    #[test]
    fn rejects_surface_snapshot_with_unknown_surface_kind_from_frame() {
        let frame = pane_surface_snapshot_with_kind_and_run_metadata_frame(
            protocol::SurfaceKind(99),
            RunMetadataFixture::default(),
            RowMetadataFixture::default(),
        );
        let err = surface_update_from_frame(&frame)
            .expect_err("surface snapshot with unknown surface kind should be rejected");

        assert!(err.to_string().contains("unknown surface kind"));
    }

    #[test]
    fn rejects_surface_snapshot_with_palette_diff() {
        let frame = pane_surface_snapshot_with_palette_diff_frame();
        let err = surface_update_from_frame(&frame)
            .expect_err("surface snapshot with palette diff should be rejected");

        assert!(err.to_string().contains("cannot carry palette diff"));
    }

    #[test]
    fn rejects_non_color_surface_patch_with_palette_diff() {
        let frame = pane_surface_patch_with_palette_diff_frame(protocol::PatchKind::ModeOnly);
        let err = surface_update_from_frame(&frame)
            .expect_err("mode-only surface patch with palette diff should be rejected");

        assert!(err.to_string().contains("cannot carry palette diff"));

        let color_frame =
            pane_surface_patch_with_palette_diff_frame(protocol::PatchKind::ColorOnly);
        let color_update =
            surface_update_from_frame(&color_frame).expect("color-only palette diff is valid");
        assert_eq!(
            color_update.patch_kind,
            Some(protocol::PatchKind::ColorOnly)
        );
        assert_eq!(
            color_update
                .colors
                .as_ref()
                .and_then(|colors| colors.palette_diff_start),
            Some(0)
        );
    }

    #[test]
    fn rejects_workspace_tree_with_missing_or_empty_ids() {
        for (frame, expected) in [
            (
                workspace_tree_frame_with_ids(
                    None,
                    Some("tab-1"),
                    Some("tab-1"),
                    Some("pane-1"),
                    Some("pane-1"),
                    None,
                ),
                "missing workspace session_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some(""),
                    Some("tab-1"),
                    Some("tab-1"),
                    Some("pane-1"),
                    Some("pane-1"),
                    None,
                ),
                "empty workspace session_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some("local"),
                    None,
                    Some("tab-1"),
                    Some("pane-1"),
                    Some("pane-1"),
                    None,
                ),
                "missing workspace active_tab_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some("local"),
                    Some(""),
                    Some("tab-1"),
                    Some("pane-1"),
                    Some("pane-1"),
                    None,
                ),
                "empty workspace active_tab_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some("local"),
                    Some("tab-1"),
                    None,
                    Some("pane-1"),
                    Some("pane-1"),
                    None,
                ),
                "missing workspace tab_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some("local"),
                    Some("tab-1"),
                    Some(""),
                    Some("pane-1"),
                    Some("pane-1"),
                    None,
                ),
                "empty workspace tab_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some("local"),
                    Some("tab-1"),
                    Some("tab-1"),
                    None,
                    Some("pane-1"),
                    None,
                ),
                "missing workspace active_pane_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some("local"),
                    Some("tab-1"),
                    Some("tab-1"),
                    Some(""),
                    Some("pane-1"),
                    None,
                ),
                "empty workspace active_pane_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some("local"),
                    Some("tab-1"),
                    Some("tab-1"),
                    Some("pane-1"),
                    None,
                    None,
                ),
                "missing workspace pane_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some("local"),
                    Some("tab-1"),
                    Some("tab-1"),
                    Some("pane-1"),
                    Some(""),
                    None,
                ),
                "empty workspace pane_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some("local"),
                    Some("tab-1"),
                    Some("tab-1"),
                    Some("pane-1"),
                    Some("pane-1"),
                    Some(None),
                ),
                "missing workspace pane_id",
            ),
            (
                workspace_tree_frame_with_ids(
                    Some("local"),
                    Some("tab-1"),
                    Some("tab-1"),
                    Some("pane-1"),
                    Some("pane-1"),
                    Some(Some("")),
                ),
                "empty workspace pane_id",
            ),
        ] {
            let err =
                workspace_summary_from_frame(&frame).expect_err("workspace IDs should be required");
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn rejects_surface_updates_with_missing_or_empty_pane_id() {
        for (frame, expected) in [
            (
                pane_surface_snapshot_with_pane_id(None),
                "missing surface snapshot pane_id",
            ),
            (
                pane_surface_snapshot_with_pane_id(Some("")),
                "empty surface snapshot pane_id",
            ),
            (
                pane_surface_patch_with_pane_id(None),
                "missing surface patch pane_id",
            ),
            (
                pane_surface_patch_with_pane_id(Some("")),
                "empty surface patch pane_id",
            ),
        ] {
            let err =
                surface_update_from_frame(&frame).expect_err("surface pane ID should be required");
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn rejects_attach_status_and_scrollback_chunk_with_missing_or_empty_pane_id() {
        for (frame, expected) in [
            (
                attach_status_frame_with_pane_id_and_surface_state(
                    None,
                    protocol::AttachSurfaceState::Snapshot,
                ),
                "missing attach status pane_id",
            ),
            (
                attach_status_frame_with_pane_id_and_surface_state(
                    Some(""),
                    protocol::AttachSurfaceState::Snapshot,
                ),
                "empty attach status pane_id",
            ),
        ] {
            let err = attach_status_from_frame(&frame).expect_err("attach status pane ID required");
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }

        for (frame, expected) in [
            (
                scrollback_chunk_with_pane_id(None),
                "missing scrollback chunk pane_id",
            ),
            (
                scrollback_chunk_with_pane_id(Some("")),
                "empty scrollback chunk pane_id",
            ),
        ] {
            let err =
                scrollback_chunk_from_frame(&frame).expect_err("scrollback chunk pane ID required");
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn surface_updates_reject_unknown_cursor_and_mode_enums_from_frame() {
        let snapshot_frame = pane_surface_snapshot_with_cursor_and_modes_frame(
            protocol::CursorShape(99),
            protocol::MouseTrackingMode::None,
            protocol::MouseFormat::X10,
        );
        let err = surface_update_from_frame(&snapshot_frame)
            .expect_err("surface snapshot with unknown cursor shape should be rejected");
        assert!(err.to_string().contains("unknown cursor shape"));

        let patch_frame = pane_surface_patch_with_cursor_and_modes_frame(
            protocol::CursorShape(99),
            protocol::MouseTrackingMode::None,
            protocol::MouseFormat::X10,
        );
        let err = surface_update_from_frame(&patch_frame)
            .expect_err("surface patch with unknown cursor shape should be rejected");
        assert!(err.to_string().contains("unknown cursor shape"));

        let patch_frame = pane_surface_patch_with_cursor_and_modes_frame(
            protocol::CursorShape::Block,
            protocol::MouseTrackingMode(99),
            protocol::MouseFormat::X10,
        );
        let err = surface_update_from_frame(&patch_frame)
            .expect_err("surface patch with unknown mouse tracking mode should be rejected");
        assert!(err.to_string().contains("unknown mouse tracking mode"));

        let patch_frame = pane_surface_patch_with_cursor_and_modes_frame(
            protocol::CursorShape::Block,
            protocol::MouseTrackingMode::Any,
            protocol::MouseFormat(99),
        );
        let err = surface_update_from_frame(&patch_frame)
            .expect_err("surface patch with unknown mouse format should be rejected");
        assert!(err.to_string().contains("unknown mouse format"));
    }

    #[test]
    fn rejects_no_row_surface_patch_with_row_updates_from_frame() {
        for kind in [
            protocol::PatchKind::CursorOnly,
            protocol::PatchKind::ModeOnly,
            protocol::PatchKind::ColorOnly,
            protocol::PatchKind::FullRefreshRequired,
        ] {
            let frame = pane_surface_patch_with_row_frame(kind);
            let err = surface_update_from_frame(&frame)
                .expect_err("no-row patch with row updates should be rejected");

            assert!(err.to_string().contains("cannot carry row updates"));
        }
    }

    #[test]
    fn rejects_surface_patch_with_unknown_patch_kind_from_frame() {
        let frame = pane_surface_patch_with_row_frame(protocol::PatchKind(99));
        let err = surface_update_from_frame(&frame)
            .expect_err("surface patch with unknown patch kind should be rejected");

        assert!(err.to_string().contains("unknown surface patch kind"));
    }

    #[test]
    fn rejects_surface_patch_with_unknown_semantic_enums_from_frame() {
        let prompt_frame = pane_surface_patch_with_run_metadata_frame(
            protocol::PatchKind::ReplaceRows,
            RowMetadataFixture {
                semantic_prompt: protocol::RowSemanticPrompt(99),
                ..RowMetadataFixture::default()
            },
            RunMetadataFixture::default(),
        );
        let err = surface_update_from_frame(&prompt_frame)
            .expect_err("surface patch with unknown row semantic prompt should be rejected");
        assert!(err.to_string().contains("unknown row semantic prompt"));

        let content_frame = pane_surface_patch_with_run_metadata_frame(
            protocol::PatchKind::ReplaceRows,
            RowMetadataFixture::default(),
            RunMetadataFixture {
                semantic_content: protocol::CellSemanticContent(99),
                ..RunMetadataFixture::default()
            },
        );
        let err = surface_update_from_frame(&content_frame)
            .expect_err("surface patch with unknown cell semantic content should be rejected");
        assert!(err.to_string().contains("unknown semantic content"));
    }

    #[test]
    fn decodes_scrollback_chunk_hyperlink_table_from_frame() {
        let frame = scrollback_chunk_with_hyperlink_frame();
        let chunk = scrollback_chunk_from_frame(&frame).expect("scrollback chunk");

        assert_eq!(
            chunk.hyperlinks,
            vec![HyperlinkSummary {
                id: 7,
                uri: "https://example.test/link".to_owned(),
                osc8_id: "link-id".to_owned(),
                params: "id=link-id".to_owned(),
            }]
        );
        assert_eq!(chunk.lines[0].runs[0].hyperlink_id, 7);
        assert_eq!(chunk.lines[0].text, "linked");
    }

    #[test]
    fn rejects_scrollback_chunk_with_unknown_semantic_enums() {
        let prompt_frame = scrollback_chunk_with_run_metadata_frame(
            RowMetadataFixture {
                semantic_prompt: protocol::RowSemanticPrompt(99),
                ..RowMetadataFixture::default()
            },
            RunMetadataFixture::hyperlink(),
        );
        let prompt_err = scrollback_chunk_from_frame(&prompt_frame)
            .expect_err("scrollback row with unknown prompt should be rejected");
        assert!(
            prompt_err
                .to_string()
                .contains("unknown row semantic prompt")
        );

        let content_frame = scrollback_chunk_with_run_metadata_frame(
            RowMetadataFixture::default(),
            RunMetadataFixture {
                semantic_content: protocol::CellSemanticContent(99),
                ..RunMetadataFixture::hyperlink()
            },
        );
        let content_err = scrollback_chunk_from_frame(&content_frame)
            .expect_err("scrollback run with unknown semantic content should be rejected");
        assert!(content_err.to_string().contains("unknown semantic content"));
    }

    #[test]
    fn rejects_scrollback_chunk_with_palette_diff() {
        let frame = scrollback_chunk_with_palette_diff_frame();
        let err = scrollback_chunk_from_frame(&frame)
            .expect_err("scrollback chunk with palette diff should be rejected");

        assert!(err.to_string().contains("cannot carry palette diff"));
    }

    #[test]
    fn rejects_scrollback_chunk_with_zero_public_line_numbers() {
        let start_frame = scrollback_chunk_with_public_lines(
            Some("pane-1"),
            RowMetadataFixture::default(),
            RunMetadataFixture::hyperlink(),
            ScrollbackPublicLinesFixture {
                start_line: 0,
                ..ScrollbackPublicLinesFixture::default()
            },
        );
        let start_err = scrollback_chunk_from_frame(&start_frame)
            .expect_err("zero scrollback chunk start_line should be rejected");
        assert!(
            start_err
                .to_string()
                .contains("scrollback chunk start_line must be 1-based"),
            "{start_err}"
        );

        let row_frame = scrollback_chunk_with_public_lines(
            Some("pane-1"),
            RowMetadataFixture::default(),
            RunMetadataFixture::hyperlink(),
            ScrollbackPublicLinesFixture {
                row_line: 0,
                ..ScrollbackPublicLinesFixture::default()
            },
        );
        let row_err = scrollback_chunk_from_frame(&row_frame)
            .expect_err("zero scrollback row line should be rejected");
        assert!(
            row_err
                .to_string()
                .contains("scrollback row line must be 1-based"),
            "{row_err}"
        );
    }

    #[test]
    fn client_surface_preserves_structured_row_runs_across_updates() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row_with_runs(
                0,
                vec![
                    styled_run("wide", 1, vec![1, 1, 1, 1]),
                    styled_run("字", 2, vec![2]),
                ],
            )],
        );
        snapshot.styles.resize(
            5,
            StyleSummary {
                fg_rgba: 0,
                bg_rgba: 0,
                underline_rgba: 0,
                flags: 0,
            },
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");

        assert_eq!(surface.render_text(), "wide字");
        assert_eq!(
            surface.row_runs[0],
            vec![
                styled_run("wide", 1, vec![1, 1, 1, 1]),
                styled_run("字", 2, vec![2]),
            ]
        );

        let patch = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row_with_runs(
                0,
                vec![
                    styled_run("prompt", 3, vec![1, 1, 1, 1, 1, 1]),
                    CellRunSummary {
                        text: " output".to_owned(),
                        cell_widths: vec![1, 1, 1, 1, 1, 1, 1],
                        style_id: 4,
                        flags: CELL_RUN_FLAG_HYPERLINK_PRESENT,
                        hyperlink_id: 0,
                        semantic_content: protocol::CellSemanticContent::Input,
                    },
                ],
            )],
        );
        surface.apply_patch(&patch).expect("apply patch");

        assert_eq!(surface.render_text(), "prompt output");
        assert_eq!(surface.row_runs[0][0].style_id, 3);
        assert_eq!(surface.row_runs[0][1].style_id, 4);
        assert_eq!(
            surface.row_runs[0][1].flags,
            CELL_RUN_FLAG_HYPERLINK_PRESENT
        );
        assert_eq!(
            surface.row_runs[0][1].semantic_content,
            protocol::CellSemanticContent::Input
        );
    }

    #[test]
    fn client_surface_rejects_patch_with_unknown_hyperlink_id() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let patch = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row_with_runs(
                0,
                vec![CellRunSummary {
                    text: "linked".to_owned(),
                    cell_widths: vec![1, 1, 1, 1, 1, 1],
                    style_id: 0,
                    flags: CELL_RUN_FLAG_HYPERLINK_PRESENT,
                    hyperlink_id: 7,
                    semantic_content: protocol::CellSemanticContent::Output,
                }],
            )],
        );

        let err = surface
            .apply_patch(&patch)
            .expect_err("unknown hyperlink id should be rejected");

        assert!(err.to_string().contains("unknown hyperlink_id"));
        assert_eq!(surface.version, 1);
        assert_eq!(surface.render_text(), "top");
    }

    #[test]
    fn client_surface_rejects_patch_with_unknown_style_id() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let patch = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row_with_runs(
                0,
                vec![styled_run("styled", 1, vec![1, 1, 1, 1, 1, 1])],
            )],
        );

        let err = surface
            .apply_patch(&patch)
            .expect_err("unknown style id should be rejected");

        assert!(err.to_string().contains("unknown style_id"));
        assert_eq!(surface.version, 1);
        assert_eq!(surface.render_text(), "top");
    }

    #[test]
    fn client_surface_rejects_patch_with_out_of_bounds_row_without_mutation() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top"), surface_row(1, "bottom")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let before = surface.clone();
        let patch = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row(0, "changed"), surface_row(3, "outside")],
        );

        let err = surface
            .apply_patch(&patch)
            .expect_err("out-of-bounds row should be rejected");

        assert!(err.to_string().contains("outside 3 row surface"));
        assert_eq!(surface, before);
    }

    #[test]
    fn client_surface_rejects_patch_with_duplicate_row_without_mutation() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top"), surface_row(1, "bottom")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let before = surface.clone();
        let patch = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row(0, "first"), surface_row(0, "second")],
        );

        let err = surface
            .apply_patch(&patch)
            .expect_err("duplicate row should be rejected");

        assert!(err.to_string().contains("repeated"));
        assert_eq!(surface, before);
    }

    #[test]
    fn client_surface_rejects_no_row_patch_with_rows_without_mutation() {
        for kind in [
            protocol::PatchKind::CursorOnly,
            protocol::PatchKind::ModeOnly,
            protocol::PatchKind::ColorOnly,
            protocol::PatchKind::FullRefreshRequired,
        ] {
            let mut snapshot = surface_update(
                SurfaceUpdateKind::Snapshot,
                1,
                None,
                vec![surface_row(0, "top"), surface_row(1, "bottom")],
            );
            snapshot.colors = Some(TerminalColorSummary {
                default_fg_rgba: 0xeeeeeeff,
                default_bg_rgba: 0x111111ff,
                cursor_rgba: 0,
                cursor_rgba_set: false,
                palette_rgba: vec![0x000000ff],
                palette_diff_start: None,
                palette_diff_rgba: Vec::new(),
            });
            let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
            let before = surface.clone();
            let mut patch = surface_update(
                SurfaceUpdateKind::Patch,
                2,
                Some(1),
                vec![surface_row(0, "changed")],
            );
            patch.patch_kind = Some(kind);
            if kind == protocol::PatchKind::ColorOnly {
                patch.colors = Some(surface.colors.clone());
            }

            let err = surface
                .apply_patch(&patch)
                .expect_err("no-row patch with rows should be rejected");

            assert!(err.to_string().contains("cannot carry row updates"));
            assert_eq!(surface, before);
        }
    }

    #[test]
    fn client_surface_rejects_unknown_patch_kind_without_mutation() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top"), surface_row(1, "bottom")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let before = surface.clone();
        let mut patch = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        patch.patch_kind = Some(protocol::PatchKind(99));

        let err = surface
            .apply_patch(&patch)
            .expect_err("unknown patch kind should be rejected");

        assert!(err.to_string().contains("unknown surface patch kind"));
        assert_eq!(surface, before);
    }

    #[test]
    fn client_surface_rejects_patch_with_unknown_terminal_enums_without_mutation() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top"), surface_row(1, "bottom")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let before = surface.clone();
        let mut patch = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row(0, "changed")],
        );
        patch.row_updates[0].semantic_prompt = protocol::RowSemanticPrompt(99);

        let err = surface
            .apply_patch(&patch)
            .expect_err("unknown row semantic prompt should be rejected");

        assert!(err.to_string().contains("unknown row semantic prompt"));
        assert_eq!(surface, before);

        let patch = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row_with_runs(
                0,
                vec![CellRunSummary {
                    text: "changed".to_owned(),
                    cell_widths: vec![1, 1, 1, 1, 1, 1, 1],
                    style_id: 0,
                    flags: 0,
                    hyperlink_id: 0,
                    semantic_content: protocol::CellSemanticContent(99),
                }],
            )],
        );

        let err = surface
            .apply_patch(&patch)
            .expect_err("unknown cell semantic content should be rejected");

        assert!(err.to_string().contains("unknown semantic content"));
        assert_eq!(surface, before);
    }

    #[test]
    fn client_surface_rejects_snapshot_with_unknown_surface_kind() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top")],
        );
        snapshot.surface = Some(protocol::SurfaceKind(99));

        let err = ClientPaneSurface::from_snapshot(&snapshot)
            .expect_err("unknown snapshot surface kind should be rejected");

        assert!(err.to_string().contains("unknown surface kind"));
    }

    #[test]
    fn client_surface_rejects_unknown_cursor_and_mode_enums_without_mutation() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let before = surface.clone();

        let mut patch = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        patch.patch_kind = Some(protocol::PatchKind::CursorOnly);
        patch.cursor = Some(CursorSummary {
            row: 0,
            col: 0,
            visible: true,
            shape: protocol::CursorShape(99),
            blinking: true,
        });
        let err = surface
            .apply_patch(&patch)
            .expect_err("unknown cursor shape should be rejected");
        assert!(err.to_string().contains("unknown cursor shape"));
        assert_eq!(surface, before);

        let mut patch = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        patch.patch_kind = Some(protocol::PatchKind::ModeOnly);
        patch.modes.mouse_tracking_mode = protocol::MouseTrackingMode(99);
        let err = surface
            .apply_patch(&patch)
            .expect_err("unknown mouse tracking mode should be rejected");
        assert!(err.to_string().contains("unknown mouse tracking mode"));
        assert_eq!(surface, before);

        let mut patch = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        patch.patch_kind = Some(protocol::PatchKind::ModeOnly);
        patch.modes.mouse_format = protocol::MouseFormat(99);
        let err = surface
            .apply_patch(&patch)
            .expect_err("unknown mouse format should be rejected");
        assert!(err.to_string().contains("unknown mouse format"));
        assert_eq!(surface, before);
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
            blinking: true,
        });
        patch.title = "cursor title".to_owned();
        patch.working_directory = "file://localhost/tmp/cursor".to_owned();
        patch.colors = Some(surface.colors.clone());
        let original_colors = surface.colors.clone();
        surface.apply_patch(&patch).expect("apply patch");

        assert_eq!(surface.version, 2);
        assert_eq!(surface.render_text(), "top\nbottom");
        assert_eq!(surface.cursor, patch.cursor);
        assert_eq!(surface.title, "cursor title");
        assert_eq!(surface.working_directory, "file://localhost/tmp/cursor");
        assert_eq!(surface.colors, original_colors);
    }

    #[test]
    fn speculative_echo_predicts_printable_key_without_mutating_confirmed_surface() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "ab")],
        );
        snapshot.cursor = Some(CursorSummary {
            row: 0,
            col: 2,
            visible: true,
            shape: protocol::CursorShape::Block,
            blinking: false,
        });
        let surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut overlay = SpeculativeEchoOverlay::default();

        let predicted = overlay
            .predict_printable_key(&surface, 7, "c")
            .expect("predicted render");

        assert_eq!(predicted, "abc");
        assert_eq!(
            overlay.render_underlined(&surface).as_deref(),
            Some("ab\x1b[4mc\x1b[24m")
        );
        assert_eq!(surface.render_text(), "ab");
        assert_eq!(
            overlay.prediction,
            Some(SpeculativeEchoPrediction {
                pane_id: "pane-1".to_owned(),
                base_version: 1,
                input_seq: 7,
                row: 0,
                col: 2,
                text: "c".to_owned(),
            })
        );
    }

    #[test]
    fn speculative_echo_rejects_non_single_cell_or_non_append_inputs() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "ab")],
        );
        snapshot.cursor = Some(CursorSummary {
            row: 0,
            col: 1,
            visible: true,
            shape: protocol::CursorShape::Block,
            blinking: false,
        });
        let surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut overlay = SpeculativeEchoOverlay::default();

        assert_eq!(overlay.predict_printable_key(&surface, 1, "Z"), None);
        assert_eq!(overlay.predict_printable_key(&surface, 1, "\n"), None);
        assert_eq!(overlay.predict_printable_key(&surface, 1, "wide:中"), None);
        assert_eq!(overlay.prediction, None);
    }

    #[test]
    fn speculative_echo_reconciles_confirmed_and_mismatched_updates() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "ab")],
        );
        snapshot.cursor = Some(CursorSummary {
            row: 0,
            col: 2,
            visible: true,
            shape: protocol::CursorShape::Block,
            blinking: false,
        });
        let surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut overlay = SpeculativeEchoOverlay::default();
        assert_eq!(
            overlay.predict_printable_key(&surface, 1, "c").as_deref(),
            Some("abc")
        );

        let confirmed = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row(0, "abc")],
        );
        assert_eq!(
            overlay.reconcile_update(&confirmed),
            SpeculativeEchoReconcile::Confirmed
        );
        assert!(overlay.prediction_allowed());
        assert_eq!(overlay.prediction, None);

        assert_eq!(
            overlay.predict_printable_key(&surface, 2, "d").as_deref(),
            Some("abd")
        );
        let mismatch = surface_update(
            SurfaceUpdateKind::Patch,
            3,
            Some(1),
            vec![surface_row(0, "abX")],
        );
        assert_eq!(
            overlay.reconcile_update(&mismatch),
            SpeculativeEchoReconcile::Mismatched
        );
        assert_eq!(overlay.prediction, None);
    }

    #[test]
    fn speculative_echo_clears_on_incompatible_cursor_update() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "ab")],
        );
        snapshot.cursor = Some(CursorSummary {
            row: 0,
            col: 2,
            visible: true,
            shape: protocol::CursorShape::Block,
            blinking: false,
        });
        let surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut overlay = SpeculativeEchoOverlay::default();
        assert_eq!(
            overlay.predict_printable_key(&surface, 1, "c").as_deref(),
            Some("abc")
        );

        let mut cursor_only = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        cursor_only.patch_kind = Some(protocol::PatchKind::CursorOnly);
        cursor_only.cursor = Some(CursorSummary {
            row: 1,
            col: 0,
            visible: true,
            shape: protocol::CursorShape::Block,
            blinking: false,
        });

        assert_eq!(
            overlay.reconcile_update(&cursor_only),
            SpeculativeEchoReconcile::Mismatched
        );
        assert_eq!(overlay.prediction, None);
        assert!(overlay.render(&surface).is_none());
    }

    #[test]
    fn speculative_echo_rebases_after_same_pane_no_row_patch() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "ab")],
        );
        snapshot.cursor = Some(CursorSummary {
            row: 0,
            col: 2,
            visible: true,
            shape: protocol::CursorShape::Block,
            blinking: false,
        });
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut overlay = SpeculativeEchoOverlay::default();
        assert_eq!(
            overlay.predict_printable_key(&surface, 1, "c").as_deref(),
            Some("abc")
        );

        let mut mode_only = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        mode_only.patch_kind = Some(protocol::PatchKind::ModeOnly);
        mode_only.cursor = snapshot.cursor;

        assert_eq!(
            overlay.reconcile_update(&mode_only),
            SpeculativeEchoReconcile::Pending
        );
        surface
            .apply_update(&mode_only)
            .expect("apply mode-only patch");
        assert_eq!(
            overlay.render_underlined(&surface).as_deref(),
            Some("ab\x1b[4mc\x1b[24m")
        );
        assert_eq!(
            overlay
                .prediction
                .as_ref()
                .map(|prediction| prediction.base_version),
            Some(2)
        );
    }

    #[test]
    fn speculative_echo_backs_off_after_repeated_misses() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "ab")],
        );
        snapshot.cursor = Some(CursorSummary {
            row: 0,
            col: 2,
            visible: true,
            shape: protocol::CursorShape::Block,
            blinking: false,
        });
        let surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut overlay = SpeculativeEchoOverlay::default();

        for input_seq in 1..=2 {
            assert!(
                overlay
                    .predict_printable_key(&surface, input_seq, "c")
                    .is_some()
            );
            let mismatch = surface_update(
                SurfaceUpdateKind::Patch,
                u64::from(input_seq + 1),
                Some(1),
                vec![surface_row(0, "abX")],
            );
            assert_eq!(
                overlay.reconcile_update(&mismatch),
                SpeculativeEchoReconcile::Mismatched
            );
        }

        assert!(!overlay.prediction_allowed());
        assert_eq!(overlay.predict_printable_key(&surface, 3, "c"), None);
        assert_eq!(overlay.predict_printable_key(&surface, 4, "c"), None);
        assert_eq!(overlay.predict_printable_key(&surface, 5, "\n"), None);
        assert!(!overlay.prediction_allowed());
        assert_eq!(overlay.predict_printable_key(&surface, 5, "c"), None);
        assert!(overlay.prediction_allowed());
        assert_eq!(
            overlay.predict_printable_key(&surface, 6, "c").as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn styled_surface_rendering_uses_structured_runs_without_mutating_plain_text() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![SurfaceRowUpdate {
                runs: vec![
                    styled_run("red", 1, vec![1, 1, 1]),
                    CellRunSummary::plain(" plain "),
                    styled_run("bold", 2, vec![1, 1, 1, 1]),
                ],
                ..surface_row(0, "red plain bold")
            }],
        );
        snapshot.styles = vec![
            StyleSummary {
                fg_rgba: 0,
                bg_rgba: 0,
                underline_rgba: 0,
                flags: 0,
            },
            StyleSummary {
                fg_rgba: 0xff0000ff,
                bg_rgba: 0,
                underline_rgba: 0,
                flags: 0,
            },
            StyleSummary {
                fg_rgba: 0,
                bg_rgba: 0x001122ff,
                underline_rgba: 0,
                flags: 1 << 0,
            },
        ];
        let surface = ClientPaneSurface::from_snapshot(&snapshot).expect("styled surface");

        assert!(surface.has_styled_runs());
        assert_eq!(surface.render_text(), "red plain bold");
        assert_eq!(
            surface.render_styled_text(),
            "\x1b[38;2;255;0;0mred\x1b[0m plain \x1b[1;48;2;0;17;34mbold\x1b[0m"
        );
        assert_eq!(surface.render_text(), "red plain bold");
    }

    #[test]
    fn speculative_echo_preserves_styled_rows_when_underlining_prediction() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![SurfaceRowUpdate {
                runs: vec![
                    styled_run("red", 1, vec![1, 1, 1]),
                    CellRunSummary::plain(" plain"),
                ],
                ..surface_row(0, "red plain")
            }],
        );
        snapshot.cursor = Some(CursorSummary {
            row: 0,
            col: 9,
            visible: true,
            shape: protocol::CursorShape::Block,
            blinking: false,
        });
        snapshot.styles = vec![
            StyleSummary {
                fg_rgba: 0,
                bg_rgba: 0,
                underline_rgba: 0,
                flags: 0,
            },
            StyleSummary {
                fg_rgba: 0xff0000ff,
                bg_rgba: 0,
                underline_rgba: 0,
                flags: 1 << 1,
            },
        ];
        let surface = ClientPaneSurface::from_snapshot(&snapshot).expect("styled surface");
        let mut overlay = SpeculativeEchoOverlay::default();
        overlay
            .predict_printable_key(&surface, 7, "!")
            .expect("predict styled row");

        assert_eq!(
            overlay.render_underlined_styled(&surface).as_deref(),
            Some("\x1b[3;38;2;255;0;0mred\x1b[0m plain\x1b[4m!\x1b[24m")
        );
        assert_eq!(surface.render_text(), "red plain");
    }

    #[test]
    fn client_attach_state_can_render_cached_surface_with_or_without_styles() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![SurfaceRowUpdate {
                runs: vec![
                    CellRunSummary::plain("plain "),
                    styled_run("italic", 1, vec![1, 1, 1, 1, 1, 1]),
                ],
                ..surface_row(0, "plain italic")
            }],
        );
        snapshot.styles = vec![
            StyleSummary {
                fg_rgba: 0,
                bg_rgba: 0,
                underline_rgba: 0,
                flags: 0,
            },
            StyleSummary {
                fg_rgba: 0,
                bg_rgba: 0,
                underline_rgba: 0,
                flags: 1 << 1,
            },
        ];
        let mut state = ClientAttachState::default();
        assert_eq!(
            state
                .render_surface_update_styled(&snapshot, true)
                .expect("render styled snapshot"),
            "plain \x1b[3mitalic\x1b[0m"
        );

        assert_eq!(
            state.cached_surface_text_styled("pane-1", false).as_deref(),
            Some("plain italic")
        );
        assert_eq!(
            state.cached_surface_text_styled("pane-1", true).as_deref(),
            Some("plain \x1b[3mitalic\x1b[0m")
        );
    }

    #[test]
    fn terminal_metadata_summary_formats_visible_values() {
        let metadata = TerminalMetadataSummary {
            title: "pane\u{1b} title".to_owned(),
            working_directory: "file://localhost/tmp/nmux".to_owned(),
        };

        assert_eq!(
            metadata.display_lines(),
            vec![
                "title=pane\u{fffd} title".to_owned(),
                "working-directory=file://localhost/tmp/nmux".to_owned()
            ]
        );
    }

    #[test]
    fn client_surface_applies_color_only_patch_without_rows() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top")],
        );
        snapshot.colors = Some(TerminalColorSummary {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x111111ff,
            cursor_rgba: 0,
            cursor_rgba_set: false,
            palette_rgba: vec![0x000000ff],
            palette_diff_start: None,
            palette_diff_rgba: Vec::new(),
        });
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut patch = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        patch.patch_kind = Some(protocol::PatchKind::ColorOnly);
        patch.modes.bracketed_paste = true;
        patch.colors = Some(TerminalColorSummary {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x222222ff,
            cursor_rgba: 0,
            cursor_rgba_set: false,
            palette_rgba: Vec::new(),
            palette_diff_start: Some(1),
            palette_diff_rgba: vec![0x112233ff],
        });

        surface.apply_patch(&patch).expect("apply color-only patch");

        assert_eq!(surface.version, 2);
        assert_eq!(surface.render_text(), "top");
        assert_eq!(surface.colors.default_bg_rgba, 0x222222ff);
        assert_eq!(surface.colors.palette_rgba, vec![0x000000ff, 0x112233ff]);
        assert_eq!(surface.colors.palette_diff_start, None);
        assert_eq!(surface.colors.palette_diff_rgba, Vec::<u32>::new());
        assert!(surface.modes.bracketed_paste);
    }

    #[test]
    fn client_surface_rejects_invalid_color_only_palette_diff_without_mutation() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top")],
        );
        snapshot.cursor = Some(CursorSummary {
            row: 0,
            col: 0,
            visible: true,
            shape: protocol::CursorShape::Block,
            blinking: true,
        });
        snapshot.colors = Some(TerminalColorSummary {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x111111ff,
            cursor_rgba: 0,
            cursor_rgba_set: false,
            palette_rgba: vec![0x000000ff],
            palette_diff_start: None,
            palette_diff_rgba: Vec::new(),
        });
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let before = surface.clone();
        let mut patch = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        patch.patch_kind = Some(protocol::PatchKind::ColorOnly);
        patch.cursor = Some(CursorSummary {
            row: 0,
            col: 3,
            visible: true,
            shape: protocol::CursorShape::Beam,
            blinking: false,
        });
        patch.modes.bracketed_paste = true;
        patch.title = "bad color patch".to_owned();
        patch.colors = Some(TerminalColorSummary {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x222222ff,
            cursor_rgba: 0,
            cursor_rgba_set: false,
            palette_rgba: Vec::new(),
            palette_diff_start: Some(2),
            palette_diff_rgba: vec![0x112233ff],
        });

        let err = surface
            .apply_patch(&patch)
            .expect_err("invalid palette diff should be rejected");

        assert!(err.to_string().contains("palette diff start"));
        assert_eq!(surface, before);
    }

    #[test]
    fn client_surface_rejects_non_color_patch_with_palette_diff_without_mutation() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top")],
        );
        snapshot.colors = Some(TerminalColorSummary {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x111111ff,
            cursor_rgba: 0,
            cursor_rgba_set: false,
            palette_rgba: vec![0x000000ff],
            palette_diff_start: None,
            palette_diff_rgba: Vec::new(),
        });
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let before = surface.clone();
        let mut patch = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        patch.patch_kind = Some(protocol::PatchKind::ModeOnly);
        patch.colors = Some(TerminalColorSummary {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x111111ff,
            cursor_rgba: 0,
            cursor_rgba_set: false,
            palette_rgba: Vec::new(),
            palette_diff_start: Some(0),
            palette_diff_rgba: vec![0x112233ff],
        });

        let err = surface
            .apply_patch(&patch)
            .expect_err("non-color palette diff should be rejected");

        assert!(err.to_string().contains("cannot carry palette diff"));
        assert_eq!(surface, before);
    }

    #[test]
    fn client_surface_rejects_replace_rows_patch_that_changes_colors() {
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top")],
        );
        snapshot.colors = Some(TerminalColorSummary {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x111111ff,
            cursor_rgba: 0,
            cursor_rgba_set: false,
            palette_rgba: vec![0x000000ff],
            palette_diff_start: None,
            palette_diff_rgba: Vec::new(),
        });
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut patch = surface_update(
            SurfaceUpdateKind::Patch,
            2,
            Some(1),
            vec![surface_row(0, "new top")],
        );
        patch.colors = Some(TerminalColorSummary {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x222222ff,
            cursor_rgba: 0,
            cursor_rgba_set: false,
            palette_rgba: vec![0x000000ff],
            palette_diff_start: None,
            palette_diff_rgba: Vec::new(),
        });

        let err = surface
            .apply_patch(&patch)
            .expect_err("replace-rows color-changing patch should be rejected");
        assert!(
            err.to_string()
                .contains("replace-rows patch changes terminal colors"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn client_surface_applies_mode_only_patch_without_rows() {
        let snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            1,
            None,
            vec![surface_row(0, "top")],
        );
        let mut surface = ClientPaneSurface::from_snapshot(&snapshot).expect("client surface");
        let mut patch = surface_update(SurfaceUpdateKind::Patch, 2, Some(1), Vec::new());
        patch.patch_kind = Some(protocol::PatchKind::ModeOnly);
        patch.modes.bracketed_paste = true;
        patch.title = "mode title".to_owned();
        patch.working_directory = "file://localhost/tmp/mode".to_owned();
        patch.colors = None;
        let original_colors = surface.colors.clone();

        surface.apply_patch(&patch).expect("apply patch");

        assert_eq!(surface.version, 2);
        assert_eq!(surface.render_text(), "top");
        assert!(surface.modes.bracketed_paste);
        assert_eq!(surface.title, "mode title");
        assert_eq!(surface.working_directory, "file://localhost/tmp/mode");
        assert_eq!(surface.colors, original_colors);
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
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: attach_status_summary("pane-1", 2),
                surface: Some(snapshot_update),
                scrollback: None,
            })
            .expect("render snapshot");

        assert_eq!(
            rendered.surface_text.as_deref(),
            Some("nmux pane-1\nserver-owned terminal state")
        );
        assert!(rendered.surface_metadata.is_empty());
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
                status: attach_status_summary_with_state(
                    "pane-1",
                    3,
                    protocol::AttachSurfaceState::Patch,
                ),
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
    fn client_attach_state_uses_attach_status_pane_for_current_surface_cache() {
        let mut state = ClientAttachState::default();
        let mut cached = surface_update(
            SurfaceUpdateKind::Snapshot,
            7,
            None,
            vec![surface_row(0, "cached status pane")],
        );
        cached.pane_id = "pane-2".to_owned();
        state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-2".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: AttachStatusSummary {
                    pane_id: "pane-2".to_owned(),
                    surface_version: 7,
                    surface_state: protocol::AttachSurfaceState::Snapshot,
                },
                surface: Some(cached),
                scrollback: None,
            })
            .expect("seed cached surface");

        let rendered = state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: AttachStatusSummary {
                    pane_id: "pane-2".to_owned(),
                    surface_version: 7,
                    surface_state: protocol::AttachSurfaceState::Current,
                },
                surface: None,
                scrollback: None,
            })
            .expect("render current surface from cache");

        assert_eq!(rendered.workspace.pane_id, "pane-1");
        assert_eq!(rendered.surface_text.as_deref(), Some("cached status pane"));
    }

    #[test]
    fn client_attach_state_rejects_current_surface_without_cached_pane() {
        let mut state = ClientAttachState::default();

        let err = state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: AttachStatusSummary {
                    pane_id: "pane-1".to_owned(),
                    surface_version: 7,
                    surface_state: protocol::AttachSurfaceState::Current,
                },
                surface: None,
                scrollback: None,
            })
            .expect_err("current attach without cached surface should fail");

        assert!(
            err.to_string()
                .contains("current attach has no cached surface"),
            "{err}"
        );
    }

    #[test]
    fn client_attach_state_rejects_current_surface_version_mismatch() {
        let mut state = ClientAttachState::default();
        let cached = surface_update(
            SurfaceUpdateKind::Snapshot,
            7,
            None,
            vec![surface_row(0, "cached")],
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
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: AttachStatusSummary {
                    pane_id: "pane-1".to_owned(),
                    surface_version: 7,
                    surface_state: protocol::AttachSurfaceState::Snapshot,
                },
                surface: Some(cached),
                scrollback: None,
            })
            .expect("seed cached surface");

        let err = state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: AttachStatusSummary {
                    pane_id: "pane-1".to_owned(),
                    surface_version: 8,
                    surface_state: protocol::AttachSurfaceState::Current,
                },
                surface: None,
                scrollback: None,
            })
            .expect_err("current attach version mismatch should fail");

        assert!(
            err.to_string()
                .contains("current attach surface version mismatch"),
            "{err}"
        );
    }

    #[test]
    fn client_attach_state_rejects_missing_surface_for_non_current_status() {
        let mut state = ClientAttachState::default();

        let err = state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: attach_status_summary("pane-1", 7),
                surface: None,
                scrollback: None,
            })
            .expect_err("non-current attach status without surface should fail");

        assert!(
            err.to_string()
                .contains("requires a matching surface frame"),
            "{err}"
        );
    }

    #[test]
    fn client_attach_state_rejects_surface_for_current_status() {
        let mut state = ClientAttachState::default();
        let update = surface_update(
            SurfaceUpdateKind::Snapshot,
            7,
            None,
            vec![surface_row(0, "unexpected")],
        );

        let err = state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: attach_status_summary_with_state(
                    "pane-1",
                    7,
                    protocol::AttachSurfaceState::Current,
                ),
                surface: Some(update),
                scrollback: None,
            })
            .expect_err("current attach status with surface should fail");

        assert!(
            err.to_string()
                .contains("Current cannot include a surface frame"),
            "{err}"
        );
    }

    #[test]
    fn client_attach_state_rejects_attach_status_surface_kind_mismatch() {
        let mut state = ClientAttachState::default();
        let update = surface_update(
            SurfaceUpdateKind::Patch,
            8,
            Some(7),
            vec![surface_row(0, "patch")],
        );

        let err = state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: attach_status_summary("pane-1", 8),
                surface: Some(update),
                scrollback: None,
            })
            .expect_err("attach status surface kind mismatch should fail");

        assert!(
            err.to_string()
                .contains("does not match surface update Patch"),
            "{err}"
        );
    }

    #[test]
    fn client_attach_state_round_trips_cached_surface() {
        let mut state = ClientAttachState::default();
        let expected_scope = test_socket_identity(10, 20);
        state.apply_scope(Some(expected_scope));
        let mut snapshot = surface_update(
            SurfaceUpdateKind::Snapshot,
            7,
            None,
            vec![
                SurfaceRowUpdate {
                    row: 0,
                    text: "cached".to_owned(),
                    runs: vec![
                        CellRunSummary {
                            text: "cache".to_owned(),
                            cell_widths: vec![1, 1, 1, 1, 1],
                            style_id: 1,
                            flags: 1,
                            hyperlink_id: 7,
                            semantic_content: protocol::CellSemanticContent::Prompt,
                        },
                        CellRunSummary::plain("d"),
                    ],
                    dirty_hash: 0,
                    row_state_hash: 1,
                    semantic_prompt: protocol::RowSemanticPrompt::Prompt,
                    dirty: true,
                    kitty_virtual_placeholder: true,
                },
                surface_row(2, "tail"),
            ],
        );
        snapshot.styles.push(StyleSummary {
            fg_rgba: 0xff00_0000,
            bg_rgba: 0,
            underline_rgba: 0,
            flags: 1,
        });
        snapshot.hyperlinks.push(HyperlinkSummary {
            id: 7,
            uri: "https://example.test/cache".to_owned(),
            osc8_id: String::new(),
            params: String::new(),
        });
        snapshot.modes.bracketed_paste = true;
        snapshot.modes.focus_reporting = true;
        snapshot.modes.mouse_tracking = true;
        snapshot.modes.mouse_tracking_mode = protocol::MouseTrackingMode::Any;
        snapshot.modes.mouse_format = protocol::MouseFormat::SgrPixels;
        snapshot.title = "cached title".to_owned();
        snapshot.working_directory = "file://localhost/tmp/cached".to_owned();
        snapshot.colors = Some(TerminalColorSummary {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x111111ff,
            cursor_rgba: 0xff00ffff,
            cursor_rgba_set: true,
            palette_rgba: vec![0x000000ff, 0x112233ff],
            palette_diff_start: None,
            palette_diff_rgba: Vec::new(),
        });
        snapshot.cursor = Some(CursorSummary {
            row: 2,
            col: 4,
            visible: true,
            shape: protocol::CursorShape::Beam,
            blinking: false,
        });
        let expected_styles = snapshot.styles.clone();
        let expected_hyperlinks = snapshot.hyperlinks.clone();
        let expected_modes = snapshot.modes;
        let expected_cursor = snapshot.cursor;
        let expected_title = snapshot.title.clone();
        let expected_working_directory = snapshot.working_directory.clone();
        let expected_colors = snapshot.colors.clone().expect("snapshot colors");
        state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: attach_status_summary("pane-1", 7),
                surface: Some(snapshot),
                scrollback: Some(ScrollbackChunkSummary {
                    pane_id: "pane-1".to_owned(),
                    scrollback_version: 11,
                    start_line: 4,
                    total_lines: 9,
                    styles: default_style_summaries(),
                    hyperlinks: Vec::new(),
                    colors: TerminalColorSummary::default(),
                    lines: vec![scrollback_line(4, "cached scrollback")],
                }),
            })
            .expect("render snapshot");

        let decoded = ClientAttachState::decode(&state.encode()).expect("decode state");
        assert_eq!(decoded.scope, Some(expected_scope));
        assert_eq!(decoded.known_surfaces(), state.known_surfaces());
        assert_eq!(decoded.cached_surface_modes("pane-1"), Some(expected_modes));
        assert_eq!(decoded.cached_surface_modes("missing"), None);
        assert_eq!(
            decoded.known_surfaces_for_scope(Some(expected_scope)),
            state.known_surfaces()
        );
        assert_eq!(
            decoded.known_surfaces_for_scope(Some(test_socket_identity(10, 21))),
            Vec::new()
        );
        assert_eq!(
            decoded
                .known_surfaces_for_scope(Some(test_socket_identity_with_ctime(10, 20, 100, 201))),
            Vec::new()
        );
        assert_eq!(decoded.cached_scrollback_version("pane-1", 4, 1), Some(11));
        assert_eq!(decoded.cached_scrollback_version("pane-1", 5, 1), None);
        assert_eq!(
            decoded.cached_scrollback_version_for_scope(Some(expected_scope), "pane-1", 4, 1),
            Some(11)
        );
        assert_eq!(
            decoded.cached_scrollback_version_for_scope(
                Some(test_socket_identity(10, 21)),
                "pane-1",
                4,
                1,
            ),
            None
        );
        assert_eq!(
            decoded.cached_scrollback_version_for_scope(
                Some(test_socket_identity_with_ctime(10, 20, 100, 201)),
                "pane-1",
                4,
                1,
            ),
            None
        );
        assert_eq!(
            decoded.scrollbacks,
            vec![ClientPaneScrollback {
                pane_id: "pane-1".to_owned(),
                version: 11,
                start_line: 4,
                line_count: 1,
                total_lines: 9,
            }]
        );
        assert_eq!(decoded.surfaces[0].surface, protocol::SurfaceKind::Main);
        assert_eq!(decoded.surfaces[0].cursor, expected_cursor);
        assert_eq!(decoded.surfaces[0].modes, expected_modes);
        assert_eq!(decoded.surfaces[0].title, expected_title);
        assert_eq!(
            decoded.surfaces[0].working_directory,
            expected_working_directory
        );
        assert_eq!(decoded.surfaces[0].colors, expected_colors);
        assert_eq!(decoded.surfaces[0].render_text(), "cached\n\ntail");
        assert_eq!(
            decoded
                .cached_surface_text_styled("pane-1", true)
                .as_deref(),
            Some("\x1b[1;38;2;255;0;0mcache\x1b[0md\n\ntail")
        );
        assert_eq!(
            decoded.surfaces[0].row_semantic_prompts[0],
            protocol::RowSemanticPrompt::Prompt
        );
        assert!(decoded.surfaces[0].row_dirty[0]);
        assert!(decoded.surfaces[0].row_kitty_placeholders[0]);
        assert_eq!(decoded.surfaces[0].row_state_hashes[0], 1);
        assert_eq!(decoded.surfaces[0].styles, expected_styles);
        assert_eq!(decoded.surfaces[0].hyperlinks, expected_hyperlinks);
        assert_eq!(decoded.surfaces[0].row_runs[0].len(), 2);
        assert_eq!(decoded.surfaces[0].row_runs[0][0].text, "cache");
        assert_eq!(decoded.surfaces[0].row_runs[0][0].style_id, 1);
        assert_eq!(decoded.surfaces[0].row_runs[0][0].flags, 1);
        assert_eq!(decoded.surfaces[0].row_runs[0][0].hyperlink_id, 7);
        assert_eq!(
            decoded.surfaces[0].row_runs[0][0].semantic_content,
            protocol::CellSemanticContent::Prompt
        );
    }

    #[test]
    fn client_attach_state_save_creates_parent_and_removes_temp_file() {
        let state_dir = test_socket_path().with_extension("state-dir");
        let state_path = state_dir.join("client.state");
        let _ = fs::remove_dir_all(&state_dir);

        let mut state = ClientAttachState::default();
        state.apply_scope(Some(test_socket_identity(42, 77)));
        state.save(&state_path).expect("save state");

        let loaded = ClientAttachState::load(&state_path).expect("load saved state");
        assert_eq!(loaded.scope, Some(test_socket_identity(42, 77)));

        let temp_files = fs::read_dir(&state_dir)
            .expect("read state dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(temp_files, 0, "state save left temporary files behind");

        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn client_attach_state_decodes_legacy_socket_scope_as_nonmatching_identity() {
        let decoded = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 7\nscope socket 10 20\nsurface 70616e652d31 7 80 24 0\ncursor none\nmodes 0 0 0 0 0 0 1 0 0\nrow 0 636163686564\nend\n",
        )
        .expect("decode legacy scoped state");

        assert_eq!(
            decoded.known_surfaces_for_scope(Some(test_socket_identity(10, 20))),
            Vec::new()
        );
    }

    #[test]
    fn client_attach_state_decodes_cached_surface_without_surface_kind() {
        let decoded = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 1\nsurface 70616e652d31 7 80 24\ncursor none\nrow 0 636163686564\nend\n",
        )
        .expect("decode old state");

        assert_eq!(decoded.scope, None);
        assert_eq!(
            decoded.known_surfaces_for_scope(Some(test_socket_identity(1, 2))),
            Vec::new()
        );
        assert_eq!(decoded.surfaces[0].surface, protocol::SurfaceKind::Main);
        assert_eq!(decoded.surfaces[0].modes, TerminalModeSummary::default());
        assert_eq!(decoded.surfaces[0].title, "");
        assert_eq!(decoded.surfaces[0].colors, TerminalColorSummary::default());
        assert_eq!(
            decoded.surfaces[0].row_semantic_prompts[0],
            protocol::RowSemanticPrompt::None
        );
        assert!(!decoded.surfaces[0].row_dirty[0]);
        assert!(!decoded.surfaces[0].row_kitty_placeholders[0]);
        assert_eq!(decoded.surfaces[0].styles, default_style_summaries());
        assert!(decoded.surfaces[0].hyperlinks.is_empty());
        assert_eq!(decoded.surfaces[0].render_text(), "cached");
        assert!(decoded.scrollbacks.is_empty());
    }

    #[test]
    fn client_attach_state_rejects_zero_cached_scrollback_ranges() {
        for (state, expected) in [
            (
                "NMUX_CLIENT_STATE 7\nscrollback 70616e652d31 1 0 1 3\n",
                "cached scrollback start_line must be 1-based",
            ),
            (
                "NMUX_CLIENT_STATE 7\nscrollback 70616e652d31 1 1 0 3\n",
                "cached scrollback line_count must be nonzero",
            ),
        ] {
            let err = ClientAttachState::decode(state)
                .expect_err("zero cached scrollback range should be rejected");
            assert!(err.to_string().contains(expected), "{err}");
        }
    }

    #[test]
    fn client_attach_state_rejects_invalid_cached_surface_kind() {
        let err = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 7\nsurface 70616e652d31 7 80 24 99\ncursor none\nrow 0 636163686564\nend\n",
        )
        .expect_err("invalid cached surface kind should be rejected");

        assert!(err.to_string().contains("invalid surface kind"));
    }

    #[test]
    fn client_attach_state_rejects_invalid_cached_cursor_shape() {
        let err = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 7\nsurface 70616e652d31 7 80 24 0\ncursor 0 0 1 99 1\nrow 0 636163686564\nend\n",
        )
        .expect_err("invalid cached cursor shape should be rejected");

        assert!(err.to_string().contains("invalid cursor shape"));
    }

    #[test]
    fn client_attach_state_rejects_invalid_cached_row_semantic_prompt() {
        let err = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 7\nsurface 70616e652d31 7 80 24 0\ncursor none\nrowmeta 0 99\nrow 0 636163686564\nend\n",
        )
        .expect_err("invalid cached row semantic prompt should be rejected");

        assert!(err.to_string().contains("invalid row semantic prompt"));
    }

    #[test]
    fn client_attach_state_rejects_invalid_cached_cell_semantic_content() {
        let err = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 7\nsurface 70616e652d31 7 80 24 0\ncursor none\nrow 0 636163686564\nrun 0 636163686564 0101010101 0 0 0 99\nend\n",
        )
        .expect_err("invalid cached cell semantic content should be rejected");

        assert!(err.to_string().contains("invalid cell semantic content"));
    }

    #[test]
    fn client_attach_state_rejects_unknown_cached_style_id() {
        let err = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 7\nsurface 70616e652d31 7 80 24 0\ncursor none\nrow 0 636163686564\nrun 0 636163686564 0101010101 1 0 0 0\nend\n",
        )
        .expect_err("unknown cached style id should be rejected");

        assert!(err.to_string().contains("unknown style_id"));
    }

    #[test]
    fn client_attach_state_rejects_unknown_cached_hyperlink_id() {
        let err = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 7\nsurface 70616e652d31 7 80 24 0\ncursor none\nrow 0 6c696e6b6564\nrun 0 6c696e6b6564 010101010101 0 1 7 0\nend\n",
        )
        .expect_err("dangling hyperlink id should be rejected");

        assert!(err.to_string().contains("unknown hyperlink_id"));
    }

    #[test]
    fn client_attach_state_rejects_invalid_hyperlink_table() {
        let err = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 7\nsurface 70616e652d31 7 80 24 0\ncursor none\nhyperlink 0 68747470733a2f2f6578616d706c652e74657374 - -\nrow 0 636163686564\nend\n",
        )
        .expect_err("reserved hyperlink id should be rejected");

        assert!(err.to_string().contains("reserved id 0"));
    }

    #[test]
    fn client_attach_state_rejects_invalid_cached_mouse_mode() {
        let err = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 7\nsurface 70616e652d31 7 80 24 0\ncursor none\nmodes 0 1 0 0 0 0 1 99 2\nrow 0 636163686564\nend\n",
        )
        .expect_err("invalid cached mouse tracking mode should be rejected");

        assert!(err.to_string().contains("invalid mouse tracking mode"));
    }

    #[test]
    fn client_attach_state_rejects_invalid_cached_mouse_format() {
        let err = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 7\nsurface 70616e652d31 7 80 24 0\ncursor none\nmodes 0 1 0 0 0 0 1 4 99\nrow 0 636163686564\nend\n",
        )
        .expect_err("invalid cached mouse format should be rejected");

        assert!(err.to_string().contains("invalid mouse format"));
    }

    #[test]
    fn client_attach_state_scope_change_drops_cached_scrollback() {
        let mut state = ClientAttachState::default();
        state.apply_scope(Some(test_socket_identity(10, 20)));
        state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: attach_status_summary("pane-1", 2),
                surface: Some(surface_update(
                    SurfaceUpdateKind::Snapshot,
                    2,
                    None,
                    vec![surface_row(0, "cached surface")],
                )),
                scrollback: Some(ScrollbackChunkSummary {
                    pane_id: "pane-1".to_owned(),
                    scrollback_version: 3,
                    start_line: 1,
                    total_lines: 2,
                    styles: default_style_summaries(),
                    hyperlinks: Vec::new(),
                    colors: TerminalColorSummary::default(),
                    lines: vec![scrollback_line(1, "cached")],
                }),
            })
            .expect("render scrollback");
        assert_eq!(state.cached_scrollback_version("pane-1", 1, 1), Some(3));

        state.apply_scope(Some(test_socket_identity(10, 21)));

        assert_eq!(state.cached_scrollback_version("pane-1", 1, 1), None);
        assert!(state.scrollbacks.is_empty());
    }

    #[test]
    fn client_attach_state_preserves_distinct_cached_scrollback_ranges() {
        let mut state = ClientAttachState::default();
        state.apply_scope(Some(test_socket_identity(10, 20)));

        state.cache_scrollback_chunk(&ScrollbackChunkSummary {
            pane_id: "pane-1".to_owned(),
            scrollback_version: 3,
            start_line: 1,
            total_lines: 6,
            styles: default_style_summaries(),
            hyperlinks: Vec::new(),
            colors: TerminalColorSummary::default(),
            lines: vec![scrollback_line(1, "one"), scrollback_line(2, "two")],
        });
        state.cache_scrollback_chunk(&ScrollbackChunkSummary {
            pane_id: "pane-1".to_owned(),
            scrollback_version: 4,
            start_line: 3,
            total_lines: 6,
            styles: default_style_summaries(),
            hyperlinks: Vec::new(),
            colors: TerminalColorSummary::default(),
            lines: vec![scrollback_line(3, "three")],
        });
        state.cache_scrollback_chunk(&ScrollbackChunkSummary {
            pane_id: "pane-1".to_owned(),
            scrollback_version: 5,
            start_line: 1,
            total_lines: 6,
            styles: default_style_summaries(),
            hyperlinks: Vec::new(),
            colors: TerminalColorSummary::default(),
            lines: vec![scrollback_line(1, "one"), scrollback_line(2, "two")],
        });

        assert_eq!(state.cached_scrollback_version("pane-1", 1, 2), Some(5));
        assert_eq!(state.cached_scrollback_version("pane-1", 3, 1), Some(4));
        assert_eq!(state.scrollbacks.len(), 2);

        let decoded = ClientAttachState::decode(&state.encode()).expect("decode state");
        assert_eq!(decoded.cached_scrollback_version("pane-1", 1, 2), Some(5));
        assert_eq!(decoded.cached_scrollback_version("pane-1", 3, 1), Some(4));
        assert_eq!(decoded.scrollbacks.len(), 2);
    }

    #[test]
    fn client_attach_state_does_not_persist_empty_scrollback_chunks() {
        let mut state = ClientAttachState::default();
        state.cache_scrollback_chunk(&ScrollbackChunkSummary {
            pane_id: "pane-1".to_owned(),
            scrollback_version: 6,
            start_line: 999,
            total_lines: 3,
            styles: default_style_summaries(),
            hyperlinks: Vec::new(),
            colors: TerminalColorSummary::default(),
            lines: Vec::new(),
        });

        assert!(state.scrollbacks.is_empty());
        assert!(!state.encode().contains("scrollback "));
        let decoded = ClientAttachState::decode(&state.encode()).expect("decode state");
        assert!(decoded.scrollbacks.is_empty());
    }

    #[test]
    fn client_attach_state_decodes_cached_cursor_without_blinking() {
        let decoded = ClientAttachState::decode(
            "NMUX_CLIENT_STATE 1\nsurface 70616e652d31 7 80 24\ncursor 2 4 1 1\nrow 0 636163686564\nend\n",
        )
        .expect("decode old cursor state");

        assert_eq!(
            decoded.surfaces[0].cursor,
            Some(CursorSummary {
                row: 2,
                col: 4,
                visible: true,
                shape: protocol::CursorShape::Beam,
                blinking: true,
            })
        );
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
                styles: default_style_summaries(),
                hyperlinks: Vec::new(),
                colors: TerminalColorSummary::default(),
                lines: vec![scrollback_line(1, "real process output")],
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
    fn serve_one_with_host_uses_active_pane_for_input_and_scrollback() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        rename_initial_pane(&mut session, "pane-2");
        let mut host = PlanningHost::default();
        host.start_pane("pane-2", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host).expect("serve one");
            host
        });
        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                request: AttachRequest {
                    focused_pane_id: None,
                    ..AttachOptions::default().request
                },
                scrollback_start_line: 1,
                scrollback_line_count: 1,
                ..AttachOptions::default()
            },
        )
        .expect("attach snapshot");
        let host = server.join().expect("server thread");

        assert_eq!(snapshot.workspace.pane_id, "pane-2");
        assert_eq!(
            snapshot
                .scrollback
                .as_ref()
                .map(|chunk| chunk.pane_id.as_str()),
            Some("pane-2")
        );
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-2".to_owned(),
            bytes: b"a".to_vec(),
        }));
        assert!(!host.events().iter().any(|event| matches!(
            event,
            HostEvent::Input { pane_id, .. } if pane_id == "pane-1"
        )));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_with_client_options_uses_attach_status_pane_for_scrollback_precondition() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let workspace_session = Session::initial();
        let mut status_session = Session::initial();
        rename_initial_pane(&mut status_session, "pane-2");

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            read_attach_request(&mut stream).expect("read attach request");
            wire::write_default_frame(
                &mut stream,
                &workspace_session.workspace_tree_frame("local-client", 1),
            )
            .expect("write workspace");
            wire::write_default_frame(
                &mut stream,
                &workspace_session.presence_update_frame(
                    "local-client",
                    2,
                    &Actor {
                        id: "local-actor".to_owned(),
                        user_id: "local-user".to_owned(),
                        display_name: "local".to_owned(),
                        mode: AttachMode::ReadOnly,
                        focused_pane_id: Some("pane-2".to_owned()),
                    },
                ),
            )
            .expect("write presence");
            wire::write_default_frame(
                &mut stream,
                &status_session.attach_status_frame(
                    "local-client",
                    3,
                    "pane-2",
                    protocol::AttachSurfaceState::Current,
                ),
            )
            .expect("write attach status");

            let fetch = read_scrollback_fetch_from_stream(&mut stream).expect("scrollback fetch");
            assert_eq!(fetch.pane_id, "pane-2");
            assert_eq!(fetch.known_scrollback_version, 9);
            let chunk = status_session
                .scrollback_chunk_frame_for_pane(
                    "local-client",
                    4,
                    "pane-2",
                    fetch.start_line,
                    fetch.line_count,
                )
                .expect("pane-2 scrollback chunk");
            wire::write_default_frame(&mut stream, &chunk).expect("write scrollback chunk");
        });

        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                request: AttachRequest {
                    mode: AttachMode::ReadOnly,
                    ..AttachOptions::default().request
                },
                input_text: None,
                known_scrollback_versions: vec![KnownScrollbackVersion {
                    pane_id: "pane-2".to_owned(),
                    start_line: 1,
                    line_count: 2,
                    version: 9,
                }],
                ..AttachOptions::default()
            },
        )
        .expect("attach snapshot");
        server.join().expect("server thread");

        assert_eq!(snapshot.workspace.pane_id, "pane-1");
        assert_eq!(snapshot.status.pane_id, "pane-2");
        assert_eq!(
            snapshot
                .scrollback
                .as_ref()
                .map(|chunk| chunk.pane_id.as_str()),
            Some("pane-2")
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_from_stream_rejects_surface_pane_mismatch_with_attach_status() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let workspace_session = Session::initial();
        let mut status_session = Session::initial();
        rename_initial_pane(&mut status_session, "pane-2");

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            wire::write_default_frame(
                &mut stream,
                &workspace_session.workspace_tree_frame("local-client", 1),
            )
            .expect("write workspace");
            wire::write_default_frame(
                &mut stream,
                &workspace_session.presence_update_frame(
                    "local-client",
                    2,
                    &Actor {
                        id: "local-actor".to_owned(),
                        user_id: "local-user".to_owned(),
                        display_name: "local".to_owned(),
                        mode: AttachMode::ReadOnly,
                        focused_pane_id: Some("pane-2".to_owned()),
                    },
                ),
            )
            .expect("write presence");
            wire::write_default_frame(
                &mut stream,
                &status_session.attach_status_frame(
                    "local-client",
                    3,
                    "pane-2",
                    protocol::AttachSurfaceState::Snapshot,
                ),
            )
            .expect("write attach status");
            wire::write_default_frame(
                &mut stream,
                &workspace_session.pane_surface_frame("local-client", 4),
            )
            .expect("write mismatched surface");
        });

        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        let err = attach_from_stream(&mut stream).expect_err("pane mismatch should fail");
        server.join().expect("server thread");

        assert!(
            err.to_string()
                .contains("does not match attach status pane_id"),
            "unexpected error: {err}"
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_from_stream_rejects_surface_kind_mismatch_with_attach_status() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        session.apply_pane_output("pane-1", b"new output\n");

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            wire::write_default_frame(
                &mut stream,
                &session.workspace_tree_frame("local-client", 1),
            )
            .expect("write workspace");
            wire::write_default_frame(
                &mut stream,
                &session.presence_update_frame(
                    "local-client",
                    2,
                    &Actor {
                        id: "local-actor".to_owned(),
                        user_id: "local-user".to_owned(),
                        display_name: "local".to_owned(),
                        mode: AttachMode::ReadOnly,
                        focused_pane_id: Some("pane-1".to_owned()),
                    },
                ),
            )
            .expect("write presence");
            wire::write_default_frame(
                &mut stream,
                &session.attach_status_frame(
                    "local-client",
                    3,
                    "pane-1",
                    protocol::AttachSurfaceState::Snapshot,
                ),
            )
            .expect("write attach status");
            wire::write_default_frame(
                &mut stream,
                &session.pane_surface_patch_frame("local-client", 4, 2),
            )
            .expect("write mismatched patch");
        });

        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        let err = attach_from_stream(&mut stream).expect_err("surface kind mismatch should fail");
        server.join().expect("server thread");

        assert!(
            err.to_string()
                .contains("does not match surface update Patch"),
            "unexpected error: {err}"
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_from_stream_rejects_snapshot_frame_for_patch_status() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let session = Session::initial();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            wire::write_default_frame(
                &mut stream,
                &session.workspace_tree_frame("local-client", 1),
            )
            .expect("write workspace");
            wire::write_default_frame(
                &mut stream,
                &session.presence_update_frame(
                    "local-client",
                    2,
                    &Actor {
                        id: "local-actor".to_owned(),
                        user_id: "local-user".to_owned(),
                        display_name: "local".to_owned(),
                        mode: AttachMode::ReadOnly,
                        focused_pane_id: Some("pane-1".to_owned()),
                    },
                ),
            )
            .expect("write presence");
            wire::write_default_frame(
                &mut stream,
                &session.attach_status_frame(
                    "local-client",
                    3,
                    "pane-1",
                    protocol::AttachSurfaceState::Patch,
                ),
            )
            .expect("write attach status");
            wire::write_default_frame(&mut stream, &session.pane_surface_frame("local-client", 4))
                .expect("write mismatched snapshot");
        });

        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        let err = attach_from_stream(&mut stream).expect_err("surface kind mismatch should fail");
        server.join().expect("server thread");

        assert!(
            err.to_string()
                .contains("does not match surface update Snapshot"),
            "unexpected error: {err}"
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn serve_live_with_host_uses_active_pane_for_surface_and_scrollback() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        rename_initial_pane(&mut session, "pane-2");
        let mut host = ScriptedOutputHost::new(vec![Vec::new()]);
        host.start_pane("pane-2", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_live_n_with_host(&listener, &mut session, &mut host, 1, 1).expect("serve live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        let mut request = AttachOptions::default().request;
        request.mode = AttachMode::ReadOnly;
        request.focused_pane_id = Some("pane-2".to_owned());
        write_attach_request(&mut stream, &request).expect("write attach request");

        let snapshot = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(snapshot.workspace.pane_id, "pane-2");
        assert_eq!(
            snapshot
                .surface
                .as_ref()
                .map(|surface| surface.pane_id.as_str()),
            Some("pane-2")
        );

        send_scrollback_fetch(
            &mut stream,
            "pane-2",
            ScrollbackRange {
                start_line: 1,
                line_count: 1,
            },
        )
        .expect("send scrollback fetch");
        let chunk = read_scrollback_chunk_from_stream(&mut stream).expect("read scrollback");
        assert_eq!(chunk.pane_id, "pane-2");

        drop(stream);
        let host = server.join().expect("server thread");
        assert!(!host.events.iter().any(|event| matches!(
            event,
            HostEvent::Input { pane_id, .. } if pane_id == "pane-1"
        )));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn concurrent_live_clients_exchange_join_presence() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = EchoHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start echo pane");

        let server = thread::spawn(move || {
            serve_live_n_with_host(&listener, &mut session, &mut host, 2, usize::MAX)
                .expect("serve concurrent live");
        });

        let mut reader = UnixStream::connect(&socket_path).expect("connect reader");
        write_attach_request(
            &mut reader,
            &AttachRequest {
                actor_id: "reader".to_owned(),
                user_id: "reader-user".to_owned(),
                display_name: "Reader".to_owned(),
                mode: AttachMode::ReadOnly,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("write reader attach");
        let reader_attach = attach_from_stream(&mut reader).expect("reader attach");
        assert_eq!(reader_attach.presence.actor_id, "reader");

        let mut writer = UnixStream::connect(&socket_path).expect("connect writer");
        write_attach_request(
            &mut writer,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "writer-user".to_owned(),
                display_name: "Writer".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("write writer attach");
        let writer_attach = attach_from_stream(&mut writer).expect("writer attach");
        assert_eq!(writer_attach.presence.actor_id, "writer");

        assert_eq!(
            read_live_surface_update_from_stream(&mut reader).expect("reader presence"),
            LiveSurfaceRead::Presence(PresenceSummary {
                actor_id: "writer".to_owned(),
                user_id: "writer-user".to_owned(),
                display_name: "Writer".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
            })
        );
        assert_eq!(
            read_live_surface_update_from_stream(&mut writer).expect("writer presence"),
            LiveSurfaceRead::Presence(PresenceSummary {
                actor_id: "reader".to_owned(),
                user_id: "reader-user".to_owned(),
                display_name: "Reader".to_owned(),
                mode: AttachMode::ReadOnly,
                focused_pane_id: Some("pane-1".to_owned()),
            })
        );

        drop(reader);
        drop(writer);
        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_rejects_missing_active_pane_without_guessing_default() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        session.tabs[0].active_pane_id = "missing-pane".to_owned();

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        let mut request = AttachOptions::default().request;
        request.focused_pane_id = None;
        write_attach_request(&mut stream, &request).expect("write attach request");
        let err = attach_from_stream(&mut stream).expect_err("missing active pane should fail");
        server.join().expect("server thread");

        assert!(
            err.to_string()
                .contains("active pane not found: missing-pane"),
            "missing active-pane context: {err}"
        );
        assert!(
            err.to_string().contains("server error"),
            "missing server error prefix: {err}"
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_rejects_missing_active_tab_without_guessing_first_tab() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        session.active_tab_id = "missing-tab".to_owned();

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        let mut request = AttachOptions::default().request;
        request.focused_pane_id = None;
        write_attach_request(&mut stream, &request).expect("write attach request");
        let err = attach_from_stream(&mut stream).expect_err("missing active tab should fail");
        server.join().expect("server thread");

        assert!(
            err.to_string()
                .contains("active tab not found: missing-tab"),
            "missing active-tab context: {err}"
        );
        assert!(
            err.to_string().contains("server error"),
            "missing server error prefix: {err}"
        );

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
        send_scrollback_fetch(
            &mut stream,
            "pane-1",
            ScrollbackRange {
                start_line: 4,
                line_count: 1,
            },
        )
        .expect("send scrollback fetch");
        let scrollback = read_scrollback_chunk_from_stream(&mut stream).expect("scrollback chunk");
        server.join().expect("server thread");

        assert!(snapshot.surface.is_some());
        assert_eq!(
            scrollback,
            ScrollbackChunkSummary {
                pane_id: "pane-1".to_owned(),
                scrollback_version: 2,
                start_line: 4,
                total_lines: 4,
                styles: default_style_summaries(),
                hyperlinks: Vec::new(),
                colors: TerminalColorSummary::default(),
                lines: vec![scrollback_line(4, "z")],
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
                scrollback_start_line: 4,
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
                start_line: 4,
                total_lines: 4,
                styles: default_style_summaries(),
                hyperlinks: Vec::new(),
                colors: TerminalColorSummary::default(),
                lines: vec![scrollback_line(4, "custom")],
            })
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_with_client_options_forwards_paste_input() {
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
                input_text: None,
                paste_text: Some("pasted\ntext".to_owned()),
                scrollback_start_line: 4,
                scrollback_line_count: 2,
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
                start_line: 4,
                total_lines: 5,
                styles: default_style_summaries(),
                hyperlinks: Vec::new(),
                colors: TerminalColorSummary::default(),
                lines: vec![scrollback_line(4, "pasted"), scrollback_line(5, "text")],
            })
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn one_shot_attach_forwards_named_key_before_scrollback() {
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
        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                input_text: None,
                key_name: Some("delete".to_owned()),
                scrollback_start_line: 1,
                scrollback_line_count: 2,
                ..AttachOptions::default()
            },
        )
        .expect("attach snapshot");
        let host = server.join().expect("server thread");

        assert!(snapshot.scrollback.is_some());
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"\x1b[3~".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn one_shot_attach_forwards_focus_when_reporting_enabled() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        session.tabs[0].root.modes.focus_reporting = true;
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host).expect("serve one");
            host
        });
        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                input_text: None,
                focus: Some(true),
                scrollback_start_line: 1,
                scrollback_line_count: 2,
                ..AttachOptions::default()
            },
        )
        .expect("attach snapshot");
        let host = server.join().expect("server thread");

        assert!(snapshot.scrollback.is_some());
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"\x1b[I".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn one_shot_attach_reports_structured_input_errors_before_scrollback() {
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
        let err = attach_with_client_options(
            &socket_path,
            AttachOptions {
                input_text: None,
                key_name: Some("enter".to_owned()),
                key_modifiers: 2,
                ..AttachOptions::default()
            },
        )
        .expect_err("unsupported modified key should report server error");
        let host = server.join().expect("server thread");

        assert!(
            err.to_string()
                .contains("server error: terminal engine cannot encode modified key name: enter"),
            "unexpected error: {err}"
        );
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn one_shot_attach_reports_mouse_disabled_before_scrollback() {
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
        let err = attach_with_client_options(
            &socket_path,
            AttachOptions {
                input_text: None,
                mouse: Some(AttachMouseInput {
                    row: 0,
                    col: 0,
                    pixel_x: None,
                    pixel_y: None,
                    button: protocol::MouseButton::Left,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                }),
                ..AttachOptions::default()
            },
        )
        .expect_err("mouse input should report server error");
        let host = server.join().expect("server thread");

        assert!(
            err.to_string()
                .contains("server error: input rejected: mouse tracking is disabled"),
            "unexpected error: {err}"
        );
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn one_shot_attach_reports_mouse_out_of_bounds_before_scrollback() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        session.tabs[0].root.modes.mouse_tracking = true;
        session.tabs[0].root.modes.mouse_tracking_mode = protocol::MouseTrackingMode::Normal;
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host).expect("serve one");
            host
        });
        let err = attach_with_client_options(
            &socket_path,
            AttachOptions {
                input_text: None,
                mouse: Some(AttachMouseInput {
                    row: 24,
                    col: 0,
                    pixel_x: None,
                    pixel_y: None,
                    button: protocol::MouseButton::Left,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                }),
                ..AttachOptions::default()
            },
        )
        .expect_err("out-of-bounds mouse input should report server error");
        let host = server.join().expect("server thread");

        assert!(
            err.to_string().contains(
                "server error: input rejected: mouse coordinates are outside pane bounds"
            ),
            "unexpected error: {err}"
        );
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_with_client_options_reports_input_error_frames() {
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
        let err = attach_with_client_options(
            &socket_path,
            AttachOptions {
                input_text: None,
                paste_text: Some("bad\x1b[201~paste".to_owned()),
                ..AttachOptions::default()
            },
        )
        .expect_err("unsafe paste should report server error");
        let host = server.join().expect("server thread");

        assert!(
            err.to_string()
                .contains("server error: paste input contains a bracketed paste terminator"),
            "unexpected error: {err}"
        );
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_with_client_options_reports_host_write_failure_error_frames() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = FailingWriteHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start failing write pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host).expect("serve one");
        });
        let err = attach_with_client_options(
            &socket_path,
            AttachOptions {
                input_text: Some("fail".to_owned()),
                ..AttachOptions::default()
            },
        )
        .expect_err("host write failure should report server error");
        server.join().expect("server thread");

        assert!(
            err.to_string().contains(
                "server error: input forwarding failed: host I/O error during write_input for pane-1: simulated write failure"
            ),
            "unexpected error: {err}"
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_reports_initial_output_poll_failure_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = FailingReadHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start failing read pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host)
                .expect("serve one with read failure");
        });
        let err = attach_with_client_options(
            &socket_path,
            AttachOptions {
                input_text: None,
                ..AttachOptions::default()
            },
        )
        .expect_err("output read failure should report server error");
        server.join().expect("server thread");

        assert!(
            err.to_string().contains(
                "server error: output polling failed: host I/O error during try_read_output for pane-1: simulated read failure"
            ),
            "unexpected error: {err}"
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn one_shot_attach_reports_post_input_output_poll_failure_before_scrollback() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = FailingReadHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start failing read pane");
        host.fail_reads = false;

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host)
                .expect("serve one with post-input read failure");
        });
        let err = attach_with_client_options(
            &socket_path,
            AttachOptions {
                input_text: Some("poll-fails".to_owned()),
                ..AttachOptions::default()
            },
        )
        .expect_err("post-input output read failure should report server error");
        server.join().expect("server thread");

        assert!(
            err.to_string().contains(
                "server error: output polling failed: host I/O error during try_read_output for pane-1: simulated read failure"
            ),
            "unexpected error: {err}"
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_reports_missing_scrollback_pane_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "reader".to_owned(),
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

        send_scrollback_fetch(
            &mut stream,
            "missing-pane",
            ScrollbackRange {
                start_line: 1,
                line_count: 2,
            },
        )
        .expect("send missing scrollback fetch");
        let frame = wire::read_default_frame(&mut stream).expect("read error frame");
        let error = error_summary_from_frame(&frame).expect("decode error");
        assert_eq!(
            error,
            ErrorSummary {
                code: protocol::ErrorCode::PaneNotFound,
                message: "pane not found: missing-pane".to_owned(),
                retryable: false,
                pane_id: Some("missing-pane".to_owned()),
                input_seq: 0,
            }
        );

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_reports_stale_scrollback_version_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "reader".to_owned(),
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

        let mut sequence = ClientFrameSequence::default();
        send_scrollback_fetch_with_known_version(
            &mut stream,
            &mut sequence,
            "pane-1",
            ScrollbackFetchSpec {
                range: ScrollbackRange {
                    start_line: 1,
                    line_count: 2,
                },
                known_scrollback_version: 999,
            },
        )
        .expect("send stale scrollback fetch");
        let frame = wire::read_default_frame(&mut stream).expect("read error frame");
        let error = error_summary_from_frame(&frame).expect("decode error");
        assert_eq!(
            error,
            ErrorSummary {
                code: protocol::ErrorCode::StaleVersion,
                message: "stale scrollback version for pane-1: client=999 server=1".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 0,
            }
        );
        send_scrollback_fetch_with_known_version(
            &mut stream,
            &mut sequence,
            "pane-1",
            ScrollbackFetchSpec {
                range: ScrollbackRange {
                    start_line: 1,
                    line_count: 1,
                },
                known_scrollback_version: 0,
            },
        )
        .expect("retry scrollback fetch without precondition");
        let scrollback = read_scrollback_chunk_from_stream(&mut stream).expect("scrollback chunk");
        assert_eq!(scrollback.scrollback_version, 1);
        assert_eq!(
            scrollback.lines,
            vec![scrollback_line(1, "booting nmux workspace")]
        );

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_accepts_matching_scrollback_version() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "reader".to_owned(),
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

        let mut sequence = ClientFrameSequence::default();
        send_scrollback_fetch_with_known_version(
            &mut stream,
            &mut sequence,
            "pane-1",
            ScrollbackFetchSpec {
                range: ScrollbackRange {
                    start_line: 1,
                    line_count: 1,
                },
                known_scrollback_version: 1,
            },
        )
        .expect("send current scrollback fetch");
        let scrollback = read_scrollback_chunk_from_stream(&mut stream).expect("scrollback chunk");
        assert_eq!(scrollback.scrollback_version, 1);
        assert_eq!(
            scrollback.lines,
            vec![scrollback_line(1, "booting nmux workspace")]
        );

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn attach_reports_missing_input_pane_with_error_frame() {
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
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");
        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);

        send_key_input(&mut stream, "missing-pane", "input").expect("send input");
        let frame = wire::read_default_frame(&mut stream).expect("read error frame");
        let error = error_summary_from_frame(&frame).expect("decode error");
        assert_eq!(
            error,
            ErrorSummary {
                code: protocol::ErrorCode::PaneNotFound,
                message: "pane not found: missing-pane".to_owned(),
                retryable: false,
                pane_id: Some("missing-pane".to_owned()),
                input_seq: 1,
            }
        );

        let host = server.join().expect("server thread");
        assert!(!host.events().iter().any(|event| matches!(
            event,
            HostEvent::Input { pane_id, .. } if pane_id == "missing-pane"
        )));
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

        let mut sequence = ClientFrameSequence::default();
        send_key_input_with_sequence(&mut stream, &mut sequence, "pane-1", "first")
            .expect("send first input");
        let first_update = read_surface_update_from_stream(&mut stream).expect("first update");
        assert_eq!(first_update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(first_update.base_version, Some(2));
        assert_eq!(first_update.version, 3);
        assert!(first_update.text.ends_with("first"));

        send_key_input_with_sequence(&mut stream, &mut sequence, "pane-1", "second")
            .expect("send second input");
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
    fn live_attach_sends_snapshot_for_full_refresh_required_update() {
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

        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut engine = AlternateScreenEngine;
        assert!(session.apply_pane_output_with_engine("pane-1", b"alternate", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::FullRefreshRequired)
        );
        let current_version = session.surface_version("pane-1").expect("surface version");

        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1).expect("serve live")
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                known_surfaces: vec![KnownSurfaceVersion {
                    pane_id: "pane-1".to_owned(),
                    version: current_version - 1,
                }],
                ..AttachOptions::default().request
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        let update = initial.surface.expect("initial surface update");
        assert_eq!(update.kind, SurfaceUpdateKind::Snapshot);
        assert_eq!(update.version, current_version);
        assert_eq!(update.base_version, None);
        assert_eq!(update.patch_kind, None);
        assert_eq!(update.surface, Some(protocol::SurfaceKind::Alternate));

        let optional_update =
            read_optional_surface_update_from_stream(&mut stream).expect("optional surface update");
        assert_eq!(optional_update, None);

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
                pane_tree: Some(WorkspacePaneSummary {
                    pane_id: "pane-1".to_owned(),
                    cols: 100,
                    rows: 30,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    split_axis: protocol::SplitAxis::None,
                    children: Vec::new(),
                }),
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
                blinking: true,
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
    fn live_attach_reports_resize_failure_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = FailingResizeHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start failing resize pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1)
                .expect("serve live with resize failure");
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");
        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert!(initial.surface.is_some());

        send_resize_intent(&mut stream, "pane-1", 100, 30).expect("send resize intent");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::Unknown,
                message: "resize failed: host I/O error during resize_pane for pane-1: simulated resize failure".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 0,
            })
        );

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_reports_output_poll_failure_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = FailingReadHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start failing read pane");
        host.fail_reads = false;

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1)
                .expect("serve live with read failure");
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");
        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert!(initial.surface.is_some());

        send_key_input(&mut stream, "pane-1", "poll-fails").expect("send input");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::Unknown,
                message: "output polling failed: host I/O error during try_read_output for pane-1: simulated read failure".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 0,
            })
        );

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_reports_missing_scrollback_pane_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1).expect("serve live");
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");
        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert!(initial.surface.is_some());

        send_scrollback_fetch(&mut stream, "missing-pane", scrollback_range(1, 2))
            .expect("send missing scrollback fetch");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::PaneNotFound,
                message: "pane not found: missing-pane".to_owned(),
                retryable: false,
                pane_id: Some("missing-pane".to_owned()),
                input_seq: 0,
            })
        );

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_reports_stale_scrollback_version_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1).expect("serve live");
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");
        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert!(initial.surface.is_some());

        let mut sequence = ClientFrameSequence::default();
        send_scrollback_fetch_with_known_version(
            &mut stream,
            &mut sequence,
            "pane-1",
            scrollback_fetch_spec(1, 2, 999),
        )
        .expect("send stale scrollback fetch");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::StaleVersion,
                message: "stale scrollback version for pane-1: client=999 server=1".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 0,
            })
        );
        send_scrollback_fetch_with_known_version(
            &mut stream,
            &mut sequence,
            "pane-1",
            scrollback_fetch_spec(1, 1, 0),
        )
        .expect("retry live scrollback fetch without precondition");
        let scrollback =
            read_scrollback_chunk_from_stream(&mut stream).expect("live scrollback chunk");
        assert_eq!(scrollback.scrollback_version, 1);
        assert_eq!(
            scrollback.lines,
            vec![scrollback_line(1, "booting nmux workspace")]
        );
        drop(stream);

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_reports_missing_resize_pane_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1)
                .expect("serve live with missing resize pane");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");
        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert!(initial.surface.is_some());

        send_resize_intent(&mut stream, "missing-pane", 100, 30).expect("send resize intent");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::PaneNotFound,
                message: "pane not found: missing-pane".to_owned(),
                retryable: false,
                pane_id: Some("missing-pane".to_owned()),
                input_seq: 0,
            })
        );

        let host = server.join().expect("server thread");
        assert!(!host.events().iter().any(|event| matches!(
            event,
            HostEvent::Resized {
                pane_id,
                cols: 100,
                rows: 30,
            } if pane_id == "missing-pane"
        )));
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_reports_missing_input_pane_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1)
                .expect("serve live with missing input pane");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");
        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert!(initial.surface.is_some());

        send_key_input(&mut stream, "missing-pane", "input").expect("send input");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::PaneNotFound,
                message: "pane not found: missing-pane".to_owned(),
                retryable: false,
                pane_id: Some("missing-pane".to_owned()),
                input_seq: 1,
            })
        );

        let host = server.join().expect("server thread");
        assert!(!host.events().iter().any(|event| matches!(
            event,
            HostEvent::Input { pane_id, .. } if pane_id == "missing-pane"
        )));
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
    fn live_read_only_attach_rejects_explicit_input_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

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

        send_key_input(&mut stream, "pane-1", "denied").expect("send input");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::PermissionDenied,
                message: "input rejected: actor is read-only".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 1,
            })
        );

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_read_only_attach_rejects_resize_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

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

        send_resize_intent(&mut stream, "pane-1", 100, 30).expect("send resize");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::PermissionDenied,
                message: "resize rejected: actor is read-only".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 0,
            })
        );

        let host = server.join().expect("server thread");
        assert!(!host.events().iter().any(|event| matches!(
            event,
            HostEvent::Resized {
                pane_id,
                cols: 100,
                rows: 30,
            } if pane_id == "pane-1"
        )));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_reports_focus_disabled_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1)
                .expect("serve focus-disabled live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);

        send_focus_input(&mut stream, "pane-1", true).expect("send focus input");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::PermissionDenied,
                message: "input rejected: focus reporting is disabled".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 1,
            })
        );

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_current_surface_attach_reports_focus_disabled_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 2)
                .expect("serve current focus-disabled live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: vec![KnownSurfaceVersion {
                    pane_id: "pane-1".to_owned(),
                    version: 2,
                }],
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);
        assert_eq!(initial.surface, None);

        send_focus_input(&mut stream, "pane-1", true).expect("send focus input");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::PermissionDenied,
                message: "input rejected: focus reporting is disabled".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 1,
            })
        );

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_current_surface_attach_forwards_focus_when_reporting_enabled() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        session.tabs[0].root.modes.focus_reporting = true;
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 2)
                .expect("serve current focus live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: vec![KnownSurfaceVersion {
                    pane_id: "pane-1".to_owned(),
                    version: 2,
                }],
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);
        assert_eq!(initial.surface, None);

        send_focus_input(&mut stream, "pane-1", true).expect("send focus input");
        let host = server.join().expect("server thread");
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"\x1b[I".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_current_surface_attach_forwards_key_input() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 2)
                .expect("serve current key live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: vec![KnownSurfaceVersion {
                    pane_id: "pane-1".to_owned(),
                    version: 2,
                }],
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);
        assert_eq!(initial.surface, None);

        send_key_input(&mut stream, "pane-1", "current-key").expect("send key input");
        let host = server.join().expect("server thread");
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"current-key".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_current_surface_attach_forwards_paste_input() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        session.tabs[0].root.modes.bracketed_paste = true;
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 2)
                .expect("serve current paste live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: vec![KnownSurfaceVersion {
                    pane_id: "pane-1".to_owned(),
                    version: 2,
                }],
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);
        assert_eq!(initial.surface, None);

        send_paste_input(&mut stream, "pane-1", "current-paste").expect("send paste input");
        let host = server.join().expect("server thread");
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"\x1b[200~current-paste\x1b[201~".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_current_surface_attach_forwards_named_key_input() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 2)
                .expect("serve current named-key live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: vec![KnownSurfaceVersion {
                    pane_id: "pane-1".to_owned(),
                    version: 2,
                }],
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);
        assert_eq!(initial.surface, None);

        send_named_key_input(&mut stream, "pane-1", "delete").expect("send named-key input");
        let host = server.join().expect("server thread");
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"\x1b[3~".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_current_surface_attach_reports_mouse_disabled_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 2)
                .expect("serve current mouse-disabled live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: vec![KnownSurfaceVersion {
                    pane_id: "pane-1".to_owned(),
                    version: 2,
                }],
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);
        assert_eq!(initial.surface, None);

        send_mouse_input(
            &mut stream,
            "pane-1",
            AttachMouseInput {
                row: 0,
                col: 0,
                pixel_x: None,
                pixel_y: None,
                button: protocol::MouseButton::Left,
                action: protocol::MouseAction::Press,
                modifiers: 0,
            },
        )
        .expect("send mouse input");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::PermissionDenied,
                message: "input rejected: mouse tracking is disabled".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 1,
            })
        );

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn live_current_surface_attach_forwards_mouse_with_libghostty_vt() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = ScriptedOutputHost::new(vec![b"\x1b[?1000h\x1b[?1006h".to_vec()]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_live_n_with_host_and_terminal_engine_kind(
                &listener,
                &mut session,
                &mut host,
                1,
                2,
                TerminalEngineKind::LibghosttyVt,
            )
            .expect("serve current mouse live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: vec![KnownSurfaceVersion {
                    pane_id: "pane-1".to_owned(),
                    version: 3,
                }],
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);
        assert_eq!(initial.surface, None);

        send_mouse_input(
            &mut stream,
            "pane-1",
            AttachMouseInput {
                row: 0,
                col: 0,
                pixel_x: None,
                pixel_y: None,
                button: protocol::MouseButton::Left,
                action: protocol::MouseAction::Press,
                modifiers: 0,
            },
        )
        .expect("send mouse input");
        let host = server.join().expect("server thread");
        assert!(host.events.contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"\x1b[<0;1;1M".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn live_libghostty_vt_mode_only_patch_updates_cached_modes() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = ScriptedOutputHost::new(vec![
            b"ready\n".to_vec(),
            Vec::new(),
            b"\x1b[?2004h\x1b[?1004h".to_vec(),
        ]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_live_n_with_host_and_terminal_engine_kind(
                &listener,
                &mut session,
                &mut host,
                1,
                2,
                TerminalEngineKind::LibghosttyVt,
            )
            .expect("serve mode-only live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        let mut state = ClientAttachState::default();
        let rendered = state.render_attach(initial).expect("render initial attach");
        let initial_text = rendered.surface_text.expect("initial surface text");
        assert!(initial_text.contains("ready"));
        assert_eq!(
            state.cached_surface_modes("pane-1"),
            Some(TerminalModeSummary::default())
        );

        let update = loop {
            match read_live_surface_update_from_stream(&mut stream).expect("live update") {
                LiveSurfaceRead::Update(update) => break update,
                LiveSurfaceRead::NoFrame => continue,
                other => panic!("expected mode-only surface update, got {other:?}"),
            }
        };
        assert_eq!(update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(update.patch_kind, Some(protocol::PatchKind::ModeOnly));
        assert_eq!(update.row_updates, Vec::new());
        assert!(update.modes.bracketed_paste);
        assert!(update.modes.focus_reporting);

        let updated_text = state
            .render_surface_update(&update)
            .expect("render mode-only update");
        assert_eq!(updated_text, initial_text);
        assert_eq!(Some(updated_text), state.cached_surface_text("pane-1"));
        assert_eq!(state.cached_surface_modes("pane-1"), Some(update.modes));

        let decoded = ClientAttachState::decode(&state.encode()).expect("decode state");
        assert_eq!(decoded.cached_surface_modes("pane-1"), Some(update.modes));
        assert_eq!(
            decoded.cached_surface_text("pane-1"),
            state.cached_surface_text("pane-1")
        );

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn live_libghostty_vt_color_only_patch_updates_cached_colors() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = ScriptedOutputHost::new(vec![
            b"ready\n".to_vec(),
            Vec::new(),
            b"\x1b]12;#ff00ff\x1b\\\x1b]4;1;#112233\x1b\\".to_vec(),
        ]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_live_n_with_host_and_terminal_engine_kind(
                &listener,
                &mut session,
                &mut host,
                1,
                2,
                TerminalEngineKind::LibghosttyVt,
            )
            .expect("serve color-only live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        let mut state = ClientAttachState::default();
        let rendered = state.render_attach(initial).expect("render initial attach");
        let initial_text = rendered.surface_text.expect("initial surface text");
        assert!(initial_text.contains("ready"));
        assert!(
            !state.surfaces[0].colors.cursor_rgba_set,
            "initial cursor color should not be explicit"
        );

        let update = loop {
            match read_live_surface_update_from_stream(&mut stream).expect("live update") {
                LiveSurfaceRead::Update(update) => break update,
                LiveSurfaceRead::NoFrame => continue,
                other => panic!("expected color-only surface update, got {other:?}"),
            }
        };
        assert_eq!(update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(update.patch_kind, Some(protocol::PatchKind::ColorOnly));
        assert_eq!(update.row_updates, Vec::new());
        let update_colors = update.colors.clone().expect("color-only colors");
        assert_eq!(update_colors.cursor_rgba, 0xff00_ffff);
        assert!(update_colors.cursor_rgba_set);
        assert_eq!(update_colors.palette_rgba, Vec::<u32>::new());
        assert_eq!(update_colors.palette_diff_start, Some(1));
        assert_eq!(update_colors.palette_diff_rgba.first(), Some(&0x112233ff));

        let updated_text = state
            .render_surface_update(&update)
            .expect("render color-only update");
        assert_eq!(updated_text, initial_text);
        assert_eq!(Some(updated_text), state.cached_surface_text("pane-1"));
        assert_eq!(state.surfaces[0].colors.cursor_rgba, 0xff00_ffff);
        assert!(state.surfaces[0].colors.cursor_rgba_set);
        assert_eq!(
            state.surfaces[0].colors.palette_rgba.get(1),
            Some(&0x112233ff)
        );
        assert_eq!(state.surfaces[0].colors.palette_diff_start, None);
        assert_eq!(
            state.surfaces[0].colors.palette_diff_rgba,
            Vec::<u32>::new()
        );

        let decoded = ClientAttachState::decode(&state.encode()).expect("decode state");
        assert_eq!(decoded.surfaces[0].colors, state.surfaces[0].colors);
        assert_eq!(
            decoded.cached_surface_text("pane-1"),
            state.cached_surface_text("pane-1")
        );

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn live_libghostty_vt_cursor_only_patch_updates_cached_cursor() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host =
            ScriptedOutputHost::new(vec![b"ready\n".to_vec(), Vec::new(), b"\x1b[2;5H".to_vec()]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_live_n_with_host_and_terminal_engine_kind(
                &listener,
                &mut session,
                &mut host,
                1,
                2,
                TerminalEngineKind::LibghosttyVt,
            )
            .expect("serve cursor-only live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        let mut state = ClientAttachState::default();
        let rendered = state.render_attach(initial).expect("render initial attach");
        let initial_text = rendered.surface_text.expect("initial surface text");
        assert!(initial_text.contains("ready"));
        let initial_cursor = state.surfaces[0].cursor.expect("initial cursor");

        let update = loop {
            match read_live_surface_update_from_stream(&mut stream).expect("live update") {
                LiveSurfaceRead::Update(update) => break update,
                LiveSurfaceRead::NoFrame => continue,
                other => panic!("expected cursor-only surface update, got {other:?}"),
            }
        };
        assert_eq!(update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(update.patch_kind, Some(protocol::PatchKind::CursorOnly));
        assert_eq!(update.row_updates, Vec::new());
        let update_cursor = update.cursor.expect("cursor-only cursor");
        assert_ne!(update_cursor, initial_cursor);
        assert_eq!(update_cursor.row, 1);
        assert_eq!(update_cursor.col, 4);

        let updated_text = state
            .render_surface_update(&update)
            .expect("render cursor-only update");
        assert_eq!(updated_text, initial_text);
        assert_eq!(Some(updated_text), state.cached_surface_text("pane-1"));
        assert_eq!(state.surfaces[0].cursor, Some(update_cursor));

        let decoded = ClientAttachState::decode(&state.encode()).expect("decode state");
        assert_eq!(decoded.surfaces[0].cursor, Some(update_cursor));
        assert_eq!(
            decoded.cached_surface_text("pane-1"),
            state.cached_surface_text("pane-1")
        );

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn live_libghostty_vt_replace_rows_patch_updates_cached_row_metadata() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = ScriptedOutputHost::new(vec![
            b"ready\n".to_vec(),
            Vec::new(),
            b"\x1b]133;A\x1b\\prompt \x1b]133;B\x1b\\input\x1b]133;C\x1b\\output".to_vec(),
        ]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_live_n_with_host_and_terminal_engine_kind(
                &listener,
                &mut session,
                &mut host,
                1,
                2,
                TerminalEngineKind::LibghosttyVt,
            )
            .expect("serve replace-rows live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        let mut state = ClientAttachState::default();
        let rendered = state.render_attach(initial).expect("render initial attach");
        let initial_text = rendered.surface_text.expect("initial surface text");
        assert!(initial_text.contains("ready"));

        let update = loop {
            match read_live_surface_update_from_stream(&mut stream).expect("live update") {
                LiveSurfaceRead::Update(update) => break update,
                LiveSurfaceRead::NoFrame => continue,
                other => panic!("expected replace-rows surface update, got {other:?}"),
            }
        };
        assert_eq!(update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(update.patch_kind, Some(protocol::PatchKind::ReplaceRows));
        let prompt_row = update
            .row_updates
            .iter()
            .find(|row| row.text == "prompt inputoutput")
            .expect("prompt row update");
        assert_eq!(
            prompt_row.semantic_prompt,
            protocol::RowSemanticPrompt::Prompt
        );
        assert!(prompt_row.dirty);
        assert_ne!(prompt_row.row_state_hash, prompt_row.dirty_hash);
        let semantic_content: Vec<_> = prompt_row
            .runs
            .iter()
            .map(|run| run.semantic_content)
            .collect();
        assert_eq!(
            semantic_content,
            vec![
                protocol::CellSemanticContent::Prompt,
                protocol::CellSemanticContent::Input,
                protocol::CellSemanticContent::Output,
            ]
        );
        let prompt_row_index =
            usize::try_from(prompt_row.row).expect("prompt row index fits in usize");
        let prompt_row_state_hash = prompt_row.row_state_hash;

        let updated_text = state
            .render_surface_update(&update)
            .expect("render replace-rows update");
        assert_ne!(updated_text, initial_text);
        assert_eq!(Some(updated_text), state.cached_surface_text("pane-1"));
        assert_eq!(
            state.surfaces[0].row_semantic_prompts[prompt_row_index],
            protocol::RowSemanticPrompt::Prompt
        );
        assert!(state.surfaces[0].row_dirty[prompt_row_index]);
        assert_eq!(
            state.surfaces[0].row_state_hashes[prompt_row_index],
            prompt_row_state_hash
        );
        assert_eq!(
            state.surfaces[0].row_runs[prompt_row_index]
                .iter()
                .map(|run| run.semantic_content)
                .collect::<Vec<_>>(),
            semantic_content
        );

        let decoded = ClientAttachState::decode(&state.encode()).expect("decode state");
        assert_eq!(
            decoded.surfaces[0].row_semantic_prompts[prompt_row_index],
            protocol::RowSemanticPrompt::Prompt
        );
        assert!(decoded.surfaces[0].row_dirty[prompt_row_index]);
        assert_eq!(
            decoded.surfaces[0].row_state_hashes[prompt_row_index],
            prompt_row_state_hash
        );
        assert_eq!(
            decoded.surfaces[0].row_runs[prompt_row_index]
                .iter()
                .map(|run| run.semantic_content)
                .collect::<Vec<_>>(),
            semantic_content
        );

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn live_libghostty_vt_replace_rows_patch_updates_cached_hyperlink_runs() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = ScriptedOutputHost::new(vec![
            b"ready\n".to_vec(),
            Vec::new(),
            b"\x1b]8;;https://example.com\x1b\\linked\x1b]8;;\x1b\\ text".to_vec(),
        ]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_live_n_with_host_and_terminal_engine_kind(
                &listener,
                &mut session,
                &mut host,
                1,
                2,
                TerminalEngineKind::LibghosttyVt,
            )
            .expect("serve hyperlink live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(&mut stream, &AttachOptions::default().request)
            .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        let mut state = ClientAttachState::default();
        let rendered = state.render_attach(initial).expect("render initial attach");
        let initial_text = rendered.surface_text.expect("initial surface text");
        assert!(initial_text.contains("ready"));

        let update = loop {
            match read_live_surface_update_from_stream(&mut stream).expect("live update") {
                LiveSurfaceRead::Update(update) => break update,
                LiveSurfaceRead::NoFrame => continue,
                other => panic!("expected hyperlink replace-rows update, got {other:?}"),
            }
        };
        assert_eq!(update.kind, SurfaceUpdateKind::Patch);
        assert_eq!(update.patch_kind, Some(protocol::PatchKind::ReplaceRows));
        let link_row = update
            .row_updates
            .iter()
            .find(|row| row.text.contains("linked text"))
            .expect("hyperlink row update");
        let linked_run = link_row
            .runs
            .iter()
            .find(|run| run.text == "linked")
            .expect("linked run");
        assert_ne!(
            linked_run.flags & CELL_RUN_FLAG_HYPERLINK_PRESENT,
            0,
            "hyperlink presence flag missing from live row update"
        );
        assert_eq!(linked_run.hyperlink_id, 0);
        let plain_run = link_row
            .runs
            .iter()
            .find(|run| run.text == " text")
            .expect("plain run");
        assert_eq!(plain_run.flags & CELL_RUN_FLAG_HYPERLINK_PRESENT, 0);
        let link_row_index = usize::try_from(link_row.row).expect("link row index fits in usize");

        let updated_text = state
            .render_surface_update(&update)
            .expect("render hyperlink update");
        assert_ne!(updated_text, initial_text);
        assert_eq!(Some(updated_text), state.cached_surface_text("pane-1"));
        let cached_linked_run = state.surfaces[0].row_runs[link_row_index]
            .iter()
            .find(|run| run.text == "linked")
            .expect("cached linked run");
        assert_ne!(
            cached_linked_run.flags & CELL_RUN_FLAG_HYPERLINK_PRESENT,
            0,
            "hyperlink presence flag missing from cached row run"
        );
        assert_eq!(cached_linked_run.hyperlink_id, 0);

        let decoded = ClientAttachState::decode(&state.encode()).expect("decode state");
        let decoded_linked_run = decoded.surfaces[0].row_runs[link_row_index]
            .iter()
            .find(|run| run.text == "linked")
            .expect("decoded linked run");
        assert_ne!(
            decoded_linked_run.flags & CELL_RUN_FLAG_HYPERLINK_PRESENT,
            0,
            "hyperlink presence flag missing after state encode/decode"
        );
        assert_eq!(decoded_linked_run.hyperlink_id, 0);

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_poll_writes_terminal_query_reply_to_host() {
        let mut session = Session::initial();
        let mut engines = PaneTerminalEngines::new(TerminalEngineKind::LibghosttyVt);
        let mut host = ScriptedOutputHost::new(vec![b"\x1b[?7$p".to_vec()]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        assert!(
            poll_pane_output_with_host_and_engines(&mut session, &mut engines, &mut host, "pane-1")
                .expect("poll output")
        );

        let replies = host
            .events
            .iter()
            .filter_map(|event| match event {
                HostEvent::Input { bytes, .. } => Some(bytes.as_slice()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(replies, vec![b"\x1b[?7;1$y".as_slice()]);
    }

    #[test]
    fn live_attach_reports_mouse_out_of_bounds_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        session.tabs[0].root.modes.mouse_tracking = true;
        session.tabs[0].root.modes.mouse_tracking_mode = protocol::MouseTrackingMode::Normal;
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1)
                .expect("serve mouse-bounds live");
            host
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);

        send_mouse_input(
            &mut stream,
            "pane-1",
            AttachMouseInput {
                row: 0,
                col: 80,
                pixel_x: None,
                pixel_y: None,
                button: protocol::MouseButton::Left,
                action: protocol::MouseAction::Press,
                modifiers: 0,
            },
        )
        .expect("send mouse input");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::PermissionDenied,
                message: "input rejected: mouse coordinates are outside pane bounds".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 1,
            })
        );

        let host = server.join().expect("server thread");
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn live_attach_reports_host_write_failure_with_error_frame() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = FailingWriteHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start failing write pane");

        let server = thread::spawn(move || {
            serve_live_one_with_host(&listener, &mut session, &mut host, 1)
                .expect("serve live with write failure");
        });
        let mut stream = UnixStream::connect(&socket_path).expect("connect client");
        write_attach_request(
            &mut stream,
            &AttachRequest {
                actor_id: "writer".to_owned(),
                user_id: "local-user".to_owned(),
                display_name: "local".to_owned(),
                mode: AttachMode::ReadWrite,
                focused_pane_id: Some("pane-1".to_owned()),
                known_surfaces: Vec::new(),
            },
        )
        .expect("write attach request");

        let initial = attach_from_stream(&mut stream).expect("initial attach");
        assert_eq!(initial.presence.mode, AttachMode::ReadWrite);

        send_key_input(&mut stream, "pane-1", "fail").expect("send input");
        let error = read_live_surface_update_from_stream(&mut stream).expect("live error");
        assert_eq!(
            error,
            LiveSurfaceRead::Error(ErrorSummary {
                code: protocol::ErrorCode::Unknown,
                message: "input forwarding failed: host I/O error during write_input for pane-1: simulated write failure".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 1,
            })
        );

        server.join().expect("server thread");
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
        send_scrollback_fetch(&mut stream, "pane-1", scrollback_range(1, 2))
            .expect("send scrollback fetch");
        let scrollback = read_scrollback_chunk_from_stream(&mut stream).expect("scrollback chunk");

        assert_eq!(
            scrollback,
            ScrollbackChunkSummary {
                pane_id: "pane-1".to_owned(),
                scrollback_version: 1,
                start_line: 1,
                total_lines: 3,
                styles: default_style_summaries(),
                hyperlinks: Vec::new(),
                colors: TerminalColorSummary::default(),
                lines: vec![
                    scrollback_line(1, "booting nmux workspace"),
                    scrollback_line(2, "nmux pane-1"),
                ],
            }
        );

        server.join().expect("server thread");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn current_surface_attach_still_fetches_scrollback() {
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
        assert_eq!(
            snapshot.status,
            AttachStatusSummary {
                pane_id: "pane-1".to_owned(),
                surface_version: 2,
                surface_state: protocol::AttachSurfaceState::Current,
            }
        );
        assert_eq!(snapshot.surface, None);
        assert_eq!(
            snapshot
                .scrollback
                .as_ref()
                .map(|chunk| chunk.lines.clone()),
            Some(vec![
                scrollback_line(1, "booting nmux workspace"),
                scrollback_line(2, "nmux pane-1"),
            ])
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn current_surface_attach_uses_attached_pane_for_cached_scrollback_precondition() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        rename_initial_pane(&mut session, "pane-2");

        let scope = socket_identity(&socket_path).ok();
        let mut state = ClientAttachState::default();
        state.apply_scope(scope);
        let surface = surface_update_from_frame(
            &session
                .pane_surface_frame_for_pane("local-client", 3, "pane-2")
                .expect("pane-2 surface frame"),
        )
        .expect("surface snapshot");
        state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-2".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: attach_status_summary("pane-2", 2),
                surface: Some(surface),
                scrollback: Some(ScrollbackChunkSummary {
                    pane_id: "pane-2".to_owned(),
                    scrollback_version: 1,
                    start_line: 1,
                    total_lines: 1,
                    styles: default_style_summaries(),
                    hyperlinks: Vec::new(),
                    colors: TerminalColorSummary::default(),
                    lines: vec![
                        scrollback_line(1, "booting pane-2"),
                        scrollback_line(2, "nmux pane-2"),
                    ],
                }),
            })
            .expect("seed pane-2 cached state");
        {
            let pane = &mut session.tabs[0].root;
            pane.scrollback_version = 2;
            pane.scrollback_lines.push("history only".to_owned());
            pane.scrollback_row_runs
                .push(vec![CellRun::plain("history only")]);
            pane.scrollback_semantic_prompts
                .push(protocol::RowSemanticPrompt::None);
            pane.scrollback_dirty_rows.push(false);
            pane.scrollback_kitty_placeholders.push(false);
        }

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_attach_request(&mut stream).expect("read attach request");
            assert!(
                request
                    .known_surfaces
                    .iter()
                    .any(|known| { known.pane_id == "pane-2" && known.version == 2 })
            );
            wire::write_default_frame(
                &mut stream,
                &session.workspace_tree_frame("local-client", 1),
            )
            .expect("write workspace");
            wire::write_default_frame(
                &mut stream,
                &session.presence_update_frame("local-client", 2, &request.actor()),
            )
            .expect("write presence");
            wire::write_default_frame(
                &mut stream,
                &session.attach_status_frame(
                    "local-client",
                    3,
                    "pane-2",
                    protocol::AttachSurfaceState::Current,
                ),
            )
            .expect("write attach status");

            let mut seq = 4;
            let first = read_scrollback_fetch_from_stream(&mut stream).expect("first fetch");
            assert_eq!(first.pane_id, "pane-2");
            assert_eq!(first.known_scrollback_version, 1);
            write_scrollback_fetch_error(
                &mut stream,
                &session,
                &mut seq,
                &first,
                protocol::ErrorCode::StaleVersion,
            )
            .expect("write stale version");

            let retry = read_scrollback_fetch_from_stream(&mut stream).expect("retry fetch");
            assert_eq!(retry.pane_id, "pane-2");
            assert_eq!(retry.known_scrollback_version, 0);
            let chunk = session
                .scrollback_chunk_frame_for_pane(
                    "local-client",
                    seq,
                    "pane-2",
                    retry.start_line,
                    retry.line_count,
                )
                .expect("pane-2 scrollback chunk");
            wire::write_default_frame(&mut stream, &chunk).expect("write chunk");
        });

        let rendered = attach_render_once(&socket_path, read_only_attach_options(), &mut state)
            .expect("attach render");
        server.join().expect("server thread");

        assert_eq!(rendered.workspace.pane_id, "pane-2");
        assert_eq!(state.cached_scrollback_version("pane-2", 1, 2), Some(2));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn current_surface_attach_retries_stale_cached_scrollback() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        {
            let pane = &mut session.tabs[0].root;
            pane.scrollback_version = 2;
            pane.scrollback_lines.push("history only".to_owned());
            pane.scrollback_row_runs
                .push(vec![CellRun::plain("history only")]);
            pane.scrollback_semantic_prompts
                .push(protocol::RowSemanticPrompt::None);
            pane.scrollback_dirty_rows.push(false);
            pane.scrollback_kitty_placeholders.push(false);
        }
        let scope = socket_identity(&socket_path).ok();
        let mut state = ClientAttachState::default();
        state.apply_scope(scope);
        let surface = surface_update_from_frame(&session.pane_surface_frame("local-client", 3))
            .expect("surface snapshot");
        state
            .render_attach(AttachSnapshot {
                workspace: WorkspaceSummary {
                    session_id: "local".to_owned(),
                    tab_id: "tab-1".to_owned(),
                    pane_id: "pane-1".to_owned(),
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    pane_tree: None,
                },
                presence: presence_summary(AttachMode::ReadWrite),
                status: attach_status_summary("pane-1", 2),
                surface: Some(surface),
                scrollback: Some(ScrollbackChunkSummary {
                    pane_id: "pane-1".to_owned(),
                    scrollback_version: 1,
                    start_line: 1,
                    total_lines: 3,
                    styles: default_style_summaries(),
                    hyperlinks: Vec::new(),
                    colors: TerminalColorSummary::default(),
                    lines: vec![
                        scrollback_line(1, "booting nmux workspace"),
                        scrollback_line(2, "nmux pane-1"),
                    ],
                }),
            })
            .expect("seed cached state");

        let server = thread::spawn(move || serve_one(&listener, &mut session).expect("serve one"));
        let rendered = attach_render_once(&socket_path, read_only_attach_options(), &mut state)
            .expect("attach render");
        server.join().expect("server thread");

        assert_eq!(
            rendered.surface_text.as_deref(),
            Some("nmux pane-1\nserver-owned terminal state")
        );
        let scrollback = rendered.scrollback.expect("fresh scrollback after retry");
        assert_eq!(scrollback.scrollback_version, 2);
        assert_eq!(
            scrollback.lines,
            vec![
                scrollback_line(1, "booting nmux workspace"),
                scrollback_line(2, "nmux pane-1"),
            ]
        );
        assert_eq!(state.cached_scrollback_version("pane-1", 1, 2), Some(2));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn current_surface_attach_forwards_explicit_input_before_scrollback() {
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
        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                request: AttachRequest {
                    actor_id: "writer".to_owned(),
                    user_id: "local-user".to_owned(),
                    display_name: "local".to_owned(),
                    mode: AttachMode::ReadWrite,
                    focused_pane_id: Some("pane-1".to_owned()),
                    known_surfaces: vec![KnownSurfaceVersion {
                        pane_id: "pane-1".to_owned(),
                        version: 2,
                    }],
                },
                input_text: Some("current-input".to_owned()),
                key_name: None,
                key_names: Vec::new(),
                key_modifiers: 0,
                paste_text: None,
                focus: None,
                mouse: None,
                scrollback_start_line: 1,
                scrollback_line_count: 2,
                scrollback_tail_count: None,
                fetch_scrollback: true,
                known_scrollback_version: 0,
                known_scrollback_versions: Vec::new(),
                connect_timeout: None,
            },
        )
        .expect("attach snapshot");
        let host = server.join().expect("server thread");

        assert_eq!(snapshot.surface, None);
        assert!(snapshot.scrollback.is_some());
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"current-input".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn current_surface_attach_forwards_paste_before_scrollback() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        session.tabs[0].root.modes.bracketed_paste = true;
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host).expect("serve one");
            host
        });
        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                request: AttachRequest {
                    actor_id: "writer".to_owned(),
                    user_id: "local-user".to_owned(),
                    display_name: "local".to_owned(),
                    mode: AttachMode::ReadWrite,
                    focused_pane_id: Some("pane-1".to_owned()),
                    known_surfaces: vec![KnownSurfaceVersion {
                        pane_id: "pane-1".to_owned(),
                        version: 2,
                    }],
                },
                input_text: None,
                key_name: None,
                key_names: Vec::new(),
                key_modifiers: 0,
                paste_text: Some("current-paste".to_owned()),
                focus: None,
                mouse: None,
                scrollback_start_line: 1,
                scrollback_line_count: 2,
                scrollback_tail_count: None,
                fetch_scrollback: true,
                known_scrollback_version: 0,
                known_scrollback_versions: Vec::new(),
                connect_timeout: None,
            },
        )
        .expect("attach snapshot");
        let host = server.join().expect("server thread");

        assert_eq!(snapshot.surface, None);
        assert!(snapshot.scrollback.is_some());
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"\x1b[200~current-paste\x1b[201~".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn current_surface_attach_forwards_named_key_before_scrollback() {
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
        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                request: AttachRequest {
                    actor_id: "writer".to_owned(),
                    user_id: "local-user".to_owned(),
                    display_name: "local".to_owned(),
                    mode: AttachMode::ReadWrite,
                    focused_pane_id: Some("pane-1".to_owned()),
                    known_surfaces: vec![KnownSurfaceVersion {
                        pane_id: "pane-1".to_owned(),
                        version: 2,
                    }],
                },
                input_text: None,
                key_name: Some("delete".to_owned()),
                key_names: Vec::new(),
                key_modifiers: 0,
                paste_text: None,
                focus: None,
                mouse: None,
                scrollback_start_line: 1,
                scrollback_line_count: 2,
                scrollback_tail_count: None,
                fetch_scrollback: true,
                known_scrollback_version: 0,
                known_scrollback_versions: Vec::new(),
                connect_timeout: None,
            },
        )
        .expect("attach snapshot");
        let host = server.join().expect("server thread");

        assert_eq!(snapshot.surface, None);
        assert!(snapshot.scrollback.is_some());
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"\x1b[3~".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn current_surface_attach_forwards_named_key_sequence_before_scrollback() {
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
        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                request: AttachRequest {
                    actor_id: "writer".to_owned(),
                    user_id: "local-user".to_owned(),
                    display_name: "local".to_owned(),
                    mode: AttachMode::ReadWrite,
                    focused_pane_id: Some("pane-1".to_owned()),
                    known_surfaces: vec![KnownSurfaceVersion {
                        pane_id: "pane-1".to_owned(),
                        version: 2,
                    }],
                },
                input_text: None,
                key_name: None,
                key_names: vec!["escape".to_owned(), "enter".to_owned()],
                key_modifiers: 0,
                paste_text: None,
                focus: None,
                mouse: None,
                scrollback_start_line: 1,
                scrollback_line_count: 2,
                scrollback_tail_count: None,
                fetch_scrollback: true,
                known_scrollback_version: 0,
                known_scrollback_versions: Vec::new(),
                connect_timeout: None,
            },
        )
        .expect("attach snapshot");
        let host = server.join().expect("server thread");
        let input_bytes = host
            .events()
            .iter()
            .filter_map(|event| match event {
                HostEvent::Input { bytes, .. } => Some(bytes.as_slice()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(snapshot.surface, None);
        assert!(snapshot.scrollback.is_some());
        assert_eq!(input_bytes, vec![b"\x1b".as_slice(), b"\r".as_slice()]);

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn current_surface_attach_forwards_focus_before_scrollback() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        session.tabs[0].root.modes.focus_reporting = true;
        let mut host = PlanningHost::default();
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start planning pane");

        let server = thread::spawn(move || {
            serve_one_with_host(&listener, &mut session, &mut host).expect("serve one");
            host
        });
        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                request: AttachRequest {
                    actor_id: "writer".to_owned(),
                    user_id: "local-user".to_owned(),
                    display_name: "local".to_owned(),
                    mode: AttachMode::ReadWrite,
                    focused_pane_id: Some("pane-1".to_owned()),
                    known_surfaces: vec![KnownSurfaceVersion {
                        pane_id: "pane-1".to_owned(),
                        version: 2,
                    }],
                },
                input_text: None,
                key_name: None,
                key_names: Vec::new(),
                key_modifiers: 0,
                paste_text: None,
                focus: Some(true),
                mouse: None,
                scrollback_start_line: 1,
                scrollback_line_count: 2,
                scrollback_tail_count: None,
                fetch_scrollback: true,
                known_scrollback_version: 0,
                known_scrollback_versions: Vec::new(),
                connect_timeout: None,
            },
        )
        .expect("attach snapshot");
        let host = server.join().expect("server thread");

        assert_eq!(snapshot.surface, None);
        assert!(snapshot.scrollback.is_some());
        assert!(host.events().contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"\x1b[I".to_vec(),
        }));

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn current_surface_attach_reports_focus_disabled_before_scrollback() {
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
        let err = attach_with_client_options(
            &socket_path,
            AttachOptions {
                request: AttachRequest {
                    actor_id: "writer".to_owned(),
                    user_id: "local-user".to_owned(),
                    display_name: "local".to_owned(),
                    mode: AttachMode::ReadWrite,
                    focused_pane_id: Some("pane-1".to_owned()),
                    known_surfaces: vec![KnownSurfaceVersion {
                        pane_id: "pane-1".to_owned(),
                        version: 2,
                    }],
                },
                input_text: None,
                key_name: None,
                key_names: Vec::new(),
                key_modifiers: 0,
                paste_text: None,
                focus: Some(true),
                mouse: None,
                scrollback_start_line: 1,
                scrollback_line_count: 2,
                scrollback_tail_count: None,
                fetch_scrollback: true,
                known_scrollback_version: 0,
                known_scrollback_versions: Vec::new(),
                connect_timeout: None,
            },
        )
        .expect_err("focus input should report server error");
        let host = server.join().expect("server thread");

        assert!(
            err.to_string()
                .contains("server error: input rejected: focus reporting is disabled"),
            "unexpected error: {err}"
        );
        assert!(
            err.to_string().contains("code=PermissionDenied")
                && err.to_string().contains("pane_id=pane-1")
                && err.to_string().contains("input_seq=1"),
            "missing structured server error attribution: {err}"
        );
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn current_surface_attach_reports_mouse_disabled_before_scrollback() {
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
        let err = attach_with_client_options(
            &socket_path,
            AttachOptions {
                request: AttachRequest {
                    actor_id: "writer".to_owned(),
                    user_id: "local-user".to_owned(),
                    display_name: "local".to_owned(),
                    mode: AttachMode::ReadWrite,
                    focused_pane_id: Some("pane-1".to_owned()),
                    known_surfaces: vec![KnownSurfaceVersion {
                        pane_id: "pane-1".to_owned(),
                        version: 2,
                    }],
                },
                input_text: None,
                key_name: None,
                key_names: Vec::new(),
                key_modifiers: 0,
                paste_text: None,
                focus: None,
                mouse: Some(AttachMouseInput {
                    row: 0,
                    col: 0,
                    pixel_x: None,
                    pixel_y: None,
                    button: protocol::MouseButton::Left,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                }),
                scrollback_start_line: 1,
                scrollback_line_count: 2,
                scrollback_tail_count: None,
                fetch_scrollback: true,
                known_scrollback_version: 0,
                known_scrollback_versions: Vec::new(),
                connect_timeout: None,
            },
        )
        .expect_err("mouse input should report server error");
        let host = server.join().expect("server thread");

        assert!(
            err.to_string()
                .contains("server error: input rejected: mouse tracking is disabled"),
            "unexpected error: {err}"
        );
        assert!(
            !host
                .events()
                .iter()
                .any(|event| matches!(event, HostEvent::Input { .. }))
        );

        let _ = fs::remove_file(socket_path);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn current_surface_attach_forwards_mouse_before_scrollback() {
        let socket_path = test_socket_path();
        let listener = bind_listener(&socket_path).expect("bind listener");
        let mut session = Session::initial();
        let mut host = ScriptedOutputHost::new(vec![b"\x1b[?1000h\x1b[?1006h".to_vec()]);
        host.start_pane("pane-1", &session.tabs[0].root.host)
            .expect("start scripted pane");

        let server = thread::spawn(move || {
            serve_n_with_host_and_terminal_engine_kind(
                &listener,
                &mut session,
                &mut host,
                1,
                TerminalEngineKind::LibghosttyVt,
            )
            .expect("serve one");
            host
        });
        let snapshot = attach_with_client_options(
            &socket_path,
            AttachOptions {
                request: AttachRequest {
                    actor_id: "writer".to_owned(),
                    user_id: "local-user".to_owned(),
                    display_name: "local".to_owned(),
                    mode: AttachMode::ReadWrite,
                    focused_pane_id: Some("pane-1".to_owned()),
                    known_surfaces: vec![KnownSurfaceVersion {
                        pane_id: "pane-1".to_owned(),
                        version: 3,
                    }],
                },
                input_text: None,
                key_name: None,
                key_names: Vec::new(),
                key_modifiers: 0,
                paste_text: None,
                focus: None,
                mouse: Some(AttachMouseInput {
                    row: 0,
                    col: 0,
                    pixel_x: None,
                    pixel_y: None,
                    button: protocol::MouseButton::Left,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                }),
                scrollback_start_line: 1,
                scrollback_line_count: 2,
                scrollback_tail_count: None,
                fetch_scrollback: true,
                known_scrollback_version: 0,
                known_scrollback_versions: Vec::new(),
                connect_timeout: None,
            },
        )
        .expect("attach snapshot");
        let host = server.join().expect("server thread");

        assert_eq!(snapshot.surface, None);
        assert!(snapshot.scrollback.is_some());
        assert!(host.events.contains(&HostEvent::Input {
            pane_id: "pane-1".to_owned(),
            bytes: b"\x1b[<0;1;1M".to_vec(),
        }));

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
    fn rejects_attach_request_with_unknown_attach_mode() {
        let frame = attach_request_frame_with_mode(protocol::AttachMode(99));
        let err = read_attach_request(&mut frame.as_slice())
            .expect_err("unknown attach mode should be rejected");

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("unknown attach mode"));
    }

    #[test]
    fn rejects_attach_request_with_missing_or_empty_identity_fields() {
        for (frame, expected) in [
            (
                attach_request_frame_with_fields(
                    None,
                    Some("local-user"),
                    Some("local"),
                    Some("pane-1"),
                    None,
                ),
                "missing attach actor_id",
            ),
            (
                attach_request_frame_with_fields(
                    Some(""),
                    Some("local-user"),
                    Some("local"),
                    Some("pane-1"),
                    None,
                ),
                "empty attach actor_id",
            ),
            (
                attach_request_frame_with_fields(
                    Some("local-actor"),
                    None,
                    Some("local"),
                    Some("pane-1"),
                    None,
                ),
                "missing attach user_id",
            ),
            (
                attach_request_frame_with_fields(
                    Some("local-actor"),
                    Some(""),
                    Some("local"),
                    Some("pane-1"),
                    None,
                ),
                "empty attach user_id",
            ),
            (
                attach_request_frame_with_fields(
                    Some("local-actor"),
                    Some("local-user"),
                    None,
                    Some("pane-1"),
                    None,
                ),
                "missing attach display_name",
            ),
            (
                attach_request_frame_with_fields(
                    Some("local-actor"),
                    Some("local-user"),
                    Some(""),
                    Some("pane-1"),
                    None,
                ),
                "empty attach display_name",
            ),
        ] {
            let err = read_attach_request(&mut frame.as_slice()).expect_err("attach id rejected");
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn rejects_attach_request_with_empty_focused_or_known_surface_panes() {
        for (frame, expected) in [
            (
                attach_request_frame_with_fields(
                    Some("local-actor"),
                    Some("local-user"),
                    Some("local"),
                    Some(""),
                    None,
                ),
                "empty focused pane_id",
            ),
            (
                attach_request_frame_with_fields(
                    Some("local-actor"),
                    Some("local-user"),
                    Some("local"),
                    Some("pane-1"),
                    Some(None),
                ),
                "missing known surface pane_id",
            ),
            (
                attach_request_frame_with_fields(
                    Some("local-actor"),
                    Some("local-user"),
                    Some("local"),
                    Some("pane-1"),
                    Some(Some("")),
                ),
                "empty known surface pane_id",
            ),
        ] {
            let err =
                read_attach_request(&mut frame.as_slice()).expect_err("attach pane id rejected");
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }
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
    fn reconnect_loop_renders_snapshot_patch_then_cached_current_surface() {
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
        assert_eq!(
            third.surface_text.as_deref(),
            Some("booting nmux workspace\nnmux pane-1\nserver-owned terminal state\nloop update")
        );
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
    fn rejects_presence_update_with_unknown_attach_mode() {
        let frame = presence_update_frame_with_mode(protocol::AttachMode(99));
        let err = presence_from_frame(&frame)
            .expect_err("presence update with unknown attach mode should be rejected");

        assert!(err.to_string().contains("unknown attach mode"));
    }

    #[test]
    fn rejects_presence_update_with_missing_or_empty_identity_fields() {
        for (frame, expected) in [
            (
                presence_update_frame_with_fields(
                    None,
                    Some("local-user"),
                    Some("local"),
                    protocol::AttachMode::ReadOnly,
                    protocol::PresenceKind::Joined,
                    Some("pane-1"),
                ),
                "missing presence actor_id",
            ),
            (
                presence_update_frame_with_fields(
                    Some(""),
                    Some("local-user"),
                    Some("local"),
                    protocol::AttachMode::ReadOnly,
                    protocol::PresenceKind::Joined,
                    Some("pane-1"),
                ),
                "empty presence actor_id",
            ),
            (
                presence_update_frame_with_fields(
                    Some("local-actor"),
                    None,
                    Some("local"),
                    protocol::AttachMode::ReadOnly,
                    protocol::PresenceKind::Joined,
                    Some("pane-1"),
                ),
                "missing presence user_id",
            ),
            (
                presence_update_frame_with_fields(
                    Some("local-actor"),
                    Some(""),
                    Some("local"),
                    protocol::AttachMode::ReadOnly,
                    protocol::PresenceKind::Joined,
                    Some("pane-1"),
                ),
                "empty presence user_id",
            ),
            (
                presence_update_frame_with_fields(
                    Some("local-actor"),
                    Some("local-user"),
                    None,
                    protocol::AttachMode::ReadOnly,
                    protocol::PresenceKind::Joined,
                    Some("pane-1"),
                ),
                "missing presence display_name",
            ),
            (
                presence_update_frame_with_fields(
                    Some("local-actor"),
                    Some("local-user"),
                    Some(""),
                    protocol::AttachMode::ReadOnly,
                    protocol::PresenceKind::Joined,
                    Some("pane-1"),
                ),
                "empty presence display_name",
            ),
        ] {
            let err = presence_from_frame(&frame)
                .expect_err("presence update identity fields should be required");
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn rejects_presence_update_with_empty_focused_pane_id() {
        let frame = presence_update_frame_with_fields(
            Some("local-actor"),
            Some("local-user"),
            Some("local"),
            protocol::AttachMode::ReadOnly,
            protocol::PresenceKind::Joined,
            Some(""),
        );
        let err = presence_from_frame(&frame)
            .expect_err("empty focused pane ID should be rejected when present");

        assert!(err.to_string().contains("empty presence focused_pane_id"));
    }

    #[test]
    fn rejects_unknown_control_plane_enums_from_frames() {
        let workspace_err = workspace_summary_from_frame(&workspace_tree_frame_with_resize_policy(
            protocol::ResizePolicy(99),
        ))
        .expect_err("workspace with unknown resize policy should be rejected");
        assert!(workspace_err.to_string().contains("unknown resize policy"));

        let pane_kind_err = workspace_summary_from_frame(&workspace_tree_frame_with_pane_enums(
            protocol::PaneKind(99),
            protocol::SplitAxis::None,
            protocol::ResizePolicy::Fixed,
            None,
        ))
        .expect_err("workspace with unknown pane kind should be rejected");
        assert!(pane_kind_err.to_string().contains("unknown pane kind"));

        let split_axis_err = workspace_summary_from_frame(&workspace_tree_frame_with_pane_enums(
            protocol::PaneKind::Pty,
            protocol::SplitAxis(99),
            protocol::ResizePolicy::Fixed,
            None,
        ))
        .expect_err("workspace with unknown split axis should be rejected");
        assert!(split_axis_err.to_string().contains("unknown split axis"));

        let child_err = workspace_summary_from_frame(&workspace_tree_frame_with_pane_enums(
            protocol::PaneKind::Pty,
            protocol::SplitAxis::Horizontal,
            protocol::ResizePolicy::Fixed,
            Some((
                protocol::PaneKind(99),
                protocol::SplitAxis::None,
                protocol::ResizePolicy::Fixed,
            )),
        ))
        .expect_err("workspace with unknown child pane kind should be rejected");
        assert!(child_err.to_string().contains("unknown pane kind"));

        let resize_err =
            resize_intent_from_frame(&resize_intent_frame_with_reason(protocol::ResizeReason(99)))
                .expect_err("resize intent with unknown reason should be rejected");
        assert!(resize_err.to_string().contains("unknown resize reason"));

        let presence_err = presence_from_frame(&presence_update_frame_with_mode_and_kind(
            protocol::AttachMode::ReadOnly,
            protocol::PresenceKind(99),
        ))
        .expect_err("presence update with unknown kind should be rejected");
        assert!(presence_err.to_string().contains("unknown presence kind"));

        let status_err = attach_status_from_frame(&attach_status_frame_with_surface_state(
            protocol::AttachSurfaceState(99),
        ))
        .expect_err("attach status with unknown surface state should be rejected");
        assert!(
            status_err
                .to_string()
                .contains("unknown attach surface state")
        );

        let error_err = error_summary_from_frame(&error_frame_with_code(protocol::ErrorCode(99)))
            .expect_err("error frame with unknown code should be rejected");
        assert!(error_err.to_string().contains("unknown error code"));
    }

    #[test]
    fn decodes_error_from_server_frame() {
        let frame = Session::initial().error_frame(
            "local-client",
            3,
            protocol::ErrorCode::Unknown,
            "unsupported input",
            ErrorRetryability::NotRetryable,
        );
        let error = error_summary_from_frame(&frame).expect("error summary");

        assert_eq!(
            error,
            ErrorSummary {
                code: protocol::ErrorCode::Unknown,
                message: "unsupported input".to_owned(),
                retryable: false,
                pane_id: None,
                input_seq: 0,
            }
        );
    }

    #[test]
    fn rejects_error_frame_with_missing_or_empty_message() {
        for (frame, expected) in [
            (
                error_frame_with_fields(protocol::ErrorCode::Unknown, None, None),
                "missing error message",
            ),
            (
                error_frame_with_fields(protocol::ErrorCode::Unknown, Some(""), None),
                "empty error message",
            ),
        ] {
            let err = error_summary_from_frame(&frame)
                .expect_err("error frame message should be required");
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn rejects_error_frame_with_empty_pane_id() {
        let frame = error_frame_with_fields(
            protocol::ErrorCode::PaneNotFound,
            Some("pane missing"),
            Some(""),
        );
        let err =
            error_summary_from_frame(&frame).expect_err("empty error pane ID should be rejected");

        assert!(err.to_string().contains("empty error pane_id"));
    }

    #[test]
    fn live_surface_read_reports_closed_on_mid_connection_disconnect() {
        let (mut client, server) = UnixStream::pair().expect("socket pair");
        drop(server);

        assert_eq!(
            read_live_surface_update_from_stream(&mut client).expect("read closed"),
            LiveSurfaceRead::Closed
        );
    }

    #[test]
    fn client_frame_sequence_increments_envelope_and_input_sequences() {
        let (mut client, mut server) = UnixStream::pair().expect("socket pair");
        let mut sequence = ClientFrameSequence::default();

        send_key_input_with_sequence(&mut client, &mut sequence, "pane-1", "a").expect("send key");
        send_resize_intent_with_sequence(&mut client, &mut sequence, "pane-1", 100, 40)
            .expect("send resize");
        send_mouse_input_with_sequence(
            &mut client,
            &mut sequence,
            "pane-1",
            AttachMouseInput {
                row: 1,
                col: 2,
                pixel_x: None,
                pixel_y: None,
                button: protocol::MouseButton::Left,
                action: protocol::MouseAction::Press,
                modifiers: 0,
            },
        )
        .expect("send mouse");

        let key_frame = wire::read_default_frame(&mut server).expect("read key");
        let resize_frame = wire::read_default_frame(&mut server).expect("read resize");
        let mouse_frame = wire::read_default_frame(&mut server).expect("read mouse");

        let key_envelope =
            protocol::size_prefixed_root_as_envelope(&key_frame).expect("key envelope");
        let resize_envelope =
            protocol::size_prefixed_root_as_envelope(&resize_frame).expect("resize envelope");
        let mouse_envelope =
            protocol::size_prefixed_root_as_envelope(&mouse_frame).expect("mouse envelope");
        assert_eq!(key_envelope.seq(), 1);
        assert_eq!(resize_envelope.seq(), 2);
        assert_eq!(mouse_envelope.seq(), 3);

        let key = input_summary_from_frame(&key_frame).expect("key input");
        let resize = resize_intent_from_frame(&resize_frame).expect("resize intent");
        let mouse = input_summary_from_frame(&mouse_frame).expect("mouse input");
        assert_eq!(key.input_seq, 1);
        assert_eq!(resize.cols, 100);
        assert_eq!(resize.reason, protocol::ResizeReason::FrontendViewport);
        assert_eq!(mouse.input_seq, 2);
    }

    #[test]
    fn client_frame_sequence_keeps_input_seq_monotonic_across_live_control_frames() {
        let (mut client, mut server) = UnixStream::pair().expect("socket pair");
        let mut sequence = ClientFrameSequence::default();

        send_key_input_with_sequence(&mut client, &mut sequence, "pane-1", "first")
            .expect("send first key");
        send_scrollback_fetch_with_sequence(
            &mut client,
            &mut sequence,
            "pane-1",
            scrollback_range(1, 8),
        )
        .expect("send scrollback fetch");
        send_resize_intent_with_sequence(&mut client, &mut sequence, "pane-1", 120, 50)
            .expect("send resize");
        send_paste_input_with_sequence(&mut client, &mut sequence, "pane-1", "second")
            .expect("send paste");

        let first_frame = wire::read_default_frame(&mut server).expect("read first input");
        let fetch_frame = wire::read_default_frame(&mut server).expect("read fetch");
        let resize_frame = wire::read_default_frame(&mut server).expect("read resize");
        let second_frame = wire::read_default_frame(&mut server).expect("read second input");

        let first_envelope =
            protocol::size_prefixed_root_as_envelope(&first_frame).expect("first envelope");
        let fetch_envelope =
            protocol::size_prefixed_root_as_envelope(&fetch_frame).expect("fetch envelope");
        let resize_envelope =
            protocol::size_prefixed_root_as_envelope(&resize_frame).expect("resize envelope");
        let second_envelope =
            protocol::size_prefixed_root_as_envelope(&second_frame).expect("second envelope");
        assert_eq!(first_envelope.seq(), 1);
        assert_eq!(fetch_envelope.seq(), 2);
        assert_eq!(resize_envelope.seq(), 3);
        assert_eq!(second_envelope.seq(), 4);

        let first = input_summary_from_frame(&first_frame).expect("first input");
        let fetch = scrollback_fetch_from_frame(&fetch_frame).expect("scrollback fetch");
        let resize = resize_intent_from_frame(&resize_frame).expect("resize intent");
        let second = input_summary_from_frame(&second_frame).expect("second input");
        assert_eq!(first.input_seq, 1);
        assert_eq!(fetch.start_line, 1);
        assert_eq!(fetch.line_count, 8);
        assert_eq!(resize.cols, 120);
        assert_eq!(resize.rows, 50);
        assert_eq!(second.input_seq, 2);
    }

    #[test]
    fn resize_intent_sequence_can_mark_user_command_reason() {
        let (mut client, mut server) = UnixStream::pair().expect("socket pair");
        let mut sequence = ClientFrameSequence::default();

        send_resize_intent_with_reason_and_sequence(
            &mut client,
            &mut sequence,
            "pane-1",
            100,
            40,
            protocol::ResizeReason::UserCommand,
        )
        .expect("send resize");

        let resize_frame = wire::read_default_frame(&mut server).expect("read resize");
        let resize = resize_intent_from_frame(&resize_frame).expect("resize intent");
        assert_eq!(resize.cols, 100);
        assert_eq!(resize.rows, 40);
        assert_eq!(resize.reason, protocol::ResizeReason::UserCommand);
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
                paste_text: None,
                key_name: None,
                key_modifiers: 0,
                mouse: None,
                requires_focus_reporting: false,
                requires_mouse_tracking: false,
            }
        );
    }

    #[test]
    fn decodes_named_key_input_from_client_frame() {
        let frame = Session::initial().named_key_input_frame(
            "local-client",
            3,
            "actor-1",
            "pane-1",
            2,
            "numpad-enter",
        );
        let input = input_summary_from_frame(&frame).expect("input summary");

        assert_eq!(input.pane_id, "pane-1");
        assert_eq!(input.actor_id, "actor-1");
        assert_eq!(input.input_seq, 2);
        assert_eq!(input.bytes, Vec::<u8>::new());
        assert_eq!(input.key_name.as_deref(), Some("numpad-enter"));
        assert_eq!(input.key_modifiers, 0);
        assert!(!input.requires_focus_reporting);
    }

    #[test]
    fn decodes_named_key_modifiers_from_client_frame() {
        let frame = Session::initial().named_key_input_frame_with_modifiers(
            "local-client",
            3,
            "actor-1",
            "pane-1",
            2,
            "arrow-up",
            2,
        );
        let input = input_summary_from_frame(&frame).expect("input summary");

        assert_eq!(input.bytes, Vec::<u8>::new());
        assert_eq!(input.key_name.as_deref(), Some("arrow-up"));
        assert_eq!(input.key_modifiers, 2);
    }

    #[test]
    fn rejects_input_with_unsupported_modifier_bits() {
        let key_frame = Session::initial().named_key_input_frame_with_modifiers(
            "local-client",
            3,
            "actor-1",
            "pane-1",
            2,
            "arrow-up",
            0x10,
        );
        let key_err =
            input_summary_from_frame(&key_frame).expect_err("key modifier bits should be rejected");
        assert!(
            key_err
                .to_string()
                .contains("unsupported input modifier bits")
        );

        let mouse_frame = Session::initial().mouse_input_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 3,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 2,
            },
            MouseInputSpec {
                row: 4,
                col: 5,
                pixel_x: None,
                pixel_y: None,
                button: protocol::MouseButton::Left,
                action: protocol::MouseAction::Press,
                modifiers: 0x10,
            },
        );
        let mouse_err = input_summary_from_frame(&mouse_frame)
            .expect_err("mouse modifier bits should be rejected");
        assert!(
            mouse_err
                .to_string()
                .contains("unsupported input modifier bits")
        );
    }

    #[test]
    fn rejects_input_with_missing_kind_payloads() {
        for (kind, expected) in [
            (protocol::InputKind::Key, "missing key input"),
            (protocol::InputKind::RawBytes, "missing raw input"),
            (protocol::InputKind::Paste, "missing paste input"),
            (protocol::InputKind::Focus, "missing focus input"),
            (protocol::InputKind::Mouse, "missing mouse input"),
        ] {
            let frame = input_frame_without_payload(kind);
            let err = input_summary_from_frame(&frame).expect_err("missing input payload rejected");
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?} for {kind:?}, got {err}"
            );
        }
    }

    #[test]
    fn rejects_pane_scoped_client_frames_with_missing_or_empty_ids() {
        for (frame, expected) in [
            (
                input_frame_with_ids(None, Some("actor-1")),
                "missing input pane_id",
            ),
            (
                input_frame_with_ids(Some(""), Some("actor-1")),
                "empty input pane_id",
            ),
            (
                input_frame_with_ids(Some("pane-1"), None),
                "missing input actor_id",
            ),
            (
                input_frame_with_ids(Some("pane-1"), Some("")),
                "empty input actor_id",
            ),
        ] {
            let err = input_summary_from_frame(&frame).expect_err("input ids should be required");
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }

        for (frame, expected) in [
            (
                resize_intent_frame_with_ids(None, Some("actor-1")),
                "missing resize pane_id",
            ),
            (
                resize_intent_frame_with_ids(Some(""), Some("actor-1")),
                "empty resize pane_id",
            ),
            (
                resize_intent_frame_with_ids(Some("pane-1"), None),
                "missing resize actor_id",
            ),
            (
                resize_intent_frame_with_ids(Some("pane-1"), Some("")),
                "empty resize actor_id",
            ),
        ] {
            let err = resize_intent_from_frame(&frame).expect_err("resize ids should be required");
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }

        for (frame, expected) in [
            (
                scrollback_fetch_frame_with_ids(None, Some("actor-1")),
                "missing scrollback fetch pane_id",
            ),
            (
                scrollback_fetch_frame_with_ids(Some(""), Some("actor-1")),
                "empty scrollback fetch pane_id",
            ),
            (
                scrollback_fetch_frame_with_ids(Some("pane-1"), None),
                "missing scrollback fetch actor_id",
            ),
            (
                scrollback_fetch_frame_with_ids(Some("pane-1"), Some("")),
                "empty scrollback fetch actor_id",
            ),
        ] {
            let err =
                scrollback_fetch_from_frame(&frame).expect_err("scrollback ids should be required");
            assert!(
                err.to_string().contains(expected),
                "expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn named_key_input_uses_daemon_owned_keypad_mode() {
        let mut session = Session::initial();
        let enter = InputSummary {
            pane_id: "pane-1".to_owned(),
            actor_id: "actor-1".to_owned(),
            input_seq: 1,
            text: String::new(),
            bytes: Vec::new(),
            paste_text: None,
            key_name: Some("numpad-enter".to_owned()),
            key_modifiers: 0,
            mouse: None,
            requires_focus_reporting: false,
            requires_mouse_tracking: false,
        };
        let digit = InputSummary {
            key_name: Some("numpad-7".to_owned()),
            ..enter.clone()
        };

        assert_eq!(
            enter
                .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
                .expect("normal enter"),
            b"\r"
        );
        assert_eq!(
            digit
                .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
                .expect("normal digit"),
            b"7"
        );

        session.tabs[0].root.modes.application_keypad = true;
        assert_eq!(
            enter
                .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
                .expect("application enter"),
            b"\x1bOM"
        );
        assert_eq!(
            digit
                .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
                .expect("application digit"),
            b"\x1bOw"
        );
    }

    #[test]
    fn named_key_input_uses_daemon_owned_application_cursor_mode() {
        let mut session = Session::initial();
        let arrow = InputSummary {
            pane_id: "pane-1".to_owned(),
            actor_id: "actor-1".to_owned(),
            input_seq: 1,
            text: String::new(),
            bytes: Vec::new(),
            paste_text: None,
            key_name: Some("arrow-up".to_owned()),
            key_modifiers: 0,
            mouse: None,
            requires_focus_reporting: false,
            requires_mouse_tracking: false,
        };

        assert_eq!(
            arrow
                .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
                .expect("normal arrow"),
            b"\x1b[A"
        );

        session.tabs[0].root.modes.application_cursor = true;
        assert_eq!(
            arrow
                .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
                .expect("application arrow"),
            b"\x1bOA"
        );
    }

    #[test]
    fn named_key_input_forwards_common_navigation_and_control_keys() {
        let session = Session::initial();
        let cases = [
            ("enter", b"\r".as_slice()),
            ("tab", b"\t".as_slice()),
            ("space", b" ".as_slice()),
            ("backspace", b"\x7f".as_slice()),
            ("escape", b"\x1b".as_slice()),
            ("insert", b"\x1b[2~".as_slice()),
            ("delete", b"\x1b[3~".as_slice()),
            ("home", b"\x1b[H".as_slice()),
            ("end", b"\x1b[F".as_slice()),
            ("page-up", b"\x1b[5~".as_slice()),
            ("page-down", b"\x1b[6~".as_slice()),
            ("f1", b"\x1bOP".as_slice()),
            ("f4", b"\x1bOS".as_slice()),
            ("f5", b"\x1b[15~".as_slice()),
            ("f10", b"\x1b[21~".as_slice()),
            ("f12", b"\x1b[24~".as_slice()),
        ];

        for (key_name, expected) in cases {
            let input = InputSummary {
                pane_id: "pane-1".to_owned(),
                actor_id: "actor-1".to_owned(),
                input_seq: 1,
                text: String::new(),
                bytes: Vec::new(),
                paste_text: None,
                key_name: Some(key_name.to_owned()),
                key_modifiers: 0,
                mouse: None,
                requires_focus_reporting: false,
                requires_mouse_tracking: false,
            };

            assert_eq!(
                input
                    .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
                    .expect("named key"),
                expected,
                "wrong bytes for {key_name}"
            );
        }
    }

    #[test]
    fn named_key_input_rejects_unsupported_key_names() {
        let session = Session::initial();
        let input = InputSummary {
            pane_id: "pane-1".to_owned(),
            actor_id: "actor-1".to_owned(),
            input_seq: 1,
            text: String::new(),
            bytes: Vec::new(),
            paste_text: None,
            key_name: Some("f13".to_owned()),
            key_modifiers: 0,
            mouse: None,
            requires_focus_reporting: false,
            requires_mouse_tracking: false,
        };

        let err = input
            .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
            .expect_err("unsupported key name");
        assert!(err.to_string().contains("unsupported key name: f13"));
    }

    #[test]
    fn modified_named_key_forwards_with_interim_encoder() {
        let session = Session::initial();
        let input = InputSummary {
            pane_id: "pane-1".to_owned(),
            actor_id: "actor-1".to_owned(),
            input_seq: 1,
            text: String::new(),
            bytes: Vec::new(),
            paste_text: None,
            key_name: Some("arrow-up".to_owned()),
            key_modifiers: 2,
            mouse: None,
            requires_focus_reporting: false,
            requires_mouse_tracking: false,
        };

        let bytes = input
            .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
            .expect("modified key supported");
        assert_eq!(bytes, b"\x1b[1;3A");
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
    fn decodes_plain_paste_input_from_client_frame() {
        let frame = Session::initial().paste_input_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 3,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 2,
            },
            PasteInputSpec {
                text: "hello\n",
                bracketed: false,
            },
        );
        let input = input_summary_from_frame(&frame).expect("input summary");

        assert_eq!(
            input,
            InputSummary {
                pane_id: "pane-1".to_owned(),
                actor_id: "actor-1".to_owned(),
                input_seq: 2,
                text: "hello\n".to_owned(),
                bytes: b"hello\n".to_vec(),
                paste_text: Some("hello\n".to_owned()),
                key_name: None,
                key_modifiers: 0,
                mouse: None,
                requires_focus_reporting: false,
                requires_mouse_tracking: false,
            }
        );
    }

    #[test]
    fn decodes_bracketed_paste_input_from_client_frame() {
        let frame = Session::initial().paste_input_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 3,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 2,
            },
            PasteInputSpec {
                text: "hello\n",
                bracketed: true,
            },
        );
        let input = input_summary_from_frame(&frame).expect("input summary");

        assert_eq!(input.bytes, b"hello\n".to_vec());
        assert_eq!(input.text, "hello\n");
        assert_eq!(input.paste_text.as_deref(), Some("hello\n"));
    }

    #[test]
    fn paste_input_wrapping_uses_daemon_owned_mode() {
        let mut session = Session::initial();
        let bracketed_client_frame = Session::initial().paste_input_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 3,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 2,
            },
            PasteInputSpec {
                text: "hello\n",
                bracketed: true,
            },
        );
        let input =
            input_summary_from_frame(&bracketed_client_frame).expect("bracketed input summary");

        assert_eq!(
            input
                .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
                .expect("plain paste"),
            b"hello\n"
        );

        session.tabs[0].root.modes.bracketed_paste = true;
        assert_eq!(
            input
                .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
                .expect("daemon bracketed paste"),
            b"\x1b[200~hello\n\x1b[201~"
        );
    }

    #[test]
    fn decodes_focus_input_from_client_frame() {
        let gained = Session::initial().focus_input_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 3,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 2,
            },
            FocusInputSpec { focused: true },
        );
        let gained = input_summary_from_frame(&gained).expect("focus gained summary");
        assert_eq!(gained.bytes, b"\x1b[I".to_vec());
        assert!(gained.requires_focus_reporting);

        let lost = Session::initial().focus_input_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 3,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 3,
            },
            FocusInputSpec { focused: false },
        );
        let lost = input_summary_from_frame(&lost).expect("focus lost summary");
        assert_eq!(lost.bytes, b"\x1b[O".to_vec());
        assert!(lost.requires_focus_reporting);
    }

    #[test]
    fn decodes_mouse_input_from_client_frame() {
        let frame = Session::initial().mouse_input_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 3,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 2,
            },
            MouseInputSpec {
                row: 4,
                col: 5,
                pixel_x: None,
                pixel_y: None,
                button: protocol::MouseButton::Left,
                action: protocol::MouseAction::Press,
                modifiers: 3,
            },
        );
        let input = input_summary_from_frame(&frame).expect("mouse summary");

        assert_eq!(input.pane_id, "pane-1");
        assert_eq!(input.actor_id, "actor-1");
        assert_eq!(input.input_seq, 2);
        assert_eq!(
            input.mouse,
            Some(MouseSummary {
                row: 4,
                col: 5,
                pixel_x: None,
                pixel_y: None,
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 3,
            })
        );
        assert!(input.requires_mouse_tracking);

        let frame = Session::initial().mouse_input_frame(
            InputFrameContext {
                connection_id: "conn-1",
                seq: 9,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 2,
            },
            MouseInputSpec {
                row: 4,
                col: 5,
                pixel_x: Some(33),
                pixel_y: Some(65),
                button: protocol::MouseButton::Left,
                action: protocol::MouseAction::Press,
                modifiers: 3,
            },
        );
        let input = input_summary_from_frame(&frame).expect("mouse pixel summary");
        assert_eq!(
            input.mouse,
            Some(MouseSummary {
                row: 4,
                col: 5,
                pixel_x: Some(33),
                pixel_y: Some(65),
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 3,
            })
        );
    }

    #[test]
    fn rejects_mouse_input_with_unknown_button_or_action() {
        let button_frame = Session::initial().mouse_input_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 3,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 2,
            },
            MouseInputSpec {
                row: 4,
                col: 5,
                pixel_x: None,
                pixel_y: None,
                button: protocol::MouseButton(99),
                action: protocol::MouseAction::Press,
                modifiers: 0,
            },
        );
        let err = input_summary_from_frame(&button_frame)
            .expect_err("unknown mouse button should be rejected");
        assert!(err.to_string().contains("unknown mouse button"));

        let action_frame = Session::initial().mouse_input_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 3,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 2,
            },
            MouseInputSpec {
                row: 4,
                col: 5,
                pixel_x: None,
                pixel_y: None,
                button: protocol::MouseButton::Left,
                action: protocol::MouseAction(99),
                modifiers: 0,
            },
        );
        let err = input_summary_from_frame(&action_frame)
            .expect_err("unknown mouse action should be rejected");
        assert!(err.to_string().contains("unknown mouse action"));
    }

    #[test]
    fn focus_input_forwarding_requires_daemon_owned_mode() {
        let mut session = Session::initial();
        let input = InputSummary {
            pane_id: "pane-1".to_owned(),
            actor_id: "actor-1".to_owned(),
            input_seq: 1,
            text: "\x1b[I".to_owned(),
            bytes: b"\x1b[I".to_vec(),
            paste_text: None,
            key_name: None,
            key_modifiers: 0,
            mouse: None,
            requires_focus_reporting: true,
            requires_mouse_tracking: false,
        };

        assert_eq!(
            input.forwarding_rejection(&session),
            Some(InputRejection::FocusReportingDisabled)
        );
        session.tabs[0].root.modes.focus_reporting = true;
        assert_eq!(input.forwarding_rejection(&session), None);
    }

    #[test]
    fn mouse_input_forwarding_requires_daemon_owned_mode() {
        let mut session = Session::initial();
        let input = InputSummary {
            pane_id: "pane-1".to_owned(),
            actor_id: "actor-1".to_owned(),
            input_seq: 1,
            text: String::new(),
            bytes: Vec::new(),
            paste_text: None,
            key_name: None,
            key_modifiers: 0,
            mouse: Some(MouseSummary {
                row: 0,
                col: 0,
                pixel_x: None,
                pixel_y: None,
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 0,
            }),
            requires_focus_reporting: false,
            requires_mouse_tracking: true,
        };

        assert_eq!(
            input.forwarding_rejection(&session),
            Some(InputRejection::MouseTrackingDisabled)
        );
        session.tabs[0].root.modes.mouse_tracking = true;
        assert_eq!(
            input.forwarding_rejection(&session),
            Some(InputRejection::MouseTrackingDisabled)
        );
        session.tabs[0].root.modes.mouse_tracking_mode = protocol::MouseTrackingMode::Normal;
        assert_eq!(input.forwarding_rejection(&session), None);
    }

    #[test]
    fn mouse_input_forwarding_rejects_coordinates_outside_daemon_pane_bounds() {
        let mut session = Session::initial();
        session.tabs[0].root.modes.mouse_tracking = true;
        session.tabs[0].root.modes.mouse_tracking_mode = protocol::MouseTrackingMode::Normal;
        let input = InputSummary {
            pane_id: "pane-1".to_owned(),
            actor_id: "actor-1".to_owned(),
            input_seq: 1,
            text: String::new(),
            bytes: Vec::new(),
            paste_text: None,
            key_name: None,
            key_modifiers: 0,
            mouse: Some(MouseSummary {
                row: 24,
                col: 79,
                pixel_x: None,
                pixel_y: None,
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 0,
            }),
            requires_focus_reporting: false,
            requires_mouse_tracking: true,
        };

        assert_eq!(
            input.forwarding_rejection(&session),
            Some(InputRejection::MouseCoordinatesOutOfBounds)
        );

        let input = InputSummary {
            mouse: Some(MouseSummary {
                row: 23,
                col: 80,
                pixel_x: None,
                pixel_y: None,
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 0,
            }),
            ..input
        };
        assert_eq!(
            input.forwarding_rejection(&session),
            Some(InputRejection::MouseCoordinatesOutOfBounds)
        );

        let input = InputSummary {
            mouse: Some(MouseSummary {
                row: 23,
                col: 79,
                pixel_x: None,
                pixel_y: None,
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 0,
            }),
            ..input
        };
        assert_eq!(input.forwarding_rejection(&session), None);

        let input = InputSummary {
            mouse: Some(MouseSummary {
                row: 23,
                col: 79,
                pixel_x: Some(10_000),
                pixel_y: Some(20_000),
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 0,
            }),
            ..input
        };
        assert_eq!(input.forwarding_rejection(&session), None);
    }

    #[test]
    fn mouse_input_forwarding_uses_daemon_owned_tracking_mode() {
        let press = MouseSummary {
            row: 0,
            col: 0,
            pixel_x: None,
            pixel_y: None,
            button: MouseButton::Left,
            action: MouseAction::Press,
            modifiers: 0,
        };
        let release = MouseSummary {
            action: MouseAction::Release,
            ..press
        };
        let button_motion = MouseSummary {
            action: MouseAction::Motion,
            ..press
        };
        let any_motion = MouseSummary {
            button: MouseButton::None,
            action: MouseAction::Motion,
            ..press
        };

        assert!(!mouse_input_allowed(
            protocol::MouseTrackingMode::None,
            press
        ));
        assert!(mouse_input_allowed(protocol::MouseTrackingMode::X10, press));
        assert!(!mouse_input_allowed(
            protocol::MouseTrackingMode::X10,
            release
        ));
        assert!(mouse_input_allowed(
            protocol::MouseTrackingMode::Normal,
            press
        ));
        assert!(mouse_input_allowed(
            protocol::MouseTrackingMode::Normal,
            release
        ));
        assert!(!mouse_input_allowed(
            protocol::MouseTrackingMode::Normal,
            button_motion
        ));
        assert!(mouse_input_allowed(
            protocol::MouseTrackingMode::Button,
            button_motion
        ));
        assert!(!mouse_input_allowed(
            protocol::MouseTrackingMode::Button,
            any_motion
        ));
        assert!(mouse_input_allowed(
            protocol::MouseTrackingMode::Any,
            any_motion
        ));
    }

    #[test]
    fn mouse_input_forwards_with_interim_sgr_encoder() {
        let mut session = Session::initial();
        session.tabs[0].root.modes.mouse_tracking = true;
        session.tabs[0].root.modes.mouse_tracking_mode = protocol::MouseTrackingMode::Normal;
        session.tabs[0].root.modes.mouse_format = protocol::MouseFormat::Sgr;
        let input = InputSummary {
            pane_id: "pane-1".to_owned(),
            actor_id: "actor-1".to_owned(),
            input_seq: 1,
            text: String::new(),
            bytes: Vec::new(),
            paste_text: None,
            key_name: None,
            key_modifiers: 0,
            mouse: Some(MouseSummary {
                row: 4,
                col: 5,
                pixel_x: None,
                pixel_y: None,
                button: MouseButton::WheelDown,
                action: MouseAction::Press,
                modifiers: 2,
            }),
            requires_focus_reporting: false,
            requires_mouse_tracking: true,
        };

        assert_eq!(input.forwarding_rejection(&session), None);
        assert_eq!(
            input
                .forwarded_bytes(&session, &mut PaneTerminalEngines::interim())
                .expect("mouse input"),
            b"\x1b[<81;6;5M"
        );
    }

    #[test]
    fn rejects_bracketed_paste_terminator_in_paste_text() {
        let frame = Session::initial().paste_input_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 3,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 2,
            },
            PasteInputSpec {
                text: "bad\x1b[201~paste",
                bracketed: false,
            },
        );
        let input = input_summary_from_frame(&frame).expect("input summary");
        let err = input
            .forwarded_bytes(&Session::initial(), &mut PaneTerminalEngines::interim())
            .expect_err("unsafe paste rejected");

        assert!(
            err.to_string()
                .contains("paste input contains a bracketed paste terminator"),
            "{err}"
        );
    }

    #[test]
    fn decodes_scrollback_fetch_from_client_frame() {
        let frame = Session::initial().scrollback_fetch_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 4,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 0,
            },
            scrollback_fetch_spec(1, 2, 1),
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
    fn rejects_scrollback_chunk_with_noncontiguous_public_line_numbers() {
        let frame = scrollback_chunk_with_public_lines_and_total(
            Some("pane-1"),
            RowMetadataFixture::default(),
            RunMetadataFixture::hyperlink(),
            ScrollbackPublicLinesFixture {
                row_line: 2,
                total_lines: 2,
                ..ScrollbackPublicLinesFixture::default()
            },
        );
        let err = scrollback_chunk_from_frame(&frame)
            .expect_err("noncontiguous scrollback row line should be rejected");

        assert!(
            err.to_string()
                .contains("does not match expected public line 1"),
            "{err}"
        );
    }

    #[test]
    fn rejects_scrollback_chunk_rows_beyond_total_lines() {
        let frame = scrollback_chunk_with_public_lines_and_total(
            Some("pane-1"),
            RowMetadataFixture::default(),
            RunMetadataFixture::hyperlink(),
            ScrollbackPublicLinesFixture {
                start_line: 2,
                row_line: 2,
                ..ScrollbackPublicLinesFixture::default()
            },
        );
        let err = scrollback_chunk_from_frame(&frame)
            .expect_err("scrollback row beyond total_lines should be rejected");

        assert!(err.to_string().contains("exceeds total_lines 1"), "{err}");
    }

    #[test]
    fn rejects_scrollback_fetch_with_zero_range_from_frame() {
        for (frame, expected) in [
            (
                scrollback_fetch_frame_with_ids_and_range(Some("pane-1"), Some("actor-1"), 0, 2),
                "scrollback fetch start_line must be 1-based",
            ),
            (
                scrollback_fetch_frame_with_ids_and_range(Some("pane-1"), Some("actor-1"), 1, 0),
                "scrollback fetch line_count must be nonzero",
            ),
        ] {
            let err = scrollback_fetch_from_frame(&frame)
                .expect_err("zero scrollback range should be rejected");
            assert!(err.to_string().contains(expected), "{err}");
        }
    }

    #[test]
    fn default_scrollback_fetch_uses_no_version_precondition() {
        let (mut client, mut server) = UnixStream::pair().expect("socket pair");

        send_scrollback_fetch(&mut client, "pane-1", scrollback_range(1, 2))
            .expect("send scrollback fetch");
        let frame = wire::read_default_frame(&mut server).expect("read scrollback fetch");
        let fetch = scrollback_fetch_from_frame(&frame).expect("scrollback fetch");

        assert_eq!(fetch.known_scrollback_version, 0);
    }

    #[test]
    fn decodes_scrollback_chunk_from_server_frame() {
        let mut session = Session::initial();
        session.tabs[0].root.styles.push(PaneStyle {
            fg_rgba: 0xff00_0000,
            bg_rgba: 0x0000_00ff,
            underline_rgba: 0,
            flags: 1,
        });
        session.tabs[0].root.scrollback_lines[1] = "styled字".to_owned();
        session.tabs[0].root.scrollback_row_runs[1] = vec![
            CellRun {
                text: "styled".to_owned(),
                cell_widths: vec![1, 1, 1, 1, 1, 1],
                style_id: 1,
                flags: CELL_RUN_FLAG_HYPERLINK_PRESENT,
                hyperlink_id: 0,
                semantic_content: protocol::CellSemanticContent::Prompt,
            },
            CellRun {
                text: "字".to_owned(),
                cell_widths: vec![2],
                style_id: 0,
                flags: 0,
                hyperlink_id: 0,
                semantic_content: protocol::CellSemanticContent::Input,
            },
        ];
        let frame = session.scrollback_chunk_frame("local-client", 4, 2, 2);
        let chunk = scrollback_chunk_from_frame(&frame).expect("scrollback chunk");

        assert_eq!(chunk.pane_id, "pane-1");
        assert_eq!(chunk.scrollback_version, 1);
        assert_eq!(chunk.start_line, 2);
        assert_eq!(chunk.total_lines, 3);
        assert_eq!(chunk.styles.len(), 2);
        assert_eq!(chunk.styles[1].fg_rgba, 0xff00_0000);
        let styled_runs = vec![
            CellRunSummary {
                text: "styled".to_owned(),
                cell_widths: vec![1, 1, 1, 1, 1, 1],
                style_id: 1,
                flags: CELL_RUN_FLAG_HYPERLINK_PRESENT,
                hyperlink_id: 0,
                semantic_content: protocol::CellSemanticContent::Prompt,
            },
            CellRunSummary {
                text: "字".to_owned(),
                cell_widths: vec![2],
                style_id: 0,
                flags: 0,
                hyperlink_id: 0,
                semantic_content: protocol::CellSemanticContent::Input,
            },
        ];
        assert_eq!(
            chunk.lines,
            vec![
                ScrollbackLine {
                    line: 2,
                    text: "styled字".to_owned(),
                    dirty_hash: stable_test_row_hash("styled字"),
                    row_state_hash: test_row_state_hash(
                        &styled_runs,
                        TestRowStateMetadata {
                            semantic_prompt: protocol::RowSemanticPrompt::None,
                            dirty: false,
                            kitty_virtual_placeholder: false,
                        },
                    ),
                    runs: styled_runs,
                    semantic_prompt: protocol::RowSemanticPrompt::None,
                    dirty: false,
                    kitty_virtual_placeholder: false,
                },
                scrollback_line(3, "server-owned terminal state"),
            ]
        );
        assert_eq!(chunk.lines[0].runs.len(), 2);
        assert_eq!(chunk.lines[0].runs[0].style_id, 1);
        assert_eq!(
            chunk.lines[0].runs[0].flags,
            CELL_RUN_FLAG_HYPERLINK_PRESENT
        );
        assert_eq!(chunk.lines[0].runs[1].cell_widths, vec![2]);
        assert_eq!(chunk.lines[1].runs[0].text, "server-owned terminal state");
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

    #[derive(Debug, Default)]
    struct FailingWriteHost {
        running: bool,
    }

    impl ProcessHost for FailingWriteHost {
        fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
            self.running = true;
            Ok(PaneProcess {
                pane_id: pane_id.to_owned(),
                host_id: spec.id.clone(),
                status: ProcessStatus::Running,
            })
        }

        fn write_input(&mut self, pane_id: &str, _bytes: &[u8]) -> Result<(), HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
            Err(HostError::Io {
                pane_id: pane_id.to_owned(),
                operation: "write_input".to_owned(),
                message: "simulated write failure".to_owned(),
            })
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
                host_id: "failing-write".to_owned(),
                status: ProcessStatus::Exited,
            })
        }
    }

    impl ProcessOutput for FailingWriteHost {
        fn try_read_output(
            &mut self,
            pane_id: &str,
            _bytes: &mut [u8],
        ) -> Result<usize, HostError> {
            if self.running {
                Ok(0)
            } else {
                Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                })
            }
        }
    }

    #[derive(Debug, Default)]
    struct FailingResizeHost {
        running: bool,
    }

    impl ProcessHost for FailingResizeHost {
        fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
            self.running = true;
            Ok(PaneProcess {
                pane_id: pane_id.to_owned(),
                host_id: spec.id.clone(),
                status: ProcessStatus::Running,
            })
        }

        fn write_input(&mut self, pane_id: &str, _bytes: &[u8]) -> Result<(), HostError> {
            if self.running {
                Ok(())
            } else {
                Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                })
            }
        }

        fn resize_pane(&mut self, pane_id: &str, _cols: u32, _rows: u32) -> Result<(), HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
            Err(HostError::Io {
                pane_id: pane_id.to_owned(),
                operation: "resize_pane".to_owned(),
                message: "simulated resize failure".to_owned(),
            })
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
                host_id: "failing-resize".to_owned(),
                status: ProcessStatus::Exited,
            })
        }
    }

    impl ProcessOutput for FailingResizeHost {
        fn try_read_output(
            &mut self,
            pane_id: &str,
            _bytes: &mut [u8],
        ) -> Result<usize, HostError> {
            if self.running {
                Ok(0)
            } else {
                Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                })
            }
        }
    }

    #[derive(Debug, Default)]
    struct FailingReadHost {
        running: bool,
        fail_reads: bool,
    }

    impl ProcessHost for FailingReadHost {
        fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
            self.running = true;
            self.fail_reads = true;
            Ok(PaneProcess {
                pane_id: pane_id.to_owned(),
                host_id: spec.id.clone(),
                status: ProcessStatus::Running,
            })
        }

        fn write_input(&mut self, pane_id: &str, _bytes: &[u8]) -> Result<(), HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
            self.fail_reads = true;
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
                host_id: "failing-read".to_owned(),
                status: ProcessStatus::Exited,
            })
        }
    }

    impl ProcessOutput for FailingReadHost {
        fn try_read_output(
            &mut self,
            pane_id: &str,
            _bytes: &mut [u8],
        ) -> Result<usize, HostError> {
            if !self.running {
                return Err(HostError::NotRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
            if self.fail_reads {
                return Err(HostError::Io {
                    pane_id: pane_id.to_owned(),
                    operation: "try_read_output".to_owned(),
                    message: "simulated read failure".to_owned(),
                });
            }
            Ok(0)
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
