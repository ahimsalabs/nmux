use std::hash::{Hash, Hasher};

use flatbuffers::FlatBufferBuilder;
use nmux_proto::{PROTOCOL_VERSION, protocol};

use crate::host::{CommandSpec, HostSpec};
use crate::terminal::{
    CellRun, InterimTextTerminalEngine, PaneStyle, TerminalColors, TerminalCursor, TerminalEngine,
    TerminalInput, TerminalModes,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub version: u64,
    pub active_tab_id: String,
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub id: String,
    pub user_id: String,
    pub display_name: String,
    pub mode: AttachMode,
    pub focused_pane_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachMode {
    ReadOnly,
    ReadWrite,
}

impl AttachMode {
    fn as_protocol(self) -> protocol::AttachMode {
        match self {
            Self::ReadOnly => protocol::AttachMode::ReadOnly,
            Self::ReadWrite => protocol::AttachMode::ReadWrite,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorRetryability {
    Retryable,
    NotRetryable,
}

impl ErrorRetryability {
    fn as_bool(self) -> bool {
        matches!(self, Self::Retryable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputFrameContext<'a> {
    pub connection_id: &'a str,
    pub seq: u64,
    pub actor_id: &'a str,
    pub pane_id: &'a str,
    pub input_seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteInputSpec<'a> {
    pub text: &'a str,
    pub bracketed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusInputSpec {
    pub focused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseInputSpec {
    pub row: u32,
    pub col: u32,
    pub pixel_x: Option<u32>,
    pub pixel_y: Option<u32>,
    pub button: protocol::MouseButton,
    pub action: protocol::MouseAction,
    pub modifiers: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbackRange {
    pub start_line: u64,
    pub line_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbackFetchSpec {
    pub range: ScrollbackRange,
    pub known_scrollback_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub active_pane_id: String,
    pub root: Pane,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub id: String,
    pub split_axis: protocol::SplitAxis,
    pub children: Vec<Pane>,
    pub host: HostSpec,
    pub surface_version: u64,
    pub last_patch_kind: protocol::PatchKind,
    pub last_row_update_indices: Vec<u32>,
    pub scrollback_version: u64,
    pub cols: u32,
    pub rows: u32,
    pub resize_policy: protocol::ResizePolicy,
    pub surface: protocol::SurfaceKind,
    pub cursor: Cursor,
    pub modes: TerminalModes,
    pub terminal_title: String,
    pub terminal_working_directory: String,
    pub colors: TerminalColors,
    pub last_palette_diff: Option<PaletteDiff>,
    pub styles: Vec<PaneStyle>,
    pub surface_lines: Vec<String>,
    pub surface_row_runs: Vec<Vec<CellRun>>,
    pub surface_semantic_prompts: Vec<protocol::RowSemanticPrompt>,
    pub surface_dirty_rows: Vec<bool>,
    pub surface_kitty_placeholders: Vec<bool>,
    pub scrollback_lines: Vec<String>,
    pub scrollback_row_runs: Vec<Vec<CellRun>>,
    pub scrollback_semantic_prompts: Vec<protocol::RowSemanticPrompt>,
    pub scrollback_dirty_rows: Vec<bool>,
    pub scrollback_kitty_placeholders: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteDiff {
    pub start: u32,
    pub colors: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneSurface {
    pub pane_id: String,
    pub version: u64,
    pub cols: u32,
    pub rows: u32,
    pub surface: protocol::SurfaceKind,
    pub cursor: Cursor,
    pub modes: TerminalModes,
    pub title: String,
    pub working_directory: String,
    pub colors: TerminalColors,
    pub styles: Vec<PaneStyle>,
    pub lines: Vec<String>,
    pub row_runs: Vec<Vec<CellRun>>,
    pub semantic_prompts: Vec<protocol::RowSemanticPrompt>,
    pub dirty_rows: Vec<bool>,
    pub kitty_placeholders: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneScrollback {
    pub pane_id: String,
    pub version: u64,
    pub styles: Vec<PaneStyle>,
    pub colors: TerminalColors,
    pub lines: Vec<String>,
    pub row_runs: Vec<Vec<CellRun>>,
    pub semantic_prompts: Vec<protocol::RowSemanticPrompt>,
    pub dirty_rows: Vec<bool>,
    pub kitty_placeholders: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub row: u32,
    pub col: u32,
    pub visible: bool,
    pub shape: protocol::CursorShape,
    pub blinking: bool,
}

impl Session {
    pub fn initial() -> Self {
        let surface_lines = vec![
            "nmux pane-1".to_owned(),
            "server-owned terminal state".to_owned(),
        ];
        let scrollback_lines = vec![
            "booting nmux workspace".to_owned(),
            "nmux pane-1".to_owned(),
            "server-owned terminal state".to_owned(),
        ];
        let surface_row_count = surface_lines.len();
        let scrollback_row_count = scrollback_lines.len();
        Self {
            id: "local".to_owned(),
            version: 1,
            active_tab_id: "tab-1".to_owned(),
            tabs: vec![Tab {
                id: "tab-1".to_owned(),
                title: "local".to_owned(),
                active_pane_id: "pane-1".to_owned(),
                root: Pane {
                    id: "pane-1".to_owned(),
                    split_axis: protocol::SplitAxis::None,
                    children: Vec::new(),
                    host: HostSpec::local("local", CommandSpec::new("sh")),
                    surface_version: 2,
                    last_patch_kind: protocol::PatchKind::ReplaceRows,
                    last_row_update_indices: vec![0, 1],
                    scrollback_version: 1,
                    cols: 80,
                    rows: 24,
                    resize_policy: protocol::ResizePolicy::Fixed,
                    surface: protocol::SurfaceKind::Main,
                    cursor: Cursor {
                        row: 1,
                        col: 0,
                        visible: true,
                        shape: protocol::CursorShape::Block,
                        blinking: true,
                    },
                    modes: TerminalModes::default(),
                    terminal_title: String::new(),
                    terminal_working_directory: String::new(),
                    colors: TerminalColors::default(),
                    last_palette_diff: None,
                    styles: vec![PaneStyle::default()],
                    surface_lines,
                    surface_row_runs: vec![
                        vec![CellRun::plain("nmux pane-1")],
                        vec![CellRun::plain("server-owned terminal state")],
                    ],
                    surface_semantic_prompts: vec![
                        protocol::RowSemanticPrompt::None;
                        surface_row_count
                    ],
                    surface_dirty_rows: vec![false; surface_row_count],
                    surface_kitty_placeholders: vec![false; surface_row_count],
                    scrollback_lines,
                    scrollback_row_runs: vec![
                        vec![CellRun::plain("booting nmux workspace")],
                        vec![CellRun::plain("nmux pane-1")],
                        vec![CellRun::plain("server-owned terminal state")],
                    ],
                    scrollback_semantic_prompts: vec![
                        protocol::RowSemanticPrompt::None;
                        scrollback_row_count
                    ],
                    scrollback_dirty_rows: vec![false; scrollback_row_count],
                    scrollback_kitty_placeholders: vec![false; scrollback_row_count],
                },
            }],
        }
    }

    pub fn from_pane_output(output: &[u8]) -> Self {
        let mut session = Self::initial();
        if let Some(pane) = session.pane_mut("pane-1") {
            pane.surface_lines.clear();
            pane.surface_row_runs.clear();
            pane.surface_semantic_prompts.clear();
            pane.surface_dirty_rows.clear();
            pane.surface_kitty_placeholders.clear();
            pane.scrollback_lines.clear();
            pane.scrollback_row_runs.clear();
            pane.scrollback_semantic_prompts.clear();
            pane.scrollback_dirty_rows.clear();
            pane.scrollback_kitty_placeholders.clear();
        }
        session.apply_pane_output("pane-1", output);
        session
    }

    pub fn apply_pane_output(&mut self, pane_id: &str, output: &[u8]) -> bool {
        let mut engine = InterimTextTerminalEngine::default();
        self.apply_pane_output_with_engine(pane_id, output, &mut engine)
    }

    pub fn apply_pane_output_with_engine(
        &mut self,
        pane_id: &str,
        output: &[u8],
        engine: &mut dyn TerminalEngine,
    ) -> bool {
        let Some(pane) = self.pane_mut(pane_id) else {
            return false;
        };

        let input = TerminalInput {
            pane_id: &pane.id,
            cols: pane.cols,
            rows: pane.rows,
            surface: pane.surface,
            cursor: TerminalCursor::from(&pane.cursor),
            modes: pane.modes,
            title: &pane.terminal_title,
            working_directory: &pane.terminal_working_directory,
            colors: pane.colors.clone(),
            styles: &pane.styles,
            surface_lines: &pane.surface_lines,
            surface_row_runs: &pane.surface_row_runs,
            surface_semantic_prompts: &pane.surface_semantic_prompts,
            surface_dirty_rows: &pane.surface_dirty_rows,
            surface_kitty_placeholders: &pane.surface_kitty_placeholders,
            scrollback_lines: &pane.scrollback_lines,
            scrollback_row_runs: &pane.scrollback_row_runs,
            scrollback_semantic_prompts: &pane.scrollback_semantic_prompts,
            scrollback_dirty_rows: &pane.scrollback_dirty_rows,
            scrollback_kitty_placeholders: &pane.scrollback_kitty_placeholders,
        };
        let Some(update) = engine.apply_output(input, output) else {
            return false;
        };

        apply_terminal_update(pane, update, false)
    }

    pub fn commit_pane_resize(&mut self, pane_id: &str, cols: u32, rows: u32) -> bool {
        let mut engine = InterimTextTerminalEngine::default();
        self.commit_pane_resize_with_engine(pane_id, cols, rows, &mut engine)
    }

    pub fn commit_pane_resize_with_engine(
        &mut self,
        pane_id: &str,
        cols: u32,
        rows: u32,
        engine: &mut dyn TerminalEngine,
    ) -> bool {
        let Some(pane) = self.pane_mut(pane_id) else {
            return false;
        };
        if pane.cols == cols && pane.rows == rows {
            return false;
        }

        let input = TerminalInput {
            pane_id: &pane.id,
            cols: pane.cols,
            rows: pane.rows,
            surface: pane.surface,
            cursor: TerminalCursor::from(&pane.cursor),
            modes: pane.modes,
            title: &pane.terminal_title,
            working_directory: &pane.terminal_working_directory,
            colors: pane.colors.clone(),
            styles: &pane.styles,
            surface_lines: &pane.surface_lines,
            surface_row_runs: &pane.surface_row_runs,
            surface_semantic_prompts: &pane.surface_semantic_prompts,
            surface_dirty_rows: &pane.surface_dirty_rows,
            surface_kitty_placeholders: &pane.surface_kitty_placeholders,
            scrollback_lines: &pane.scrollback_lines,
            scrollback_row_runs: &pane.scrollback_row_runs,
            scrollback_semantic_prompts: &pane.scrollback_semantic_prompts,
            scrollback_dirty_rows: &pane.scrollback_dirty_rows,
            scrollback_kitty_placeholders: &pane.scrollback_kitty_placeholders,
        };
        let Some(update) = engine.resize(input, cols, rows) else {
            return false;
        };

        pane.cols = cols;
        pane.rows = rows;
        apply_terminal_update(pane, update, true);
        self.version = self.version.saturating_add(1);
        true
    }

    pub fn pane_resize_policy(&self, pane_id: &str) -> Option<protocol::ResizePolicy> {
        self.pane(pane_id).map(|pane| pane.resize_policy)
    }

    pub fn set_pane_resize_policy(
        &mut self,
        pane_id: &str,
        policy: protocol::ResizePolicy,
    ) -> bool {
        let Some(pane) = self.pane_mut(pane_id) else {
            return false;
        };
        if pane.resize_policy == policy {
            return false;
        }

        pane.resize_policy = policy;
        self.version = self.version.saturating_add(1);
        true
    }

    pub fn set_pane_nmux_environment(
        &mut self,
        pane_id: &str,
        socket_endpoint: impl Into<String>,
        parent_origin_chain: Option<&str>,
    ) -> bool {
        let session_id = self.id.clone();
        let Some(pane) = self.pane_mut(pane_id) else {
            return false;
        };
        let socket_endpoint = socket_endpoint.into();
        let origin = pane_origin_chain(parent_origin_chain, &pane.host.id);
        pane.host.command.env.retain(|(key, _)| {
            !matches!(
                key.as_str(),
                "NMUX" | "NMUX_SESSION_ID" | "NMUX_PANE_ID" | "NMUX_SOCKET" | "NMUX_ORIGIN"
            )
        });
        pane.host.command.env.extend([
            ("NMUX".to_owned(), "1".to_owned()),
            ("NMUX_SESSION_ID".to_owned(), session_id),
            ("NMUX_PANE_ID".to_owned(), pane_id.to_owned()),
            ("NMUX_SOCKET".to_owned(), socket_endpoint),
            ("NMUX_ORIGIN".to_owned(), origin),
        ]);
        true
    }

    pub fn active_pane_id(&self) -> Option<&str> {
        let tab = self.tabs.iter().find(|tab| tab.id == self.active_tab_id)?;
        self.pane_in_tab(tab, &tab.active_pane_id)
            .is_some()
            .then_some(tab.active_pane_id.as_str())
    }

    pub fn add_tab(
        &mut self,
        tab_id: impl Into<String>,
        title: impl Into<String>,
        pane_id: impl Into<String>,
        host: HostSpec,
    ) -> bool {
        let tab_id = tab_id.into();
        let title = title.into();
        let pane_id = pane_id.into();
        if tab_id.is_empty()
            || pane_id.is_empty()
            || self.tabs.iter().any(|tab| tab.id == tab_id)
            || self.pane(&pane_id).is_some()
        {
            return false;
        }
        let Some((cols, rows)) = self
            .tabs
            .iter()
            .find(|tab| tab.id == self.active_tab_id)
            .map(|tab| (tab.root.cols, tab.root.rows))
        else {
            return false;
        };

        self.tabs.push(Tab {
            id: tab_id,
            title,
            active_pane_id: pane_id.clone(),
            root: new_pane_from_template(&pane_id, host, cols, rows),
        });
        self.version = self.version.saturating_add(1);
        true
    }

    pub fn switch_tab(&mut self, tab_id: &str) -> bool {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
            return false;
        };
        if self.pane_in_tab(tab, &tab.active_pane_id).is_none() {
            return false;
        }
        if self.active_tab_id == tab_id {
            return false;
        }
        self.active_tab_id = tab_id.to_owned();
        self.version = self.version.saturating_add(1);
        true
    }

    pub fn close_tab(&mut self, tab_id: &str) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return false;
        };
        self.tabs.remove(index);
        if self.active_tab_id == tab_id {
            let next_index = index.min(self.tabs.len() - 1);
            self.active_tab_id = self.tabs[next_index].id.clone();
        }
        self.version = self.version.saturating_add(1);
        true
    }

    pub fn focus_pane(&mut self, pane_id: &str) -> bool {
        let Some(tab_index) = self
            .tabs
            .iter()
            .position(|tab| tab.id == self.active_tab_id)
        else {
            return false;
        };
        if self.pane_in_tab(&self.tabs[tab_index], pane_id).is_none() {
            return false;
        }
        if self.tabs[tab_index].active_pane_id == pane_id {
            return false;
        }
        self.tabs[tab_index].active_pane_id = pane_id.to_owned();
        self.version = self.version.saturating_add(1);
        true
    }

    pub fn split_active_pane(
        &mut self,
        axis: protocol::SplitAxis,
        new_pane_id: impl Into<String>,
        new_host: HostSpec,
    ) -> bool {
        let Some(pane_id) = self.active_pane_id().map(ToOwned::to_owned) else {
            return false;
        };
        self.split_pane(&pane_id, axis, new_pane_id, new_host)
    }

    pub fn split_pane(
        &mut self,
        pane_id: &str,
        axis: protocol::SplitAxis,
        new_pane_id: impl Into<String>,
        new_host: HostSpec,
    ) -> bool {
        if !matches!(
            axis,
            protocol::SplitAxis::Horizontal | protocol::SplitAxis::Vertical
        ) {
            return false;
        }
        let new_pane_id = new_pane_id.into();
        if new_pane_id.is_empty() || self.pane(&new_pane_id).is_some() {
            return false;
        }
        let Some(tab_index) = self
            .tabs
            .iter()
            .position(|tab| tab.id == self.active_tab_id)
        else {
            return false;
        };
        let split = {
            let tab = &mut self.tabs[tab_index];
            split_pane_node(&mut tab.root, pane_id, axis, &new_pane_id, new_host)
        };
        if !split {
            return false;
        }
        self.tabs[tab_index].active_pane_id = new_pane_id;
        self.version = self.version.saturating_add(1);
        true
    }

    pub fn resize_intent_allowed(
        policy: protocol::ResizePolicy,
        reason: protocol::ResizeReason,
    ) -> bool {
        match policy {
            protocol::ResizePolicy::Manual => reason == protocol::ResizeReason::UserCommand,
            _ => true,
        }
    }

    pub fn initial_actor(mode: AttachMode) -> Actor {
        Actor {
            id: "local-actor".to_owned(),
            user_id: "local-user".to_owned(),
            display_name: "local".to_owned(),
            mode,
            focused_pane_id: Some("pane-1".to_owned()),
        }
    }

    pub fn input_allowed(actor: &Actor) -> bool {
        actor.mode == AttachMode::ReadWrite
    }

    pub fn workspace_tree_frame(&self, connection_id: &str, seq: u64) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let mut tab_offsets = Vec::with_capacity(self.tabs.len());
        for tab in &self.tabs {
            let pane = build_pane_node(&mut builder, &tab.root);

            let tab_id = builder.create_string(&tab.id);
            let title = builder.create_string(&tab.title);
            let active_pane_id = builder.create_string(&tab.active_pane_id);
            let tab = protocol::TabNode::create(
                &mut builder,
                &protocol::TabNodeArgs {
                    tab_id: Some(tab_id),
                    title: Some(title),
                    root: Some(pane),
                    active_pane_id: Some(active_pane_id),
                },
            );
            tab_offsets.push(tab);
        }

        let tabs = builder.create_vector(&tab_offsets);
        let session_id = builder.create_string(&self.id);
        let active_tab_id = builder.create_string(&self.active_tab_id);
        let snapshot = protocol::WorkspaceTreeSnapshot::create(
            &mut builder,
            &protocol::WorkspaceTreeSnapshotArgs {
                version: self.version,
                session_id: Some(session_id),
                tabs: Some(tabs),
                active_tab_id: Some(active_tab_id),
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::WorkspaceTreeSnapshot,
                body: Some(snapshot.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    pub fn initial_pane_surface(&self) -> PaneSurface {
        self.pane_surface("pane-1").expect("initial pane exists")
    }

    pub fn pane_surface(&self, pane_id: &str) -> Option<PaneSurface> {
        let pane = self.pane(pane_id)?;
        Some(PaneSurface {
            pane_id: pane.id.clone(),
            version: pane.surface_version,
            cols: pane.cols,
            rows: pane.rows,
            surface: pane.surface,
            cursor: pane.cursor.clone(),
            modes: pane.modes,
            title: pane.terminal_title.clone(),
            working_directory: pane.terminal_working_directory.clone(),
            colors: pane.colors.clone(),
            styles: pane.styles.clone(),
            lines: pane.surface_lines.clone(),
            row_runs: row_runs_for_lines(&pane.surface_lines, &pane.surface_row_runs),
            semantic_prompts: row_semantic_prompts_for_lines(
                &pane.surface_lines,
                &pane.surface_semantic_prompts,
            ),
            dirty_rows: row_dirty_flags_for_lines(&pane.surface_lines, &pane.surface_dirty_rows),
            kitty_placeholders: row_kitty_placeholders_for_lines(
                &pane.surface_lines,
                &pane.surface_kitty_placeholders,
            ),
        })
    }

    pub fn initial_scrollback(&self) -> PaneScrollback {
        self.pane_scrollback("pane-1").expect("initial pane exists")
    }

    pub fn pane_scrollback(&self, pane_id: &str) -> Option<PaneScrollback> {
        let pane = self.pane(pane_id)?;
        Some(PaneScrollback {
            pane_id: pane.id.clone(),
            version: pane.scrollback_version,
            styles: pane.styles.clone(),
            colors: pane.colors.clone(),
            lines: pane.scrollback_lines.clone(),
            row_runs: row_runs_for_lines(&pane.scrollback_lines, &pane.scrollback_row_runs),
            semantic_prompts: row_semantic_prompts_for_lines(
                &pane.scrollback_lines,
                &pane.scrollback_semantic_prompts,
            ),
            dirty_rows: row_dirty_flags_for_lines(
                &pane.scrollback_lines,
                &pane.scrollback_dirty_rows,
            ),
            kitty_placeholders: row_kitty_placeholders_for_lines(
                &pane.scrollback_lines,
                &pane.scrollback_kitty_placeholders,
            ),
        })
    }

    pub fn surface_version(&self, pane_id: &str) -> Option<u64> {
        self.pane(pane_id).map(|pane| pane.surface_version)
    }

    pub fn scrollback_version(&self, pane_id: &str) -> Option<u64> {
        self.pane(pane_id).map(|pane| pane.scrollback_version)
    }

    pub fn surface_patch_kind(&self, pane_id: &str) -> Option<protocol::PatchKind> {
        self.pane(pane_id).map(|pane| pane.last_patch_kind)
    }

    pub fn pane_focus_reporting(&self, pane_id: &str) -> bool {
        self.pane(pane_id)
            .is_some_and(|pane| pane.modes.focus_reporting)
    }

    pub fn pane_bracketed_paste(&self, pane_id: &str) -> bool {
        self.pane(pane_id)
            .is_some_and(|pane| pane.modes.bracketed_paste)
    }

    pub fn pane_mouse_tracking(&self, pane_id: &str) -> bool {
        self.pane(pane_id)
            .is_some_and(|pane| pane.modes.mouse_tracking)
    }

    pub fn pane_mouse_tracking_mode(&self, pane_id: &str) -> Option<protocol::MouseTrackingMode> {
        self.pane(pane_id)
            .map(|pane| pane.modes.mouse_tracking_mode)
    }

    pub fn pane_mouse_format(&self, pane_id: &str) -> Option<protocol::MouseFormat> {
        self.pane(pane_id).map(|pane| pane.modes.mouse_format)
    }

    pub fn pane_size(&self, pane_id: &str) -> Option<(u32, u32)> {
        self.pane(pane_id).map(|pane| (pane.cols, pane.rows))
    }

    pub fn leaf_pane_ids(&self) -> Vec<String> {
        let mut pane_ids = Vec::new();
        for tab in &self.tabs {
            collect_leaf_pane_ids(&tab.root, &mut pane_ids);
        }
        pane_ids
    }

    pub fn pane_host(&self, pane_id: &str) -> Option<&HostSpec> {
        self.pane(pane_id).map(|pane| &pane.host)
    }

    pub fn pane_application_keypad(&self, pane_id: &str) -> bool {
        self.pane(pane_id)
            .is_some_and(|pane| pane.modes.application_keypad)
    }

    pub fn pane_application_cursor(&self, pane_id: &str) -> bool {
        self.pane(pane_id)
            .is_some_and(|pane| pane.modes.application_cursor)
    }

    fn pane(&self, pane_id: &str) -> Option<&Pane> {
        self.tabs
            .iter()
            .find_map(|tab| pane_node(&tab.root, pane_id))
    }

    fn pane_mut(&mut self, pane_id: &str) -> Option<&mut Pane> {
        self.tabs
            .iter_mut()
            .find_map(|tab| pane_node_mut(&mut tab.root, pane_id))
    }

    fn pane_in_tab<'a>(&self, tab: &'a Tab, pane_id: &str) -> Option<&'a Pane> {
        pane_node(&tab.root, pane_id)
    }

    pub fn pane_surface_frame(&self, connection_id: &str, seq: u64) -> Vec<u8> {
        self.pane_surface_frame_for_pane(connection_id, seq, "pane-1")
            .expect("initial pane exists")
    }

    pub fn pane_surface_frame_for_pane(
        &self,
        connection_id: &str,
        seq: u64,
        pane_id: &str,
    ) -> Option<Vec<u8>> {
        let surface = self.pane_surface(pane_id)?;
        let mut builder = FlatBufferBuilder::new();

        let mut row_offsets = Vec::with_capacity(surface.lines.len());
        for (row, (line, line_runs)) in surface
            .lines
            .iter()
            .zip(surface.row_runs.iter())
            .enumerate()
        {
            let runs = build_cell_runs(&mut builder, line_runs);
            let row_metadata = RowStateMetadata {
                semantic_prompt: surface
                    .semantic_prompts
                    .get(row)
                    .copied()
                    .unwrap_or(protocol::RowSemanticPrompt::None),
                dirty: surface.dirty_rows.get(row).copied().unwrap_or(false),
                kitty_virtual_placeholder: surface
                    .kitty_placeholders
                    .get(row)
                    .copied()
                    .unwrap_or(false),
            };
            let row = protocol::SurfaceRow::create(
                &mut builder,
                &protocol::SurfaceRowArgs {
                    row: row as u32,
                    runs: Some(runs),
                    dirty_hash: stable_row_hash(line),
                    row_state_hash: row_state_hash(line_runs, row_metadata),
                    semantic_prompt: row_metadata.semantic_prompt,
                    dirty: row_metadata.dirty,
                    kitty_virtual_placeholder: row_metadata.kitty_virtual_placeholder,
                },
            );
            row_offsets.push(row);
        }

        let rows_data = builder.create_vector(&row_offsets);
        let mut style_offsets = Vec::with_capacity(surface.styles.len());
        for style in &surface.styles {
            style_offsets.push(protocol::Style::create(
                &mut builder,
                &protocol::StyleArgs {
                    fg_rgba: style.fg_rgba,
                    bg_rgba: style.bg_rgba,
                    underline_rgba: style.underline_rgba,
                    flags: style.flags,
                },
            ));
        }
        let styles = builder.create_vector(&style_offsets);
        let hyperlinks = builder.create_vector::<flatbuffers::WIPOffset<protocol::Hyperlink>>(&[]);
        let cursor = protocol::CursorState::create(
            &mut builder,
            &protocol::CursorStateArgs {
                row: surface.cursor.row,
                col: surface.cursor.col,
                visible: surface.cursor.visible,
                shape: surface.cursor.shape,
                blinking: surface.cursor.blinking,
            },
        );
        let modes = build_terminal_modes(&mut builder, surface.modes);
        let metadata =
            build_terminal_metadata(&mut builder, &surface.title, &surface.working_directory);
        let colors = build_terminal_colors(&mut builder, &surface.colors, None, true);
        let pane_id = builder.create_string(&surface.pane_id);
        let snapshot = protocol::PaneSurfaceSnapshot::create(
            &mut builder,
            &protocol::PaneSurfaceSnapshotArgs {
                pane_id: Some(pane_id),
                version: surface.version,
                surface: surface.surface,
                cols: surface.cols,
                rows: surface.rows,
                cursor: Some(cursor),
                modes: Some(modes),
                metadata: Some(metadata),
                colors: Some(colors),
                styles: Some(styles),
                rows_data: Some(rows_data),
                hyperlinks: Some(hyperlinks),
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::PaneSurfaceSnapshot,
                body: Some(snapshot.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        Some(builder.finished_data().to_vec())
    }

    pub fn pane_surface_patch_frame(
        &self,
        connection_id: &str,
        seq: u64,
        base_version: u64,
    ) -> Vec<u8> {
        self.pane_surface_patch_frame_for_pane(connection_id, seq, "pane-1", base_version)
            .expect("initial pane exists")
    }

    pub fn pane_surface_patch_frame_for_pane(
        &self,
        connection_id: &str,
        seq: u64,
        pane_id: &str,
        base_version: u64,
    ) -> Option<Vec<u8>> {
        let surface = self.pane_surface(pane_id)?;
        let patch_kind = self
            .pane(&surface.pane_id)
            .map(|pane| pane.last_patch_kind)
            .unwrap_or(protocol::PatchKind::ReplaceRows);
        let row_update_indices = self
            .pane(&surface.pane_id)
            .map(|pane| pane.last_row_update_indices.as_slice())
            .unwrap_or(&[]);
        let mut builder = FlatBufferBuilder::new();

        let mut row_offsets = Vec::new();
        if patch_kind == protocol::PatchKind::ReplaceRows {
            row_offsets.reserve(row_update_indices.len());
            for row in row_update_indices {
                let row = *row as usize;
                let line = surface.lines.get(row).map_or("", String::as_str);
                let fallback_runs;
                let line_runs = if let Some(line_runs) = surface.row_runs.get(row) {
                    line_runs.as_slice()
                } else {
                    fallback_runs = vec![CellRun::plain(line)];
                    fallback_runs.as_slice()
                };
                let runs = build_cell_runs(&mut builder, line_runs);
                let row_metadata = RowStateMetadata {
                    semantic_prompt: surface
                        .semantic_prompts
                        .get(row)
                        .copied()
                        .unwrap_or(protocol::RowSemanticPrompt::None),
                    dirty: surface.dirty_rows.get(row).copied().unwrap_or(false),
                    kitty_virtual_placeholder: surface
                        .kitty_placeholders
                        .get(row)
                        .copied()
                        .unwrap_or(false),
                };
                let row_update = protocol::RowUpdate::create(
                    &mut builder,
                    &protocol::RowUpdateArgs {
                        row: row as u32,
                        runs: Some(runs),
                        dirty_hash: stable_row_hash(line),
                        row_state_hash: row_state_hash(line_runs, row_metadata),
                        semantic_prompt: row_metadata.semantic_prompt,
                        dirty: row_metadata.dirty,
                        kitty_virtual_placeholder: row_metadata.kitty_virtual_placeholder,
                    },
                );
                row_offsets.push(row_update);
            }
        }

        let row_updates = builder.create_vector(&row_offsets);
        let cursor = protocol::CursorState::create(
            &mut builder,
            &protocol::CursorStateArgs {
                row: surface.cursor.row,
                col: surface.cursor.col,
                visible: surface.cursor.visible,
                shape: surface.cursor.shape,
                blinking: surface.cursor.blinking,
            },
        );
        let modes = build_terminal_modes(&mut builder, surface.modes);
        let metadata =
            build_terminal_metadata(&mut builder, &surface.title, &surface.working_directory);
        let palette_diff = self
            .pane(&surface.pane_id)
            .and_then(|pane| pane.last_palette_diff.as_ref())
            .filter(|_| patch_kind == protocol::PatchKind::ColorOnly);
        let include_full_palette = patch_kind != protocol::PatchKind::ColorOnly;
        let colors = build_terminal_colors(
            &mut builder,
            &surface.colors,
            palette_diff,
            include_full_palette,
        );
        let pane_id = builder.create_string(&surface.pane_id);
        let patch = protocol::PaneSurfacePatch::create(
            &mut builder,
            &protocol::PaneSurfacePatchArgs {
                pane_id: Some(pane_id),
                base_version,
                version: surface.version,
                kind: patch_kind,
                row_updates: Some(row_updates),
                cursor: Some(cursor),
                modes: Some(modes),
                metadata: Some(metadata),
                colors: Some(colors),
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::PaneSurfacePatch,
                body: Some(patch.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        Some(builder.finished_data().to_vec())
    }

    pub fn scrollback_chunk_frame(
        &self,
        connection_id: &str,
        seq: u64,
        start_line: u64,
        line_count: u32,
    ) -> Vec<u8> {
        self.scrollback_chunk_frame_for_pane(connection_id, seq, "pane-1", start_line, line_count)
            .expect("initial pane exists")
    }

    pub fn scrollback_chunk_frame_for_pane(
        &self,
        connection_id: &str,
        seq: u64,
        pane_id: &str,
        start_line: u64,
        line_count: u32,
    ) -> Option<Vec<u8>> {
        if start_line == 0 || line_count == 0 {
            return None;
        }
        let scrollback = self.pane_scrollback(pane_id)?;
        let mut builder = FlatBufferBuilder::new();

        let start = usize::try_from(start_line.saturating_sub(1)).unwrap_or(usize::MAX);
        let count = line_count as usize;
        let end = start.saturating_add(count).min(scrollback.lines.len());
        let selected = scrollback
            .lines
            .get(start..end)
            .map_or(&[][..], |rows| rows);

        let mut row_offsets = Vec::with_capacity(selected.len());
        for (offset, line) in selected.iter().enumerate() {
            let row_index = start.saturating_add(offset);
            let runs = build_cell_runs(&mut builder, &scrollback.row_runs[row_index]);
            let row_metadata = RowStateMetadata {
                semantic_prompt: scrollback
                    .semantic_prompts
                    .get(row_index)
                    .copied()
                    .unwrap_or(protocol::RowSemanticPrompt::None),
                dirty: scrollback
                    .dirty_rows
                    .get(row_index)
                    .copied()
                    .unwrap_or(false),
                kitty_virtual_placeholder: scrollback
                    .kitty_placeholders
                    .get(row_index)
                    .copied()
                    .unwrap_or(false),
            };
            let row = protocol::ScrollbackRow::create(
                &mut builder,
                &protocol::ScrollbackRowArgs {
                    line: start_line + offset as u64,
                    runs: Some(runs),
                    dirty_hash: stable_row_hash(line),
                    row_state_hash: row_state_hash(&scrollback.row_runs[row_index], row_metadata),
                    semantic_prompt: row_metadata.semantic_prompt,
                    dirty: row_metadata.dirty,
                    kitty_virtual_placeholder: row_metadata.kitty_virtual_placeholder,
                },
            );
            row_offsets.push(row);
        }

        let rows = builder.create_vector(&row_offsets);
        let mut style_offsets = Vec::with_capacity(scrollback.styles.len());
        for style in &scrollback.styles {
            style_offsets.push(protocol::Style::create(
                &mut builder,
                &protocol::StyleArgs {
                    fg_rgba: style.fg_rgba,
                    bg_rgba: style.bg_rgba,
                    underline_rgba: style.underline_rgba,
                    flags: style.flags,
                },
            ));
        }
        let styles = builder.create_vector(&style_offsets);
        let hyperlinks = builder.create_vector::<flatbuffers::WIPOffset<protocol::Hyperlink>>(&[]);
        let colors = build_terminal_colors(&mut builder, &scrollback.colors, None, true);
        let pane_id = builder.create_string(&scrollback.pane_id);
        let chunk = protocol::ScrollbackChunk::create(
            &mut builder,
            &protocol::ScrollbackChunkArgs {
                pane_id: Some(pane_id),
                scrollback_version: scrollback.version,
                start_line,
                total_lines: scrollback.lines.len() as u64,
                rows: Some(rows),
                styles: Some(styles),
                colors: Some(colors),
                hyperlinks: Some(hyperlinks),
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::ScrollbackChunk,
                body: Some(chunk.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        Some(builder.finished_data().to_vec())
    }

    pub fn scrollback_fetch_frame(
        &self,
        context: InputFrameContext<'_>,
        fetch_spec: ScrollbackFetchSpec,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let pane_id = builder.create_string(context.pane_id);
        let actor_id = builder.create_string(context.actor_id);
        let fetch = protocol::ScrollbackFetch::create(
            &mut builder,
            &protocol::ScrollbackFetchArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                start_line: fetch_spec.range.start_line,
                line_count: fetch_spec.range.line_count,
                known_scrollback_version: fetch_spec.known_scrollback_version,
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(context.connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq: context.seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::ScrollbackFetch,
                body: Some(fetch.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    pub fn presence_update_frame(&self, connection_id: &str, seq: u64, actor: &Actor) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let actor_id = builder.create_string(&actor.id);
        let user_id = builder.create_string(&actor.user_id);
        let display_name = builder.create_string(&actor.display_name);
        let focused_pane_id = actor
            .focused_pane_id
            .as_ref()
            .map(|pane_id| builder.create_string(pane_id));
        let presence = protocol::PresenceUpdate::create(
            &mut builder,
            &protocol::PresenceUpdateArgs {
                actor_id: Some(actor_id),
                user_id: Some(user_id),
                display_name: Some(display_name),
                mode: actor.mode.as_protocol(),
                kind: protocol::PresenceKind::Joined,
                focused_pane_id,
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::PresenceUpdate,
                body: Some(presence.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    pub fn attach_status_frame(
        &self,
        connection_id: &str,
        seq: u64,
        pane_id: &str,
        surface_state: protocol::AttachSurfaceState,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let pane_id_offset = builder.create_string(pane_id);
        let status = protocol::AttachStatus::create(
            &mut builder,
            &protocol::AttachStatusArgs {
                pane_id: Some(pane_id_offset),
                surface_version: self.surface_version(pane_id).unwrap_or_default(),
                surface_state,
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::AttachStatus,
                body: Some(status.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    pub fn error_frame(
        &self,
        connection_id: &str,
        seq: u64,
        code: protocol::ErrorCode,
        message: &str,
        retryability: ErrorRetryability,
    ) -> Vec<u8> {
        self.error_frame_with_context(connection_id, seq, code, message, retryability, None, 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn error_frame_with_context(
        &self,
        connection_id: &str,
        seq: u64,
        code: protocol::ErrorCode,
        message: &str,
        retryability: ErrorRetryability,
        pane_id: Option<&str>,
        input_seq: u64,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let message = builder.create_string(message);
        let pane_id = pane_id.map(|pane_id| builder.create_string(pane_id));
        let error = protocol::Error::create(
            &mut builder,
            &protocol::ErrorArgs {
                code,
                message: Some(message),
                retryable: retryability.as_bool(),
                pane_id,
                input_seq,
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::Error,
                body: Some(error.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    pub fn key_input_frame(
        &self,
        connection_id: &str,
        seq: u64,
        actor_id: &str,
        pane_id: &str,
        input_seq: u64,
        text: &str,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let text = builder.create_string(text);
        let key = protocol::KeyInput::create(
            &mut builder,
            &protocol::KeyInputArgs {
                text_utf8: Some(text),
                key_name: None,
                modifiers: 0,
            },
        );
        let pane_id = builder.create_string(pane_id);
        let actor_id = builder.create_string(actor_id);
        let input = protocol::InputEvent::create(
            &mut builder,
            &protocol::InputEventArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                input_seq,
                kind: protocol::InputKind::Key,
                key: Some(key),
                mouse: None,
                paste: None,
                raw: None,
                focus: None,
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::InputEvent,
                body: Some(input.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    pub fn named_key_input_frame(
        &self,
        connection_id: &str,
        seq: u64,
        actor_id: &str,
        pane_id: &str,
        input_seq: u64,
        key_name: &str,
    ) -> Vec<u8> {
        self.named_key_input_frame_with_modifiers(
            connection_id,
            seq,
            actor_id,
            pane_id,
            input_seq,
            key_name,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn named_key_input_frame_with_modifiers(
        &self,
        connection_id: &str,
        seq: u64,
        actor_id: &str,
        pane_id: &str,
        input_seq: u64,
        key_name: &str,
        modifiers: u32,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let key_name = builder.create_string(key_name);
        let key = protocol::KeyInput::create(
            &mut builder,
            &protocol::KeyInputArgs {
                text_utf8: None,
                key_name: Some(key_name),
                modifiers,
            },
        );
        let pane_id = builder.create_string(pane_id);
        let actor_id = builder.create_string(actor_id);
        let input = protocol::InputEvent::create(
            &mut builder,
            &protocol::InputEventArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                input_seq,
                kind: protocol::InputKind::Key,
                key: Some(key),
                mouse: None,
                paste: None,
                raw: None,
                focus: None,
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::InputEvent,
                body: Some(input.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    pub fn raw_input_frame(
        &self,
        connection_id: &str,
        seq: u64,
        actor_id: &str,
        pane_id: &str,
        input_seq: u64,
        bytes: &[u8],
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let bytes = builder.create_vector(bytes);
        let raw = protocol::RawInput::create(
            &mut builder,
            &protocol::RawInputArgs { bytes: Some(bytes) },
        );
        let pane_id = builder.create_string(pane_id);
        let actor_id = builder.create_string(actor_id);
        let input = protocol::InputEvent::create(
            &mut builder,
            &protocol::InputEventArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                input_seq,
                kind: protocol::InputKind::RawBytes,
                key: None,
                mouse: None,
                paste: None,
                raw: Some(raw),
                focus: None,
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::InputEvent,
                body: Some(input.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    pub fn paste_input_frame(
        &self,
        context: InputFrameContext<'_>,
        paste_input: PasteInputSpec<'_>,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let text = builder.create_string(paste_input.text);
        let paste = protocol::PasteInput::create(
            &mut builder,
            &protocol::PasteInputArgs {
                text_utf8: Some(text),
                bracketed: paste_input.bracketed,
            },
        );
        let pane_id = builder.create_string(context.pane_id);
        let actor_id = builder.create_string(context.actor_id);
        let input = protocol::InputEvent::create(
            &mut builder,
            &protocol::InputEventArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                input_seq: context.input_seq,
                kind: protocol::InputKind::Paste,
                key: None,
                mouse: None,
                paste: Some(paste),
                raw: None,
                focus: None,
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(context.connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq: context.seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::InputEvent,
                body: Some(input.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    pub fn focus_input_frame(
        &self,
        context: InputFrameContext<'_>,
        focus_input: FocusInputSpec,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let focus = protocol::FocusInput::create(
            &mut builder,
            &protocol::FocusInputArgs {
                focused: focus_input.focused,
            },
        );
        let pane_id = builder.create_string(context.pane_id);
        let actor_id = builder.create_string(context.actor_id);
        let input = protocol::InputEvent::create(
            &mut builder,
            &protocol::InputEventArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                input_seq: context.input_seq,
                kind: protocol::InputKind::Focus,
                key: None,
                mouse: None,
                paste: None,
                raw: None,
                focus: Some(focus),
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(context.connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq: context.seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::InputEvent,
                body: Some(input.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    pub fn mouse_input_frame(
        &self,
        context: InputFrameContext<'_>,
        mouse_input: MouseInputSpec,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let has_pixels = mouse_input.pixel_x.is_some() && mouse_input.pixel_y.is_some();

        let mouse = protocol::MouseInput::create(
            &mut builder,
            &protocol::MouseInputArgs {
                row: mouse_input.row,
                col: mouse_input.col,
                button: mouse_input.button,
                modifiers: mouse_input.modifiers,
                action: mouse_input.action,
                has_pixels,
                pixel_x: mouse_input.pixel_x.unwrap_or_default(),
                pixel_y: mouse_input.pixel_y.unwrap_or_default(),
            },
        );
        let pane_id = builder.create_string(context.pane_id);
        let actor_id = builder.create_string(context.actor_id);
        let input = protocol::InputEvent::create(
            &mut builder,
            &protocol::InputEventArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                input_seq: context.input_seq,
                kind: protocol::InputKind::Mouse,
                key: None,
                mouse: Some(mouse),
                paste: None,
                raw: None,
                focus: None,
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(context.connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq: context.seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::InputEvent,
                body: Some(input.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resize_intent_frame(
        &self,
        connection_id: &str,
        seq: u64,
        actor_id: &str,
        pane_id: &str,
        cols: u32,
        rows: u32,
        reason: protocol::ResizeReason,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let pane_id = builder.create_string(pane_id);
        let actor_id = builder.create_string(actor_id);
        let resize = protocol::ResizeIntent::create(
            &mut builder,
            &protocol::ResizeIntentArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                desired_cols: cols,
                desired_rows: rows,
                reason,
            },
        );

        let envelope_session_id = builder.create_string(&self.id);
        let connection_id = builder.create_string(connection_id);
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(envelope_session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::ResizeIntent,
                body: Some(resize.as_union_value()),
            },
        );

        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }
}

fn pane_origin_chain(parent_origin_chain: Option<&str>, origin: &str) -> String {
    match parent_origin_chain
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(parent_origin_chain) => format!("{parent_origin_chain}>{origin}"),
        None => origin.to_owned(),
    }
}

fn pane_node<'a>(pane: &'a Pane, pane_id: &str) -> Option<&'a Pane> {
    if pane.id == pane_id {
        return Some(pane);
    }
    pane.children
        .iter()
        .find_map(|child| pane_node(child, pane_id))
}

fn pane_node_mut<'a>(pane: &'a mut Pane, pane_id: &str) -> Option<&'a mut Pane> {
    if pane.id == pane_id {
        return Some(pane);
    }
    pane.children
        .iter_mut()
        .find_map(|child| pane_node_mut(child, pane_id))
}

fn collect_leaf_pane_ids(pane: &Pane, pane_ids: &mut Vec<String>) {
    if pane.children.is_empty() {
        pane_ids.push(pane.id.clone());
        return;
    }
    for child in &pane.children {
        collect_leaf_pane_ids(child, pane_ids);
    }
}

fn split_pane_node(
    pane: &mut Pane,
    pane_id: &str,
    axis: protocol::SplitAxis,
    new_pane_id: &str,
    new_host: HostSpec,
) -> bool {
    if pane.id == pane_id {
        if !pane.children.is_empty() {
            return false;
        }
        let (first_cols, first_rows, second_cols, second_rows) =
            match split_child_sizes(pane.cols, pane.rows, axis) {
                Some(sizes) => sizes,
                None => return false,
            };
        let mut existing = pane.clone();
        existing.cols = first_cols;
        existing.rows = first_rows;
        existing.host.command.initial_size = Some((first_cols, first_rows));
        existing.surface_version = existing.surface_version.saturating_add(1);
        existing.last_patch_kind = protocol::PatchKind::FullRefreshRequired;
        existing.last_row_update_indices = all_row_indices(existing.surface_lines.len());
        existing.last_palette_diff = None;

        let new_pane = new_pane_from_template(new_pane_id, new_host, second_cols, second_rows);
        pane.id = format!("split-{pane_id}-{new_pane_id}");
        pane.split_axis = axis;
        pane.children = vec![existing, new_pane];
        pane.cols = first_cols.saturating_add(second_cols);
        pane.rows = first_rows.max(second_rows);
        if axis == protocol::SplitAxis::Horizontal {
            pane.cols = first_cols.max(second_cols);
            pane.rows = first_rows.saturating_add(second_rows);
        }
        pane.surface_version = 0;
        pane.last_patch_kind = protocol::PatchKind::FullRefreshRequired;
        pane.last_row_update_indices.clear();
        pane.last_palette_diff = None;
        return true;
    }

    pane.children
        .iter_mut()
        .any(|child| split_pane_node(child, pane_id, axis, new_pane_id, new_host.clone()))
}

fn split_child_sizes(
    cols: u32,
    rows: u32,
    axis: protocol::SplitAxis,
) -> Option<(u32, u32, u32, u32)> {
    match axis {
        protocol::SplitAxis::Horizontal if rows >= 2 => {
            let first_rows = rows / 2;
            Some((cols, first_rows, cols, rows - first_rows))
        }
        protocol::SplitAxis::Vertical if cols >= 2 => {
            let first_cols = cols / 2;
            Some((first_cols, rows, cols - first_cols, rows))
        }
        _ => None,
    }
}

fn new_pane_from_template(id: &str, host: HostSpec, cols: u32, rows: u32) -> Pane {
    let mut host = host;
    host.id = id.to_owned();
    host.command.initial_size = Some((cols, rows));
    let surface_lines = vec![
        format!("nmux {id}"),
        "server-owned terminal state".to_owned(),
    ];
    let scrollback_lines = vec![
        "booting nmux workspace".to_owned(),
        format!("nmux {id}"),
        "server-owned terminal state".to_owned(),
    ];
    let surface_row_count = surface_lines.len();
    let scrollback_row_count = scrollback_lines.len();
    Pane {
        id: id.to_owned(),
        split_axis: protocol::SplitAxis::None,
        children: Vec::new(),
        host,
        surface_version: 1,
        last_patch_kind: protocol::PatchKind::ReplaceRows,
        last_row_update_indices: all_row_indices(surface_row_count),
        scrollback_version: 1,
        cols,
        rows,
        resize_policy: protocol::ResizePolicy::Fixed,
        surface: protocol::SurfaceKind::Main,
        cursor: Cursor {
            row: 1,
            col: 0,
            visible: true,
            shape: protocol::CursorShape::Block,
            blinking: true,
        },
        modes: TerminalModes::default(),
        terminal_title: String::new(),
        terminal_working_directory: String::new(),
        colors: TerminalColors::default(),
        last_palette_diff: None,
        styles: vec![PaneStyle::default()],
        surface_lines,
        surface_row_runs: vec![
            vec![CellRun::plain(format!("nmux {id}"))],
            vec![CellRun::plain("server-owned terminal state")],
        ],
        surface_semantic_prompts: vec![protocol::RowSemanticPrompt::None; surface_row_count],
        surface_dirty_rows: vec![false; surface_row_count],
        surface_kitty_placeholders: vec![false; surface_row_count],
        scrollback_lines,
        scrollback_row_runs: vec![
            vec![CellRun::plain("booting nmux workspace")],
            vec![CellRun::plain(format!("nmux {id}"))],
            vec![CellRun::plain("server-owned terminal state")],
        ],
        scrollback_semantic_prompts: vec![protocol::RowSemanticPrompt::None; scrollback_row_count],
        scrollback_dirty_rows: vec![false; scrollback_row_count],
        scrollback_kitty_placeholders: vec![false; scrollback_row_count],
    }
}

fn build_pane_node<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    pane: &Pane,
) -> flatbuffers::WIPOffset<protocol::PaneNode<'a>> {
    let children = if pane.children.is_empty() {
        None
    } else {
        let mut child_offsets = Vec::with_capacity(pane.children.len());
        for child in &pane.children {
            child_offsets.push(build_pane_node(builder, child));
        }
        Some(builder.create_vector(&child_offsets))
    };
    let pane_id = builder.create_string(&pane.id);
    protocol::PaneNode::create(
        builder,
        &protocol::PaneNodeArgs {
            pane_id: Some(pane_id),
            kind: protocol::PaneKind::Pty,
            split_axis: pane.split_axis,
            children,
            surface_version: pane.surface_version,
            cols: pane.cols,
            rows: pane.rows,
            resize_policy: pane.resize_policy,
        },
    )
}

impl From<&Cursor> for TerminalCursor {
    fn from(cursor: &Cursor) -> Self {
        Self {
            row: cursor.row,
            col: cursor.col,
            visible: cursor.visible,
            shape: cursor.shape,
            blinking: cursor.blinking,
        }
    }
}

impl From<TerminalCursor> for Cursor {
    fn from(cursor: TerminalCursor) -> Self {
        Self {
            row: cursor.row,
            col: cursor.col,
            visible: cursor.visible,
            shape: cursor.shape,
            blinking: cursor.blinking,
        }
    }
}

fn build_terminal_modes<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    modes: TerminalModes,
) -> flatbuffers::WIPOffset<protocol::TerminalModeState<'a>> {
    protocol::TerminalModeState::create(
        builder,
        &protocol::TerminalModeStateArgs {
            bracketed_paste: modes.bracketed_paste,
            mouse_tracking: modes.mouse_tracking,
            focus_reporting: modes.focus_reporting,
            application_keypad: modes.application_keypad,
            application_cursor: modes.application_cursor,
            origin: modes.origin,
            wraparound: modes.wraparound,
            mouse_tracking_mode: modes.mouse_tracking_mode,
            mouse_format: modes.mouse_format,
        },
    )
}

fn build_terminal_metadata<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    title: &str,
    working_directory: &str,
) -> flatbuffers::WIPOffset<protocol::TerminalMetadataState<'a>> {
    let title = builder.create_string(title);
    let working_directory = builder.create_string(working_directory);
    protocol::TerminalMetadataState::create(
        builder,
        &protocol::TerminalMetadataStateArgs {
            title: Some(title),
            working_directory: Some(working_directory),
        },
    )
}

fn build_terminal_colors<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    colors: &TerminalColors,
    palette_diff: Option<&PaletteDiff>,
    include_full_palette: bool,
) -> flatbuffers::WIPOffset<protocol::TerminalColorState<'a>> {
    let palette_rgba = include_full_palette.then(|| builder.create_vector(&colors.palette_rgba));
    let palette_diff_rgba =
        palette_diff.map(|palette_diff| builder.create_vector(&palette_diff.colors));
    protocol::TerminalColorState::create(
        builder,
        &protocol::TerminalColorStateArgs {
            default_fg_rgba: colors.default_fg_rgba,
            default_bg_rgba: colors.default_bg_rgba,
            cursor_rgba: colors.cursor_rgba,
            cursor_rgba_set: colors.cursor_rgba_set,
            palette_rgba,
            palette_diff_start: palette_diff.map_or(0, |palette_diff| palette_diff.start),
            palette_diff_rgba,
        },
    )
}

fn apply_terminal_update(
    pane: &mut Pane,
    update: crate::terminal::TerminalUpdate,
    force_surface_version: bool,
) -> bool {
    let cursor = Cursor::from(update.cursor);
    let surface_row_runs = row_runs_for_lines(&update.surface_lines, &update.surface_row_runs);
    let surface_semantic_prompts =
        row_semantic_prompts_for_lines(&update.surface_lines, &update.surface_semantic_prompts);
    let surface_dirty_rows =
        row_dirty_flags_for_lines(&update.surface_lines, &update.surface_dirty_rows);
    let surface_kitty_placeholders =
        row_kitty_placeholders_for_lines(&update.surface_lines, &update.surface_kitty_placeholders);
    let scrollback_row_runs =
        row_runs_for_lines(&update.scrollback_lines, &update.scrollback_row_runs);
    let scrollback_semantic_prompts = row_semantic_prompts_for_lines(
        &update.scrollback_lines,
        &update.scrollback_semantic_prompts,
    );
    let scrollback_dirty_rows =
        row_dirty_flags_for_lines(&update.scrollback_lines, &update.scrollback_dirty_rows);
    let scrollback_kitty_placeholders = row_kitty_placeholders_for_lines(
        &update.scrollback_lines,
        &update.scrollback_kitty_placeholders,
    );
    let modes_changed = pane.modes != update.modes;
    let title_changed = pane.terminal_title != update.title;
    let working_directory_changed = pane.terminal_working_directory != update.working_directory;
    let colors_changed = pane.colors != update.colors;
    let palette_diff = palette_diff(&pane.colors.palette_rgba, &update.colors.palette_rgba);
    let updated_surface_row_count = update.surface_lines.len().min(pane.rows as usize);
    let row_count_changed =
        force_surface_version && pane.surface_lines.len() != updated_surface_row_count;
    let rows_changed = pane.surface_lines != update.surface_lines;
    let surface_kind_changed = pane.surface != update.surface;
    let styles_changed = pane.styles != update.styles;
    let cursor_changed = pane.cursor != cursor;
    let row_runs_changed = pane.surface_row_runs != surface_row_runs;
    let semantic_prompts_changed = pane.surface_semantic_prompts != surface_semantic_prompts;
    let dirty_rows_changed = pane.surface_dirty_rows != surface_dirty_rows;
    let kitty_placeholders_changed = pane.surface_kitty_placeholders != surface_kitty_placeholders;
    let row_update_indices = surface_row_update_indices(
        pane,
        &update.surface_lines,
        &surface_row_runs,
        &surface_semantic_prompts,
        &surface_dirty_rows,
        &surface_kitty_placeholders,
        force_surface_version,
    );
    let surface_changed = force_surface_version
        || surface_kind_changed
        || styles_changed
        || rows_changed
        || row_runs_changed
        || semantic_prompts_changed
        || dirty_rows_changed
        || kitty_placeholders_changed
        || cursor_changed
        || modes_changed
        || title_changed
        || working_directory_changed
        || colors_changed;
    let scrollback_changed = pane.scrollback_lines != update.scrollback_lines
        || pane.scrollback_row_runs != scrollback_row_runs
        || pane.scrollback_semantic_prompts != scrollback_semantic_prompts
        || pane.scrollback_dirty_rows != scrollback_dirty_rows
        || pane.scrollback_kitty_placeholders != scrollback_kitty_placeholders;

    pane.scrollback_lines = update.scrollback_lines;
    pane.scrollback_row_runs = scrollback_row_runs;
    pane.scrollback_semantic_prompts = scrollback_semantic_prompts;
    pane.scrollback_dirty_rows = scrollback_dirty_rows;
    pane.scrollback_kitty_placeholders = scrollback_kitty_placeholders;
    pane.surface = update.surface;
    pane.modes = update.modes;
    pane.terminal_title = update.title;
    pane.terminal_working_directory = update.working_directory;
    pane.colors = update.colors;
    pane.styles = update.styles;
    pane.surface_lines = update.surface_lines;
    pane.surface_row_runs = surface_row_runs;
    pane.surface_semantic_prompts = surface_semantic_prompts;
    pane.surface_dirty_rows = surface_dirty_rows;
    pane.surface_kitty_placeholders = surface_kitty_placeholders;

    // The terminal engine can return more rows than the declared pane size
    // (e.g., 25 lines for a 24-row pane). Truncate all parallel surface
    // vectors so serialized row indices never exceed `pane.rows - 1`.
    let max_rows = pane.rows as usize;
    pane.surface_lines.truncate(max_rows);
    pane.surface_row_runs.truncate(max_rows);
    pane.surface_semantic_prompts.truncate(max_rows);
    pane.surface_dirty_rows.truncate(max_rows);
    pane.surface_kitty_placeholders.truncate(max_rows);
    pane.cursor = cursor;

    if scrollback_changed {
        pane.scrollback_version = pane.scrollback_version.saturating_add(1);
    }

    if surface_changed {
        pane.surface_version = pane.surface_version.saturating_add(1);
        let patch_kind = terminal_patch_kind(TerminalPatchChanges {
            force_full_refresh: force_surface_version,
            requested: update.patch_kind,
            row_count_changed,
            rows_changed,
            row_runs_changed,
            semantic_prompts_changed,
            dirty_rows_changed,
            kitty_placeholders_changed,
            surface_kind_changed,
            styles_changed,
            cursor_changed,
            modes_changed,
            title_changed,
            working_directory_changed,
            colors_changed,
        });
        pane.last_patch_kind = patch_kind;
        pane.last_palette_diff = if patch_kind == protocol::PatchKind::ColorOnly {
            palette_diff
        } else {
            None
        };
        pane.last_row_update_indices = if pane.last_patch_kind == protocol::PatchKind::ReplaceRows {
            row_update_indices
                .into_iter()
                .filter(|row| (*row as usize) < max_rows)
                .collect()
        } else {
            Vec::new()
        };
    } else {
        pane.last_palette_diff = None;
    }

    surface_changed || scrollback_changed
}

fn palette_diff(old: &[u32], new: &[u32]) -> Option<PaletteDiff> {
    if old == new {
        return None;
    }
    let start = old
        .iter()
        .zip(new.iter())
        .position(|(old, new)| old != new)
        .unwrap_or_else(|| old.len().min(new.len()));
    Some(PaletteDiff {
        start: start as u32,
        colors: new[start..].to_vec(),
    })
}

#[derive(Debug, Clone, Copy)]
struct TerminalPatchChanges {
    force_full_refresh: bool,
    requested: protocol::PatchKind,
    row_count_changed: bool,
    rows_changed: bool,
    row_runs_changed: bool,
    semantic_prompts_changed: bool,
    dirty_rows_changed: bool,
    kitty_placeholders_changed: bool,
    surface_kind_changed: bool,
    styles_changed: bool,
    cursor_changed: bool,
    modes_changed: bool,
    title_changed: bool,
    working_directory_changed: bool,
    colors_changed: bool,
}

fn terminal_patch_kind(changes: TerminalPatchChanges) -> protocol::PatchKind {
    let rows_or_row_metadata_changed = changes.rows_changed
        || changes.row_runs_changed
        || changes.semantic_prompts_changed
        || changes.dirty_rows_changed
        || changes.kitty_placeholders_changed;
    if changes.force_full_refresh
        || changes.row_count_changed
        || changes.surface_kind_changed
        || changes.styles_changed
        || (changes.colors_changed && rows_or_row_metadata_changed)
    {
        protocol::PatchKind::FullRefreshRequired
    } else if rows_or_row_metadata_changed {
        protocol::PatchKind::ReplaceRows
    } else if changes.colors_changed {
        protocol::PatchKind::ColorOnly
    } else if changes.modes_changed {
        protocol::PatchKind::ModeOnly
    } else if changes.cursor_changed || changes.title_changed || changes.working_directory_changed {
        protocol::PatchKind::CursorOnly
    } else {
        changes.requested
    }
}

fn surface_row_update_indices(
    pane: &Pane,
    lines: &[String],
    row_runs: &[Vec<CellRun>],
    semantic_prompts: &[protocol::RowSemanticPrompt],
    dirty_rows: &[bool],
    kitty_placeholders: &[bool],
    force_all: bool,
) -> Vec<u32> {
    if force_all {
        return all_row_indices(pane.rows as usize);
    }
    if pane.surface_lines.len() != lines.len() {
        return all_row_indices(lines.len());
    }

    let mut changed = Vec::new();
    for row in 0..lines.len() {
        if pane.surface_lines.get(row) != lines.get(row)
            || pane.surface_row_runs.get(row) != row_runs.get(row)
            || pane.surface_semantic_prompts.get(row) != semantic_prompts.get(row)
            || pane.surface_dirty_rows.get(row) != dirty_rows.get(row)
            || pane.surface_kitty_placeholders.get(row) != kitty_placeholders.get(row)
        {
            changed.push(row as u32);
        }
    }
    changed
}

fn all_row_indices(len: usize) -> Vec<u32> {
    (0..len).map(|row| row as u32).collect()
}

fn stable_row_hash(line: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in line.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RowStateMetadata {
    semantic_prompt: protocol::RowSemanticPrompt,
    dirty: bool,
    kitty_virtual_placeholder: bool,
}

fn row_state_hash(runs: &[CellRun], metadata: RowStateMetadata) -> u64 {
    let mut hasher = StableHasher::new();
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

struct StableHasher(u64);

impl StableHasher {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325_u64)
    }
}

impl Hasher for StableHasher {
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

fn row_runs_for_lines(lines: &[String], row_runs: &[Vec<CellRun>]) -> Vec<Vec<CellRun>> {
    if row_runs.len() == lines.len()
        && row_runs
            .iter()
            .zip(lines)
            .all(|(runs, line)| cell_runs_text(runs) == *line)
    {
        row_runs.to_vec()
    } else {
        lines
            .iter()
            .map(|line| vec![CellRun::plain(line.clone())])
            .collect()
    }
}

fn row_semantic_prompts_for_lines(
    lines: &[String],
    semantic_prompts: &[protocol::RowSemanticPrompt],
) -> Vec<protocol::RowSemanticPrompt> {
    if semantic_prompts.len() == lines.len() {
        semantic_prompts.to_vec()
    } else {
        crate::terminal::plain_row_semantic_prompts(lines)
    }
}

fn row_dirty_flags_for_lines(lines: &[String], dirty_rows: &[bool]) -> Vec<bool> {
    if dirty_rows.len() == lines.len() {
        dirty_rows.to_vec()
    } else {
        crate::terminal::plain_row_dirty_flags(lines)
    }
}

fn row_kitty_placeholders_for_lines(lines: &[String], kitty_placeholders: &[bool]) -> Vec<bool> {
    if kitty_placeholders.len() == lines.len() {
        kitty_placeholders.to_vec()
    } else {
        crate::terminal::plain_row_kitty_placeholders(lines)
    }
}

fn cell_runs_text(runs: &[CellRun]) -> String {
    crate::terminal::cell_runs_text(runs)
}

fn build_cell_runs<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    runs: &[CellRun],
) -> flatbuffers::WIPOffset<
    flatbuffers::Vector<'a, flatbuffers::ForwardsUOffset<protocol::CellRun<'a>>>,
> {
    let mut run_offsets = Vec::with_capacity(runs.len());
    for run in runs {
        run_offsets.push(build_cell_run(builder, run));
    }
    builder.create_vector(&run_offsets)
}

fn build_cell_run<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    run: &CellRun,
) -> flatbuffers::WIPOffset<protocol::CellRun<'a>> {
    let text = builder.create_string(&run.text);
    let widths = builder.create_vector(&run.cell_widths);
    protocol::CellRun::create(
        builder,
        &protocol::CellRunArgs {
            text_utf8: Some(text),
            cell_widths: Some(widths),
            style_id: run.style_id,
            flags: run.flags,
            hyperlink_id: 0,
            semantic_content: run.semantic_content,
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::host::{CommandSpec, HostKind, HostSpec};
    use crate::terminal::{
        CellRun, PaneStyle, TerminalColors, TerminalCursor, TerminalEngine, TerminalInput,
        TerminalModes, TerminalUpdate,
    };

    use nmux_proto::{PROTOCOL_VERSION, protocol};

    use super::{
        AttachMode, Cursor, FocusInputSpec, InputFrameContext, MouseInputSpec, PasteInputSpec,
        ScrollbackFetchSpec, ScrollbackRange, Session,
    };

    fn env_value<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter()
            .find_map(|(env_key, value)| (env_key == key).then_some(value.as_str()))
    }

    #[test]
    fn initial_session_has_one_fixed_size_pane() {
        let session = Session::initial();

        assert_eq!(session.id, "local");
        assert_eq!(session.version, 1);
        assert_eq!(session.active_tab_id, "tab-1");
        assert_eq!(session.tabs.len(), 1);

        let tab = &session.tabs[0];
        assert_eq!(tab.id, "tab-1");
        assert_eq!(tab.active_pane_id, "pane-1");
        assert_eq!(tab.root.id, "pane-1");
        assert_eq!(tab.root.host.id, "local");
        assert_eq!(tab.root.host.kind, HostKind::Local);
        assert_eq!(tab.root.host.command.program, "sh");
        assert_eq!(tab.root.surface_version, 2);
        assert_eq!(tab.root.cols, 80);
        assert_eq!(tab.root.rows, 24);
        assert_eq!(
            tab.root.surface_lines,
            vec![
                "nmux pane-1".to_owned(),
                "server-owned terminal state".to_owned(),
            ]
        );
    }

    #[test]
    fn workspace_tree_frame_decodes_to_initial_session() {
        let frame = Session::initial().workspace_tree_frame("conn-1", 7);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 7);
        assert_eq!(
            envelope.body_type(),
            protocol::EnvelopeBody::WorkspaceTreeSnapshot
        );

        let snapshot = envelope
            .body_as_workspace_tree_snapshot()
            .expect("workspace tree body");
        assert_eq!(snapshot.version(), 1);
        assert_eq!(snapshot.session_id(), Some("local"));
        assert_eq!(snapshot.active_tab_id(), Some("tab-1"));

        let tabs = snapshot.tabs().expect("tabs");
        assert_eq!(tabs.len(), 1);

        let tab = tabs.get(0);
        assert_eq!(tab.tab_id(), Some("tab-1"));
        assert_eq!(tab.title(), Some("local"));
        assert_eq!(tab.active_pane_id(), Some("pane-1"));

        let pane = tab.root().expect("root pane");
        assert_eq!(pane.pane_id(), Some("pane-1"));
        assert_eq!(pane.kind(), protocol::PaneKind::Pty);
        assert_eq!(pane.split_axis(), protocol::SplitAxis::None);
        assert_eq!(pane.surface_version(), 2);
        assert_eq!(pane.cols(), 80);
        assert_eq!(pane.rows(), 24);
        assert_eq!(pane.resize_policy(), protocol::ResizePolicy::Fixed);
    }

    #[test]
    fn committed_pane_resize_updates_workspace_snapshot() {
        let mut session = Session::initial();

        assert!(session.commit_pane_resize("pane-1", 100, 30));
        assert_eq!(session.version, 2);

        let frame = session.workspace_tree_frame("conn-1", 7);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope
            .body_as_workspace_tree_snapshot()
            .expect("snapshot");
        assert_eq!(snapshot.version(), 2);

        let tabs = snapshot.tabs().expect("tabs");
        let tab = tabs.get(0);
        let pane = tab.root().expect("pane");
        assert_eq!(pane.surface_version(), 3);
        assert_eq!(pane.cols(), 100);
        assert_eq!(pane.rows(), 30);
        assert_eq!(pane.resize_policy(), protocol::ResizePolicy::Fixed);
    }

    #[test]
    fn pane_resize_policy_is_published_in_workspace_snapshot() {
        let mut session = Session::initial();

        assert!(session.set_pane_resize_policy("pane-1", protocol::ResizePolicy::Manual));
        assert_eq!(
            session.pane_resize_policy("pane-1"),
            Some(protocol::ResizePolicy::Manual)
        );

        let frame = session.workspace_tree_frame("conn-1", 7);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope
            .body_as_workspace_tree_snapshot()
            .expect("snapshot");
        assert_eq!(snapshot.version(), 2);

        let tabs = snapshot.tabs().expect("tabs");
        let pane = tabs.get(0).root().expect("pane");
        assert_eq!(pane.resize_policy(), protocol::ResizePolicy::Manual);
    }

    #[test]
    fn split_active_pane_creates_recursive_workspace_tree_and_focuses_new_pane() {
        let mut session = Session::initial();

        assert!(session.split_active_pane(
            protocol::SplitAxis::Vertical,
            "pane-2",
            HostSpec::local("local-2", CommandSpec::new("sh"))
        ));

        assert_eq!(session.version, 2);
        assert_eq!(session.active_pane_id(), Some("pane-2"));
        assert_eq!(session.tabs[0].active_pane_id, "pane-2");
        assert_eq!(
            session.pane_size("pane-1"),
            Some((40, 24)),
            "existing pane should take the first half of a vertical split"
        );
        assert_eq!(session.pane_size("pane-2"), Some((40, 24)));
        assert_eq!(session.surface_version("pane-1"), Some(3));
        assert_eq!(session.surface_version("pane-2"), Some(1));
        assert_eq!(
            session.leaf_pane_ids(),
            vec!["pane-1".to_owned(), "pane-2".to_owned()]
        );
        assert_eq!(
            session
                .pane_host("pane-1")
                .expect("pane-1 host")
                .command
                .initial_size,
            Some((40, 24))
        );
        assert_eq!(
            session
                .pane_host("pane-2")
                .expect("pane-2 host")
                .command
                .initial_size,
            Some((40, 24))
        );
        assert_eq!(
            session.pane_surface("pane-2").expect("new pane").lines,
            vec![
                "nmux pane-2".to_owned(),
                "server-owned terminal state".to_owned(),
            ]
        );

        let frame = session.workspace_tree_frame("conn-1", 7);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope
            .body_as_workspace_tree_snapshot()
            .expect("snapshot");
        assert_eq!(snapshot.version(), 2);
        let tabs = snapshot.tabs().expect("tabs");
        let tab = tabs.get(0);
        assert_eq!(tab.active_pane_id(), Some("pane-2"));
        let root = tab.root().expect("root");
        assert_eq!(root.split_axis(), protocol::SplitAxis::Vertical);
        assert_eq!(root.cols(), 80);
        assert_eq!(root.rows(), 24);
        let children = root.children().expect("children");
        assert_eq!(children.len(), 2);
        let first = children.get(0);
        assert_eq!(first.pane_id(), Some("pane-1"));
        assert_eq!(first.split_axis(), protocol::SplitAxis::None);
        assert_eq!(first.cols(), 40);
        assert_eq!(first.rows(), 24);
        let second = children.get(1);
        assert_eq!(second.pane_id(), Some("pane-2"));
        assert_eq!(second.cols(), 40);
        assert_eq!(second.rows(), 24);
    }

    #[test]
    fn focus_pane_routes_active_pane_to_existing_split_leaf() {
        let mut session = Session::initial();
        assert!(session.split_active_pane(
            protocol::SplitAxis::Horizontal,
            "pane-2",
            HostSpec::local("local-2", CommandSpec::new("sh"))
        ));

        assert_eq!(session.active_pane_id(), Some("pane-2"));
        assert!(session.focus_pane("pane-1"));
        assert_eq!(session.active_pane_id(), Some("pane-1"));
        assert_eq!(session.tabs[0].active_pane_id, "pane-1");
        assert!(!session.focus_pane("missing"));
        assert_eq!(session.active_pane_id(), Some("pane-1"));
    }

    #[test]
    fn split_pane_can_target_existing_leaf_and_preserves_other_leaves() {
        let mut session = Session::initial();
        assert!(session.split_active_pane(
            protocol::SplitAxis::Vertical,
            "pane-2",
            HostSpec::local("local-2", CommandSpec::new("sh"))
        ));

        assert!(session.split_pane(
            "pane-1",
            protocol::SplitAxis::Horizontal,
            "pane-3",
            HostSpec::local("local-3", CommandSpec::new("sh"))
        ));

        assert_eq!(session.version, 3);
        assert_eq!(session.active_pane_id(), Some("pane-3"));
        assert_eq!(
            session.leaf_pane_ids(),
            vec![
                "pane-1".to_owned(),
                "pane-3".to_owned(),
                "pane-2".to_owned()
            ]
        );
        assert_eq!(session.pane_size("pane-1"), Some((40, 12)));
        assert_eq!(session.pane_size("pane-3"), Some((40, 12)));
        assert_eq!(session.pane_size("pane-2"), Some((40, 24)));

        let frame = session.workspace_tree_frame("conn-1", 7);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope
            .body_as_workspace_tree_snapshot()
            .expect("snapshot");
        let root = snapshot.tabs().expect("tabs").get(0).root().expect("root");
        assert_eq!(root.split_axis(), protocol::SplitAxis::Vertical);
        let root_children = root.children().expect("root children");
        assert_eq!(root_children.len(), 2);
        let nested = root_children.get(0);
        assert_eq!(nested.split_axis(), protocol::SplitAxis::Horizontal);
        let nested_children = nested.children().expect("nested children");
        assert_eq!(nested_children.get(0).pane_id(), Some("pane-1"));
        assert_eq!(nested_children.get(1).pane_id(), Some("pane-3"));
        assert_eq!(root_children.get(1).pane_id(), Some("pane-2"));
    }

    #[test]
    fn split_pane_is_scoped_to_active_tab_and_rejects_invalid_targets() {
        let mut session = Session::initial();
        assert!(session.add_tab(
            "tab-2",
            "logs",
            "tab-2-pane-1",
            HostSpec::local("local-2", CommandSpec::new("sh"))
        ));
        assert!(session.switch_tab("tab-2"));
        let version = session.version;

        assert!(!session.split_pane(
            "pane-1",
            protocol::SplitAxis::Vertical,
            "pane-2",
            HostSpec::local("local-3", CommandSpec::new("sh"))
        ));
        assert_eq!(session.version, version);

        assert!(session.split_pane(
            "tab-2-pane-1",
            protocol::SplitAxis::Vertical,
            "tab-2-pane-2",
            HostSpec::local("local-4", CommandSpec::new("sh"))
        ));
        let version = session.version;
        assert!(!session.split_pane(
            "tab-2-pane-1",
            protocol::SplitAxis::None,
            "tab-2-pane-3",
            HostSpec::local("local-5", CommandSpec::new("sh"))
        ));
        assert!(!session.split_pane(
            "tab-2-pane-1",
            protocol::SplitAxis::Vertical,
            "tab-2-pane-2",
            HostSpec::local("local-6", CommandSpec::new("sh"))
        ));
        assert_eq!(session.version, version);
        assert_eq!(
            session.leaf_pane_ids(),
            vec![
                "pane-1".to_owned(),
                "tab-2-pane-1".to_owned(),
                "tab-2-pane-2".to_owned()
            ]
        );
    }

    #[test]
    fn add_and_switch_tab_updates_workspace_tree_active_tab() {
        let mut session = Session::initial();

        assert!(session.add_tab(
            "tab-2",
            "logs",
            "tab-2-pane-1",
            HostSpec::local("local-2", CommandSpec::new("sh"))
        ));
        assert_eq!(session.version, 2);
        assert_eq!(session.active_tab_id, "tab-1");
        assert_eq!(
            session.leaf_pane_ids(),
            vec!["pane-1".to_owned(), "tab-2-pane-1".to_owned()]
        );
        assert!(!session.add_tab(
            "tab-2",
            "duplicate",
            "tab-3-pane-1",
            HostSpec::local("local-3", CommandSpec::new("sh"))
        ));
        assert!(!session.add_tab(
            "tab-3",
            "duplicate pane",
            "pane-1",
            HostSpec::local("local-3", CommandSpec::new("sh"))
        ));

        assert!(session.switch_tab("tab-2"));
        assert_eq!(session.version, 3);
        assert_eq!(session.active_tab_id, "tab-2");
        assert_eq!(session.active_pane_id(), Some("tab-2-pane-1"));
        assert!(!session.switch_tab("missing"));

        let frame = session.workspace_tree_frame("conn-1", 7);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope
            .body_as_workspace_tree_snapshot()
            .expect("snapshot");
        assert_eq!(snapshot.version(), 3);
        assert_eq!(snapshot.active_tab_id(), Some("tab-2"));
        let tabs = snapshot.tabs().expect("tabs");
        assert_eq!(tabs.len(), 2);
        let tab = tabs.get(1);
        assert_eq!(tab.tab_id(), Some("tab-2"));
        assert_eq!(tab.title(), Some("logs"));
        assert_eq!(tab.active_pane_id(), Some("tab-2-pane-1"));
        assert_eq!(
            tab.root().expect("tab-2 root").pane_id(),
            Some("tab-2-pane-1")
        );
    }

    #[test]
    fn close_tab_removes_tab_and_selects_neighbor() {
        let mut session = Session::initial();
        assert!(!session.close_tab("tab-1"));
        assert!(session.add_tab(
            "tab-2",
            "logs",
            "tab-2-pane-1",
            HostSpec::local("local-2", CommandSpec::new("sh"))
        ));
        assert!(session.add_tab(
            "tab-3",
            "shell",
            "tab-3-pane-1",
            HostSpec::local("local-3", CommandSpec::new("sh"))
        ));
        assert!(session.switch_tab("tab-2"));

        assert!(session.close_tab("tab-2"));
        assert_eq!(session.version, 5);
        assert_eq!(session.active_tab_id, "tab-3");
        assert_eq!(session.active_pane_id(), Some("tab-3-pane-1"));
        assert_eq!(
            session.leaf_pane_ids(),
            vec!["pane-1".to_owned(), "tab-3-pane-1".to_owned()]
        );
        assert!(!session.close_tab("missing"));

        let frame = session.workspace_tree_frame("conn-1", 7);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope
            .body_as_workspace_tree_snapshot()
            .expect("snapshot");
        assert_eq!(snapshot.active_tab_id(), Some("tab-3"));
        let tabs = snapshot.tabs().expect("tabs");
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.get(0).tab_id(), Some("tab-1"));
        assert_eq!(tabs.get(1).tab_id(), Some("tab-3"));
    }

    #[test]
    fn pane_nmux_environment_replaces_existing_identity_keys() {
        let mut session = Session::initial();
        session.tabs[0].root.host.command = CommandSpec::new("sh").with_envs([
            ("NMUX", "old"),
            ("NMUX_SOCKET", "/tmp/old.sock"),
            ("NMUX_EXTRA", "keep"),
        ]);

        assert!(session.set_pane_nmux_environment("pane-1", "/tmp/nmux.sock", None));
        let env = &session.tabs[0].root.host.command.env;

        assert_eq!(env_value(env, "NMUX"), Some("1"));
        assert_eq!(env_value(env, "NMUX_SESSION_ID"), Some("local"));
        assert_eq!(env_value(env, "NMUX_PANE_ID"), Some("pane-1"));
        assert_eq!(env_value(env, "NMUX_SOCKET"), Some("/tmp/nmux.sock"));
        assert_eq!(env_value(env, "NMUX_ORIGIN"), Some("local"));
        assert_eq!(env_value(env, "NMUX_EXTRA"), Some("keep"));
    }

    #[test]
    fn pane_nmux_environment_appends_parent_origin_chain() {
        let mut session = Session::initial();

        assert!(session.set_pane_nmux_environment(
            "pane-1",
            "/tmp/nmux.sock",
            Some("outer>middle")
        ));
        let env = &session.tabs[0].root.host.command.env;

        assert_eq!(env_value(env, "NMUX_ORIGIN"), Some("outer>middle>local"));
    }

    #[test]
    fn manual_resize_policy_rejects_frontend_viewport_intents() {
        assert!(!Session::resize_intent_allowed(
            protocol::ResizePolicy::Manual,
            protocol::ResizeReason::FrontendViewport
        ));
        assert!(Session::resize_intent_allowed(
            protocol::ResizePolicy::Manual,
            protocol::ResizeReason::UserCommand
        ));
        assert!(Session::resize_intent_allowed(
            protocol::ResizePolicy::Fixed,
            protocol::ResizeReason::FrontendViewport
        ));
    }

    #[test]
    fn pane_surface_frame_decodes_to_initial_surface() {
        let frame = Session::initial().pane_surface_frame("conn-1", 8);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 8);
        assert_eq!(
            envelope.body_type(),
            protocol::EnvelopeBody::PaneSurfaceSnapshot
        );

        let snapshot = envelope
            .body_as_pane_surface_snapshot()
            .expect("pane surface body");
        assert_eq!(snapshot.pane_id(), Some("pane-1"));
        assert_eq!(snapshot.version(), 2);
        assert_eq!(snapshot.surface(), protocol::SurfaceKind::Main);
        assert_eq!(snapshot.cols(), 80);
        assert_eq!(snapshot.rows(), 24);
        let metadata = snapshot.metadata().expect("metadata");
        assert_eq!(metadata.title(), Some(""));
        assert_eq!(metadata.working_directory(), Some(""));

        let cursor = snapshot.cursor().expect("cursor");
        assert_eq!(cursor.row(), 1);
        assert_eq!(cursor.col(), 0);
        assert!(cursor.visible());
        assert_eq!(cursor.shape(), protocol::CursorShape::Block);
        assert!(cursor.blinking());
        let modes = snapshot.modes().expect("modes");
        assert!(!modes.bracketed_paste());
        assert!(!modes.mouse_tracking());
        assert_eq!(
            modes.mouse_tracking_mode(),
            protocol::MouseTrackingMode::None
        );
        assert_eq!(modes.mouse_format(), protocol::MouseFormat::X10);
        assert!(!modes.focus_reporting());
        assert!(modes.wraparound());

        let styles = snapshot.styles().expect("styles");
        assert_eq!(styles.len(), 1);

        let rows = snapshot.rows_data().expect("rows");
        assert_eq!(rows.len(), 2);

        let first_row = rows.get(0);
        assert_eq!(first_row.row(), 0);
        assert_ne!(first_row.dirty_hash(), 0);
        assert_eq!(
            first_row.semantic_prompt(),
            protocol::RowSemanticPrompt::None
        );
        assert!(!first_row.dirty());
        assert!(!first_row.kitty_virtual_placeholder());
        let first_runs = first_row.runs().expect("runs");
        assert_eq!(first_runs.len(), 1);
        assert_eq!(first_runs.get(0).text_utf8(), Some("nmux pane-1"));
    }

    #[test]
    fn pane_surface_frame_is_pane_scoped() {
        let mut session = Session::initial();
        let mut second = session.tabs[0].clone();
        second.id = "tab-2".to_owned();
        second.active_pane_id = "pane-2".to_owned();
        second.root.id = "pane-2".to_owned();
        second.root.surface_version = 9;
        second.root.cols = 100;
        second.root.rows = 10;
        second.root.surface = protocol::SurfaceKind::Alternate;
        second.root.cursor = Cursor {
            row: 3,
            col: 4,
            visible: false,
            shape: protocol::CursorShape::Beam,
            blinking: true,
        };
        second.root.surface_lines = vec!["pane two".to_owned()];
        session.tabs.push(second);

        assert!(
            session
                .pane_surface_frame_for_pane("conn-1", 8, "missing")
                .is_none()
        );

        let frame = session
            .pane_surface_frame_for_pane("conn-1", 8, "pane-2")
            .expect("pane surface");
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope.body_as_pane_surface_snapshot().expect("snapshot");
        assert_eq!(snapshot.pane_id(), Some("pane-2"));
        assert_eq!(snapshot.version(), 9);
        assert_eq!(snapshot.cols(), 100);
        assert_eq!(snapshot.rows(), 10);
        assert_eq!(snapshot.surface(), protocol::SurfaceKind::Alternate);
        let cursor = snapshot.cursor().expect("cursor");
        assert_eq!(cursor.row(), 3);
        assert_eq!(cursor.col(), 4);
        assert!(!cursor.visible());
        assert_eq!(cursor.shape(), protocol::CursorShape::Beam);
        assert!(cursor.blinking());
        let rows = snapshot.rows_data().expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows.get(0).runs().expect("runs").get(0).text_utf8(),
            Some("pane two")
        );
    }

    #[test]
    fn pane_surface_frame_preserves_stored_cell_runs_and_styles() {
        let mut session = Session::initial();
        let pane = session.pane_mut("pane-1").expect("pane");
        pane.colors = TerminalColors {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x111111ff,
            cursor_rgba: 0xff00ffff,
            cursor_rgba_set: true,
            palette_rgba: vec![0x000000ff, 0x112233ff],
        };
        pane.styles.push(PaneStyle {
            fg_rgba: 0xff00_0000,
            bg_rgba: 0,
            underline_rgba: 0,
            flags: 1,
        });
        pane.surface_lines = vec!["red plain".to_owned()];
        pane.surface_row_runs = vec![vec![
            CellRun {
                text: "red".to_owned(),
                cell_widths: vec![1, 1, 1],
                style_id: 1,
                flags: crate::terminal::CELL_RUN_FLAG_HYPERLINK_PRESENT,
                hyperlink_id: 7,
                semantic_content: protocol::CellSemanticContent::Prompt,
            },
            CellRun::plain(" plain"),
        ]];

        let frame = session.pane_surface_frame("conn-1", 8);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope.body_as_pane_surface_snapshot().expect("snapshot");

        let styles = snapshot.styles().expect("styles");
        assert_eq!(styles.len(), 2);
        assert_eq!(styles.get(1).fg_rgba(), 0xff00_0000);
        assert_eq!(styles.get(1).flags(), 1);
        assert_eq!(snapshot.hyperlinks().expect("hyperlinks").len(), 0);
        let colors = snapshot.colors().expect("colors");
        assert_eq!(colors.default_fg_rgba(), 0xeeeeeeff);
        assert_eq!(colors.default_bg_rgba(), 0x111111ff);
        assert_eq!(colors.cursor_rgba(), 0xff00ffff);
        assert!(colors.cursor_rgba_set());
        let palette = colors.palette_rgba().expect("palette");
        assert_eq!(palette.len(), 2);
        assert_eq!(palette.get(1), 0x112233ff);

        let rows = snapshot.rows_data().expect("rows");
        let runs = rows.get(0).runs().expect("runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs.get(0).text_utf8(), Some("red"));
        assert_eq!(runs.get(0).style_id(), 1);
        assert_eq!(
            runs.get(0).flags(),
            crate::terminal::CELL_RUN_FLAG_HYPERLINK_PRESENT
        );
        assert_eq!(runs.get(0).hyperlink_id(), 0);
        assert_eq!(runs.get(0).cell_widths().expect("widths").len(), 3);
        assert_eq!(
            runs.get(0).semantic_content(),
            protocol::CellSemanticContent::Prompt
        );
        assert_eq!(runs.get(1).text_utf8(), Some(" plain"));
        assert_eq!(runs.get(1).style_id(), 0);
    }

    #[test]
    fn row_state_hash_covers_render_metadata_beyond_text() {
        let mut session = Session::initial();
        let pane = session.pane_mut("pane-1").expect("pane");
        pane.surface_lines = vec!["same".to_owned(), "same".to_owned()];
        pane.surface_row_runs = vec![
            vec![CellRun::plain("same")],
            vec![CellRun {
                text: "same".to_owned(),
                cell_widths: vec![1, 1, 1, 1],
                style_id: 0,
                flags: crate::terminal::CELL_RUN_FLAG_HYPERLINK_PRESENT,
                hyperlink_id: 0,
                semantic_content: protocol::CellSemanticContent::Output,
            }],
        ];

        let frame = session.pane_surface_frame("conn-1", 8);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope.body_as_pane_surface_snapshot().expect("snapshot");
        let rows = snapshot.rows_data().expect("rows");

        assert_eq!(rows.get(0).dirty_hash(), rows.get(1).dirty_hash());
        assert_ne!(rows.get(0).row_state_hash(), rows.get(1).row_state_hash());
    }

    #[test]
    fn scrollback_row_state_hash_covers_render_metadata_beyond_text() {
        let mut session = Session::initial();
        let pane = session.pane_mut("pane-1").expect("pane");
        pane.scrollback_lines = vec!["same".to_owned(), "same".to_owned()];
        pane.scrollback_row_runs = vec![
            vec![CellRun::plain("same")],
            vec![CellRun {
                text: "same".to_owned(),
                cell_widths: vec![1, 1, 1, 1],
                style_id: 0,
                flags: crate::terminal::CELL_RUN_FLAG_HYPERLINK_PRESENT,
                hyperlink_id: 0,
                semantic_content: protocol::CellSemanticContent::Output,
            }],
        ];

        let frame = session.scrollback_chunk_frame("conn-1", 8, 1, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let chunk = envelope.body_as_scrollback_chunk().expect("chunk");
        let rows = chunk.rows().expect("rows");

        assert_eq!(rows.get(0).dirty_hash(), rows.get(1).dirty_hash());
        assert_ne!(rows.get(0).row_state_hash(), rows.get(1).row_state_hash());
    }

    #[test]
    fn pane_surface_patch_frame_decodes_to_replace_rows_patch() {
        let frame = Session::initial().pane_surface_patch_frame("conn-1", 10, 1);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 10);
        assert_eq!(
            envelope.body_type(),
            protocol::EnvelopeBody::PaneSurfacePatch
        );

        let patch = envelope
            .body_as_pane_surface_patch()
            .expect("pane surface patch body");
        assert_eq!(patch.pane_id(), Some("pane-1"));
        assert_eq!(patch.base_version(), 1);
        assert_eq!(patch.version(), 2);
        assert_eq!(patch.kind(), protocol::PatchKind::ReplaceRows);
        let metadata = patch.metadata().expect("metadata");
        assert_eq!(metadata.title(), Some(""));
        assert_eq!(metadata.working_directory(), Some(""));

        let cursor = patch.cursor().expect("cursor");
        assert_eq!(cursor.row(), 1);
        assert_eq!(cursor.col(), 0);
        assert!(cursor.blinking());
        let modes = patch.modes().expect("modes");
        assert!(!modes.bracketed_paste());
        assert!(!modes.focus_reporting());
        assert!(modes.wraparound());

        let rows = patch.row_updates().expect("row updates");
        assert_eq!(rows.len(), 2);

        let first_row = rows.get(0);
        assert_eq!(first_row.row(), 0);
        assert_eq!(
            first_row.semantic_prompt(),
            protocol::RowSemanticPrompt::None
        );
        assert!(!first_row.dirty());
        assert_eq!(
            first_row.runs().expect("runs").get(0).semantic_content(),
            protocol::CellSemanticContent::Output
        );
        assert!(!first_row.kitty_virtual_placeholder());
        let first_runs = first_row.runs().expect("runs");
        assert_eq!(first_runs.get(0).text_utf8(), Some("nmux pane-1"));
    }

    #[test]
    fn replace_rows_patch_only_carries_changed_rows() {
        struct OneRowEngine;

        impl TerminalEngine for OneRowEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"one row");
                let mut lines = input.surface_lines.to_vec();
                lines[1] = "changed row".to_owned();
                Some(TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    lines,
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
        let mut engine = OneRowEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"one row", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::ReplaceRows)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 10, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::ReplaceRows);

        let rows = patch.row_updates().expect("row updates");
        assert_eq!(rows.len(), 1);
        let row = rows.get(0);
        assert_eq!(row.row(), 1);
        assert_eq!(
            row.runs().expect("runs").get(0).text_utf8(),
            Some("changed row")
        );
    }

    #[test]
    fn pane_surface_patch_frame_is_pane_scoped() {
        let mut session = Session::initial();
        let mut second = session.tabs[0].clone();
        second.id = "tab-2".to_owned();
        second.active_pane_id = "pane-2".to_owned();
        second.root.id = "pane-2".to_owned();
        second.root.surface_version = 4;
        second.root.last_patch_kind = protocol::PatchKind::CursorOnly;
        second.root.cursor = Cursor {
            row: 1,
            col: 8,
            visible: true,
            shape: protocol::CursorShape::Underline,
            blinking: true,
        };
        session.tabs.push(second);

        assert!(
            session
                .pane_surface_patch_frame_for_pane("conn-1", 10, "missing", 3)
                .is_none()
        );

        let frame = session
            .pane_surface_patch_frame_for_pane("conn-1", 10, "pane-2", 3)
            .expect("pane patch");
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.pane_id(), Some("pane-2"));
        assert_eq!(patch.base_version(), 3);
        assert_eq!(patch.version(), 4);
        assert_eq!(patch.kind(), protocol::PatchKind::CursorOnly);
        assert_eq!(patch.row_updates().expect("rows").len(), 0);
        let cursor = patch.cursor().expect("cursor");
        assert_eq!(cursor.row(), 1);
        assert_eq!(cursor.col(), 8);
        assert_eq!(cursor.shape(), protocol::CursorShape::Underline);
        assert!(cursor.blinking());
    }

    #[test]
    fn scrollback_chunk_frame_decodes_requested_range() {
        let frame = Session::initial().scrollback_chunk_frame("conn-1", 11, 1, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 11);
        assert_eq!(
            envelope.body_type(),
            protocol::EnvelopeBody::ScrollbackChunk
        );

        let chunk = envelope
            .body_as_scrollback_chunk()
            .expect("scrollback chunk body");
        assert_eq!(chunk.pane_id(), Some("pane-1"));
        assert_eq!(chunk.scrollback_version(), 1);
        assert_eq!(chunk.start_line(), 1);
        assert_eq!(chunk.total_lines(), 3);

        let rows = chunk.rows().expect("scrollback rows");
        assert_eq!(rows.len(), 2);

        let first = rows.get(0);
        assert_eq!(first.line(), 1);
        assert_eq!(first.semantic_prompt(), protocol::RowSemanticPrompt::None);
        assert!(!first.dirty());
        assert!(!first.kitty_virtual_placeholder());
        let first_runs = first.runs().expect("first runs");
        assert_eq!(
            first_runs.get(0).text_utf8(),
            Some("booting nmux workspace")
        );

        let second = rows.get(1);
        assert_eq!(second.line(), 2);
        let second_runs = second.runs().expect("second runs");
        assert_eq!(second_runs.get(0).text_utf8(), Some("nmux pane-1"));
        assert_eq!(
            second_runs.get(0).semantic_content(),
            protocol::CellSemanticContent::Output
        );
    }

    #[test]
    fn scrollback_chunk_frame_preserves_stored_cell_runs_and_styles() {
        let mut session = Session::initial();
        let pane = session.pane_mut("pane-1").expect("pane");
        pane.colors = TerminalColors {
            default_fg_rgba: 0xeeeeeeff,
            default_bg_rgba: 0x111111ff,
            cursor_rgba: 0,
            cursor_rgba_set: false,
            palette_rgba: vec![0x000000ff, 0x445566ff],
        };
        pane.styles.push(PaneStyle {
            fg_rgba: 0xff00_0000,
            bg_rgba: 0,
            underline_rgba: 0,
            flags: 1,
        });
        pane.scrollback_lines = vec!["red plain".to_owned()];
        pane.scrollback_row_runs = vec![vec![
            CellRun {
                text: "red".to_owned(),
                cell_widths: vec![1, 1, 1],
                style_id: 1,
                flags: 0,
                hyperlink_id: 7,
                semantic_content: protocol::CellSemanticContent::Input,
            },
            CellRun::plain(" plain"),
        ]];

        let frame = session.scrollback_chunk_frame("conn-1", 11, 1, 1);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let chunk = envelope
            .body_as_scrollback_chunk()
            .expect("scrollback chunk");

        let styles = chunk.styles().expect("styles");
        assert_eq!(styles.len(), 2);
        assert_eq!(styles.get(1).fg_rgba(), 0xff00_0000);
        assert_eq!(styles.get(1).flags(), 1);
        assert_eq!(chunk.hyperlinks().expect("hyperlinks").len(), 0);
        let colors = chunk.colors().expect("colors");
        assert_eq!(colors.default_fg_rgba(), 0xeeeeeeff);
        assert_eq!(colors.default_bg_rgba(), 0x111111ff);
        assert_eq!(colors.cursor_rgba(), 0);
        assert!(!colors.cursor_rgba_set());
        let palette = colors.palette_rgba().expect("palette");
        assert_eq!(palette.get(1), 0x445566ff);

        let rows = chunk.rows().expect("rows");
        let runs = rows.get(0).runs().expect("runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs.get(0).text_utf8(), Some("red"));
        assert_eq!(runs.get(0).style_id(), 1);
        assert_eq!(runs.get(0).hyperlink_id(), 0);
        assert_eq!(
            runs.get(0).semantic_content(),
            protocol::CellSemanticContent::Input
        );
        assert_eq!(runs.get(1).text_utf8(), Some(" plain"));
        assert_eq!(runs.get(1).style_id(), 0);
    }

    #[test]
    fn scrollback_chunk_frame_is_pane_scoped() {
        let session = Session::initial();

        assert!(
            session
                .scrollback_chunk_frame_for_pane("conn-1", 11, "missing", 1, 2)
                .is_none()
        );
        assert!(
            session
                .scrollback_chunk_frame_for_pane("conn-1", 11, "pane-1", 0, 1)
                .is_none()
        );
        assert!(
            session
                .scrollback_chunk_frame_for_pane("conn-1", 11, "pane-1", 1, 0)
                .is_none()
        );

        let frame = session
            .scrollback_chunk_frame_for_pane("conn-1", 11, "pane-1", 1, 1)
            .expect("pane scrollback");
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let chunk = envelope.body_as_scrollback_chunk().expect("chunk");
        assert_eq!(chunk.pane_id(), Some("pane-1"));
    }

    #[test]
    fn scrollback_chunk_uses_one_based_public_line_numbers() {
        let frame = Session::initial().scrollback_chunk_frame("conn-1", 11, 4, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let chunk = envelope.body_as_scrollback_chunk().expect("chunk");

        assert_eq!(chunk.start_line(), 4);
        assert_eq!(chunk.total_lines(), 3);
        assert_eq!(chunk.rows().expect("rows").len(), 0);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn ghostty_vt_scrollback_chunk_uses_backend_history() {
        let mut session = Session::initial();
        if let Some(pane) = session.pane_mut("pane-1") {
            pane.cols = 20;
            pane.rows = 2;
            pane.surface_lines.clear();
            pane.scrollback_lines.clear();
        }

        let mut engines = crate::terminal::PaneTerminalEngines::new(
            crate::terminal::TerminalEngineKind::LibghosttyVt,
        );
        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"\x1b[31mone\x1b[0m\r\ntwo\r\nthree\r\nfour",
            engines.engine_mut("pane-1")
        ));

        let surface = session.initial_pane_surface();
        let frame = session
            .scrollback_chunk_frame_for_pane("conn-1", 11, "pane-1", 1, 10)
            .expect("scrollback chunk");
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let chunk = envelope.body_as_scrollback_chunk().expect("chunk");
        assert!(
            chunk.total_lines() as usize > surface.lines.len(),
            "scrollback chunk did not include history beyond the surface"
        );

        let rows = chunk.rows().expect("scrollback rows");
        let row_text = (0..rows.len())
            .map(|index| {
                let row = rows.get(index);
                let runs = row.runs().expect("runs");
                runs.get(0).text_utf8().unwrap_or_default().to_owned()
            })
            .collect::<Vec<_>>();
        for expected in ["one", "two", "three", "four"] {
            assert!(
                row_text.iter().any(|line| line.contains(expected)),
                "scrollback chunk missing {expected}: {row_text:?}"
            );
        }

        let styled_run = (0..rows.len())
            .flat_map(|row_index| {
                let row = rows.get(row_index);
                let runs = row.runs().expect("runs");
                (0..runs.len()).map(move |run_index| runs.get(run_index))
            })
            .find(|run| run.text_utf8().is_some_and(|text| text.contains("one")))
            .expect("styled scrollback run");
        assert_ne!(
            styled_run.style_id(),
            0,
            "styled Ghostty scrollback should survive ScrollbackChunk encoding"
        );
        let styles = chunk.styles().expect("styles");
        let style = styles.get(styled_run.style_id() as usize);
        assert_ne!(
            style.fg_rgba(),
            0,
            "styled Ghostty scrollback should reference a style table entry"
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn ghostty_vt_alternate_screen_preserves_styled_main_scrollback_chunk() {
        let mut session = Session::initial();
        if let Some(pane) = session.pane_mut("pane-1") {
            pane.cols = 20;
            pane.rows = 2;
            pane.surface_lines.clear();
            pane.surface_row_runs.clear();
            pane.scrollback_lines.clear();
            pane.scrollback_row_runs.clear();
        }

        let mut engines = crate::terminal::PaneTerminalEngines::new(
            crate::terminal::TerminalEngineKind::LibghosttyVt,
        );
        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"\x1b[31mmain-red\x1b[0m\r\nmain-plain\r\nmain-tail",
            engines.engine_mut("pane-1")
        ));
        let scrollback_version = session
            .scrollback_version("pane-1")
            .expect("scrollback version");

        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"\x1b[?1049halt-red\r\nalt-tail",
            engines.engine_mut("pane-1")
        ));
        assert_eq!(
            session.scrollback_version("pane-1"),
            Some(scrollback_version),
            "alternate-screen output should not mutate main scrollback"
        );

        let frame = session
            .scrollback_chunk_frame_for_pane("conn-1", 11, "pane-1", 1, 10)
            .expect("scrollback chunk");
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let chunk = envelope.body_as_scrollback_chunk().expect("chunk");
        let rows = chunk.rows().expect("scrollback rows");
        let row_text = (0..rows.len())
            .map(|index| {
                let row = rows.get(index);
                let runs = row.runs().expect("runs");
                (0..runs.len())
                    .filter_map(|run_index| runs.get(run_index).text_utf8())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            row_text.iter().any(|line| line.contains("main-red")),
            "main scrollback missing styled line: {row_text:?}"
        );
        assert!(
            row_text.iter().all(|line| !line.contains("alt-")),
            "alternate-screen output leaked into main scrollback: {row_text:?}"
        );

        let styled_run = (0..rows.len())
            .flat_map(|row_index| {
                let row = rows.get(row_index);
                let runs = row.runs().expect("runs");
                (0..runs.len()).map(move |run_index| runs.get(run_index))
            })
            .find(|run| {
                run.text_utf8()
                    .is_some_and(|text| text.contains("main-red"))
            })
            .expect("styled main scrollback run");
        assert_ne!(
            styled_run.style_id(),
            0,
            "styled main scrollback should keep its style ID while alternate screen is active"
        );
        let styles = chunk.styles().expect("styles");
        let style = styles.get(styled_run.style_id() as usize);
        assert_ne!(
            style.fg_rgba(),
            0,
            "styled main scrollback should reference a style table entry"
        );
    }

    #[test]
    fn initial_surface_matches_tail_of_scrollback() {
        let session = Session::initial();
        let surface = session.initial_pane_surface();
        let scrollback = session.initial_scrollback();
        let tail = &scrollback.lines[scrollback.lines.len() - surface.lines.len()..];

        assert_eq!(surface.lines, tail);
    }

    #[test]
    fn scrollback_fetch_frame_decodes_requested_range() {
        let frame = Session::initial().scrollback_fetch_frame(
            InputFrameContext {
                connection_id: "conn-1",
                seq: 12,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 0,
            },
            ScrollbackFetchSpec {
                range: ScrollbackRange {
                    start_line: 1,
                    line_count: 2,
                },
                known_scrollback_version: 1,
            },
        );
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 12);
        assert_eq!(
            envelope.body_type(),
            protocol::EnvelopeBody::ScrollbackFetch
        );

        let fetch = envelope
            .body_as_scrollback_fetch()
            .expect("scrollback fetch body");
        assert_eq!(fetch.pane_id(), Some("pane-1"));
        assert_eq!(fetch.actor_id(), Some("actor-1"));
        assert_eq!(fetch.start_line(), 1);
        assert_eq!(fetch.line_count(), 2);
        assert_eq!(fetch.known_scrollback_version(), 1);
    }

    #[test]
    fn presence_update_frame_decodes_actor_mode() {
        let actor = Session::initial_actor(AttachMode::ReadWrite);
        let frame = Session::initial().presence_update_frame("conn-1", 13, &actor);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 13);
        assert_eq!(envelope.body_type(), protocol::EnvelopeBody::PresenceUpdate);

        let presence = envelope
            .body_as_presence_update()
            .expect("presence update body");
        assert_eq!(presence.actor_id(), Some("local-actor"));
        assert_eq!(presence.user_id(), Some("local-user"));
        assert_eq!(presence.display_name(), Some("local"));
        assert_eq!(presence.mode(), protocol::AttachMode::ReadWrite);
        assert_eq!(presence.kind(), protocol::PresenceKind::Joined);
        assert_eq!(presence.focused_pane_id(), Some("pane-1"));
    }

    #[test]
    fn read_only_actor_is_not_allowed_to_send_input() {
        let read_only = Session::initial_actor(AttachMode::ReadOnly);
        let read_write = Session::initial_actor(AttachMode::ReadWrite);

        assert!(!Session::input_allowed(&read_only));
        assert!(Session::input_allowed(&read_write));
    }

    #[test]
    fn finds_initial_pane_surface_version() {
        let session = Session::initial();

        assert_eq!(session.surface_version("pane-1"), Some(2));
        assert_eq!(session.surface_version("missing"), None);
    }

    #[test]
    fn finds_initial_pane_scrollback_version() {
        let session = Session::initial();

        assert_eq!(session.scrollback_version("pane-1"), Some(1));
        assert_eq!(session.scrollback_version("missing"), None);
    }

    #[test]
    fn key_input_frame_decodes_to_input_event() {
        let frame = Session::initial().key_input_frame("conn-1", 9, "actor-1", "pane-1", 3, "a");
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 9);
        assert_eq!(envelope.body_type(), protocol::EnvelopeBody::InputEvent);

        let input = envelope.body_as_input_event().expect("input event body");
        assert_eq!(input.pane_id(), Some("pane-1"));
        assert_eq!(input.actor_id(), Some("actor-1"));
        assert_eq!(input.input_seq(), 3);
        assert_eq!(input.kind(), protocol::InputKind::Key);

        let key = input.key().expect("key input");
        assert_eq!(key.text_utf8(), Some("a"));
        assert_eq!(key.key_name(), None);
        assert_eq!(key.modifiers(), 0);
    }

    #[test]
    fn named_key_input_frame_decodes_to_input_event() {
        let frame = Session::initial().named_key_input_frame(
            "conn-1",
            9,
            "actor-1",
            "pane-1",
            3,
            "numpad-enter",
        );
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 9);
        assert_eq!(envelope.body_type(), protocol::EnvelopeBody::InputEvent);

        let input = envelope.body_as_input_event().expect("input event body");
        assert_eq!(input.pane_id(), Some("pane-1"));
        assert_eq!(input.actor_id(), Some("actor-1"));
        assert_eq!(input.input_seq(), 3);
        assert_eq!(input.kind(), protocol::InputKind::Key);

        let key = input.key().expect("key input");
        assert_eq!(key.text_utf8(), None);
        assert_eq!(key.key_name(), Some("numpad-enter"));
        assert_eq!(key.modifiers(), 0);
    }

    #[test]
    fn named_key_input_frame_with_modifiers_decodes_to_input_event() {
        let frame = Session::initial().named_key_input_frame_with_modifiers(
            "conn-1", 9, "actor-1", "pane-1", 3, "arrow-up", 2,
        );
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        let input = envelope.body_as_input_event().expect("input event body");
        assert_eq!(input.kind(), protocol::InputKind::Key);

        let key = input.key().expect("key input");
        assert_eq!(key.text_utf8(), None);
        assert_eq!(key.key_name(), Some("arrow-up"));
        assert_eq!(key.modifiers(), 2);
    }

    #[test]
    fn raw_input_frame_decodes_to_input_event() {
        let frame =
            Session::initial().raw_input_frame("conn-1", 9, "actor-1", "pane-1", 3, &[0, 3, 255]);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 9);
        assert_eq!(envelope.body_type(), protocol::EnvelopeBody::InputEvent);

        let input = envelope.body_as_input_event().expect("input event body");
        assert_eq!(input.pane_id(), Some("pane-1"));
        assert_eq!(input.actor_id(), Some("actor-1"));
        assert_eq!(input.input_seq(), 3);
        assert_eq!(input.kind(), protocol::InputKind::RawBytes);

        let raw = input.raw().expect("raw input");
        let bytes = raw.bytes().expect("raw input bytes");
        assert_eq!(bytes.bytes(), &[0, 3, 255]);
    }

    #[test]
    fn paste_input_frame_decodes_to_input_event() {
        let frame = Session::initial().paste_input_frame(
            InputFrameContext {
                connection_id: "conn-1",
                seq: 9,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 3,
            },
            PasteInputSpec {
                text: "hello\n",
                bracketed: true,
            },
        );
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 9);
        assert_eq!(envelope.body_type(), protocol::EnvelopeBody::InputEvent);

        let input = envelope.body_as_input_event().expect("input event body");
        assert_eq!(input.pane_id(), Some("pane-1"));
        assert_eq!(input.actor_id(), Some("actor-1"));
        assert_eq!(input.input_seq(), 3);
        assert_eq!(input.kind(), protocol::InputKind::Paste);

        let paste = input.paste().expect("paste input");
        assert_eq!(paste.text_utf8(), Some("hello\n"));
        assert!(paste.bracketed());
    }

    #[test]
    fn focus_input_frame_decodes_to_input_event() {
        let frame = Session::initial().focus_input_frame(
            InputFrameContext {
                connection_id: "conn-1",
                seq: 9,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 3,
            },
            FocusInputSpec { focused: true },
        );
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 9);
        assert_eq!(envelope.body_type(), protocol::EnvelopeBody::InputEvent);

        let input = envelope.body_as_input_event().expect("input event body");
        assert_eq!(input.pane_id(), Some("pane-1"));
        assert_eq!(input.actor_id(), Some("actor-1"));
        assert_eq!(input.input_seq(), 3);
        assert_eq!(input.kind(), protocol::InputKind::Focus);

        let focus = input.focus().expect("focus input");
        assert!(focus.focused());
    }

    #[test]
    fn mouse_input_frame_decodes_to_input_event() {
        let frame = Session::initial().mouse_input_frame(
            InputFrameContext {
                connection_id: "conn-1",
                seq: 9,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 3,
            },
            MouseInputSpec {
                row: 4,
                col: 5,
                pixel_x: None,
                pixel_y: None,
                button: protocol::MouseButton::Left,
                action: protocol::MouseAction::Press,
                modifiers: 2,
            },
        );
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        assert_eq!(envelope.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(envelope.session_id(), Some("local"));
        assert_eq!(envelope.connection_id(), Some("conn-1"));
        assert_eq!(envelope.seq(), 9);
        assert_eq!(envelope.body_type(), protocol::EnvelopeBody::InputEvent);

        let input = envelope.body_as_input_event().expect("input event body");
        assert_eq!(input.pane_id(), Some("pane-1"));
        assert_eq!(input.actor_id(), Some("actor-1"));
        assert_eq!(input.input_seq(), 3);
        assert_eq!(input.kind(), protocol::InputKind::Mouse);

        let mouse = input.mouse().expect("mouse input");
        assert_eq!(mouse.row(), 4);
        assert_eq!(mouse.col(), 5);
        assert_eq!(mouse.button(), protocol::MouseButton::Left);
        assert_eq!(mouse.action(), protocol::MouseAction::Press);
        assert_eq!(mouse.modifiers(), 2);
        assert!(!mouse.has_pixels());
        assert_eq!(mouse.pixel_x(), 0);
        assert_eq!(mouse.pixel_y(), 0);
    }

    #[test]
    fn mouse_pixel_input_frame_decodes_to_input_event() {
        let frame = Session::initial().mouse_input_frame(
            InputFrameContext {
                connection_id: "conn-1",
                seq: 9,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 3,
            },
            MouseInputSpec {
                row: 4,
                col: 5,
                pixel_x: Some(33),
                pixel_y: Some(65),
                button: protocol::MouseButton::Left,
                action: protocol::MouseAction::Press,
                modifiers: 2,
            },
        );
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let input = envelope.body_as_input_event().expect("input event body");
        let mouse = input.mouse().expect("mouse input");
        assert_eq!(mouse.row(), 4);
        assert_eq!(mouse.col(), 5);
        assert!(mouse.has_pixels());
        assert_eq!(mouse.pixel_x(), 33);
        assert_eq!(mouse.pixel_y(), 65);
    }

    #[test]
    fn pane_output_hydrates_backend_owned_surface() {
        let mut session = Session::initial();
        assert!(session.apply_pane_output("pane-1", b"hello from pty\r\nsecond line\n"));
        let surface = session.initial_pane_surface();
        let scrollback = session.initial_scrollback();

        assert_eq!(surface.version, 3);
        assert_eq!(
            surface.lines,
            vec![
                "booting nmux workspace",
                "nmux pane-1",
                "server-owned terminal state",
                "hello from pty",
                "second line"
            ]
        );
        assert_eq!(surface.cursor.row, 4);
        assert_eq!(scrollback.lines, surface.lines);
    }

    #[test]
    fn pane_output_ignores_missing_pane() {
        let mut session = Session::initial();

        assert!(!session.apply_pane_output("missing", b"hello\n"));
        assert_eq!(session.surface_version("pane-1"), Some(2));
    }

    #[test]
    fn pane_output_can_use_injected_terminal_engine() {
        struct FixedEngine;

        impl TerminalEngine for FixedEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(input.pane_id, "pane-1");
                assert_eq!(input.cols, 80);
                assert_eq!(input.rows, 24);
                assert_eq!(input.surface, protocol::SurfaceKind::Main);
                assert_eq!(
                    input.cursor,
                    TerminalCursor {
                        row: 1,
                        col: 0,
                        visible: true,
                        shape: protocol::CursorShape::Block,
                        blinking: true
                    }
                );
                assert_eq!(input.surface_lines.len(), 2);
                assert_eq!(input.scrollback_lines.len(), 3);
                assert_eq!(output, b"ignored by test engine");
                Some(TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    protocol::SurfaceKind::Alternate,
                    TerminalCursor {
                        row: 7,
                        col: 8,
                        visible: false,
                        shape: protocol::CursorShape::Beam,
                        blinking: true,
                    },
                    vec!["engine surface".to_owned()],
                    vec!["engine scrollback".to_owned()],
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
        let mut engine = FixedEngine;

        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"ignored by test engine",
            &mut engine
        ));

        let surface = session.initial_pane_surface();
        let scrollback = session.initial_scrollback();
        assert_eq!(surface.version, 3);
        assert_eq!(surface.surface, protocol::SurfaceKind::Alternate);
        assert_eq!(surface.lines, vec!["engine surface".to_owned()]);
        assert_eq!(
            surface.cursor,
            Cursor {
                row: 7,
                col: 8,
                visible: false,
                shape: protocol::CursorShape::Beam,
                blinking: true
            }
        );
        let frame = session.pane_surface_frame("conn-1", 1);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope.body_as_pane_surface_snapshot().expect("snapshot");
        let cursor = snapshot.cursor().expect("cursor");
        assert_eq!(cursor.shape(), protocol::CursorShape::Beam);
        assert!(cursor.blinking());
        assert_eq!(snapshot.surface(), protocol::SurfaceKind::Alternate);
        assert_eq!(scrollback.lines, vec!["engine scrollback".to_owned()]);
    }

    #[test]
    fn pane_resize_can_use_injected_terminal_engine() {
        struct ResizeEngine;

        impl TerminalEngine for ResizeEngine {
            fn apply_output(
                &mut self,
                _input: TerminalInput<'_>,
                _output: &[u8],
            ) -> Option<TerminalUpdate> {
                panic!("output is not used by this test")
            }

            fn resize(
                &mut self,
                input: TerminalInput<'_>,
                cols: u32,
                rows: u32,
            ) -> Option<TerminalUpdate> {
                assert_eq!(input.pane_id, "pane-1");
                assert_eq!(input.cols, 80);
                assert_eq!(input.rows, 24);
                assert_eq!(input.surface, protocol::SurfaceKind::Main);
                assert_eq!(cols, 100);
                assert_eq!(rows, 10);
                Some(TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    TerminalCursor {
                        row: 3,
                        col: 4,
                        visible: true,
                        shape: protocol::CursorShape::Underline,
                        blinking: true,
                    },
                    vec!["resized surface".to_owned()],
                    vec!["resized scrollback".to_owned()],
                ))
            }
        }

        let mut session = Session::initial();
        let mut engine = ResizeEngine;

        assert!(session.commit_pane_resize_with_engine("pane-1", 100, 10, &mut engine));

        let surface = session.initial_pane_surface();
        let scrollback = session.initial_scrollback();
        assert_eq!(session.version, 2);
        assert_eq!(surface.version, 3);
        assert_eq!(surface.cols, 100);
        assert_eq!(surface.rows, 10);
        assert_eq!(
            surface.cursor,
            Cursor {
                row: 3,
                col: 4,
                visible: true,
                shape: protocol::CursorShape::Underline,
                blinking: true
            }
        );
        assert_eq!(surface.lines, vec!["resized surface".to_owned()]);
        assert_eq!(scrollback.lines, vec!["resized scrollback".to_owned()]);
    }

    #[test]
    fn resize_requires_full_refresh_because_patch_lacks_new_surface_extent() {
        struct ResizeEngine;

        impl TerminalEngine for ResizeEngine {
            fn apply_output(
                &mut self,
                _input: TerminalInput<'_>,
                _output: &[u8],
            ) -> Option<TerminalUpdate> {
                panic!("apply_output is not used by this test")
            }

            fn resize(
                &mut self,
                input: TerminalInput<'_>,
                _cols: u32,
                _rows: u32,
            ) -> Option<TerminalUpdate> {
                Some(TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    vec!["resized surface".to_owned()],
                    input.scrollback_lines.to_vec(),
                ))
            }
        }

        let mut session = Session::initial();
        let pane = &mut session.tabs[0].root;
        pane.rows = 10;
        pane.surface_lines = vec!["old surface".to_owned()];
        pane.surface_row_runs = vec![vec![CellRun::plain("old surface")]];
        pane.surface_semantic_prompts = vec![protocol::RowSemanticPrompt::None];
        pane.surface_dirty_rows = vec![false];
        pane.surface_kitty_placeholders = vec![false];
        pane.surface_version = 2;

        let mut engine = ResizeEngine;
        assert!(session.commit_pane_resize_with_engine("pane-1", 100, 10, &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::FullRefreshRequired)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 11, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::FullRefreshRequired);
        let rows = patch.row_updates().expect("row updates");
        assert_eq!(rows.len(), 0);

        let snapshot = session
            .pane_surface_frame_for_pane("conn-1", 12, "pane-1")
            .expect("surface snapshot");
        let envelope = protocol::size_prefixed_root_as_envelope(&snapshot).expect("valid envelope");
        let snapshot = envelope.body_as_pane_surface_snapshot().expect("snapshot");
        assert_eq!(snapshot.cols(), 100);
        assert_eq!(snapshot.rows(), 10);
        let rows = snapshot.rows_data().expect("snapshot rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows.get(0).runs().expect("runs").get(0).text_utf8(),
            Some("resized surface")
        );
    }

    #[test]
    fn stateful_terminal_engine_can_span_output_resize_and_output() {
        struct StatefulEngine {
            step: u8,
        }

        impl TerminalEngine for StatefulEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                match self.step {
                    0 => {
                        assert_eq!(input.pane_id, "pane-1");
                        assert_eq!(input.cols, 80);
                        assert_eq!(input.rows, 24);
                        assert_eq!(input.surface_lines.len(), 2);
                        assert_eq!(input.scrollback_lines.len(), 3);
                        assert_eq!(output, b"first");
                        self.step = 1;
                        Some(TerminalUpdate::plain(
                            protocol::PatchKind::ReplaceRows,
                            input.surface,
                            TerminalCursor {
                                row: 0,
                                col: 5,
                                visible: true,
                                shape: input.cursor.shape,
                                blinking: input.cursor.blinking,
                            },
                            vec!["first".to_owned()],
                            vec!["first".to_owned()],
                        ))
                    }
                    2 => {
                        assert_eq!(input.pane_id, "pane-1");
                        assert_eq!(input.cols, 100);
                        assert_eq!(input.rows, 10);
                        assert_eq!(input.surface_lines, ["first resized"]);
                        assert_eq!(input.scrollback_lines, ["first"]);
                        assert_eq!(output, b"second");
                        self.step = 3;
                        Some(TerminalUpdate::plain(
                            protocol::PatchKind::ReplaceRows,
                            input.surface,
                            TerminalCursor {
                                row: 1,
                                col: 6,
                                visible: true,
                                shape: protocol::CursorShape::Beam,
                                blinking: input.cursor.blinking,
                            },
                            vec!["first resized".to_owned(), "second".to_owned()],
                            vec!["first".to_owned(), "second".to_owned()],
                        ))
                    }
                    _ => panic!("unexpected output step {}", self.step),
                }
            }

            fn resize(
                &mut self,
                input: TerminalInput<'_>,
                cols: u32,
                rows: u32,
            ) -> Option<TerminalUpdate> {
                assert_eq!(self.step, 1);
                assert_eq!(input.pane_id, "pane-1");
                assert_eq!(input.cols, 80);
                assert_eq!(input.rows, 24);
                assert_eq!(input.surface_lines, ["first"]);
                assert_eq!(input.scrollback_lines, ["first"]);
                assert_eq!(cols, 100);
                assert_eq!(rows, 10);
                self.step = 2;
                Some(TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    TerminalCursor {
                        row: 0,
                        col: 5,
                        visible: true,
                        shape: input.cursor.shape,
                        blinking: input.cursor.blinking,
                    },
                    vec!["first resized".to_owned()],
                    input.scrollback_lines.to_vec(),
                ))
            }
        }

        let mut session = Session::initial();
        let mut engine = StatefulEngine { step: 0 };

        assert!(session.apply_pane_output_with_engine("pane-1", b"first", &mut engine));
        assert!(session.commit_pane_resize_with_engine("pane-1", 100, 10, &mut engine));
        assert!(session.apply_pane_output_with_engine("pane-1", b"second", &mut engine));
        assert_eq!(engine.step, 3);

        let surface = session.initial_pane_surface();
        let scrollback = session.initial_scrollback();
        assert_eq!(surface.version, 5);
        assert_eq!(surface.cols, 100);
        assert_eq!(surface.rows, 10);
        assert_eq!(
            surface.cursor,
            Cursor {
                row: 1,
                col: 6,
                visible: true,
                shape: protocol::CursorShape::Beam,
                blinking: true
            }
        );
        assert_eq!(
            surface.lines,
            vec!["first resized".to_owned(), "second".to_owned()]
        );
        assert_eq!(
            scrollback.lines,
            vec!["first".to_owned(), "second".to_owned()]
        );
    }

    #[test]
    fn cursor_only_engine_update_emits_cursor_only_patch() {
        struct CursorOnlyEngine;

        impl TerminalEngine for CursorOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"cursor only");
                Some(TerminalUpdate::plain(
                    protocol::PatchKind::CursorOnly,
                    input.surface,
                    TerminalCursor {
                        row: 1,
                        col: 12,
                        visible: true,
                        shape: protocol::CursorShape::Beam,
                        blinking: input.cursor.blinking,
                    },
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
        let mut engine = CursorOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"cursor only", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::CursorOnly)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 8, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::CursorOnly);
        assert_eq!(patch.base_version(), 2);
        assert_eq!(patch.version(), 3);
        assert_eq!(patch.row_updates().expect("row updates").len(), 0);

        let cursor = patch.cursor().expect("cursor");
        assert_eq!(cursor.row(), 1);
        assert_eq!(cursor.col(), 12);
        assert_eq!(cursor.shape(), protocol::CursorShape::Beam);
        assert!(cursor.blinking());
    }

    #[test]
    fn style_table_change_requires_full_refresh_patch() {
        struct StyleTableEngine;

        impl TerminalEngine for StyleTableEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"style table");
                Some(TerminalUpdate {
                    patch_kind: protocol::PatchKind::ReplaceRows,
                    surface: input.surface,
                    cursor: input.cursor,
                    modes: input.modes,
                    title: input.title.to_owned(),
                    working_directory: input.working_directory.to_owned(),
                    colors: input.colors.clone(),
                    styles: vec![
                        PaneStyle::default(),
                        PaneStyle {
                            fg_rgba: 0xff00_0000,
                            bg_rgba: 0,
                            underline_rgba: 0,
                            flags: 1,
                        },
                    ],
                    surface_lines: input.surface_lines.to_vec(),
                    surface_row_runs: input
                        .surface_lines
                        .iter()
                        .map(|line| {
                            vec![CellRun {
                                text: line.clone(),
                                cell_widths: vec![1; line.chars().count()],
                                style_id: 1,
                                flags: 0,
                                hyperlink_id: 0,
                                semantic_content: protocol::CellSemanticContent::Output,
                            }]
                        })
                        .collect(),
                    surface_semantic_prompts: input.surface_semantic_prompts.to_vec(),
                    surface_dirty_rows: input.surface_dirty_rows.to_vec(),
                    surface_kitty_placeholders: input.surface_kitty_placeholders.to_vec(),
                    scrollback_lines: input.scrollback_lines.to_vec(),
                    scrollback_row_runs: crate::terminal::plain_row_runs(input.scrollback_lines),
                    scrollback_semantic_prompts: input.scrollback_semantic_prompts.to_vec(),
                    scrollback_dirty_rows: input.scrollback_dirty_rows.to_vec(),
                    scrollback_kitty_placeholders: input.scrollback_kitty_placeholders.to_vec(),
                })
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
        let mut engine = StyleTableEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"style table", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::FullRefreshRequired)
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn ghostty_vt_style_table_change_requires_full_refresh_patch() {
        let mut session = Session::initial();
        let mut engines = crate::terminal::PaneTerminalEngines::new(
            crate::terminal::TerminalEngineKind::LibghosttyVt,
        );

        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"baseline",
            engines.engine_mut("pane-1")
        ));
        let base_version = session.surface_version("pane-1").expect("surface version");

        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"\x1b[31mred\x1b[0m",
            engines.engine_mut("pane-1")
        ));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::FullRefreshRequired)
        );

        let patch_frame = session.pane_surface_patch_frame("conn-1", 9, base_version);
        let envelope =
            protocol::size_prefixed_root_as_envelope(&patch_frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::FullRefreshRequired);
        assert_eq!(patch.row_updates().expect("row updates").len(), 0);

        let snapshot_frame = session.pane_surface_frame("conn-1", 10);
        let envelope =
            protocol::size_prefixed_root_as_envelope(&snapshot_frame).expect("valid envelope");
        let snapshot = envelope.body_as_pane_surface_snapshot().expect("snapshot");
        assert!(
            snapshot.styles().expect("styles").len() > 1,
            "snapshot should carry the expanded style table"
        );
    }

    #[test]
    fn row_run_only_change_emits_replace_rows_patch() {
        struct RowRunOnlyEngine;

        impl TerminalEngine for RowRunOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"row runs");
                Some(TerminalUpdate {
                    patch_kind: protocol::PatchKind::ReplaceRows,
                    surface: input.surface,
                    cursor: input.cursor,
                    modes: input.modes,
                    title: input.title.to_owned(),
                    working_directory: input.working_directory.to_owned(),
                    colors: input.colors.clone(),
                    styles: vec![PaneStyle::default()],
                    surface_lines: input.surface_lines.to_vec(),
                    surface_row_runs: input
                        .surface_lines
                        .iter()
                        .map(|line| {
                            if let Some(split) = line.find(' ') {
                                vec![
                                    CellRun::plain(&line[..split]),
                                    CellRun::plain(&line[split..]),
                                ]
                            } else {
                                vec![CellRun::plain(line.clone())]
                            }
                        })
                        .collect(),
                    surface_semantic_prompts: input.surface_semantic_prompts.to_vec(),
                    surface_dirty_rows: input.surface_dirty_rows.to_vec(),
                    surface_kitty_placeholders: input.surface_kitty_placeholders.to_vec(),
                    scrollback_lines: input.scrollback_lines.to_vec(),
                    scrollback_row_runs: crate::terminal::plain_row_runs(input.scrollback_lines),
                    scrollback_semantic_prompts: input.scrollback_semantic_prompts.to_vec(),
                    scrollback_dirty_rows: input.scrollback_dirty_rows.to_vec(),
                    scrollback_kitty_placeholders: input.scrollback_kitty_placeholders.to_vec(),
                })
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
        let mut engine = RowRunOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"row runs", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::ReplaceRows)
        );
    }

    #[test]
    fn mode_only_engine_update_emits_mode_only_patch() {
        struct ModeOnlyEngine;

        impl TerminalEngine for ModeOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"mode only");
                let mut update = TerminalUpdate::plain(
                    protocol::PatchKind::ModeOnly,
                    input.surface,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    input.scrollback_lines.to_vec(),
                );
                update.modes = TerminalModes {
                    bracketed_paste: true,
                    mouse_tracking: true,
                    mouse_tracking_mode: protocol::MouseTrackingMode::Button,
                    mouse_format: protocol::MouseFormat::Sgr,
                    ..input.modes
                };
                Some(update)
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
        let mut engine = ModeOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"mode only", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::ModeOnly)
        );
        let frame = session.pane_surface_patch_frame("conn-1", 9, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::ModeOnly);
        assert_eq!(patch.row_updates().expect("row updates").len(), 0);
        let modes = patch.modes().expect("modes");
        assert!(modes.bracketed_paste());
        assert!(modes.mouse_tracking());
        assert_eq!(
            modes.mouse_tracking_mode(),
            protocol::MouseTrackingMode::Button
        );
        assert_eq!(modes.mouse_format(), protocol::MouseFormat::Sgr);
        assert!(modes.wraparound());
    }

    #[test]
    fn color_state_change_emits_color_only_patch() {
        struct ColorOnlyEngine;

        impl TerminalEngine for ColorOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"color only");
                let mut update = TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    input.scrollback_lines.to_vec(),
                );
                update.modes = input.modes;
                update.title = input.title.to_owned();
                update.working_directory = input.working_directory.to_owned();
                update.colors = TerminalColors {
                    default_fg_rgba: 0xeeeeeeff,
                    default_bg_rgba: 0x111111ff,
                    cursor_rgba: 0xff00ffff,
                    cursor_rgba_set: true,
                    palette_rgba: vec![0x000000ff, 0x112233ff],
                };
                Some(update)
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
        let mut engine = ColorOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"color only", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::ColorOnly)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 9, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::ColorOnly);
        let colors = patch.colors().expect("colors");
        assert_eq!(colors.default_fg_rgba(), 0xeeeeeeff);
        assert_eq!(colors.default_bg_rgba(), 0x111111ff);
        assert_eq!(colors.cursor_rgba(), 0xff00ffff);
        assert!(colors.cursor_rgba_set());
        assert!(colors.palette_rgba().is_none());
        assert_eq!(colors.palette_diff_start(), 0);
        let palette_diff = colors.palette_diff_rgba().expect("palette diff");
        assert_eq!(palette_diff.get(0), 0x000000ff);
        assert_eq!(palette_diff.get(1), 0x112233ff);
    }

    #[test]
    fn color_and_mode_only_engine_update_emits_color_only_patch_with_modes() {
        struct ColorAndModeOnlyEngine;

        impl TerminalEngine for ColorAndModeOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"color and mode only");
                let mut update = TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    input.scrollback_lines.to_vec(),
                );
                update.modes = TerminalModes {
                    bracketed_paste: true,
                    ..input.modes
                };
                update.title = input.title.to_owned();
                update.working_directory = input.working_directory.to_owned();
                update.colors = TerminalColors {
                    default_fg_rgba: 0xeeeeeeff,
                    default_bg_rgba: 0x111111ff,
                    cursor_rgba: 0xff00ffff,
                    cursor_rgba_set: true,
                    palette_rgba: vec![0x000000ff, 0x112233ff],
                };
                Some(update)
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
        let mut engine = ColorAndModeOnlyEngine;

        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"color and mode only",
            &mut engine
        ));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::ColorOnly)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 9, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::ColorOnly);
        assert_eq!(patch.row_updates().expect("row updates").len(), 0);
        assert!(patch.modes().expect("modes").bracketed_paste());
        let colors = patch.colors().expect("colors");
        assert_eq!(colors.cursor_rgba(), 0xff00ffff);
        assert!(colors.cursor_rgba_set());
        assert!(colors.palette_rgba().is_none());
        assert_eq!(colors.palette_diff_start(), 0);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn ghostty_vt_color_state_change_emits_color_only_patch() {
        let mut session = Session::initial();
        let mut engines = crate::terminal::PaneTerminalEngines::new(
            crate::terminal::TerminalEngineKind::LibghosttyVt,
        );

        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"color baseline",
            engines.engine_mut("pane-1")
        ));
        let base_version = session.surface_version("pane-1").expect("surface version");

        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"\x1b]4;1;#112233\x1b\\",
            engines.engine_mut("pane-1")
        ));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::ColorOnly)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 9, base_version);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::ColorOnly);
        let colors = patch.colors().expect("colors");
        assert!(colors.palette_rgba().is_none());
        assert_eq!(colors.palette_diff_start(), 1);
        let palette_diff = colors.palette_diff_rgba().expect("palette diff");
        assert_eq!(palette_diff.get(0), 0x112233ff);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn ghostty_vt_palette_change_for_existing_styled_row_requires_full_refresh() {
        let mut session = Session::initial();
        if let Some(pane) = session.pane_mut("pane-1") {
            pane.surface_lines.clear();
            pane.surface_row_runs.clear();
            pane.scrollback_lines.clear();
            pane.scrollback_row_runs.clear();
        }
        let mut engines = crate::terminal::PaneTerminalEngines::new(
            crate::terminal::TerminalEngineKind::LibghosttyVt,
        );

        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"\x1b[38;5;1mpalette-red\x1b[0m",
            engines.engine_mut("pane-1")
        ));
        let base_version = session.surface_version("pane-1").expect("surface version");

        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"\x1b]4;1;#112233\x1b\\",
            engines.engine_mut("pane-1")
        ));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::FullRefreshRequired),
            "palette changes that alter existing row styles need a full snapshot"
        );

        let patch_frame = session.pane_surface_patch_frame("conn-1", 9, base_version);
        let envelope =
            protocol::size_prefixed_root_as_envelope(&patch_frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::FullRefreshRequired);
        assert_eq!(patch.row_updates().expect("row updates").len(), 0);

        let snapshot_frame = session.pane_surface_frame("conn-1", 10);
        let envelope =
            protocol::size_prefixed_root_as_envelope(&snapshot_frame).expect("valid envelope");
        let snapshot = envelope.body_as_pane_surface_snapshot().expect("snapshot");
        let rows = snapshot.rows_data().expect("rows");
        let styled_run = (0..rows.len())
            .flat_map(|row_index| {
                let row = rows.get(row_index);
                let runs = row.runs().expect("runs");
                (0..runs.len()).map(move |run_index| runs.get(run_index))
            })
            .find(|run| {
                run.text_utf8()
                    .is_some_and(|text| text.contains("palette-red"))
            })
            .expect("palette styled run");
        let styles = snapshot.styles().expect("styles");
        let style = styles.get(styled_run.style_id() as usize);
        assert_eq!(
            style.fg_rgba(),
            0x112233ff,
            "full snapshot should carry the updated style table entry"
        );
    }

    #[test]
    fn color_state_change_with_row_changes_requires_full_refresh_patch() {
        struct ColorAndRowsEngine;

        impl TerminalEngine for ColorAndRowsEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"color and rows");
                let mut update = TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    vec!["repaint with new palette".to_owned()],
                    input.scrollback_lines.to_vec(),
                );
                update.modes = input.modes;
                update.title = input.title.to_owned();
                update.working_directory = input.working_directory.to_owned();
                update.colors = TerminalColors {
                    default_fg_rgba: 0xeeeeeeff,
                    default_bg_rgba: 0x111111ff,
                    cursor_rgba: 0,
                    cursor_rgba_set: false,
                    palette_rgba: vec![0x000000ff, 0x112233ff],
                };
                Some(update)
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
        let mut engine = ColorAndRowsEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"color and rows", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::FullRefreshRequired)
        );
    }

    #[test]
    fn title_only_engine_update_emits_cursor_only_patch_with_metadata() {
        struct TitleOnlyEngine;

        impl TerminalEngine for TitleOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"title only");
                let mut update = TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    input.scrollback_lines.to_vec(),
                );
                update.title = "pane title".to_owned();
                update.working_directory = "file://localhost/tmp/nmux".to_owned();
                Some(update)
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
        let mut engine = TitleOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"title only", &mut engine));
        let surface = session.initial_pane_surface();
        assert_eq!(surface.title, "pane title");
        assert_eq!(surface.working_directory, "file://localhost/tmp/nmux");
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::CursorOnly)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 9, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::CursorOnly);
        assert_eq!(patch.row_updates().expect("row updates").len(), 0);
        assert_eq!(
            patch.metadata().expect("metadata").title(),
            Some("pane title")
        );
        assert_eq!(
            patch.metadata().expect("metadata").working_directory(),
            Some("file://localhost/tmp/nmux")
        );
    }

    #[test]
    fn working_directory_only_engine_update_emits_cursor_only_patch_with_metadata() {
        struct WorkingDirectoryOnlyEngine;

        impl TerminalEngine for WorkingDirectoryOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"working directory only");
                let mut update = TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    input.scrollback_lines.to_vec(),
                );
                update.working_directory = "file://localhost/tmp/nmux".to_owned();
                Some(update)
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
        let mut engine = WorkingDirectoryOnlyEngine;

        assert!(session.apply_pane_output_with_engine(
            "pane-1",
            b"working directory only",
            &mut engine
        ));
        let surface = session.initial_pane_surface();
        assert_eq!(surface.title, "");
        assert_eq!(surface.working_directory, "file://localhost/tmp/nmux");
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::CursorOnly)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 9, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::CursorOnly);
        assert_eq!(patch.row_updates().expect("row updates").len(), 0);
        assert_eq!(patch.metadata().expect("metadata").title(), Some(""));
        assert_eq!(
            patch.metadata().expect("metadata").working_directory(),
            Some("file://localhost/tmp/nmux")
        );
    }

    #[test]
    fn semantic_prompt_only_engine_update_emits_replace_rows_patch() {
        struct SemanticPromptOnlyEngine;

        impl TerminalEngine for SemanticPromptOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"semantic prompt");
                let mut update = TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    input.scrollback_lines.to_vec(),
                );
                update.surface_semantic_prompts =
                    vec![protocol::RowSemanticPrompt::None; input.surface_lines.len()];
                update.scrollback_semantic_prompts =
                    vec![protocol::RowSemanticPrompt::None; input.scrollback_lines.len()];
                update.surface_semantic_prompts[0] = protocol::RowSemanticPrompt::Prompt;
                Some(update)
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
        let mut engine = SemanticPromptOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"semantic prompt", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::ReplaceRows)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 9, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::ReplaceRows);
        let rows = patch.row_updates().expect("row updates");
        assert_eq!(
            rows.get(0).semantic_prompt(),
            protocol::RowSemanticPrompt::Prompt
        );
    }

    #[test]
    fn semantic_content_only_engine_update_emits_replace_rows_patch() {
        struct SemanticContentOnlyEngine;

        impl TerminalEngine for SemanticContentOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"semantic content");
                let mut update = TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    input.scrollback_lines.to_vec(),
                );
                update.surface_row_runs = input
                    .surface_lines
                    .iter()
                    .map(|line| vec![CellRun::plain(line.clone())])
                    .collect();
                update.surface_row_runs[0][0].semantic_content =
                    protocol::CellSemanticContent::Prompt;
                Some(update)
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
        let mut engine = SemanticContentOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"semantic content", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::ReplaceRows)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 9, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::ReplaceRows);
        let rows = patch.row_updates().expect("row updates");
        assert_eq!(
            rows.get(0).runs().expect("runs").get(0).semantic_content(),
            protocol::CellSemanticContent::Prompt
        );
    }

    #[test]
    fn dirty_only_engine_update_emits_replace_rows_patch() {
        struct DirtyOnlyEngine;

        impl TerminalEngine for DirtyOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"dirty row");
                let mut update = TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    input.scrollback_lines.to_vec(),
                );
                update.surface_semantic_prompts = input.surface_semantic_prompts.to_vec();
                update.scrollback_semantic_prompts = input.scrollback_semantic_prompts.to_vec();
                update.surface_dirty_rows = vec![false; input.surface_lines.len()];
                update.scrollback_dirty_rows = vec![false; input.scrollback_lines.len()];
                update.surface_dirty_rows[0] = true;
                Some(update)
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
        let mut engine = DirtyOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"dirty row", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::ReplaceRows)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 9, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::ReplaceRows);
        let rows = patch.row_updates().expect("row updates");
        assert!(rows.get(0).dirty());
    }

    #[test]
    fn kitty_placeholder_only_engine_update_emits_replace_rows_patch() {
        struct KittyPlaceholderOnlyEngine;

        impl TerminalEngine for KittyPlaceholderOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"kitty placeholder");
                let mut update = TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    input.scrollback_lines.to_vec(),
                );
                update.surface_semantic_prompts = input.surface_semantic_prompts.to_vec();
                update.scrollback_semantic_prompts = input.scrollback_semantic_prompts.to_vec();
                update.surface_dirty_rows = input.surface_dirty_rows.to_vec();
                update.scrollback_dirty_rows = input.scrollback_dirty_rows.to_vec();
                update.surface_kitty_placeholders = vec![false; input.surface_lines.len()];
                update.scrollback_kitty_placeholders = vec![false; input.scrollback_lines.len()];
                update.surface_kitty_placeholders[0] = true;
                Some(update)
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
        let mut engine = KittyPlaceholderOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"kitty placeholder", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::ReplaceRows)
        );

        let frame = session.pane_surface_patch_frame("conn-1", 9, 2);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let patch = envelope.body_as_pane_surface_patch().expect("patch");
        assert_eq!(patch.kind(), protocol::PatchKind::ReplaceRows);
        let rows = patch.row_updates().expect("row updates");
        assert!(rows.get(0).kitty_virtual_placeholder());
    }

    #[test]
    fn scrollback_only_engine_update_does_not_bump_surface_version() {
        struct ScrollbackOnlyEngine;

        impl TerminalEngine for ScrollbackOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"history only");
                let mut scrollback_lines = input.scrollback_lines.to_vec();
                scrollback_lines.push("history only".to_owned());
                Some(TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    input.surface_lines.to_vec(),
                    scrollback_lines,
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
        let mut engine = ScrollbackOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"history only", &mut engine));

        let surface = session.initial_pane_surface();
        let scrollback = session.initial_scrollback();
        assert_eq!(surface.version, 2);
        assert_eq!(scrollback.version, 2);
        assert_eq!(
            surface.lines,
            vec![
                "nmux pane-1".to_owned(),
                "server-owned terminal state".to_owned()
            ]
        );
        assert_eq!(
            scrollback.lines,
            vec![
                "booting nmux workspace".to_owned(),
                "nmux pane-1".to_owned(),
                "server-owned terminal state".to_owned(),
                "history only".to_owned(),
            ]
        );
    }

    #[test]
    fn pane_output_surface_frame_uses_process_derived_lines() {
        let frame = Session::from_pane_output(b"pty says hi\n").pane_surface_frame("conn-1", 14);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");

        let snapshot = envelope
            .body_as_pane_surface_snapshot()
            .expect("pane surface body");
        assert_eq!(snapshot.version(), 3);

        let rows = snapshot.rows_data().expect("rows");
        assert_eq!(rows.len(), 1);
        let runs = rows.get(0).runs().expect("runs");
        assert_eq!(runs.get(0).text_utf8(), Some("pty says hi"));
    }

    #[test]
    fn engine_returning_more_rows_than_pane_size_is_truncated() {
        // Regression: if the terminal engine returns 25 lines for a 24-row
        // pane, the daemon would serialize row index 24, which the client
        // correctly rejects as out of bounds.
        struct ExtraRowEngine;

        impl TerminalEngine for ExtraRowEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                _output: &[u8],
            ) -> Option<TerminalUpdate> {
                // Return rows + 1 lines (e.g., 25 for a 24-row pane).
                let mut lines: Vec<String> = (0..input.rows as usize + 1)
                    .map(|i| format!("row-{i}"))
                    .collect();
                // Pad with empties if input already had fewer.
                while lines.len() < input.rows as usize + 1 {
                    lines.push(String::new());
                }
                Some(TerminalUpdate::plain(
                    protocol::PatchKind::ReplaceRows,
                    input.surface,
                    input.cursor,
                    lines,
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
        let mut engine = ExtraRowEngine;

        // Initial pane is 24 rows. Engine returns 25 lines.
        session.apply_pane_output_with_engine("pane-1", b"trigger", &mut engine);

        // The surface frame must serialize without out-of-bounds row indices.
        let frame = session.pane_surface_frame("conn-1", 1);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope
            .body_as_pane_surface_snapshot()
            .expect("pane surface body");

        // Must have exactly 24 rows (0-indexed 0..23), not 25.
        let rows = snapshot.rows_data().expect("rows");
        assert_eq!(rows.len(), 24);
        assert_eq!(rows.get(23).row(), 23);
    }
}
