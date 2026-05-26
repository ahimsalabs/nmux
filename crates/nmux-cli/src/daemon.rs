use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::thread;
use std::time::{Duration, Instant};

use crate::error::ServeError;
use crate::local;
use clap::{ArgAction, Parser, ValueEnum};
use nmux_core::host::{
    CommandSpec, HostError, HostKind, HostSpec, LocalPtyHost, PaneProcess, ProcessHost,
    ProcessOutput,
};
use nmux_core::session::{Session, SessionActor, SessionCore, SessionEvent, SessionRegistry};
use nmux_core::terminal::TerminalEngineKind;
use nmux_proto::{protocol, wire};

const RESIZE_POLICY_NAMES: &[&str] = &["fixed", "leader", "active-client", "manual"];
const SPLIT_AXIS_NAMES: &[&str] = &["horizontal", "vertical"];
const HOST_KIND_NAMES: &[&str] = &["local", "sandbox", "container"];
const TERMINAL_ENGINE_NAMES: &[&str] = &["interim", "libghostty-vt"];
const DAEMON_TRACE_RING_CAP: usize = 1024;

pub fn run_from_iter<I, S>(argv: I) -> Result<(), Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let args = args_from_iter(argv)?;
    if args.help {
        print!("{}", usage());
        return Ok(());
    }

    if args.version || args.version_json {
        let build = local::BuildInfo::current();
        if args.version_json {
            println!("{}", local::version_json("nmux", build));
        } else {
            println!("{}", build.version_line("nmux"));
        }
        return Ok(());
    }

    if args.list_daemon_choices_json {
        println!("{}", format_daemon_choices_json());
        return Ok(());
    }

    if args.print_socket || args.print_socket_json {
        if args.print_socket_json {
            println!(
                "{}",
                local::socket_path_json(&args.socket_path, args.socket_source)
            );
        } else {
            println!("{}", args.socket_path.display());
        }
        return Ok(());
    }

    let listener = if let Some(addr) = args.tcp_listen.as_deref() {
        let listener = match local::bind_tcp_listener(addr) {
            Ok(listener) => listener,
            Err(err) => {
                report_ready_json_error(&args, &err)?;
                return Err(err.into());
            }
        };
        eprintln!("nmux daemon: listening on tcp://{addr}");
        DaemonListener::Tcp(listener)
    } else {
        let listener = match local::bind_listener(&args.socket_path) {
            Ok(listener) => listener,
            Err(err) => {
                report_ready_json_error(&args, &err)?;
                return Err(err.into());
            }
        };
        let cleanup = SocketCleanup::new(args.socket_path.clone());
        eprintln!("nmux daemon: listening on {}", args.socket_path.display());
        DaemonListener::Unix {
            listener,
            _cleanup: cleanup,
        }
    };
    let ready_json = if args.ready_json {
        Some(format_ready_json(&args))
    } else {
        None
    };

    let mut pty_host = LocalPtyHost::default();
    let session_id = args.session_id.clone();
    let session_actor =
        match create_daemon_session_actor(&args, &session_id, None, &mut pty_host) {
            Ok(actor) => actor,
            Err(err) => {
                report_ready_json_error(&args, err.as_ref())?;
                return Err(err);
            }
        };
    let mut session_registry = SessionRegistry::new();
    if !session_registry.insert(session_actor) {
        return Err(format!("duplicate daemon session id {session_id}").into());
    }
    let session_actor = session_registry
        .get_mut(&session_id)
        .ok_or_else(|| format!("daemon session {session_id} missing from registry"))?;
    if let Some(ready_json) = ready_json {
        println!("{ready_json}");
        io::stdout().flush()?;
    }

    if args.live || args.live_forever || args.live_cycles.is_some() || args.live_clients.is_some() {
        let config = if args.live_forever {
            local::ServeConfig::live_forever()
        } else {
            let cycles = args.live_cycles.unwrap_or(usize::MAX);
            let clients = args.live_clients.unwrap_or(1);
            local::ServeConfig::live(clients, cycles)
        };
        let config = config.terminal_engine_kind(args.terminal_engine_kind);
        let serve_result = {
            let mut scoped_host = ScopedLocalPtyHost::new(&session_id, &mut pty_host);
            match &listener {
                DaemonListener::Unix { listener, .. } => {
                    config.serve_with_session_core(listener, session_actor, &mut scoped_host)
                }
                DaemonListener::Tcp(listener) => {
                    serve_tcp(&config, listener, &args, session_actor, &mut scoped_host)
                }
            }
        };
        let stop_result = {
            let mut scoped_host = ScopedLocalPtyHost::new(&session_id, &mut pty_host);
            stop_panes(&mut scoped_host, &session_actor.session().leaf_pane_ids())
        };
        if let Err(err) = serve_result
            && !local::is_session_shutdown(&err)
        {
            return Err(err.into());
        }
        stop_result?;
        return Ok(());
    }

    if args.one_shot {
        let config = local::ServeConfig::one().terminal_engine_kind(args.terminal_engine_kind);
        let serve_result = {
            let mut scoped_host = ScopedLocalPtyHost::new(&session_id, &mut pty_host);
            match &listener {
                DaemonListener::Unix { listener, .. } => {
                    config.serve_with_session_core(listener, session_actor, &mut scoped_host)
                }
                DaemonListener::Tcp(listener) => {
                    serve_tcp(&config, listener, &args, session_actor, &mut scoped_host)
                }
            }
        };
        let stop_result = {
            let mut scoped_host = ScopedLocalPtyHost::new(&session_id, &mut pty_host);
            stop_panes(&mut scoped_host, &session_actor.session().leaf_pane_ids())
        };
        if let Err(err) = serve_result
            && !local::is_session_shutdown(&err)
        {
            return Err(err.into());
        }
        stop_result?;
        return Ok(());
    }

    let default_config = local::ServeConfig::one().terminal_engine_kind(args.terminal_engine_kind);
    loop {
        let serve_result = match &listener {
            DaemonListener::Unix { listener, .. } => serve_unix_registry_once(
                &default_config,
                listener,
                &args,
                &mut session_registry,
                &session_id,
                &mut pty_host,
            ),
            DaemonListener::Tcp(listener) => {
                let session_actor = session_registry
                    .get_mut(&session_id)
                    .ok_or_else(|| format!("daemon session {session_id} missing from registry"))?;
                let mut scoped_host = ScopedLocalPtyHost::new(&session_id, &mut pty_host);
                serve_tcp(
                    &default_config,
                    listener,
                    &args,
                    session_actor,
                    &mut scoped_host,
                )
            }
        };
        if let Err(err) = serve_result {
            if local::is_session_shutdown(&err) {
                if let Some(session_actor) = session_registry.get(&session_id) {
                    let mut scoped_host = ScopedLocalPtyHost::new(&session_id, &mut pty_host);
                    stop_panes(&mut scoped_host, &session_actor.session().leaf_pane_ids())?;
                }
                return Ok(());
            }
            return Err(err.into());
        }
    }
}

fn serve_tcp(
    config: &local::ServeConfig,
    listener: &std::net::TcpListener,
    args: &Args,
    session_actor: &mut SessionActor,
    host: &mut (impl ProcessHost + ProcessOutput),
) -> Result<(), ServeError> {
    let token = args
        .tcp_token
        .as_deref()
        .ok_or("--listen requires --token, --tcp-token, or NMUX_TOKEN")?;
    let mut accepted_connections = 0_usize;
    while config.connection_limit.accepts_more(accepted_connections) {
        let stream = local::accept_authenticated_tcp_client(listener, token)?;
        accepted_connections = accepted_connections.saturating_add(1);
        config.serve_stream_with_session_core(stream, session_actor, host)?;
    }
    Ok(())
}

