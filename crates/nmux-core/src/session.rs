use flatbuffers::FlatBufferBuilder;
use nmux_proto::{PROTOCOL_VERSION, protocol};

use crate::host::{CommandSpec, HostSpec};
use crate::terminal::{InterimTextTerminalEngine, TerminalCursor, TerminalEngine, TerminalInput};

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
    pub host: HostSpec,
    pub surface_version: u64,
    pub last_patch_kind: protocol::PatchKind,
    pub scrollback_version: u64,
    pub cols: u32,
    pub rows: u32,
    pub resize_policy: protocol::ResizePolicy,
    pub surface: protocol::SurfaceKind,
    pub cursor: Cursor,
    pub styles: Vec<PaneStyle>,
    pub surface_lines: Vec<String>,
    pub surface_row_runs: Vec<Vec<CellRun>>,
    pub scrollback_lines: Vec<String>,
    pub scrollback_row_runs: Vec<Vec<CellRun>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneSurface {
    pub pane_id: String,
    pub version: u64,
    pub cols: u32,
    pub rows: u32,
    pub surface: protocol::SurfaceKind,
    pub cursor: Cursor,
    pub styles: Vec<PaneStyle>,
    pub lines: Vec<String>,
    pub row_runs: Vec<Vec<CellRun>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneScrollback {
    pub pane_id: String,
    pub version: u64,
    pub lines: Vec<String>,
    pub row_runs: Vec<Vec<CellRun>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub row: u32,
    pub col: u32,
    pub visible: bool,
    pub shape: protocol::CursorShape,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneStyle {
    pub fg_rgba: u32,
    pub bg_rgba: u32,
    pub underline_rgba: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRun {
    pub text: String,
    pub cell_widths: Vec<u8>,
    pub style_id: u32,
    pub flags: u32,
    pub hyperlink_id: u32,
}

impl CellRun {
    pub fn plain(text: impl Into<String>) -> Self {
        let text = text.into();
        let cell_widths = vec![1_u8; text.chars().count()];
        Self {
            text,
            cell_widths,
            style_id: 0,
            flags: 0,
            hyperlink_id: 0,
        }
    }
}

impl Session {
    pub fn initial() -> Self {
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
                    host: HostSpec::local("local", CommandSpec::new("sh")),
                    surface_version: 2,
                    last_patch_kind: protocol::PatchKind::ReplaceRows,
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
                    },
                    styles: vec![PaneStyle::default()],
                    surface_lines: vec![
                        "nmux pane-1".to_owned(),
                        "server-owned terminal state".to_owned(),
                    ],
                    surface_row_runs: vec![
                        vec![CellRun::plain("nmux pane-1")],
                        vec![CellRun::plain("server-owned terminal state")],
                    ],
                    scrollback_lines: vec![
                        "booting nmux workspace".to_owned(),
                        "nmux pane-1".to_owned(),
                        "server-owned terminal state".to_owned(),
                    ],
                    scrollback_row_runs: vec![
                        vec![CellRun::plain("booting nmux workspace")],
                        vec![CellRun::plain("nmux pane-1")],
                        vec![CellRun::plain("server-owned terminal state")],
                    ],
                },
            }],
        }
    }

    pub fn from_pane_output(output: &[u8]) -> Self {
        let mut session = Self::initial();
        if let Some(pane) = session.pane_mut("pane-1") {
            pane.surface_lines.clear();
            pane.surface_row_runs.clear();
            pane.scrollback_lines.clear();
            pane.scrollback_row_runs.clear();
        }
        session.apply_pane_output("pane-1", output);
        session
    }

    pub fn apply_pane_output(&mut self, pane_id: &str, output: &[u8]) -> bool {
        let mut engine = InterimTextTerminalEngine;
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
            surface_lines: &pane.surface_lines,
            scrollback_lines: &pane.scrollback_lines,
        };
        let Some(update) = engine.apply_output(input, output) else {
            return false;
        };

        apply_terminal_update(pane, update, false)
    }

    pub fn commit_pane_resize(&mut self, pane_id: &str, cols: u32, rows: u32) -> bool {
        let mut engine = InterimTextTerminalEngine;
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
            surface_lines: &pane.surface_lines,
            scrollback_lines: &pane.scrollback_lines,
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
            let pane_id = builder.create_string(&tab.root.id);
            let pane = protocol::PaneNode::create(
                &mut builder,
                &protocol::PaneNodeArgs {
                    pane_id: Some(pane_id),
                    kind: protocol::PaneKind::Pty,
                    split_axis: protocol::SplitAxis::None,
                    children: None,
                    surface_version: tab.root.surface_version,
                    cols: tab.root.cols,
                    rows: tab.root.rows,
                    resize_policy: tab.root.resize_policy,
                },
            );

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
            styles: pane.styles.clone(),
            lines: pane.surface_lines.clone(),
            row_runs: row_runs_for_lines(&pane.surface_lines, &pane.surface_row_runs),
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
            lines: pane.scrollback_lines.clone(),
            row_runs: row_runs_for_lines(&pane.scrollback_lines, &pane.scrollback_row_runs),
        })
    }

    pub fn surface_version(&self, pane_id: &str) -> Option<u64> {
        self.tabs.iter().find_map(|tab| {
            if tab.root.id == pane_id {
                Some(tab.root.surface_version)
            } else {
                None
            }
        })
    }

    pub fn surface_patch_kind(&self, pane_id: &str) -> Option<protocol::PatchKind> {
        self.pane(pane_id).map(|pane| pane.last_patch_kind)
    }

    fn pane(&self, pane_id: &str) -> Option<&Pane> {
        self.tabs.iter().find_map(|tab| {
            if tab.root.id == pane_id {
                Some(&tab.root)
            } else {
                None
            }
        })
    }

    fn pane_mut(&mut self, pane_id: &str) -> Option<&mut Pane> {
        self.tabs.iter_mut().find_map(|tab| {
            if tab.root.id == pane_id {
                Some(&mut tab.root)
            } else {
                None
            }
        })
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
            let row = protocol::SurfaceRow::create(
                &mut builder,
                &protocol::SurfaceRowArgs {
                    row: row as u32,
                    runs: Some(runs),
                    dirty_hash: stable_row_hash(line),
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
        let cursor = protocol::CursorState::create(
            &mut builder,
            &protocol::CursorStateArgs {
                row: surface.cursor.row,
                col: surface.cursor.col,
                visible: surface.cursor.visible,
                shape: surface.cursor.shape,
            },
        );
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
                styles: Some(styles),
                rows_data: Some(rows_data),
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
        let mut builder = FlatBufferBuilder::new();

        let mut row_offsets = Vec::new();
        if patch_kind == protocol::PatchKind::ReplaceRows {
            row_offsets.reserve(surface.lines.len());
            for (row, (line, line_runs)) in surface
                .lines
                .iter()
                .zip(surface.row_runs.iter())
                .enumerate()
            {
                let runs = build_cell_runs(&mut builder, line_runs);
                let row = protocol::RowUpdate::create(
                    &mut builder,
                    &protocol::RowUpdateArgs {
                        row: row as u32,
                        runs: Some(runs),
                        dirty_hash: stable_row_hash(line),
                    },
                );
                row_offsets.push(row);
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
            },
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
        let scrollback = self.pane_scrollback(pane_id)?;
        let mut builder = FlatBufferBuilder::new();

        let start = usize::try_from(start_line).unwrap_or(usize::MAX);
        let count = line_count as usize;
        let end = start.saturating_add(count).min(scrollback.lines.len());
        let selected = scrollback
            .lines
            .get(start..end)
            .map_or(&[][..], |rows| rows);

        let mut row_offsets = Vec::with_capacity(selected.len());
        for (offset, line) in selected.iter().enumerate() {
            let runs = build_cell_runs(
                &mut builder,
                &scrollback.row_runs[start.saturating_add(offset)],
            );
            let row = protocol::ScrollbackRow::create(
                &mut builder,
                &protocol::ScrollbackRowArgs {
                    line: start_line + offset as u64,
                    runs: Some(runs),
                    dirty_hash: stable_row_hash(line),
                },
            );
            row_offsets.push(row);
        }

        let rows = builder.create_vector(&row_offsets);
        let pane_id = builder.create_string(&scrollback.pane_id);
        let chunk = protocol::ScrollbackChunk::create(
            &mut builder,
            &protocol::ScrollbackChunkArgs {
                pane_id: Some(pane_id),
                scrollback_version: scrollback.version,
                start_line,
                total_lines: scrollback.lines.len() as u64,
                rows: Some(rows),
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
        connection_id: &str,
        seq: u64,
        actor_id: &str,
        pane_id: &str,
        start_line: u64,
        line_count: u32,
        known_scrollback_version: u64,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        let pane_id = builder.create_string(pane_id);
        let actor_id = builder.create_string(actor_id);
        let fetch = protocol::ScrollbackFetch::create(
            &mut builder,
            &protocol::ScrollbackFetchArgs {
                pane_id: Some(pane_id),
                actor_id: Some(actor_id),
                start_line,
                line_count,
                known_scrollback_version,
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

impl From<&Cursor> for TerminalCursor {
    fn from(cursor: &Cursor) -> Self {
        Self {
            row: cursor.row,
            col: cursor.col,
            visible: cursor.visible,
            shape: cursor.shape,
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
        }
    }
}

fn apply_terminal_update(
    pane: &mut Pane,
    update: crate::terminal::TerminalUpdate,
    force_surface_version: bool,
) -> bool {
    let cursor = Cursor::from(update.cursor);
    let rows_changed = pane.surface_lines != update.surface_lines;
    let surface_kind_changed = pane.surface != update.surface;
    let surface_changed =
        force_surface_version || surface_kind_changed || rows_changed || pane.cursor != cursor;
    let scrollback_changed = pane.scrollback_lines != update.scrollback_lines;

    pane.scrollback_lines = update.scrollback_lines;
    pane.scrollback_row_runs = row_runs_for_lines(&pane.scrollback_lines, &[]);
    pane.surface = update.surface;
    pane.surface_lines = update.surface_lines;
    pane.surface_row_runs = row_runs_for_lines(&pane.surface_lines, &[]);
    pane.cursor = cursor;

    if scrollback_changed {
        pane.scrollback_version = pane.scrollback_version.saturating_add(1);
    }

    if surface_changed {
        pane.surface_version = pane.surface_version.saturating_add(1);
        pane.last_patch_kind =
            terminal_patch_kind(update.patch_kind, rows_changed, surface_kind_changed);
    }

    surface_changed || scrollback_changed
}

fn terminal_patch_kind(
    requested: protocol::PatchKind,
    rows_changed: bool,
    surface_kind_changed: bool,
) -> protocol::PatchKind {
    if surface_kind_changed {
        protocol::PatchKind::FullRefreshRequired
    } else if rows_changed {
        protocol::PatchKind::ReplaceRows
    } else if requested == protocol::PatchKind::ModeOnly {
        protocol::PatchKind::FullRefreshRequired
    } else {
        requested
    }
}

fn stable_row_hash(line: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in line.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
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

fn cell_runs_text(runs: &[CellRun]) -> String {
    let mut text = String::new();
    for run in runs {
        text.push_str(&run.text);
    }
    text
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
            hyperlink_id: run.hyperlink_id,
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::host::HostKind;
    use crate::terminal::{TerminalCursor, TerminalEngine, TerminalInput, TerminalUpdate};

    use nmux_proto::{PROTOCOL_VERSION, protocol};

    use super::{AttachMode, CellRun, Cursor, PaneStyle, Session};

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

        let cursor = snapshot.cursor().expect("cursor");
        assert_eq!(cursor.row(), 1);
        assert_eq!(cursor.col(), 0);
        assert!(cursor.visible());
        assert_eq!(cursor.shape(), protocol::CursorShape::Block);

        let styles = snapshot.styles().expect("styles");
        assert_eq!(styles.len(), 1);

        let rows = snapshot.rows_data().expect("rows");
        assert_eq!(rows.len(), 2);

        let first_row = rows.get(0);
        assert_eq!(first_row.row(), 0);
        assert_ne!(first_row.dirty_hash(), 0);
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
                flags: 0,
                hyperlink_id: 0,
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

        let rows = snapshot.rows_data().expect("rows");
        let runs = rows.get(0).runs().expect("runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs.get(0).text_utf8(), Some("red"));
        assert_eq!(runs.get(0).style_id(), 1);
        assert_eq!(runs.get(0).cell_widths().expect("widths").len(), 3);
        assert_eq!(runs.get(1).text_utf8(), Some(" plain"));
        assert_eq!(runs.get(1).style_id(), 0);
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

        let cursor = patch.cursor().expect("cursor");
        assert_eq!(cursor.row(), 1);
        assert_eq!(cursor.col(), 0);

        let rows = patch.row_updates().expect("row updates");
        assert_eq!(rows.len(), 2);

        let first_row = rows.get(0);
        assert_eq!(first_row.row(), 0);
        let first_runs = first_row.runs().expect("runs");
        assert_eq!(first_runs.get(0).text_utf8(), Some("nmux pane-1"));
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
        let first_runs = first.runs().expect("first runs");
        assert_eq!(first_runs.get(0).text_utf8(), Some("nmux pane-1"));

        let second = rows.get(1);
        assert_eq!(second.line(), 2);
        let second_runs = second.runs().expect("second runs");
        assert_eq!(
            second_runs.get(0).text_utf8(),
            Some("server-owned terminal state")
        );
    }

    #[test]
    fn scrollback_chunk_frame_is_pane_scoped() {
        let session = Session::initial();

        assert!(
            session
                .scrollback_chunk_frame_for_pane("conn-1", 11, "missing", 0, 2)
                .is_none()
        );

        let frame = session
            .scrollback_chunk_frame_for_pane("conn-1", 11, "pane-1", 0, 1)
            .expect("pane scrollback");
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let chunk = envelope.body_as_scrollback_chunk().expect("chunk");
        assert_eq!(chunk.pane_id(), Some("pane-1"));
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
            b"one\r\ntwo\r\nthree\r\nfour",
            engines.engine_mut("pane-1")
        ));

        let surface = session.initial_pane_surface();
        let frame = session
            .scrollback_chunk_frame_for_pane("conn-1", 11, "pane-1", 0, 10)
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
        let frame =
            Session::initial().scrollback_fetch_frame("conn-1", 12, "actor-1", "pane-1", 1, 2, 1);
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
                        shape: protocol::CursorShape::Block
                    }
                );
                assert_eq!(input.surface_lines.len(), 2);
                assert_eq!(input.scrollback_lines.len(), 3);
                assert_eq!(output, b"ignored by test engine");
                Some(TerminalUpdate {
                    patch_kind: protocol::PatchKind::ReplaceRows,
                    surface: protocol::SurfaceKind::Alternate,
                    cursor: TerminalCursor {
                        row: 7,
                        col: 8,
                        visible: false,
                        shape: protocol::CursorShape::Beam,
                    },
                    surface_lines: vec!["engine surface".to_owned()],
                    scrollback_lines: vec!["engine scrollback".to_owned()],
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
                shape: protocol::CursorShape::Beam
            }
        );
        let frame = session.pane_surface_frame("conn-1", 1);
        let envelope = protocol::size_prefixed_root_as_envelope(&frame).expect("valid envelope");
        let snapshot = envelope.body_as_pane_surface_snapshot().expect("snapshot");
        let cursor = snapshot.cursor().expect("cursor");
        assert_eq!(cursor.shape(), protocol::CursorShape::Beam);
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
                Some(TerminalUpdate {
                    patch_kind: protocol::PatchKind::ReplaceRows,
                    surface: input.surface,
                    cursor: TerminalCursor {
                        row: 3,
                        col: 4,
                        visible: true,
                        shape: protocol::CursorShape::Underline,
                    },
                    surface_lines: vec!["resized surface".to_owned()],
                    scrollback_lines: vec!["resized scrollback".to_owned()],
                })
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
                shape: protocol::CursorShape::Underline
            }
        );
        assert_eq!(surface.lines, vec!["resized surface".to_owned()]);
        assert_eq!(scrollback.lines, vec!["resized scrollback".to_owned()]);
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
                        Some(TerminalUpdate {
                            patch_kind: protocol::PatchKind::ReplaceRows,
                            surface: input.surface,
                            cursor: TerminalCursor {
                                row: 0,
                                col: 5,
                                visible: true,
                                shape: input.cursor.shape,
                            },
                            surface_lines: vec!["first".to_owned()],
                            scrollback_lines: vec!["first".to_owned()],
                        })
                    }
                    2 => {
                        assert_eq!(input.pane_id, "pane-1");
                        assert_eq!(input.cols, 100);
                        assert_eq!(input.rows, 10);
                        assert_eq!(input.surface_lines, ["first resized"]);
                        assert_eq!(input.scrollback_lines, ["first"]);
                        assert_eq!(output, b"second");
                        self.step = 3;
                        Some(TerminalUpdate {
                            patch_kind: protocol::PatchKind::ReplaceRows,
                            surface: input.surface,
                            cursor: TerminalCursor {
                                row: 1,
                                col: 6,
                                visible: true,
                                shape: protocol::CursorShape::Beam,
                            },
                            surface_lines: vec!["first resized".to_owned(), "second".to_owned()],
                            scrollback_lines: vec!["first".to_owned(), "second".to_owned()],
                        })
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
                Some(TerminalUpdate {
                    patch_kind: protocol::PatchKind::ReplaceRows,
                    surface: input.surface,
                    cursor: TerminalCursor {
                        row: 0,
                        col: 5,
                        visible: true,
                        shape: input.cursor.shape,
                    },
                    surface_lines: vec!["first resized".to_owned()],
                    scrollback_lines: input.scrollback_lines.to_vec(),
                })
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
                shape: protocol::CursorShape::Beam
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
                Some(TerminalUpdate {
                    patch_kind: protocol::PatchKind::CursorOnly,
                    surface: input.surface,
                    cursor: TerminalCursor {
                        row: 1,
                        col: 12,
                        visible: true,
                        shape: protocol::CursorShape::Beam,
                    },
                    surface_lines: input.surface_lines.to_vec(),
                    scrollback_lines: input.scrollback_lines.to_vec(),
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
    }

    #[test]
    fn mode_only_engine_update_requires_full_refresh_until_modes_are_modeled() {
        struct ModeOnlyEngine;

        impl TerminalEngine for ModeOnlyEngine {
            fn apply_output(
                &mut self,
                input: TerminalInput<'_>,
                output: &[u8],
            ) -> Option<TerminalUpdate> {
                assert_eq!(output, b"mode only");
                Some(TerminalUpdate {
                    patch_kind: protocol::PatchKind::ModeOnly,
                    surface: input.surface,
                    cursor: TerminalCursor {
                        row: input.cursor.row,
                        col: input.cursor.col,
                        visible: input.cursor.visible,
                        shape: protocol::CursorShape::Beam,
                    },
                    surface_lines: input.surface_lines.to_vec(),
                    scrollback_lines: input.scrollback_lines.to_vec(),
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
        let mut engine = ModeOnlyEngine;

        assert!(session.apply_pane_output_with_engine("pane-1", b"mode only", &mut engine));
        assert_eq!(
            session.surface_patch_kind("pane-1"),
            Some(protocol::PatchKind::FullRefreshRequired)
        );
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
                Some(TerminalUpdate {
                    patch_kind: protocol::PatchKind::ReplaceRows,
                    surface: input.surface,
                    cursor: input.cursor,
                    surface_lines: input.surface_lines.to_vec(),
                    scrollback_lines,
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
}
