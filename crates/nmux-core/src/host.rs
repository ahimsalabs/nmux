use std::collections::HashMap;
use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};

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
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            working_dir: None,
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

pub trait ProcessHost {
    fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError>;
    fn write_input(&mut self, pane_id: &str, bytes: &[u8]) -> Result<(), HostError>;
    fn resize_pane(&mut self, pane_id: &str, cols: u32, rows: u32) -> Result<(), HostError>;
    fn stop_pane(&mut self, pane_id: &str) -> Result<PaneProcess, HostError>;
}

#[derive(Debug, Default)]
pub struct PlanningHost {
    processes: HashMap<String, PaneProcess>,
    events: Vec<HostEvent>,
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

impl ProcessHost for LocalProcessHost {
    fn start_pane(&mut self, pane_id: &str, spec: &HostSpec) -> Result<PaneProcess, HostError> {
        if spec.kind != HostKind::Local {
            return Err(HostError::UnsupportedHostKind {
                host_id: spec.id.clone(),
                kind: spec.kind.clone(),
            });
        }

        if let Some(process) = self.processes.get(pane_id) {
            if process.process.status == ProcessStatus::Running {
                return Err(HostError::AlreadyRunning {
                    pane_id: pane_id.to_owned(),
                });
            }
        }

        let mut command = Command::new(&spec.command.program);
        command
            .args(&spec.command.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(working_dir) = &spec.command.working_dir {
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
        CommandSpec, HostError, HostEvent, HostKind, HostSpec, LocalProcessHost, PlanningHost,
        ProcessHost, ProcessStatus, UnsupportedSandboxHost,
    };

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
    fn local_process_host_rejects_non_local_specs() {
        let spec = HostSpec::sandbox("sandbox", "default", CommandSpec::new("sh"));
        let mut host = LocalProcessHost::default();

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
}