fn serve_unix_registry_once(
    config: &local::ServeConfig,
    listener: &std::os::unix::net::UnixListener,
    args: &Args,
    registry: &mut SessionRegistry,
    default_session_id: &str,
    pty_host: &mut LocalPtyHost,
) -> Result<(), ServeError> {
    let (mut stream, _) = listener.accept()?;
    let initial = local::read_client_initial_frame(&mut stream)?;
    if let local::ClientInitialFrame::Control(command) = &initial
        && command.kind == protocol::ControlCommandKind::SessionNew
    {
        return serve_session_new_command(stream, command, args, registry, pty_host);
    }

    let target_session_id = initial_target_session_id(&initial)
        .unwrap_or(default_session_id)
        .to_owned();
    let Some(actor) = registry.get_mut(&target_session_id) else {
        let fallback = registry
            .get(default_session_id)
            .ok_or_else(|| format!("daemon session {default_session_id} missing from registry"))?;
        let mut seq = 1;
        local::write_protocol_error(
            &mut stream,
            fallback.session(),
            &mut seq,
            protocol::ErrorCode::SessionNotFound,
            &format!("session not found: {}", target_session_id),
            None,
            0,
        )?;
        return Ok(());
    };
    let mut scoped_host = ScopedLocalPtyHost::new(&target_session_id, pty_host);
    config.serve_stream_with_initial_frame_and_session_core(
        stream,
        initial,
        actor,
        &mut scoped_host,
    )
}

fn serve_session_new_command(
    mut stream: std::os::unix::net::UnixStream,
    command: &local::ControlCommandSummary,
    args: &Args,
    registry: &mut SessionRegistry,
    pty_host: &mut LocalPtyHost,
) -> Result<(), ServeError> {
    let Some(session_id) = command.session_id.as_deref().filter(|value| !value.is_empty()) else {
        let fallback = registry
            .get(&args.session_id)
            .ok_or_else(|| format!("daemon session {} missing from registry", args.session_id))?;
        let mut seq = 1;
        local::write_protocol_error(
            &mut stream,
            fallback.session(),
            &mut seq,
            protocol::ErrorCode::SessionNotFound,
            "session new requires a non-empty session id",
            None,
            command.command_seq,
        )?;
        return Ok(());
    };
    if registry.get(session_id).is_some() {
        let fallback = registry
            .get(&args.session_id)
            .ok_or_else(|| format!("daemon session {} missing from registry", args.session_id))?;
        let mut seq = 1;
        local::write_protocol_error(
            &mut stream,
            fallback.session(),
            &mut seq,
            protocol::ErrorCode::Unknown,
            &format!("session already exists: {session_id}"),
            None,
            command.command_seq,
        )?;
        return Ok(());
    }

    let actor = create_daemon_session_actor(args, session_id, command.title.as_deref(), pty_host)
        .map_err(ServeError::from)?;
    let workspace_frame = actor.session().workspace_tree_frame("local-client", 1);
    if !registry.insert(actor) {
        return Err(format!("duplicate daemon session id {session_id}").into());
    }
    wire::write_default_frame(&mut stream, &workspace_frame)?;
    Ok(())
}

fn initial_target_session_id(initial: &local::ClientInitialFrame) -> Option<&str> {
    match initial {
        local::ClientInitialFrame::Attach {
            target_session_id, ..
        } => target_session_id.as_deref(),
        local::ClientInitialFrame::Control(command) => command.session_id.as_deref(),
        local::ClientInitialFrame::HealthProbe(_) => None,
    }
}

fn create_daemon_session_actor(
    args: &Args,
    session_id: &str,
    title: Option<&str>,
    pty_host: &mut LocalPtyHost,
) -> Result<SessionActor, Box<dyn std::error::Error>> {
    let session_core = build_daemon_session_core(args, session_id, title)?;
    let pane_ids = session_core.session().leaf_pane_ids();
    {
        let mut scoped_host = ScopedLocalPtyHost::new(session_id, pty_host);
        for pane_id in &pane_ids {
            let host_spec = session_core
                .session()
                .pane_host(pane_id)
                .ok_or_else(|| format!("pane {pane_id} missing host"))?
                .clone();
            scoped_host.start_pane(pane_id, &host_spec)?;
        }
    }
    let mut actor = SessionActor::new(session_core, DAEMON_TRACE_RING_CAP);
    {
        let mut scoped_host = ScopedLocalPtyHost::new(session_id, pty_host);
        wait_for_panes_output(&mut actor, &mut scoped_host, &pane_ids)?;
    }
    Ok(actor)
}

fn build_daemon_session_core(
    args: &Args,
    session_id: &str,
    title: Option<&str>,
) -> Result<SessionCore, Box<dyn std::error::Error>> {
    let mut session_core =
        SessionCore::with_terminal_engine_kind(Session::initial(), args.terminal_engine_kind);
    session_core.session_mut().id = session_id.to_owned();
    if let Some(title) = title {
        session_core.session_mut().tabs[0].title = title.to_owned();
    }
    if let Some(command) = args.command.as_deref() {
        session_core.session_mut().tabs[0].root.host.command =
            CommandSpec::new("sh").with_args(["-lc", command]);
    }
    if let Some((cols, rows)) = args.initial_size {
        session_core.session_mut().tabs[0].root.cols = cols;
        session_core.session_mut().tabs[0].root.rows = rows;
        session_core.session_mut().tabs[0]
            .root
            .host
            .command
            .initial_size = Some((cols, rows));
    }
    if let Some(working_dir) = args.working_dir.as_ref() {
        session_core.session_mut().tabs[0]
            .root
            .host
            .command
            .working_dir = Some(working_dir.clone());
    }
    if !args.env.is_empty() {
        session_core.session_mut().tabs[0]
            .root
            .host
            .command
            .env
            .extend(args.env.iter().cloned());
    }
    apply_initial_host_kind(&mut session_core.session_mut().tabs[0].root.host, args)?;
    for tab_number in 2..=args.initial_tabs {
        let pane_id = format!("tab-{tab_number}-pane-1");
        let tab_id = format!("tab-{tab_number}");
        let host = session_core
            .session()
            .pane_host("pane-1")
            .ok_or("initial pane missing host")?
            .clone();
        if session_core
            .apply(SessionEvent::AddTab {
                tab_id: tab_id.clone(),
                title: tab_id,
                pane_id,
                host,
            })
            .is_empty()
        {
            return Err(format!("failed to create initial tab {tab_number}").into());
        }
    }
    if let Some(tab_id) = args.active_tab_id.as_deref()
        && session_core
            .apply(SessionEvent::SwitchTab {
                tab_id: tab_id.to_owned(),
            })
            .is_empty()
        && session_core.session().active_tab_id != tab_id
    {
        return Err(format!("failed to switch to initial tab {tab_id}").into());
    }
    if let Some(axis) = args.initial_split {
        let active_pane_id = session_core
            .session()
            .active_pane_id()
            .ok_or("active pane missing before initial split")?
            .to_owned();
        let host = session_core
            .session()
            .pane_host(&active_pane_id)
            .ok_or("active pane missing host")?
            .clone();
        if session_core
            .apply(SessionEvent::SplitPane {
                pane_id: active_pane_id,
                axis,
                new_pane_id: "pane-2".to_owned(),
                new_host: host,
            })
            .is_empty()
        {
            return Err("failed to create initial split pane".into());
        }
    }
    let pane_ids = session_core.session().leaf_pane_ids();
    let inherited_origin = inherited_nmux_origin();
    for pane_id in &pane_ids {
        session_core.apply(SessionEvent::SetPaneResizePolicy {
            pane_id: pane_id.clone(),
            policy: args.resize_policy,
        });
        session_core.session_mut().set_pane_nmux_environment(
            pane_id,
            args.transport_endpoint(),
            inherited_origin.as_deref(),
        );
    }
    Ok(session_core)
}

