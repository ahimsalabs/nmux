use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io::{self, Read, Write};
use std::os::fd::RawFd;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

use portable_pty::{Child as PtyChild, CommandBuilder, MasterPty, PtySize, native_pty_system};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSpec {
    pub id: String,
    pub kind: HostKind,
    pub command: CommandSpec,
}

impl HostSpec {
    pub fn local(id: impl Into<String>, command: CommandSpec) -> Self {
        Self {
            id: id.into(),
            kind: HostKind::Local,
            command,
        }
    }

    pub fn sandbox(
        id: impl Into<String>,
        profile: impl Into<String>,
        command: CommandSpec,
    ) -> Self {
        Self {
            id: id.into(),
            kind: HostKind::Sandbox {
                profile: profile.into(),
            },
            command,
        }
    }

    pub fn container(
        id: impl Into<String>,
        image: impl Into<String>,
        command: CommandSpec,
    ) -> Self {
        Self {
            id: id.into(),
            kind: HostKind::Container {
                image: image.into(),
            },
            command,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKind {
    Local,
    Container { image: String },
    Sandbox { profile: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub env: Vec<(String, String)>,
    pub initial_size: Option<(u32, u32)>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            working_dir: None,
            env: Vec::new(),
            initial_size: None,
        }
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_working_dir(mut self, working_dir: impl Into<String>) -> Self {
        self.working_dir = Some(working_dir.into());
        self
    }

    pub fn with_env<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn with_envs<I, K, V>(mut self, env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.env.extend(
            env.into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }

    pub fn with_initial_size(mut self, cols: u32, rows: u32) -> Self {
        self.initial_size = Some((cols, rows));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneProcess {
    pub pane_id: String,
    pub host_id: String,
    pub status: ProcessStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    UnsupportedHostKind {
        host_id: String,
        kind: HostKind,
    },
    UnsupportedOperation {
        pane_id: String,
        operation: String,
    },
    AlreadyRunning {
        pane_id: String,
    },
    NotRunning {
        pane_id: String,
    },
    Io {
        pane_id: String,
        operation: String,
        message: String,
    },
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedHostKind { host_id, kind } => {
                write!(formatter, "unsupported host kind for {host_id}: {kind:?}")
            }
            Self::UnsupportedOperation { pane_id, operation } => {
                write!(
                    formatter,
                    "unsupported host operation {operation} for {pane_id}"
                )
            }
            Self::AlreadyRunning { pane_id } => {
                write!(formatter, "pane process is already running: {pane_id}")
            }
            Self::NotRunning { pane_id } => {
                write!(formatter, "pane process is not running: {pane_id}")
            }
            Self::Io {
                pane_id,
                operation,
                message,
            } => write!(
                formatter,
                "host I/O error during {operation} for {pane_id}: {message}"
            ),
        }
    }
}

impl std::error::Error for HostError {}

pub trait ProcessHost {
    fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError>;
    fn write_input(&mut self, pane_id: &str, bytes: &[u8]) -> Result<(), HostError>;
    fn resize_pane(&mut self, pane_id: &str, cols: u32, rows: u32) -> Result<(), HostError>;
    fn stop_pane(&mut self, pane_id: &str) -> Result<PaneProcess, HostError>;
}

pub trait ProcessOutput {
    fn try_read_output(&mut self, pane_id: &str, bytes: &mut [u8]) -> Result<usize, HostError>;

    fn notify_fd(&self) -> Option<RawFd> {
        None
    }
}

#[derive(Debug, Default)]
pub struct PlanningHost {
    processes: HashMap<String, PaneProcess>,
    events: Vec<HostEvent>,
}

#[derive(Debug, Default)]
pub struct RecordingOutput {
    output: HashMap<String, VecDeque<u8>>,
}

#[derive(Debug, Default)]
pub struct LocalProcessHost {
    processes: HashMap<String, LocalProcess>,
}

#[derive(Debug)]
struct LocalProcess {
    process: PaneProcess,
    child: Child,
    stdin: Option<ChildStdin>,
}

#[derive(Default)]
pub struct LocalPtyHost {
    processes: HashMap<String, LocalPtyProcess>,
    notify_pipe: Option<NotifyPipe>,
}

struct NotifyPipe {
    read_fd: RawFd,
    write_fd: RawFd,
}

struct LocalPtyProcess {
    process: PaneProcess,
    child: Box<dyn PtyChild + Send>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    pump: Option<JoinHandle<()>>,
    pending_output: VecDeque<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostSpawnCommand {
    program: String,
    args: Vec<String>,
    working_dir: Option<String>,
    env: Vec<(String, String)>,
}

impl HostSpawnCommand {
    fn from_command(command: &CommandSpec) -> Self {
        Self {
            program: command.program.clone(),
            args: command.args.clone(),
            working_dir: command.working_dir.clone(),
            env: command.env.clone(),
        }
    }
}

fn spawn_command_for_host(
    spec: &HostSpec,
    allocate_tty: bool,
    pane_id: &str,
) -> Result<HostSpawnCommand, HostError> {
    match &spec.kind {
        HostKind::Local => Ok(HostSpawnCommand::from_command(&spec.command)),
        HostKind::Container { image } => {
            Ok(container_spawn_command(image, &spec.command, allocate_tty))
        }
        HostKind::Sandbox { profile } => {
            sandbox_spawn_command(&spec.id, profile, &spec.command, pane_id)
        }
    }
}

fn container_spawn_command(
    image: &str,
    command: &CommandSpec,
    allocate_tty: bool,
) -> HostSpawnCommand {
    let runtime = std::env::var("NMUX_CONTAINER_RUNTIME").unwrap_or_else(|_| "docker".to_owned());
    let mut args = vec!["run".to_owned(), "--rm".to_owned(), "-i".to_owned()];
    if allocate_tty {
        args.push("-t".to_owned());
    }
    if let Some(working_dir) = &command.working_dir {
        args.extend(["--workdir".to_owned(), working_dir.clone()]);
    }
    for (key, value) in &command.env {
        args.extend(["-e".to_owned(), format!("{key}={value}")]);
    }
    args.push(image.to_owned());
    args.push(command.program.clone());
    args.extend(command.args.iter().cloned());
    HostSpawnCommand {
        program: runtime,
        args,
        working_dir: None,
        env: Vec::new(),
    }
}

#[cfg(target_os = "macos")]
fn sandbox_spawn_command(
    _host_id: &str,
    profile: &str,
    command: &CommandSpec,
    _pane_id: &str,
) -> Result<HostSpawnCommand, HostError> {
    let mut args = vec!["-p".to_owned(), profile.to_owned(), command.program.clone()];
    args.extend(command.args.iter().cloned());
    Ok(HostSpawnCommand {
        program: "sandbox-exec".to_owned(),
        args,
        working_dir: command.working_dir.clone(),
        env: command.env.clone(),
    })
}

#[cfg(not(target_os = "macos"))]
fn sandbox_spawn_command(
    host_id: &str,
    profile: &str,
    _command: &CommandSpec,
    _pane_id: &str,
) -> Result<HostSpawnCommand, HostError> {
    Err(HostError::UnsupportedHostKind {
        host_id: host_id.to_owned(),
        kind: HostKind::Sandbox {
            profile: profile.to_owned(),
        },
    })
}

impl LocalProcessHost {
    fn process_mut(&mut self, pane_id: &str) -> Result<&mut LocalProcess, HostError> {
        self.processes
            .get_mut(pane_id)
            .filter(|process| process.process.status == ProcessStatus::Running)
            .ok_or_else(|| HostError::NotRunning {
                pane_id: pane_id.to_owned(),
            })
    }

    fn io_error(pane_id: &str, operation: &str, error: impl ToString) -> HostError {
        HostError::Io {
            pane_id: pane_id.to_owned(),
            operation: operation.to_owned(),
            message: error.to_string(),
        }
    }
}

impl RecordingOutput {
    pub fn push_output(&mut self, pane_id: impl Into<String>, bytes: impl AsRef<[u8]>) {
        let queue = self.output.entry(pane_id.into()).or_default();
        queue.extend(bytes.as_ref());
    }
}

impl ProcessOutput for RecordingOutput {
    fn try_read_output(&mut self, pane_id: &str, bytes: &mut [u8]) -> Result<usize, HostError> {
        let Some(queue) = self.output.get_mut(pane_id) else {
            return Ok(0);
        };

        let count = bytes.len().min(queue.len());
        for byte in &mut bytes[..count] {
            *byte = queue.pop_front().expect("queue has count bytes");
        }
        Ok(count)
    }
}

impl LocalPtyHost {
    fn process_mut(&mut self, pane_id: &str) -> Result<&mut LocalPtyProcess, HostError> {
        self.processes
            .get_mut(pane_id)
            .filter(|process| process.process.status == ProcessStatus::Running)
            .ok_or_else(|| HostError::NotRunning {
                pane_id: pane_id.to_owned(),
            })
    }

    fn io_error(pane_id: &str, operation: &str, error: impl ToString) -> HostError {
        HostError::Io {
            pane_id: pane_id.to_owned(),
            operation: operation.to_owned(),
            message: error.to_string(),
        }
    }

    fn ensure_notify_pipe(&mut self, pane_id: &str) -> Result<&NotifyPipe, HostError> {
        if self.notify_pipe.is_none() {
            self.notify_pipe = Some(
                NotifyPipe::new().map_err(|error| Self::io_error(pane_id, "notify_pipe", error))?,
            );
        }
        Ok(self.notify_pipe.as_ref().expect("notify pipe initialized"))
    }

    fn drain_pumped_output(process: &mut LocalPtyProcess, bytes: &mut [u8]) -> usize {
        while let Ok(chunk) = process.output.try_recv() {
            process.pending_output.extend(chunk);
        }

        let count = bytes.len().min(process.pending_output.len());
        for byte in &mut bytes[..count] {
            *byte = process
                .pending_output
                .pop_front()
                .expect("pending output has count bytes");
        }
        count
    }
}

impl NotifyPipe {
    fn new() -> io::Result<Self> {
        let mut fds = [-1; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for fd in fds {
            if let Err(error) = set_fd_nonblocking(fd) {
                unsafe {
                    libc::close(fds[0]);
                    libc::close(fds[1]);
                }
                return Err(error);
            }
        }
        Ok(Self {
            read_fd: fds[0],
            write_fd: fds[1],
        })
    }

    fn duplicate_writer(&self) -> io::Result<NotifyPipeWriter> {
        let fd = unsafe { libc::dup(self.write_fd) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        if let Err(error) = set_fd_nonblocking(fd) {
            unsafe {
                libc::close(fd);
            }
            return Err(error);
        }
        Ok(NotifyPipeWriter { fd })
    }
}

impl Drop for NotifyPipe {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.read_fd);
            libc::close(self.write_fd);
        }
    }
}

struct NotifyPipeWriter {
    fd: RawFd,
}

impl NotifyPipeWriter {
    fn notify(&self) {
        let byte = [1_u8];
        let _ = unsafe { libc::write(self.fd, byte.as_ptr().cast(), byte.len()) };
    }
}

impl Drop for NotifyPipeWriter {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

fn set_fd_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl ProcessOutput for LocalPtyHost {
    fn try_read_output(&mut self, pane_id: &str, bytes: &mut [u8]) -> Result<usize, HostError> {
        let process = self.process_mut(pane_id)?;
        Ok(Self::drain_pumped_output(process, bytes))
    }

    fn notify_fd(&self) -> Option<RawFd> {
        self.notify_pipe.as_ref().map(|pipe| pipe.read_fd)
    }
}

impl ProcessHost for LocalPtyHost {
    fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
        if let Some(process) = self.processes.get(pane_id) {
            if process.process.status == ProcessStatus::Running {
                return Err(HostError::AlreadyRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
        }

        let pty_system = native_pty_system();
        let (cols, rows) = spec.command.initial_size.unwrap_or((80, 24));
        let pair = pty_system
            .openpty(PtySize {
                rows: u16::try_from(rows).unwrap_or(u16::MAX),
                cols: u16::try_from(cols).unwrap_or(u16::MAX),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| Self::io_error(pane_id, "openpty", error))?;
        let spawn_command = spawn_command_for_host(spec, true, pane_id)?;
        let mut command = CommandBuilder::new(&spawn_command.program);
        command.args(&spawn_command.args);
        for (key, value) in &spawn_command.env {
            command.env(key, value);
        }
        if let Some(working_dir) = &spawn_command.working_dir {
            command.cwd(working_dir);
        }

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| Self::io_error(pane_id, "start", error))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| Self::io_error(pane_id, "take_writer", error))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| Self::io_error(pane_id, "try_clone_reader", error))?;
        let (output_sender, output) = mpsc::channel();
        let notify_writer = self
            .ensure_notify_pipe(pane_id)?
            .duplicate_writer()
            .map_err(|error| Self::io_error(pane_id, "notify_pipe", error))?;
        let pump = thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if output_sender.send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                        notify_writer.notify();
                    }
                    Err(_) => break,
                }
            }
        });
        let process = PaneProcess {
            pane_id: pane_id.to_owned(),
            host_id: spec.id.clone(),
            status: ProcessStatus::Running,
        };

