use std::fmt;

use nmux_proto::protocol;

use crate::host::{CommandSpec, HostSpec};
use crate::session::{Cursor, Pane, Session, Tab};
use crate::terminal::{CellRun, PaneStyle, TerminalModes};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxSession {
    pub name: String,
    pub windows: Vec<TmuxWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxWindow {
    pub id: String,
    pub title: String,
    pub active: bool,
    pub panes: Vec<TmuxPane>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxPane {
    pub id: String,
    pub title: String,
    pub cols: u32,
    pub rows: u32,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxMappingError {
    EmptySession { session: String },
    EmptyWindow { session: String, window: String },
}

impl fmt::Display for TmuxMappingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySession { session } => {
                write!(formatter, "tmux session has no windows: {session}")
            }
            Self::EmptyWindow { session, window } => {
                write!(
                    formatter,
                    "tmux window has no panes: session={session} window={window}"
                )
            }
        }
    }
}

impl std::error::Error for TmuxMappingError {}

impl TmuxSession {
    pub fn to_nmux_session(&self) -> Result<Session, TmuxMappingError> {
        if self.windows.is_empty() {
            return Err(TmuxMappingError::EmptySession {
                session: self.name.clone(),
            });
        }

        let active_window = self
            .windows
            .iter()
            .find(|window| window.active)
            .unwrap_or(&self.windows[0]);
        let active_tab_id = tmux_window_tab_id(&self.name, &active_window.id);

        let tabs = self
            .windows
            .iter()
            .map(|window| self.window_to_tab(window))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Session {
            id: tmux_session_id(&self.name),
            version: 1,
            active_tab_id,
            tabs,
        })
    }

    fn window_to_tab(&self, window: &TmuxWindow) -> Result<Tab, TmuxMappingError> {
        let active_pane = window
            .panes
            .iter()
            .find(|pane| pane.active)
            .or_else(|| window.panes.first())
            .ok_or_else(|| TmuxMappingError::EmptyWindow {
                session: self.name.clone(),
                window: window.id.clone(),
            })?;

        let pane_id = tmux_pane_id(&self.name, &window.id, &active_pane.id);

        Ok(Tab {
            id: tmux_window_tab_id(&self.name, &window.id),
            title: window.title.clone(),
            active_pane_id: pane_id.clone(),
            root: Pane {
                id: pane_id.clone(),
                host: tmux_pane_host(&self.name, &window.id, &active_pane.id),
                surface_version: 1,
                last_patch_kind: protocol::PatchKind::ReplaceRows,
                scrollback_version: 1,
                cols: active_pane.cols,
                rows: active_pane.rows,
                resize_policy: protocol::ResizePolicy::Fixed,
                surface: protocol::SurfaceKind::Main,
                cursor: Cursor {
                    row: 0,
                    col: 0,
                    visible: true,
                    shape: protocol::CursorShape::Block,
                    blinking: true,
                },
                modes: TerminalModes::default(),
                styles: vec![PaneStyle::default()],
                surface_lines: Vec::new(),
                surface_row_runs: Vec::<Vec<CellRun>>::new(),
                scrollback_lines: Vec::new(),
                scrollback_row_runs: Vec::<Vec<CellRun>>::new(),
            },
        })
    }
}

fn tmux_session_id(session: &str) -> String {
    format!("tmux:{session}")
}

fn tmux_window_tab_id(session: &str, window: &str) -> String {
    format!("tmux:{session}:window:{window}")
}

fn tmux_pane_id(session: &str, window: &str, pane: &str) -> String {
    format!("tmux:{session}:window:{window}:pane:{pane}")
}

fn tmux_pane_host(session: &str, window: &str, pane: &str) -> HostSpec {
    HostSpec::local(
        format!("tmux:{session}:adapter"),
        CommandSpec::new("tmux").with_args([
            "send-keys",
            "-t",
            &format!("{session}:{window}.{pane}"),
        ]),
    )
}

#[cfg(test)]
mod tests {
    use crate::host::HostKind;

    use super::{TmuxMappingError, TmuxPane, TmuxSession, TmuxWindow};