fn inherited_nmux_origin() -> Option<String> {
    if std::env::var("NMUX").ok().as_deref() != Some("1") {
        return None;
    }

    std::env::var("NMUX_ORIGIN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

struct SocketCleanup {
    path: PathBuf,
    identity: Option<local::SocketIdentity>,
}

enum DaemonListener {
    Unix {
        listener: std::os::unix::net::UnixListener,
        _cleanup: SocketCleanup,
    },
    Tcp(std::net::TcpListener),
}

struct ScopedLocalPtyHost<'a> {
    session_id: &'a str,
    inner: &'a mut LocalPtyHost,
}

impl<'a> ScopedLocalPtyHost<'a> {
    fn new(session_id: &'a str, inner: &'a mut LocalPtyHost) -> Self {
        Self { session_id, inner }
    }

    fn host_pane_id(&self, pane_id: &str) -> String {
        format!("{}:{pane_id}", self.session_id)
    }

    fn client_pane_id(&self, host_pane_id: &str) -> String {
        host_pane_id
            .strip_prefix(self.session_id)
            .and_then(|suffix| suffix.strip_prefix(':'))
            .unwrap_or(host_pane_id)
            .to_owned()
    }

    fn client_process(&self, mut process: PaneProcess) -> PaneProcess {
        process.pane_id = self.client_pane_id(&process.pane_id);
        process
    }

    fn client_error(&self, error: HostError) -> HostError {
        match error {
            HostError::UnsupportedHostKind { .. } => error,
            HostError::UnsupportedOperation { pane_id, operation } => {
                HostError::UnsupportedOperation {
                    pane_id: self.client_pane_id(&pane_id),
                    operation,
                }
            }
            HostError::AlreadyRunning { pane_id } => HostError::AlreadyRunning {
                pane_id: self.client_pane_id(&pane_id),
            },
            HostError::NotRunning { pane_id } => HostError::NotRunning {
                pane_id: self.client_pane_id(&pane_id),
            },
            HostError::Io {
                pane_id,
                operation,
                message,
            } => HostError::Io {
                pane_id: self.client_pane_id(&pane_id),
                operation,
                message,
            },
        }
    }
}

impl ProcessHost for ScopedLocalPtyHost<'_> {
    fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
        let host_pane_id = self.host_pane_id(pane_id);
        self.inner
            .start_pane(&host_pane_id, spec)
            .map(|process| self.client_process(process))
            .map_err(|error| self.client_error(error))
    }

    fn check_pane(&mut self, pane_id: &str) -> Result<(), HostError> {
        let host_pane_id = self.host_pane_id(pane_id);
        self.inner
            .check_pane(&host_pane_id)
            .map_err(|error| self.client_error(error))
    }

    fn write_input(&mut self, pane_id: &str, bytes: &[u8]) -> Result<(), HostError> {
        let host_pane_id = self.host_pane_id(pane_id);
        self.inner
            .write_input(&host_pane_id, bytes)
            .map_err(|error| self.client_error(error))
    }

    fn resize_pane(&mut self, pane_id: &str, cols: u32, rows: u32) -> Result<(), HostError> {
        let host_pane_id = self.host_pane_id(pane_id);
        self.inner
            .resize_pane(&host_pane_id, cols, rows)
            .map_err(|error| self.client_error(error))
    }

    fn stop_pane(&mut self, pane_id: &str) -> Result<PaneProcess, HostError> {
        let host_pane_id = self.host_pane_id(pane_id);
        self.inner
            .stop_pane(&host_pane_id)
            .map(|process| self.client_process(process))
            .map_err(|error| self.client_error(error))
    }
}

impl ProcessOutput for ScopedLocalPtyHost<'_> {
    fn try_read_output(&mut self, pane_id: &str, bytes: &mut [u8]) -> Result<usize, HostError> {
        let host_pane_id = self.host_pane_id(pane_id);
        self.inner
            .try_read_output(&host_pane_id, bytes)
            .map_err(|error| self.client_error(error))
    }

    fn notify_fd(&self) -> Option<std::os::fd::RawFd> {
        self.inner.notify_fd()
    }
}

impl SocketCleanup {
    fn new(path: PathBuf) -> Self {
        let identity = local::socket_identity(&path).ok();
        Self { path, identity }
    }
}

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        if self.identity.is_some() && local::socket_identity(&self.path).ok() == self.identity {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn wait_for_panes_output(
    session_actor: &mut SessionActor,
    output: &mut dyn local::ProcessHostOutput,
    pane_ids: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        let mut changed = false;
        for pane_id in pane_ids {
            changed |=
                local::poll_pane_output_with_session_actor(session_actor, output, pane_id, 0)?;
        }
        if changed {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn stop_panes(host: &mut dyn ProcessHost, pane_ids: &[String]) -> Result<(), HostError> {
    for pane_id in pane_ids {
        host.stop_pane(pane_id)?;
    }
    Ok(())
}

struct Args {
    help: bool,
    version: bool,
    version_json: bool,
    list_daemon_choices_json: bool,
    print_socket: bool,
    print_socket_json: bool,
    ready_json: bool,
    socket_path: PathBuf,
    socket_source: local::SocketPathSource,
    session_id: String,
    tcp_listen: Option<String>,
    tcp_token: Option<String>,
    one_shot: bool,
    live: bool,
    live_forever: bool,
    live_cycles: Option<usize>,
    live_clients: Option<usize>,
    command: Option<String>,
    working_dir: Option<String>,
    env: Vec<(String, String)>,
    initial_size: Option<(u32, u32)>,
    initial_tabs: usize,
    active_tab_id: Option<String>,
    initial_split: Option<protocol::SplitAxis>,
    resize_policy: protocol::ResizePolicy,
    host_kind: HostKindArg,
    sandbox_profile: Option<String>,
    container_image: Option<String>,
    terminal_engine_kind: TerminalEngineKind,
}

impl Args {
    fn transport_endpoint(&self) -> String {
        match self.tcp_listen.as_deref() {
            Some(addr) => format!("tcp://{addr}"),
            None => self.socket_path.display().to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ResizePolicyArg {
    Fixed,
    Leader,
    #[value(name = "active-client")]
    ActiveClient,
    Manual,
}

impl From<ResizePolicyArg> for protocol::ResizePolicy {
    fn from(value: ResizePolicyArg) -> Self {
        match value {
            ResizePolicyArg::Fixed => protocol::ResizePolicy::Fixed,
            ResizePolicyArg::Leader => protocol::ResizePolicy::Leader,
            ResizePolicyArg::ActiveClient => protocol::ResizePolicy::ActiveClient,
            ResizePolicyArg::Manual => protocol::ResizePolicy::Manual,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum SplitAxisArg {
    Horizontal,
    Vertical,
}

impl From<SplitAxisArg> for protocol::SplitAxis {
    fn from(value: SplitAxisArg) -> Self {
        match value {
            SplitAxisArg::Horizontal => protocol::SplitAxis::Horizontal,
            SplitAxisArg::Vertical => protocol::SplitAxis::Vertical,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum HostKindArg {
    Local,
    Sandbox,
    Container,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum TerminalEngineArg {
    Interim,
    #[value(name = "libghostty-vt")]
    LibghosttyVt,
}

impl TerminalEngineArg {
    fn into_terminal_engine_kind(self) -> Result<TerminalEngineKind, &'static str> {
        match self {
            Self::Interim => Ok(TerminalEngineKind::InterimText),
            #[cfg(feature = "libghostty-vt")]
            Self::LibghosttyVt => Ok(TerminalEngineKind::LibghosttyVt),
            #[cfg(not(feature = "libghostty-vt"))]
            Self::LibghosttyVt => Err("libghostty-vt requires the libghostty-vt feature"),
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "nmux daemon",
    disable_help_flag = true,
    disable_version_flag = true,
    args_override_self = true
)]
struct RawArgs {
    #[arg(short = 'h', long = "help", action = ArgAction::SetTrue)]
    help: bool,
    #[arg(short = 'V', long = "version", action = ArgAction::SetTrue)]
    version: bool,
    #[arg(long = "version-json", action = ArgAction::SetTrue)]
    version_json: bool,
    #[arg(long = "list-daemon-choices-json", action = ArgAction::SetTrue)]
    list_daemon_choices_json: bool,
    #[arg(long = "print-socket", action = ArgAction::SetTrue)]
    print_socket: bool,
    #[arg(long = "print-socket-json", action = ArgAction::SetTrue)]
    print_socket_json: bool,
    #[arg(long = "ready-json", action = ArgAction::SetTrue)]
    ready_json: bool,
    #[arg(long = "socket", value_name = "PATH")]
    socket_path: Option<PathBuf>,
    #[arg(
        short = 's',
        long = "session",
        value_name = "NAME",
        allow_hyphen_values = true
    )]
    session_id: Option<String>,
    #[arg(
        long = "tcp-listen",
        alias = "listen",
        value_name = "HOST:PORT",
        allow_hyphen_values = true
    )]
    tcp_listen: Option<String>,
    #[arg(
        long = "tcp-token",
        alias = "token",
        value_name = "TOKEN",
        allow_hyphen_values = true
    )]
    tcp_token: Option<String>,
    #[arg(long = "one-shot", action = ArgAction::SetTrue)]
    one_shot: bool,
    #[arg(long = "live", action = ArgAction::SetTrue)]
    live: bool,
    #[arg(long = "live-forever", action = ArgAction::SetTrue)]
    live_forever: bool,
    #[arg(
        long = "live-cycles",
        value_name = "COUNT",
        value_parser = parse_live_cycles_arg
    )]
    live_cycles: Option<usize>,
    #[arg(
        long = "live-clients",
        value_name = "COUNT",
        value_parser = parse_live_clients_arg
    )]
    live_clients: Option<usize>,
    #[arg(long = "command", value_name = "SHELL", allow_hyphen_values = true)]
    command: Option<String>,
    #[arg(long = "cwd", value_name = "DIR", allow_hyphen_values = true)]
    working_dir: Option<String>,
    #[arg(
        long = "env",
        value_name = "KEY=VALUE",
        value_parser = parse_env_assignment,
        allow_hyphen_values = true
    )]
    env: Vec<(String, String)>,
    #[arg(long = "cols", value_name = "COUNT", value_parser = parse_cols_arg)]
    cols: Option<u32>,
    #[arg(long = "rows", value_name = "COUNT", value_parser = parse_rows_arg)]
    rows: Option<u32>,
    #[arg(long = "tabs", value_name = "COUNT", value_parser = parse_tabs_arg)]
    initial_tabs: Option<usize>,
    #[arg(long = "active-tab", value_name = "TAB_ID")]
    active_tab_id: Option<String>,
    #[arg(long = "split", value_name = "horizontal|vertical")]
    initial_split: Option<SplitAxisArg>,
    #[arg(
        long = "resize-policy",
        value_name = "fixed|leader|active-client|manual"
    )]
    resize_policy: Option<ResizePolicyArg>,
    #[arg(long = "host", value_name = "local|sandbox|container")]
    host_kind: Option<HostKindArg>,
    #[arg(
        long = "sandbox-profile",
        value_name = "PROFILE",
        allow_hyphen_values = true
    )]
    sandbox_profile: Option<String>,
    #[arg(
        long = "container-image",
        value_name = "IMAGE",
        allow_hyphen_values = true
    )]
    container_image: Option<String>,
    #[arg(long = "terminal-engine", value_name = "interim|libghostty-vt")]
    terminal_engine_kind: Option<TerminalEngineArg>,
}