        self.processes.insert(
            pane_id.to_owned(),
            LocalPtyProcess {
                process: process.clone(),
                child,
                master: pair.master,
                writer,
                output,
                pump: Some(pump),
                pending_output: VecDeque::new(),
            },
        );
        Ok(process)
    }

    fn write_input(&mut self, pane_id: &str, bytes: &[u8]) -> Result<(), HostError> {
        let process = self.process_mut(pane_id)?;
        process
            .writer
            .write_all(bytes)
            .and_then(|()| process.writer.flush())
            .map_err(|error| Self::io_error(pane_id, "write_input", error))
    }

    fn resize_pane(&mut self, pane_id: &str, cols: u32, rows: u32) -> Result<(), HostError> {
        let process = self.process_mut(pane_id)?;
        process
            .master
            .resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| Self::io_error(pane_id, "resize_pane", error))
    }

    fn stop_pane(&mut self, pane_id: &str) -> Result<PaneProcess, HostError> {
        let mut process = self
            .processes
            .remove(pane_id)
            .ok_or_else(|| HostError::NotRunning {
                pane_id: pane_id.to_owned(),
            })?;
        if process.process.status != ProcessStatus::Running {
            return Err(HostError::NotRunning {
                pane_id: pane_id.to_owned(),
            });
        }

        if process
            .child
            .try_wait()
            .map_err(|error| Self::io_error(pane_id, "stop", error))?
            .is_none()
        {
            match process.child.kill() {
                Ok(()) => {}
                Err(error)
                    if error.kind() == io::ErrorKind::InvalidInput
                        || error.raw_os_error() == Some(22) =>
                {
                    if process
                        .child
                        .try_wait()
                        .map_err(|error| Self::io_error(pane_id, "stop", error))?
                        .is_none()
                    {
                        return Err(Self::io_error(pane_id, "stop", error));
                    }
                }
                Err(error) => return Err(Self::io_error(pane_id, "stop", error)),
            }
        }
        process
            .child
            .wait()
            .map_err(|error| Self::io_error(pane_id, "stop", error))?;
        if let Some(pump) = process.pump.take() {
            let _ = pump.join();
        }

        process.process.status = ProcessStatus::Exited;
        Ok(process.process)
    }
}

