use flatbuffers::FlatBufferBuilder;
use nmux_proto::{PROTOCOL_VERSION, protocol};

use crate::host::{CommandSpec, HostSpec};

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
    pub cols: u32,
    pub rows: u32,
    pub surface_lines: Vec<String>,
    pub scrollback_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneSurface {
    pub pane_id: String,
    pub version: u64,
    pub cols: u32,
    pub rows: u32,
    pub cursor: Cursor,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneScrollback {
    pub pane_id: String,
    pub version: u64,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub row: u32,
    pub col: u32,
    pub visible: bool,
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
                    cols: 80,
                    rows: 24,
                    surface_lines: vec![
                        "nmux pane-1".to_owned(),
                        "server-owned terminal state".to_owned(),
                    ],
                    scrollback_lines: vec![
                        "booting nmux workspace".to_owned(),
                        "nmux pane-1".to_owned(),
                        "server-owned terminal state".to_owned(),
                    ],
                },
            }],
        }
    }

    pub fn from_pane_output(output: &[u8]) -> Self {
        let mut session = Self::initial();
        if let Some(pane) = session.pane_mut("pane-1") {
            pane.surface_lines.clear();
            pane.scrollback_lines.clear();
        }
        session.apply_pane_output("pane-1", output);
        session
    }

    pub fn apply_pane_output(&mut self, pane_id: &str, output: &[u8]) -> bool {
        let Some(pane) = self.pane_mut(pane_id) else {
            return false;
        };

        let lines = text_lines_from_pty_output(output);
        pane.scrollback_lines.extend(lines);
        let visible_start = pane
            .scrollback_lines
            .len()
            .saturating_sub(pane.rows as usize);
        pane.surface_lines = pane.scrollback_lines[visible_start..].to_vec();
        pane.surface_version = pane.surface_version.saturating_add(1);
        true
    }

    pub fn commit_pane_resize(&mut self, pane_id: &str, cols: u32, rows: u32) -> bool {
        let Some(pane) = self.pane_mut(pane_id) else {
            return false;
        };
        if pane.cols == cols && pane.rows == rows {
            return false;
        }

        pane.cols = cols;
        pane.rows = rows;
        let visible_start = pane
            .scrollback_lines
            .len()
            .saturating_sub(pane.rows as usize);
        pane.surface_lines = pane.scrollback_lines[visible_start..].to_vec();
        self.version = self.version.saturating_add(1);
        true
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
                    resize_policy: protocol::ResizePolicy::Fixed,
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
        let pane = &self.tabs[0].root;
        PaneSurface {
            pane_id: pane.id.clone(),
            version: pane.surface_version,
            cols: pane.cols,
            rows: pane.rows,
            cursor: Cursor {
                row: pane.surface_lines.len().saturating_sub(1) as u32,
                col: 0,
                visible: true,
            },
            lines: pane.surface_lines.clone(),
        }
    }

    pub fn initial_scrollback(&self) -> PaneScrollback {
        let pane = &self.tabs[0].root;
        PaneScrollback {
            pane_id: pane.id.clone(),
            version: 1,
            lines: pane.scrollback_lines.clone(),
        }
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
        let surface = self.initial_pane_surface();
        let mut builder = FlatBufferBuilder::new();

        let mut row_offsets = Vec::with_capacity(surface.lines.len());
        for (row, line) in surface.lines.iter().enumerate() {
            let run = build_cell_run(&mut builder, line);
            let runs = builder.create_vector(&[run]);
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
        let style = protocol::Style::create(&mut builder, &protocol::StyleArgs::default());
        let styles = builder.create_vector(&[style]);
        let cursor = protocol::CursorState::create(
            &mut builder,
            &protocol::CursorStateArgs {
                row: surface.cursor.row,
                col: surface.cursor.col,
                visible: surface.cursor.visible,
                shape: protocol::CursorShape::Block,
            },
        );
        let pane_id = builder.create_string(&surface.pane_id);
        let snapshot = protocol::PaneSurfaceSnapshot::create(
            &mut builder,
            &protocol::PaneSurfaceSnapshotArgs {
                pane_id: Some(pane_id),
                version: surface.version,
                surface: protocol::SurfaceKind::Main,
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
        builder.finished_data().to_vec()
    }

    pub fn pane_surface_patch_frame(
        &self,
        connection_id: &str,
        seq: u64,
        base_version: u64,
    ) -> Vec<u8> {
        let surface = self.initial_pane_surface();
        let mut builder = FlatBufferBuilder::new();

        let mut row_offsets = Vec::with_capacity(surface.lines.len());
        for (row, line) in surface.lines.iter().enumerate() {
            let run = build_cell_run(&mut builder, line);
            let runs = builder.create_vector(&[run]);
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

        let row_updates = builder.create_vector(&row_offsets);
        let cursor = protocol::CursorState::create(
            &mut builder,
            &protocol::CursorStateArgs {
                row: surface.cursor.row,
                col: surface.cursor.col,
                visible: surface.cursor.visible,
                shape: protocol::CursorShape::Block,
            },
        );
        let pane_id = builder.create_string(&surface.pane_id);
        let patch = protocol::PaneSurfacePatch::create(
            &mut builder,
            &protocol::PaneSurfacePatchArgs {
                pane_id: Some(pane_id),
                base_version,
                version: surface.version,
                kind: protocol::PatchKind::ReplaceRows,
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
        builder.finished_data().to_vec()
    }

    pub fn scrollback_chunk_frame(
        &self,
        connection_id: &str,
        seq: u64,
        start_line: u64,
        line_count: u32,
    ) -> Vec<u8> {
        let scrollback = self.initial_scrollback();
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
            let run = build_cell_run(&mut builder, line);
            let runs = builder.create_vector(&[run]);
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
        builder.finished_data().to_vec()
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

fn stable_row_hash(line: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in line.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn build_cell_run<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    line: &str,
) -> flatbuffers::WIPOffset<protocol::CellRun<'a>> {
    let text = builder.create_string(line);
    let widths = vec![1_u8; line.chars().count()];
    let widths = builder.create_vector(&widths);
    protocol::CellRun::create(
        builder,
        &protocol::CellRunArgs {
            text_utf8: Some(text),
            cell_widths: Some(widths),
            style_id: 0,
            flags: 0,
            hyperlink_id: 0,
        },
    )
}

fn text_lines_from_pty_output(output: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(output);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = text
        .lines()
        .map(|line| {
            line.chars()
                .filter(|ch| *ch == '\t' || !ch.is_control())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

#[cfg(test)]
mod tests {
    use crate::host::HostKind;

    use nmux_proto::{PROTOCOL_VERSION, protocol};

    use super::{AttachMode, Session};

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
        assert_eq!(pane.cols(), 100);
        assert_eq!(pane.rows(), 30);
        assert_eq!(pane.resize_policy(), protocol::ResizePolicy::Fixed);
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