fn args_from_iter<I, S>(args: I) -> Result<Args, Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let raw = RawArgs::try_parse_from(args).map_err(clap_error_message)?;
    let socket_path_set = raw.socket_path.is_some();
    let (socket_path, socket_source) = match raw.socket_path {
        Some(path) => (path, local::SocketPathSource::Explicit),
        None => local::default_socket_path_and_source(),
    };
    let session_id = match raw.session_id {
        Some(value) if value.trim().is_empty() => {
            return Err("--session requires a non-empty name".into());
        }
        Some(value) => value,
        None => "local".to_owned(),
    };
    let live_cycles = raw.live_cycles;
    let live_clients = raw.live_clients;
    let working_dir = match raw.working_dir {
        Some(value) if value.is_empty() => {
            return Err("--cwd requires a non-empty directory path".into());
        }
        value => value,
    };
    let resize_policy = raw
        .resize_policy
        .map(protocol::ResizePolicy::from)
        .unwrap_or(protocol::ResizePolicy::Fixed);
    let host_kind = raw.host_kind.unwrap_or(HostKindArg::Local);
    let sandbox_profile = match raw.sandbox_profile {
        Some(value) if value.is_empty() => {
            return Err("--sandbox-profile requires a non-empty profile".into());
        }
        value => value,
    };
    let container_image = match raw.container_image {
        Some(value) if value.is_empty() => {
            return Err("--container-image requires a non-empty image".into());
        }
        value => value,
    };
    let initial_size = match (raw.cols, raw.rows) {
        (Some(cols), Some(rows)) => Some((cols, rows)),
        (None, None) => None,
        _ => return Err("--cols and --rows must be provided together".into()),
    };
    let initial_tabs = raw.initial_tabs.unwrap_or(1);
    let active_tab_id = match raw.active_tab_id {
        Some(value) if value.is_empty() => {
            return Err("--active-tab requires a non-empty tab ID".into());
        }
        value => value,
    };
    let initial_split = raw.initial_split.map(protocol::SplitAxis::from);
    let terminal_engine_kind = raw
        .terminal_engine_kind
        .map(TerminalEngineArg::into_terminal_engine_kind)
        .transpose()
        .map_err(|err| format!("--terminal-engine {err}"))?
        .unwrap_or_default();
    let tcp_listen = match raw.tcp_listen {
        Some(value) if value.is_empty() => {
            return Err("--tcp-listen requires a non-empty HOST:PORT".into());
        }
        value => value,
    };
    let tcp_token = match raw.tcp_token.or_else(|| {
        std::env::var("NMUX_TOKEN")
            .ok()
            .filter(|value| !value.is_empty())
    }) {
        Some(value) if value.is_empty() => {
            return Err("--token requires a non-empty token".into());
        }
        value => value,
    };

    if !(raw.help
        || raw.version
        || raw.version_json
        || raw.list_daemon_choices_json
        || raw.print_socket
        || raw.print_socket_json)
    {
        validate_mode_args(DaemonModeArgs {
            one_shot: raw.one_shot,
            live: raw.live,
            live_forever: raw.live_forever,
            live_cycles,
            live_clients,
        })?;
        validate_working_dir_arg(working_dir.as_deref())?;
        validate_initial_size(initial_size)?;
        validate_initial_tabs(initial_tabs, active_tab_id.as_deref())?;
        if tcp_listen.is_some() && tcp_token.is_none() {
            return Err("--listen requires --token, --tcp-token, or NMUX_TOKEN".into());
        }
        if tcp_listen.is_none() && tcp_token.is_some() {
            return Err("--token requires --listen or --tcp-listen".into());
        }
        if tcp_listen.is_some() && socket_path_set {
            return Err("--tcp-listen cannot be combined with --socket".into());
        }
        validate_host_args(
            host_kind,
            sandbox_profile.as_deref(),
            container_image.as_deref(),
        )?;
    }

    Ok(Args {
        help: raw.help,
        version: raw.version,
        version_json: raw.version_json,
        list_daemon_choices_json: raw.list_daemon_choices_json,
        print_socket: raw.print_socket,
        print_socket_json: raw.print_socket_json,
        ready_json: raw.ready_json,
        socket_path,
        socket_source,
        session_id,
        tcp_listen,
        tcp_token,
        one_shot: raw.one_shot,
        live: raw.live,
        live_forever: raw.live_forever,
        live_cycles,
        live_clients,
        command: raw.command,
        working_dir,
        env: raw.env,
        initial_size,
        initial_tabs,
        active_tab_id,
        initial_split,
        resize_policy,
        host_kind,
        sandbox_profile,
        container_image,
        terminal_engine_kind,
    })
}

