use std::collections::HashMap;

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
    UnsupportedHostKind { host_id: String, kind: HostKind },
    AlreadyRunning { pane_id: String },
    NotRunning { pane_id: String },
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
        CommandSpec, HostError, HostEvent, HostKind, HostSpec, PlanningHost, ProcessHost,
        ProcessStatus, UnsupportedSandboxHost,
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
}