impl ProcessHost for LocalProcessHost {
    fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
        if let Some(process) = self.processes.get(pane_id) {
            if process.process.status == ProcessStatus::Running {
                return Err(HostError::AlreadyRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
        }

        let spawn_command = spawn_command_for_host(spec, false, pane_id)?;
        let mut command = Command::new(&spawn_command.program);
        command
            .args(&spawn_command.args)
            .envs(spawn_command.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(working_dir) = &spawn_command.working_dir {
            command.current_dir(working_dir);
        }

        let mut child = command
            .spawn()
            .map_err(|error| Self::io_error(pane_id, "start", error))?;
        let stdin = child.stdin.take();
        let process = PaneProcess {
            pane_id: pane_id.to_owned(),
            host_id: spec.id.clone(),
            status: ProcessStatus::Running,
        };
        self.processes.insert(
            pane_id.to_owned(),
            LocalProcess {
                process: process.clone(),
                child,
                stdin,
            },
        );
        Ok(process)
    }

    fn write_input(&mut self, pane_id: &str, bytes: &[u8]) -> Result<(), HostError> {
        let process = self.process_mut(pane_id)?;
        let stdin = process
            .stdin
            .as_mut()
            .ok_or_else(|| HostError::NotRunning {
                pane_id: pane_id.to_owned(),
            })?;
        stdin
            .write_all(bytes)
            .and_then(|()| stdin.flush())
            .map_err(|error| Self::io_error(pane_id, "write_input", error))
    }

    fn resize_pane(&mut self, pane_id: &str, _cols: u32, _rows: u32) -> Result<(), HostError> {
        self.process_mut(pane_id)?;
        Err(HostError::UnsupportedOperation {
            pane_id: pane_id.to_owned(),
            operation: "resize_pane".to_owned(),
        })
    }

    fn stop_pane(&mut self, pane_id: &str) -> Result<PaneProcess, HostError> {
        let mut process = self
            .processes
            .remove(pane_id)
            .ok_or_else(|| HostError::NotRunning {
                pane_id: pane_id.to_owned(),
            })?;
        if process.process.status != ProcessStatus::Running {
            return Err(HostError::NotRunning {
                pane_id: pane_id.to_owned(),
            });
        }

        drop(process.stdin.take());
        if process
            .child
            .try_wait()
            .map_err(|error| Self::io_error(pane_id, "stop", error))?
            .is_none()
        {
            process
                .child
                .kill()
                .map_err(|error| Self::io_error(pane_id, "stop", error))?;
        }
        process
            .child
            .wait()
            .map_err(|error| Self::io_error(pane_id, "stop", error))?;

        process.process.status = ProcessStatus::Exited;
        Ok(process.process)
    }
}

impl PlanningHost {
    pub fn events(&self) -> &[HostEvent] {
        &self.events
    }

    fn running_process(&self, pane_id: &str) -> Result<&PaneProcess, HostError> {
        self.processes
            .get(pane_id)
            .filter(|process| process.status == ProcessStatus::Running)
            .ok_or_else(|| HostError::NotRunning {
                pane_id: pane_id.to_owned(),
            })
    }
}

impl ProcessHost for PlanningHost {
    fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
        if let Some(process) = self.processes.get(pane_id) {
            if process.status == ProcessStatus::Running {
                return Err(HostError::AlreadyRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
        }

        let process = PaneProcess {
            pane_id: pane_id.to_owned(),
            host_id: spec.id.clone(),
            status: ProcessStatus::Running,
        };
        self.processes.insert(pane_id.to_owned(), process.clone());
        self.events.push(HostEvent::Started {
            pane_id: pane_id.to_owned(),
            host_id: spec.id.clone(),
            kind: spec.kind.clone(),
        });
        Ok(process)
    }

    fn write_input(&mut self, pane_id: &str, bytes: &[u8]) -> Result<(), HostError> {
        self.running_process(pane_id)?;
        self.events.push(HostEvent::Input {
            pane_id: pane_id.to_owned(),
            bytes: bytes.to_vec(),
        });
        Ok(())
    }

    fn resize_pane(&mut self, pane_id: &str, cols: u32, rows: u32) -> Result<(), HostError> {
        self.running_process(pane_id)?;
        self.events.push(HostEvent::Resized {
            pane_id: pane_id.to_owned(),
            cols,
            rows,
        });
        Ok(())
    }

    fn stop_pane(&mut self, pane_id: &str) -> Result<PaneProcess, HostError> {
        let mut process = self.running_process(pane_id)?.clone();
        process.status = ProcessStatus::Exited;
        self.processes.insert(pane_id.to_owned(), process.clone());
        self.events.push(HostEvent::Stopped {
            pane_id: pane_id.to_owned(),
        });
        Ok(process)
    }
}

impl ProcessOutput for PlanningHost {
    fn try_read_output(&mut self, pane_id: &str, _bytes: &mut [u8]) -> Result<usize, HostError> {
        self.running_process(pane_id)?;
        Ok(0)
    }
}

#[derive(Debug, Default)]
pub struct UnsupportedSandboxHost;

impl ProcessHost for UnsupportedSandboxHost {
    fn start_pane(&mut self, _pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
        Err(HostError::UnsupportedHostKind {
            host_id: spec.id.clone(),
            kind: spec.kind.clone(),
        })
    }

    fn write_input(&mut self, pane_id: &str, _bytes: &[u8]) -> Result<(), HostError> {
        Err(HostError::NotRunning {
            pane_id: pane_id.to_owned(),
        })
    }

    fn resize_pane(&mut self, pane_id: &str, _cols: u32, _rows: u32) -> Result<(), HostError> {
        Err(HostError::NotRunning {
            pane_id: pane_id.to_owned(),
        })
    }

    fn stop_pane(&mut self, pane_id: &str) -> Result<PaneProcess, HostError> {
        Err(HostError::NotRunning {
            pane_id: pane_id.to_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    Started {
        pane_id: String,
        host_id: String,
        kind: HostKind,
    },
    Input {
        pane_id: String,
        bytes: Vec<u8>,
    },
    Resized {
        pane_id: String,
        cols: u32,
        rows: u32,
    },
    Stopped {
        pane_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        CommandSpec, HostError, HostEvent, HostKind, HostSpec, LocalProcessHost, LocalPtyHost,
        PlanningHost, ProcessHost, ProcessOutput, ProcessStatus, RecordingOutput,
        UnsupportedSandboxHost, spawn_command_for_host,
    };
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn local_host_choice_can_start_pane_process() {
        let spec = HostSpec::local(
            "local",
            CommandSpec::new("sh")
                .with_args(["-lc", "echo nmux"])
                .with_working_dir("/tmp"),
        );
        let mut host = PlanningHost::default();

        let process = host.start_pane("pane-1", &spec).expect("start pane");

        assert_eq!(process.pane_id, "pane-1");
        assert_eq!(process.host_id, "local");
        assert_eq!(process.status, ProcessStatus::Running);
        assert_eq!(
            host.events(),
            &[HostEvent::Started {
                pane_id: "pane-1".to_owned(),
                host_id: "local".to_owned(),
                kind: HostKind::Local,
            }]
        );
    }

    #[test]
    fn lifecycle_events_require_running_process() {
        let spec = HostSpec::local("local", CommandSpec::new("sh"));
        let mut host = PlanningHost::default();

        host.start_pane("pane-1", &spec).expect("start pane");
        host.write_input("pane-1", b"a").expect("write input");
        host.resize_pane("pane-1", 100, 30).expect("resize pane");
        let stopped = host.stop_pane("pane-1").expect("stop pane");

        assert_eq!(stopped.status, ProcessStatus::Exited);
        assert_eq!(
            host.write_input("pane-1", b"b"),
            Err(HostError::NotRunning {
                pane_id: "pane-1".to_owned(),
            })
        );
        assert_eq!(
            host.stop_pane("pane-1"),
            Err(HostError::NotRunning {
                pane_id: "pane-1".to_owned(),
            })
        );
        assert_eq!(
            host.events(),
            &[
                HostEvent::Started {
                    pane_id: "pane-1".to_owned(),
                    host_id: "local".to_owned(),
                    kind: HostKind::Local,
                },
                HostEvent::Input {
                    pane_id: "pane-1".to_owned(),
                    bytes: b"a".to_vec(),
                },
                HostEvent::Resized {
                    pane_id: "pane-1".to_owned(),
                    cols: 100,
                    rows: 30,
                },
                HostEvent::Stopped {
                    pane_id: "pane-1".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn duplicate_start_is_explicit_error() {
        let spec = HostSpec::local("local", CommandSpec::new("sh"));
        let mut host = PlanningHost::default();

        host.start_pane("pane-1", &spec).expect("start pane");

        assert_eq!(
            host.start_pane("pane-1", &spec),
            Err(HostError::AlreadyRunning {
                pane_id: "pane-1".to_owned(),
            })
        );
    }

    #[test]
    fn sandbox_host_choice_is_represented_without_protocol_changes() {
        let spec = HostSpec::sandbox(
            "sandbox",
            "default",
            CommandSpec::new("sh").with_args(["-lc", "echo nmux"]),
        );
        let mut host = UnsupportedSandboxHost;

        assert_eq!(
            host.start_pane("pane-1", &spec),
            Err(HostError::UnsupportedHostKind {
                host_id: "sandbox".to_owned(),
                kind: HostKind::Sandbox {
                    profile: "default".to_owned(),
                },
            })
        );
    }

    #[test]
    fn local_process_host_starts_writes_and_stops_command() {
        let spec = HostSpec::local(
            "local",
            CommandSpec::new("sh").with_args(["-c", "cat >/dev/null"]),
        );
        let mut host = LocalProcessHost::default();

        let process = host.start_pane("pane-1", &spec).expect("start pane");
        assert_eq!(process.status, ProcessStatus::Running);

        host.write_input("pane-1", b"hello\n")
            .expect("write local input");
        let stopped = host.stop_pane("pane-1").expect("stop pane");

        assert_eq!(stopped.status, ProcessStatus::Exited);
        assert_eq!(
            host.write_input("pane-1", b"after stop"),
            Err(HostError::NotRunning {
                pane_id: "pane-1".to_owned(),
            })
        );
    }

    #[test]
    fn container_host_choice_lowers_to_runtime_command() {
        let spec = HostSpec::container(
            "container",
            "alpine:latest",
            CommandSpec::new("sh")
                .with_args(["-lc", "echo nmux"])
                .with_working_dir("/workspace")
                .with_env("TERM", "xterm-256color"),
        );

        let command = spawn_command_for_host(&spec, true, "pane-1").expect("container command");

        assert_eq!(command.program, "docker");
        assert_eq!(
            command.args,
            vec![
                "run",
                "--rm",
                "-i",
                "-t",
                "--workdir",
                "/workspace",
                "-e",
                "TERM=xterm-256color",
                "alpine:latest",
                "sh",
                "-lc",
                "echo nmux",
            ]
        );
        assert_eq!(command.working_dir, None);
        assert!(command.env.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn sandbox_host_choice_lowers_to_macos_sandbox_exec() {
        let spec = HostSpec::sandbox(
            "sandbox",
            "(version 1) (allow default)",
            CommandSpec::new("sh")
                .with_args(["-lc", "echo nmux"])
                .with_working_dir("/tmp")
                .with_env("TERM", "xterm-256color"),
        );

        let command = spawn_command_for_host(&spec, false, "pane-1").expect("sandbox command");

        assert_eq!(command.program, "sandbox-exec");
        assert_eq!(
            command.args,
            vec![
                "-p",
                "(version 1) (allow default)",
                "sh",
                "-lc",
                "echo nmux",
            ]
        );
        assert_eq!(command.working_dir.as_deref(), Some("/tmp"));
        assert_eq!(
            command.env,
            vec![("TERM".to_owned(), "xterm-256color".to_owned())]
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn sandbox_host_choice_reports_platform_boundary() {
        let spec = HostSpec::sandbox(
            "sandbox",
            "(version 1) (allow default)",
            CommandSpec::new("sh"),
        );

        assert_eq!(
            spawn_command_for_host(&spec, false, "pane-1"),
            Err(HostError::UnsupportedHostKind {
                host_id: "sandbox".to_owned(),
                kind: HostKind::Sandbox {
                    profile: "(version 1) (allow default)".to_owned(),
                },
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn local_process_host_starts_macos_sandbox_command() {
        let spec = HostSpec::sandbox(
            "sandbox",
            "(version 1) (allow default)",
            CommandSpec::new("sh").with_args(["-c", "cat >/dev/null"]),
        );
        let mut host = LocalProcessHost::default();

        let process = host
            .start_pane("pane-1", &spec)
            .expect("start sandbox pane");
        assert_eq!(process.status, ProcessStatus::Running);
        assert_eq!(process.host_id, "sandbox");

        host.write_input("pane-1", b"hello\n")
            .expect("write sandbox input");
        let stopped = host.stop_pane("pane-1").expect("stop sandbox pane");
        assert_eq!(stopped.status, ProcessStatus::Exited);
    }

    #[test]
    fn local_process_host_does_not_fake_pty_resize() {
        let spec = HostSpec::local(
            "local",
            CommandSpec::new("sh").with_args(["-c", "cat >/dev/null"]),
        );
        let mut host = LocalProcessHost::default();

        host.start_pane("pane-1", &spec).expect("start pane");
        assert_eq!(
            host.resize_pane("pane-1", 120, 40),
            Err(HostError::UnsupportedOperation {
                pane_id: "pane-1".to_owned(),
                operation: "resize_pane".to_owned(),
            })
        );
        host.stop_pane("pane-1").expect("stop pane");
    }

    #[test]
    fn local_pty_host_starts_resizes_writes_and_stops_command() {
        let spec = HostSpec::local(
            "local",
            CommandSpec::new("sh").with_args(["-c", "cat >/dev/null"]),
        );
        let mut host = LocalPtyHost::default();

        let process = host.start_pane("pane-1", &spec).expect("start pty pane");
        assert_eq!(process.status, ProcessStatus::Running);

        host.resize_pane("pane-1", 100, 30).expect("resize pty");
        host.write_input("pane-1", b"hello\r")
            .expect("write pty input");

        let stopped = host.stop_pane("pane-1").expect("stop pty pane");
        assert_eq!(stopped.status, ProcessStatus::Exited);
        assert_eq!(
            host.resize_pane("pane-1", 80, 24),
            Err(HostError::NotRunning {
                pane_id: "pane-1".to_owned(),
            })
        );
    }

    #[test]
    fn local_pty_host_passes_command_environment() {
        let spec = HostSpec::local(
            "local",
            CommandSpec::new("sh")
                .with_args([
                    "-c",
                    "printf 'env:%s:%s\\n' \"$NMUX_SESSION_ID\" \"$NMUX_PANE_ID\"; sleep 1",
                ])
                .with_envs([("NMUX_SESSION_ID", "session-1"), ("NMUX_PANE_ID", "pane-7")]),
        );
        let mut host = LocalPtyHost::default();
        let mut buffer = [0_u8; 128];

        host.start_pane("pane-7", &spec).expect("start pty pane");
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut output = String::new();
        while Instant::now() < deadline && !output.contains("env:session-1:pane-7") {
            let count = host
                .try_read_output("pane-7", &mut buffer)
                .expect("read pty output");
            if count > 0 {
                output.push_str(&String::from_utf8_lossy(&buffer[..count]));
            } else {
                thread::sleep(Duration::from_millis(20));
            }
        }
        host.stop_pane("pane-7").expect("stop pty pane");

        assert!(
            output.contains("env:session-1:pane-7"),
            "missing command environment in pty output:\n{output}"
        );
    }

    #[test]
    fn local_pty_output_api_reports_missing_panes_without_blocking() {
        let mut host = LocalPtyHost::default();
        let mut buffer = [0_u8; 16];

        assert_eq!(
            host.try_read_output("pane-1", &mut buffer),
            Err(HostError::NotRunning {
                pane_id: "pane-1".to_owned(),
            })
        );
    }

    #[test]
    fn recording_output_reads_queued_bytes_without_blocking() {
        let mut output = RecordingOutput::default();
        let mut buffer = [0_u8; 4];

        assert_eq!(output.try_read_output("pane-1", &mut buffer), Ok(0));

        output.push_output("pane-1", b"hello");

        assert_eq!(output.try_read_output("pane-1", &mut buffer), Ok(4));
        assert_eq!(&buffer, b"hell");
        assert_eq!(output.try_read_output("pane-1", &mut buffer), Ok(1));
        assert_eq!(&buffer[..1], b"o");
        assert_eq!(output.try_read_output("pane-1", &mut buffer), Ok(0));
    }
}