fn format_daemon_choices_json() -> String {
    let resize_policies = format_json_string_array(RESIZE_POLICY_NAMES);
    let split_axes = format_json_string_array(SPLIT_AXIS_NAMES);
    let host_kinds = format_json_string_array(HOST_KIND_NAMES);
    let terminal_engines = TERMINAL_ENGINE_NAMES
        .iter()
        .map(|name| {
            format!(
                "{{\"name\":{},\"available\":{}}}",
                local::json_string(name),
                terminal_engine_available(name)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"resize_policies\":{resize_policies},\"split_axes\":{split_axes},\"host_kinds\":{host_kinds},\"terminal_engines\":[{terminal_engines}]}}"
    )
}

fn format_ready_json(args: &Args) -> String {
    format!(
        "{{\"event\":\"ready\",\"NMUX_SOCKET\":{},\"source\":{},\"mode\":{},\"terminal_engine\":{},\"resize_policy\":{},\"host\":{}}}",
        local::json_string(&args.transport_endpoint()),
        local::json_string(if args.tcp_listen.is_some() {
            "--tcp-listen"
        } else {
            args.socket_source.label()
        }),
        local::json_string(daemon_mode_name(args)),
        local::json_string(terminal_engine_kind_name(args.terminal_engine_kind)),
        local::json_string(resize_policy_name(args.resize_policy)),
        local::json_string(host_kind_name(args.host_kind))
    )
}

fn report_ready_json_error(
    args: &Args,
    error: &(dyn std::error::Error + 'static),
) -> io::Result<()> {
    if args.ready_json {
        println!("{}", format_ready_error_json(error));
        io::stdout().flush()?;
    }
    Ok(())
}

fn format_ready_error_json(error: &(dyn std::error::Error + 'static)) -> String {
    format!(
        "{{\"event\":\"error\",\"error\":{{\"message\":{}}}}}",
        local::json_string(&error.to_string())
    )
}

fn daemon_mode_name(args: &Args) -> &'static str {
    if args.live_forever {
        "live-forever"
    } else if args.live_cycles.is_some() {
        "live-cycles"
    } else if args.live_clients.is_some() {
        "live-clients"
    } else if args.live {
        "live"
    } else if args.one_shot {
        "one-shot"
    } else {
        "serve"
    }
}

fn terminal_engine_kind_name(kind: TerminalEngineKind) -> &'static str {
    match kind {
        TerminalEngineKind::InterimText => "interim",
        #[cfg(feature = "libghostty-vt")]
        TerminalEngineKind::LibghosttyVt => "libghostty-vt",
    }
}

fn host_kind_name(kind: HostKindArg) -> &'static str {
    match kind {
        HostKindArg::Local => "local",
        HostKindArg::Sandbox => "sandbox",
        HostKindArg::Container => "container",
    }
}

fn resize_policy_name(policy: protocol::ResizePolicy) -> &'static str {
    match policy {
        protocol::ResizePolicy::Fixed => "fixed",
        protocol::ResizePolicy::Leader => "leader",
        protocol::ResizePolicy::ActiveClient => "active-client",
        protocol::ResizePolicy::Manual => "manual",
        _ => "unknown",
    }
}

fn apply_initial_host_kind(
    host: &mut HostSpec,
    args: &Args,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.host_kind {
        HostKindArg::Local => {
            host.id = "local".to_owned();
            host.kind = HostKind::Local;
        }
        HostKindArg::Sandbox => {
            let profile = args
                .sandbox_profile
                .clone()
                .ok_or("--host sandbox requires --sandbox-profile")?;
            host.id = "sandbox".to_owned();
            host.kind = HostKind::Sandbox { profile };
        }
        HostKindArg::Container => {
            let image = args
                .container_image
                .clone()
                .ok_or("--host container requires --container-image")?;
            host.id = "container".to_owned();
            host.kind = HostKind::Container { image };
        }
    }
    Ok(())
}

fn terminal_engine_available(name: &str) -> bool {
    match name {
        "interim" => true,
        #[cfg(feature = "libghostty-vt")]
        "libghostty-vt" => true,
        #[cfg(not(feature = "libghostty-vt"))]
        "libghostty-vt" => false,
        _ => false,
    }
}

fn format_json_string_array(values: &[&str]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| local::json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn parse_numeric_arg<T>(flag: &str, value: String) -> Result<T, String>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|err| format!("{flag} requires a valid number: {err}"))
}

fn parse_live_cycles_arg(value: &str) -> Result<usize, String> {
    parse_numeric_arg("--live-cycles", value.to_owned())
}

fn parse_live_clients_arg(value: &str) -> Result<usize, String> {
    parse_numeric_arg("--live-clients", value.to_owned())
}

fn parse_cols_arg(value: &str) -> Result<u32, String> {
    parse_numeric_arg("--cols", value.to_owned())
}

fn parse_rows_arg(value: &str) -> Result<u32, String> {
    parse_numeric_arg("--rows", value.to_owned())
}

fn parse_tabs_arg(value: &str) -> Result<usize, String> {
    parse_numeric_arg("--tabs", value.to_owned())
}

fn validate_initial_tabs(
    initial_tabs: usize,
    active_tab_id: Option<&str>,
) -> Result<(), &'static str> {
    if initial_tabs == 0 {
        return Err("--tabs must be greater than 0");
    }
    let Some(active_tab_id) = active_tab_id else {
        return Ok(());
    };
    let Some(tab_number) = active_tab_id.strip_prefix("tab-") else {
        return Err("--active-tab must name an initial tab created by --tabs");
    };
    let Ok(tab_number) = tab_number.parse::<usize>() else {
        return Err("--active-tab must name an initial tab created by --tabs");
    };
    if tab_number == 0 || tab_number > initial_tabs {
        return Err("--active-tab must name an initial tab created by --tabs");
    }
    Ok(())
}

fn validate_host_args(
    host_kind: HostKindArg,
    sandbox_profile: Option<&str>,
    container_image: Option<&str>,
) -> Result<(), &'static str> {
    match host_kind {
        HostKindArg::Local => {
            if sandbox_profile.is_some() {
                return Err("--sandbox-profile requires --host sandbox");
            }
            if container_image.is_some() {
                return Err("--container-image requires --host container");
            }
        }
        HostKindArg::Sandbox => {
            if sandbox_profile.is_none() {
                return Err("--host sandbox requires --sandbox-profile");
            }
            if container_image.is_some() {
                return Err("--container-image requires --host container");
            }
        }
        HostKindArg::Container => {
            if container_image.is_none() {
                return Err("--host container requires --container-image");
            }
            if sandbox_profile.is_some() {
                return Err("--sandbox-profile requires --host sandbox");
            }
        }
    }
    Ok(())
}

fn clap_error_message(error: clap::Error) -> String {
    let first_line = error.to_string();
    let first_line = first_line
        .lines()
        .next()
        .unwrap_or("invalid command line")
        .trim_start_matches("error: ")
        .to_owned();
    if let Some(index) = first_line.find(": --") {
        return first_line[index + 2..].to_owned();
    }
    first_line
}

fn parse_env_assignment(value: &str) -> Result<(String, String), String> {
    let Some((key, value)) = value.split_once('=') else {
        return Err("--env requires KEY=VALUE".to_owned());
    };
    if key.is_empty() {
        return Err("--env requires a non-empty key".to_owned());
    }
    if key.contains('\0') || value.contains('\0') {
        return Err("--env cannot contain NUL bytes".to_owned());
    }
    Ok((key.to_owned(), value.to_owned()))
}

fn validate_working_dir_arg(path: Option<&str>) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    let metadata =
        fs::metadata(path).map_err(|err| format!("--cwd must be an existing directory: {err}"))?;
    if !metadata.is_dir() {
        return Err("--cwd must be an existing directory".to_owned());
    }
    Ok(())
}

