use flatbuffers::FlatBufferBuilder;
use nmux_proto::{PROTOCOL_VERSION, protocol};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub version: u64,
    pub active_tab_id: String,
    pub tabs: Vec<Tab>,
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
    pub surface_version: u64,
    pub cols: u32,
    pub rows: u32,
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
                    surface_version: 0,
                    cols: 80,
                    rows: 24,
                },
            }],
        }
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
}

#[cfg(test)]
mod tests {
    use nmux_proto::{PROTOCOL_VERSION, protocol};

    use super::Session;

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
        assert_eq!(tab.root.cols, 80);
        assert_eq!(tab.root.rows, 24);
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
        assert_eq!(pane.surface_version(), 0);
        assert_eq!(pane.cols(), 80);
        assert_eq!(pane.rows(), 24);
        assert_eq!(pane.resize_policy(), protocol::ResizePolicy::Fixed);
    }
}