    #[test]
    fn maps_tmux_session_inventory_to_nmux_session() {
        let tmux = TmuxSession {
            name: "dev".to_owned(),
            windows: vec![
                TmuxWindow {
                    id: "1".to_owned(),
                    title: "editor".to_owned(),
                    active: false,
                    panes: vec![TmuxPane {
                        id: "%3".to_owned(),
                        title: "nvim".to_owned(),
                        cols: 120,
                        rows: 40,
                        active: true,
                    }],
                },
                TmuxWindow {
                    id: "2".to_owned(),
                    title: "tests".to_owned(),
                    active: true,
                    panes: vec![
                        TmuxPane {
                            id: "%4".to_owned(),
                            title: "cargo test".to_owned(),
                            cols: 100,
                            rows: 30,
                            active: false,
                        },
                        TmuxPane {
                            id: "%5".to_owned(),
                            title: "shell".to_owned(),
                            cols: 100,
                            rows: 30,
                            active: true,
                        },
                    ],
                },
            ],
        };

        let session = tmux.to_nmux_session().expect("mapped session");

        assert_eq!(session.id, "tmux:dev");
        assert_eq!(session.version, 1);
        assert_eq!(session.active_tab_id, "tmux:dev:window:2");
        assert_eq!(session.tabs.len(), 2);

        assert_eq!(session.tabs[0].id, "tmux:dev:window:1");
        assert_eq!(session.tabs[0].title, "editor");
        assert_eq!(session.tabs[0].active_pane_id, "tmux:dev:window:1:pane:%3");
        assert_eq!(session.tabs[0].root.id, "tmux:dev:window:1:pane:%3");
        assert_eq!(session.tabs[0].root.cols, 120);
        assert_eq!(session.tabs[0].root.rows, 40);

        let active_tab = &session.tabs[1];
        assert_eq!(active_tab.id, "tmux:dev:window:2");
        assert_eq!(active_tab.title, "tests");
        assert_eq!(active_tab.active_pane_id, "tmux:dev:window:2:pane:%5");
        assert_eq!(active_tab.root.id, "tmux:dev:window:2:pane:%5");
        assert_eq!(active_tab.root.host.id, "tmux:dev:adapter");
        assert_eq!(active_tab.root.host.kind, HostKind::Local);
        assert_eq!(active_tab.root.host.command.program, "tmux");
        assert_eq!(
            active_tab.root.host.command.args,
            vec!["send-keys", "-t", "dev:2.%5"]
        );
        assert_eq!(active_tab.root.surface_version, 1);
        assert!(active_tab.root.surface_lines.is_empty());
        assert!(active_tab.root.scrollback_lines.is_empty());
    }

    #[test]
    fn maps_first_window_and_pane_when_tmux_marks_no_active_item() {
        let tmux = TmuxSession {
            name: "fallback".to_owned(),
            windows: vec![TmuxWindow {
                id: "9".to_owned(),
                title: "first".to_owned(),
                active: false,
                panes: vec![TmuxPane {
                    id: "%1".to_owned(),
                    title: "shell".to_owned(),
                    cols: 80,
                    rows: 24,
                    active: false,
                }],
            }],
        };

        let session = tmux.to_nmux_session().expect("mapped session");

        assert_eq!(session.active_tab_id, "tmux:fallback:window:9");
        assert_eq!(
            session.tabs[0].active_pane_id,
            "tmux:fallback:window:9:pane:%1"
        );
    }

    #[test]
    fn rejects_empty_tmux_inventory() {
        let tmux = TmuxSession {
            name: "empty".to_owned(),
            windows: Vec::new(),
        };

        assert_eq!(
            tmux.to_nmux_session(),
            Err(TmuxMappingError::EmptySession {
                session: "empty".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_tmux_window_without_panes() {
        let tmux = TmuxSession {
            name: "dev".to_owned(),
            windows: vec![TmuxWindow {
                id: "1".to_owned(),
                title: "empty".to_owned(),
                active: true,
                panes: Vec::new(),
            }],
        };

        assert_eq!(
            tmux.to_nmux_session(),
            Err(TmuxMappingError::EmptyWindow {
                session: "dev".to_owned(),
                window: "1".to_owned(),
            })
        );
    }
}