fn validate_initial_size(size: Option<(u32, u32)>) -> Result<(), String> {
    if size.is_some_and(|(cols, rows)| {
        cols == 0 || rows == 0 || cols > u16::MAX as u32 || rows > u16::MAX as u32
    }) {
        return Err("--cols and --rows must be between 1 and 65535".to_owned());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
struct DaemonModeArgs {
    one_shot: bool,
    live: bool,
    live_forever: bool,
    live_cycles: Option<usize>,
    live_clients: Option<usize>,
}

fn validate_mode_args(args: DaemonModeArgs) -> Result<(), &'static str> {
    if args.one_shot
        && (args.live
            || args.live_forever
            || args.live_cycles.is_some()
            || args.live_clients.is_some())
    {
        return Err("--one-shot cannot be combined with live daemon modes");
    }
    if args.live && (args.live_forever || args.live_cycles.is_some() || args.live_clients.is_some())
    {
        return Err(
            "--live cannot be combined with --live-forever, --live-cycles, or --live-clients",
        );
    }
    if args.live_forever && (args.live_cycles.is_some() || args.live_clients.is_some()) {
        return Err("--live-forever cannot be combined with --live-cycles or --live-clients");
    }
    if args.live_cycles == Some(0) {
        return Err("--live-cycles must be greater than 0");
    }
    if args.live_clients == Some(0) {
        return Err("--live-clients must be greater than 0");
    }
    Ok(())
}

fn usage() -> &'static str {
    #[cfg(feature = "libghostty-vt")]
    {
        "\
nmux daemon - serve an nmux session over a local Unix socket

Usage:
  nmux daemon [OPTIONS]

Options:
  --socket PATH                         Unix socket path
  -s, --session NAME                    Session name published by this daemon
  --tcp-listen, --listen HOST:PORT      Listen on TCP instead of a Unix socket
  --tcp-token, --token TOKEN            Shared token for TCP transport authentication
  --print-socket                        Print the resolved socket path and exit
  --print-socket-json                   Print the resolved socket path/source as JSON
  --list-daemon-choices-json            List daemon configuration choices as JSON
  --ready-json                          Print a JSON ready event after bind and pane startup
  --one-shot                            Serve one attach client
  --live                                Serve one live client until detach
  --live-forever                        Serve live clients until session shutdown
  --live-cycles COUNT                   Serve a bounded live client
  --live-clients COUNT                  Bound accepted live clients for tests
  --command SHELL                       Run a shell command in the pane PTY
  --cwd DIR                             Run the pane command from existing DIR
  --env KEY=VALUE                       Add an environment variable to the pane command
  --cols COUNT                          Initial pane PTY columns; both dimensions required
  --rows COUNT                          Initial pane PTY rows; both dimensions required
  --tabs COUNT                          Start with COUNT tabs
  --active-tab TAB_ID                   Select the initial active tab
  --split horizontal|vertical           Start with pane-1 split into pane-1 and pane-2
  --resize-policy fixed|leader|active-client|manual
                                         Publish and enforce pane resize policy
  --host local|sandbox|container        Run pane commands locally, through sandbox-exec, or a container runtime
  --sandbox-profile PROFILE             macOS sandbox-exec profile for --host sandbox
  --container-image IMAGE               Container image for --host container
  --terminal-engine interim|libghostty-vt
                                         Backend terminal engine implementation
  --version-json                         Show version as JSON
  -V, --version                         Show version
  -h, --help                            Show this help

Notes:
  Default socket: --socket, else valid absolute $NMUX_SOCKET, else valid absolute $XDG_RUNTIME_DIR/nmux/nmux.sock, else /tmp/nmux-$UID/nmux.sock.
  --listen requires --token, --tcp-token, or NMUX_TOKEN and cannot be combined with --socket.
  Informational flags exit before daemon-mode validation or socket/PTY work.
  --ready-json does not exit; it emits one stdout line after socket bind and pane startup.
  Existing socket paths are not replaced automatically.
  When started inside nmux, NMUX_ORIGIN is appended for child pane commands.
  --host container uses $NMUX_CONTAINER_RUNTIME or docker, and passes pane cwd/env into the runtime.
  --host sandbox currently uses macOS sandbox-exec and reports an unsupported host on other platforms.
  Default terminal engine: libghostty-vt.
  Use --terminal-engine interim for the portable fallback engine.

Examples:
  nmux daemon --one-shot --command \"printf 'ready\\n'; cat >/dev/null\"
  nmux daemon --listen 127.0.0.1:7007 --token TOKEN
  nmux daemon --live --command \"printf 'ready\\n'; cat\"
  nmux daemon --live-forever --command \"printf 'ready\\n'; cat\"
  nmux daemon --live-clients 2 --command \"printf 'ready\\n'; cat\"
"
    }

    #[cfg(not(feature = "libghostty-vt"))]
    {
        "\
nmux daemon - serve an nmux session over a local Unix socket

Usage:
  nmux daemon [OPTIONS]

Options:
  --socket PATH                         Unix socket path
  -s, --session NAME                    Session name published by this daemon
  --tcp-listen, --listen HOST:PORT      Listen on TCP instead of a Unix socket
  --tcp-token, --token TOKEN            Shared token for TCP transport authentication
  --print-socket                        Print the resolved socket path and exit
  --print-socket-json                   Print the resolved socket path/source as JSON
  --list-daemon-choices-json            List daemon configuration choices as JSON
  --ready-json                          Print a JSON ready event after bind and pane startup
  --one-shot                            Serve one attach client
  --live                                Serve one live client until detach
  --live-forever                        Serve live clients until session shutdown
  --live-cycles COUNT                   Serve a bounded live client
  --live-clients COUNT                  Bound accepted live clients for tests
  --command SHELL                       Run a shell command in the pane PTY
  --cwd DIR                             Run the pane command from existing DIR
  --env KEY=VALUE                       Add an environment variable to the pane command
  --cols COUNT                          Initial pane PTY columns; both dimensions required
  --rows COUNT                          Initial pane PTY rows; both dimensions required
  --tabs COUNT                          Start with COUNT tabs
  --active-tab TAB_ID                   Select the initial active tab
  --split horizontal|vertical           Start with pane-1 split into pane-1 and pane-2
  --resize-policy fixed|leader|active-client|manual
                                         Publish and enforce pane resize policy
  --host local|sandbox|container        Run pane commands locally, through sandbox-exec, or a container runtime
  --sandbox-profile PROFILE             macOS sandbox-exec profile for --host sandbox
  --container-image IMAGE               Container image for --host container
  --terminal-engine interim|libghostty-vt
                                         Backend terminal engine implementation
  --version-json                         Show version as JSON
  -V, --version                         Show version
  -h, --help                            Show this help

Notes:
  Default socket: --socket, else valid absolute $NMUX_SOCKET, else valid absolute $XDG_RUNTIME_DIR/nmux/nmux.sock, else /tmp/nmux-$UID/nmux.sock.
  --listen requires --token, --tcp-token, or NMUX_TOKEN and cannot be combined with --socket.
  Informational flags exit before daemon-mode validation or socket/PTY work.
  --ready-json does not exit; it emits one stdout line after socket bind and pane startup.
  Existing socket paths are not replaced automatically.
  When started inside nmux, NMUX_ORIGIN is appended for child pane commands.
  --host container uses $NMUX_CONTAINER_RUNTIME or docker, and passes pane cwd/env into the runtime.
  --host sandbox currently uses macOS sandbox-exec and reports an unsupported host on other platforms.
  Default terminal engine: interim.
  libghostty-vt requires building nmux with the libghostty-vt feature.

Examples:
  nmux daemon --one-shot --command \"printf 'ready\\n'; cat >/dev/null\"
  nmux daemon --listen 127.0.0.1:7007 --token TOKEN
  nmux daemon --live --command \"printf 'ready\\n'; cat\"
  nmux daemon --live-forever --command \"printf 'ready\\n'; cat\"
  nmux daemon --live-clients 2 --command \"printf 'ready\\n'; cat\"
"
    }
}

#[cfg(test)]
fn parse_resize_policy(value: &str) -> Result<protocol::ResizePolicy, &'static str> {
    match value {
        "fixed" => Ok(protocol::ResizePolicy::Fixed),
        "leader" => Ok(protocol::ResizePolicy::Leader),
        "active-client" => Ok(protocol::ResizePolicy::ActiveClient),
        "manual" => Ok(protocol::ResizePolicy::Manual),
        _ => Err("requires fixed, leader, active-client, or manual"),
    }
}

#[cfg(test)]
fn parse_terminal_engine_kind(value: &str) -> Result<TerminalEngineKind, &'static str> {
    match value {
        "interim" => Ok(TerminalEngineKind::InterimText),
        #[cfg(feature = "libghostty-vt")]
        "libghostty-vt" => Ok(TerminalEngineKind::LibghosttyVt),
        #[cfg(not(feature = "libghostty-vt"))]
        "libghostty-vt" => Err("libghostty-vt requires the libghostty-vt feature"),
        _ => Err("requires interim or libghostty-vt"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Args, DaemonModeArgs, HostKindArg, SocketCleanup, apply_initial_host_kind, args_from_iter,
        format_daemon_choices_json, format_ready_error_json, format_ready_json,
        parse_env_assignment, parse_numeric_arg, parse_resize_policy, parse_terminal_engine_kind,
        usage, validate_host_args, validate_initial_size, validate_initial_tabs,
        validate_mode_args,
    };
    use crate::local;
    use nmux_core::host::{CommandSpec, HostKind, HostSpec};
    use nmux_core::terminal::TerminalEngineKind;
    use nmux_proto::protocol;
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(0);

    fn test_socket_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("nmd-{}-{nanos:x}-{id}.sock", std::process::id()))
    }

    #[test]
    fn resize_policy_arg_accepts_documented_choices() {
        assert_eq!(
            parse_resize_policy("fixed"),
            Ok(protocol::ResizePolicy::Fixed)
        );
        assert_eq!(
            parse_resize_policy("leader"),
            Ok(protocol::ResizePolicy::Leader)
        );
        assert_eq!(
            parse_resize_policy("active-client"),
            Ok(protocol::ResizePolicy::ActiveClient)
        );
        assert_eq!(
            parse_resize_policy("manual"),
            Ok(protocol::ResizePolicy::Manual)
        );
        assert!(parse_resize_policy("max").is_err());
    }

    #[test]
    fn terminal_engine_arg_accepts_documented_choice() {
        assert_eq!(
            parse_terminal_engine_kind("interim"),
            Ok(TerminalEngineKind::InterimText)
        );
        #[cfg(feature = "libghostty-vt")]
        assert_eq!(
            parse_terminal_engine_kind("libghostty-vt"),
            Ok(TerminalEngineKind::LibghosttyVt)
        );
        #[cfg(not(feature = "libghostty-vt"))]
        assert!(parse_terminal_engine_kind("libghostty-vt").is_err());
    }

    #[test]
    fn daemon_choices_json_lists_config_vocabularies() {
        let json = format_daemon_choices_json();
        assert!(
            json.contains(
                "\"resize_policies\":[\"fixed\",\"leader\",\"active-client\",\"manual\"]"
            )
        );
        assert!(json.contains("\"split_axes\":[\"horizontal\",\"vertical\"]"));
        assert!(json.contains("\"host_kinds\":[\"local\",\"sandbox\",\"container\"]"));
        assert!(json.contains("{\"name\":\"interim\",\"available\":true}"));
        #[cfg(feature = "libghostty-vt")]
        assert!(json.contains("{\"name\":\"libghostty-vt\",\"available\":true}"));
        #[cfg(not(feature = "libghostty-vt"))]
        assert!(json.contains("{\"name\":\"libghostty-vt\",\"available\":false}"));
    }

    #[test]
    fn ready_json_reports_startup_context() {
        let args = Args {
            help: false,
            version: false,
            version_json: false,
            list_daemon_choices_json: false,
            print_socket: false,
            print_socket_json: false,
            ready_json: true,
            socket_path: PathBuf::from("/tmp/nmux-ready.sock"),
            socket_source: local::SocketPathSource::Explicit,
            session_id: "local".to_owned(),
            tcp_listen: None,
            tcp_token: None,
            one_shot: false,
            live: false,
            live_forever: true,
            live_cycles: None,
            live_clients: None,
            command: None,
            working_dir: None,
            env: Vec::new(),
            initial_size: None,
            initial_tabs: 1,
            active_tab_id: None,
            initial_split: None,
            resize_policy: protocol::ResizePolicy::ActiveClient,
            host_kind: HostKindArg::Container,
            sandbox_profile: None,
            container_image: Some("alpine:latest".to_owned()),
            terminal_engine_kind: TerminalEngineKind::InterimText,
        };

        assert_eq!(
            format_ready_json(&args),
            "{\"event\":\"ready\",\"NMUX_SOCKET\":\"/tmp/nmux-ready.sock\",\"source\":\"--socket\",\"mode\":\"live-forever\",\"terminal_engine\":\"interim\",\"resize_policy\":\"active-client\",\"host\":\"container\"}"
        );
    }

    #[test]
    fn ready_error_json_reports_startup_failure() {
        let error = std::io::Error::other("bind failed");

        assert_eq!(
            format_ready_error_json(&error),
            "{\"event\":\"error\",\"error\":{\"message\":\"bind failed\"}}"
        );
    }

    #[test]
    fn env_arg_accepts_key_value_with_equals_in_value() {
        assert_eq!(
            parse_env_assignment("NMUX_TEST=one=two"),
            Ok(("NMUX_TEST".to_owned(), "one=two".to_owned()))
        );
        assert_eq!(
            parse_env_assignment("=value"),
            Err("--env requires a non-empty key".to_owned())
        );
        assert_eq!(
            parse_env_assignment("missing"),
            Err("--env requires KEY=VALUE".to_owned())
        );
    }

    #[test]
    fn args_accept_documented_launch_context() {
        let args = args_from_iter([
            "nmux daemon",
            "--one-shot",
            "--socket",
            "/tmp/nmux-daemon-test.sock",
            "--command",
            "printf hi",
            "--cwd",
            "/tmp",
            "--env",
            "NMUX_TEST=one=two",
            "--cols",
            "100",
            "--rows",
            "30",
            "--tabs",
            "2",
            "--active-tab",
            "tab-2",
            "--split",
            "vertical",
            "--resize-policy",
            "active-client",
            "--host",
            "sandbox",
            "--sandbox-profile",
            "(version 1) (allow default)",
            "--terminal-engine",
            "interim",
        ])
        .expect("args");

        assert!(args.one_shot);
        assert_eq!(
            args.socket_path,
            PathBuf::from("/tmp/nmux-daemon-test.sock")
        );
        assert_eq!(args.socket_source, local::SocketPathSource::Explicit);
        assert_eq!(args.command.as_deref(), Some("printf hi"));
        assert_eq!(args.working_dir.as_deref(), Some("/tmp"));
        assert_eq!(
            args.env,
            vec![("NMUX_TEST".to_owned(), "one=two".to_owned())]
        );
        assert_eq!(args.initial_size, Some((100, 30)));
        assert_eq!(args.initial_tabs, 2);
        assert_eq!(args.active_tab_id.as_deref(), Some("tab-2"));
        assert_eq!(args.initial_split, Some(protocol::SplitAxis::Vertical));
        assert_eq!(args.resize_policy, protocol::ResizePolicy::ActiveClient);
        assert_eq!(args.host_kind, HostKindArg::Sandbox);
        assert_eq!(
            args.sandbox_profile.as_deref(),
            Some("(version 1) (allow default)")
        );
        assert_eq!(args.container_image, None);
        assert_eq!(args.terminal_engine_kind, TerminalEngineKind::InterimText);
    }

    #[test]
    fn args_default_terminal_engine_uses_build_default() {
        let args = args_from_iter(["nmux daemon", "--one-shot"]).expect("args");

        assert_eq!(args.terminal_engine_kind, TerminalEngineKind::default());
    }

    #[test]
    fn initial_host_kind_updates_pane_host_spec() {
        let mut host = HostSpec::local("local", CommandSpec::new("sh"));
        let args = args_from_iter([
            "nmux daemon",
            "--one-shot",
            "--host",
            "container",
            "--container-image",
            "alpine:latest",
        ])
        .expect("args");

        apply_initial_host_kind(&mut host, &args).expect("apply host kind");

        assert_eq!(host.id, "container");
        assert_eq!(
            host.kind,
            HostKind::Container {
                image: "alpine:latest".to_owned(),
            }
        );
    }

    #[test]
    fn usage_mentions_live_and_resize_policy_flags() {
        let usage = usage();
        assert!(usage.contains("--live"));
        assert!(usage.contains("--print-socket"));
        assert!(usage.contains("--print-socket-json"));
        assert!(usage.contains("--tcp-listen, --listen HOST:PORT"));
        assert!(usage.contains("--tcp-token, --token TOKEN"));
        assert!(usage.contains("--ready-json"));
        assert!(usage.contains("--list-daemon-choices-json"));
        assert!(usage.contains("--version-json"));
        assert!(usage.contains("-V, --version"));
        assert!(usage.contains("--live-forever"));
        assert!(usage.contains("--live-cycles COUNT"));
        assert!(usage.contains("--live-clients COUNT"));
        assert!(usage.contains("--cwd DIR"));
        assert!(usage.contains("--env KEY=VALUE"));
        assert!(usage.contains("--cols COUNT"));
        assert!(usage.contains("--rows COUNT"));
        assert!(usage.contains("--tabs COUNT"));
        assert!(usage.contains("--active-tab TAB_ID"));
        assert!(usage.contains("--split horizontal|vertical"));
        assert!(usage.contains("--resize-policy fixed|leader|active-client|manual"));
        assert!(usage.contains("--host local|sandbox|container"));
        assert!(usage.contains("--sandbox-profile PROFILE"));
        assert!(usage.contains("--container-image IMAGE"));
        assert!(usage.contains("--terminal-engine interim|libghostty-vt"));
        assert!(usage.contains("--host container uses $NMUX_CONTAINER_RUNTIME"));
        #[cfg(feature = "libghostty-vt")]
        assert!(usage.contains("Default terminal engine: libghostty-vt"));
        #[cfg(feature = "libghostty-vt")]
        assert!(usage.contains("Use --terminal-engine interim"));
        #[cfg(not(feature = "libghostty-vt"))]
        assert!(usage.contains("Default terminal engine: interim"));
        #[cfg(not(feature = "libghostty-vt"))]
        assert!(usage.contains("libghostty-vt requires building nmux"));
        assert!(usage.contains("--ready-json does not exit"));
        assert!(usage.contains("Existing socket paths are not replaced automatically"));
    }

    #[test]
    fn host_validation_requires_matching_options() {
        assert_eq!(validate_host_args(HostKindArg::Local, None, None), Ok(()));
        assert_eq!(
            validate_host_args(HostKindArg::Sandbox, None, None),
            Err("--host sandbox requires --sandbox-profile")
        );
        assert_eq!(
            validate_host_args(HostKindArg::Container, None, None),
            Err("--host container requires --container-image")
        );
        assert_eq!(
            validate_host_args(HostKindArg::Local, Some("profile"), None),
            Err("--sandbox-profile requires --host sandbox")
        );

        let err = match args_from_iter(["nmux daemon", "--one-shot", "--host", "container"]) {
            Ok(_) => panic!("container host without image should fail"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("--host container requires --container-image"));
    }

    #[test]
    fn mode_validation_rejects_ambiguous_daemon_modes() {
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                one_shot: true,
                live_clients: Some(2),
                ..DaemonModeArgs::default()
            }),
            Err("--one-shot cannot be combined with live daemon modes")
        );
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                live: true,
                live_forever: true,
                ..DaemonModeArgs::default()
            }),
            Err("--live cannot be combined with --live-forever, --live-cycles, or --live-clients")
        );
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                live: true,
                live_cycles: Some(1),
                ..DaemonModeArgs::default()
            }),
            Err("--live cannot be combined with --live-forever, --live-cycles, or --live-clients")
        );
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                live_forever: true,
                live_cycles: Some(1),
                ..DaemonModeArgs::default()
            }),
            Err("--live-forever cannot be combined with --live-cycles or --live-clients")
        );
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                live_cycles: Some(0),
                ..DaemonModeArgs::default()
            }),
            Err("--live-cycles must be greater than 0")
        );
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                live_clients: Some(0),
                ..DaemonModeArgs::default()
            }),
            Err("--live-clients must be greater than 0")
        );
        assert!(
            validate_mode_args(DaemonModeArgs {
                live_cycles: Some(2),
                live_clients: Some(3),
                ..DaemonModeArgs::default()
            })
            .is_ok()
        );
        assert!(
            validate_mode_args(DaemonModeArgs {
                live_forever: true,
                ..DaemonModeArgs::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn numeric_args_report_flag_names_on_parse_errors() {
        let err = parse_numeric_arg::<usize>("--live-cycles", "many".to_owned())
            .expect_err("invalid live cycle count should include flag name");
        assert!(err.contains("--live-cycles requires a valid number"));
    }

    #[test]
    fn initial_tab_validation_rejects_missing_or_out_of_range_active_tab() {
        assert_eq!(
            validate_initial_tabs(0, None),
            Err("--tabs must be greater than 0")
        );
        assert_eq!(
            validate_initial_tabs(2, Some("logs")),
            Err("--active-tab must name an initial tab created by --tabs")
        );
        assert_eq!(
            validate_initial_tabs(2, Some("tab-3")),
            Err("--active-tab must name an initial tab created by --tabs")
        );
        assert_eq!(validate_initial_tabs(2, Some("tab-2")), Ok(()));

        let err = match args_from_iter(["nmux daemon", "--one-shot", "--active-tab", ""]) {
            Ok(_) => panic!("empty active tab should fail"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("--active-tab requires a non-empty tab ID"));
    }

    #[test]
    fn initial_size_validation_requires_pair_and_valid_range() {
        let err = match args_from_iter(["nmux daemon", "--one-shot", "--cols", "80"]) {
            Ok(_) => panic!("missing rows should fail"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("--cols and --rows must be provided together"));

        assert_eq!(validate_initial_size(Some((80, 24))), Ok(()));
        assert_eq!(
            validate_initial_size(Some((0, 24))),
            Err("--cols and --rows must be between 1 and 65535".to_owned())
        );
        assert_eq!(
            validate_initial_size(Some((80, 65536))),
            Err("--cols and --rows must be between 1 and 65535".to_owned())
        );
    }

    #[test]
    fn socket_cleanup_removes_original_socket() {
        let socket_path = test_socket_path();
        let listener = UnixListener::bind(&socket_path).expect("bind socket");
        let cleanup = SocketCleanup::new(socket_path.clone());

        drop(cleanup);
        drop(listener);

        assert!(
            !socket_path.exists(),
            "socket cleanup should remove original socket path"
        );
    }

    #[test]
    fn socket_cleanup_keeps_replaced_path() {
        let socket_path = test_socket_path();
        let listener = UnixListener::bind(&socket_path).expect("bind socket");
        let cleanup = SocketCleanup::new(socket_path.clone());

        fs::remove_file(&socket_path).expect("remove original socket");
        fs::write(&socket_path, "replacement").expect("write replacement");
        drop(cleanup);
        drop(listener);

        assert_eq!(
            fs::read_to_string(&socket_path).expect("read replacement"),
            "replacement"
        );
        let _ = fs::remove_file(socket_path);
    }
}
