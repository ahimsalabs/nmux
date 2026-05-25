use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nmux_proto::{protocol, wire};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReliableFrame {
    pub bytes: Vec<u8>,
}

impl ReliableFrame {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingSurfaceSignal {
    pub pane_id: String,
    pub version: u64,
    pub snapshot_required: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SurfaceSignalSet {
    pub generation: u64,
    pub panes: BTreeMap<String, PendingSurfaceSignal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClientOutputError {
    ReliableFrameQueueFull {
        cap: usize,
        frame_len: usize,
    },
    ReliableByteQueueFull {
        cap: usize,
        pending: usize,
        frame_len: usize,
    },
    ReliableQueueClosed,
    SurfaceQueueClosed,
    Wire(String),
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClientWriteEvent {
    Reliable(ReliableFrame),
    Surface(SurfaceSignalSet),
    Closed,
}

pub(crate) struct AsyncClientOutput {
    reliable_tx: mpsc::Sender<ReliableFrame>,
    surface_tx: watch::Sender<SurfaceSignalSet>,
    latest_surfaces: SurfaceSignalSet,
    reliable_frame_cap: usize,
    reliable_byte_cap: usize,
    pending_reliable_bytes: Arc<AtomicUsize>,
}

pub(crate) struct AsyncClientOutputRx {
    reliable_rx: mpsc::Receiver<ReliableFrame>,
    surface_rx: watch::Receiver<SurfaceSignalSet>,
    pending_reliable_bytes: Arc<AtomicUsize>,
    reliable_closed: bool,
    surface_closed: bool,
}

pub(crate) trait SurfaceFrameSource {
    fn surface_patch_kind(&self, pane_id: &str) -> protocol::PatchKind;
    fn surface_snapshot_frame(&mut self, pane_id: &str, seq: u64) -> Option<Vec<u8>>;
    fn surface_patch_frame(
        &mut self,
        pane_id: &str,
        base_version: u64,
        seq: u64,
    ) -> Option<Vec<u8>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClientWriteTaskState {
    next_seq: u64,
    known_surface_versions: BTreeMap<String, u64>,
}

impl ClientWriteTaskState {
    pub(crate) fn new(next_seq: u64, known_surface_versions: BTreeMap<String, u64>) -> Self {
        Self {
            next_seq,
            known_surface_versions,
        }
    }

    pub(crate) fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub(crate) fn known_surface_version(&self, pane_id: &str) -> Option<u64> {
        self.known_surface_versions.get(pane_id).copied()
    }
}

impl AsyncClientOutput {
    pub(crate) fn new(
        reliable_frame_cap: usize,
        reliable_byte_cap: usize,
    ) -> (Self, AsyncClientOutputRx) {
        let (reliable_tx, reliable_rx) = mpsc::channel(reliable_frame_cap);
        let latest_surfaces = SurfaceSignalSet::default();
        let (surface_tx, surface_rx) = watch::channel(latest_surfaces.clone());
        let pending_reliable_bytes = Arc::new(AtomicUsize::new(0));
        (
            Self {
                reliable_tx,
                surface_tx,
                latest_surfaces,
                reliable_frame_cap,
                reliable_byte_cap,
                pending_reliable_bytes: Arc::clone(&pending_reliable_bytes),
            },
            AsyncClientOutputRx {
                reliable_rx,
                surface_rx,
                pending_reliable_bytes,
                reliable_closed: false,
                surface_closed: false,
            },
        )
    }

    pub(crate) fn try_send_reliable(&self, frame: ReliableFrame) -> Result<(), ClientOutputError> {
        let frame_len = frame.bytes.len();
        self.reserve_reliable_bytes(frame_len)?;
        match self.reliable_tx.try_send(frame) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.release_reliable_bytes(frame_len);
                Err(ClientOutputError::ReliableFrameQueueFull {
                    cap: self.reliable_frame_cap,
                    frame_len,
                })
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.release_reliable_bytes(frame_len);
                Err(ClientOutputError::ReliableQueueClosed)
            }
        }
    }

    pub(crate) fn signal_surface_changed(
        &mut self,
        pane_id: impl Into<String>,
        version: u64,
    ) -> Result<(), ClientOutputError> {
        self.update_surface_signal(pane_id.into(), version, false)
    }

    pub(crate) fn require_surface_snapshot(
        &mut self,
        pane_id: impl Into<String>,
        version: u64,
    ) -> Result<(), ClientOutputError> {
        self.update_surface_signal(pane_id.into(), version, true)
    }

    fn update_surface_signal(
        &mut self,
        pane_id: String,
        version: u64,
        snapshot_required: bool,
    ) -> Result<(), ClientOutputError> {
        let snapshot_required = snapshot_required
            || self
                .latest_surfaces
                .panes
                .get(&pane_id)
                .is_some_and(|pending| pending.snapshot_required);
        self.latest_surfaces.generation = self.latest_surfaces.generation.saturating_add(1);
        self.latest_surfaces.panes.insert(
            pane_id.clone(),
            PendingSurfaceSignal {
                pane_id,
                version,
                snapshot_required,
            },
        );
        self.surface_tx
            .send(self.latest_surfaces.clone())
            .map_err(|_| ClientOutputError::SurfaceQueueClosed)
    }

    fn reserve_reliable_bytes(&self, frame_len: usize) -> Result<(), ClientOutputError> {
        let mut pending = self.pending_reliable_bytes.load(Ordering::Relaxed);
        loop {
            let Some(next) = pending.checked_add(frame_len) else {
                return Err(ClientOutputError::ReliableByteQueueFull {
                    cap: self.reliable_byte_cap,
                    pending,
                    frame_len,
                });
            };
            if next > self.reliable_byte_cap {
                return Err(ClientOutputError::ReliableByteQueueFull {
                    cap: self.reliable_byte_cap,
                    pending,
                    frame_len,
                });
            }
            match self.pending_reliable_bytes.compare_exchange_weak(
                pending,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => pending = observed,
            }
        }
    }

    fn release_reliable_bytes(&self, frame_len: usize) {
        self.pending_reliable_bytes
            .fetch_sub(frame_len, Ordering::AcqRel);
    }
}

impl AsyncClientOutputRx {
    pub(crate) async fn recv_reliable(&mut self) -> Option<ReliableFrame> {
        let frame = self.reliable_rx.recv().await?;
        self.release_reliable_frame(&frame);
        Some(frame)
    }

    pub(crate) async fn changed_surfaces(&mut self) -> Result<SurfaceSignalSet, ClientOutputError> {
        self.surface_rx
            .changed()
            .await
            .map_err(|_| ClientOutputError::SurfaceQueueClosed)?;
        Ok(self.surface_rx.borrow_and_update().clone())
    }

    pub(crate) async fn recv_write_event(&mut self) -> ClientWriteEvent {
        loop {
            if self.reliable_closed && self.surface_closed {
                return ClientWriteEvent::Closed;
            }
            tokio::select! {
                biased;
                reliable = self.reliable_rx.recv(), if !self.reliable_closed => {
                    if let Some(frame) = reliable {
                        self.release_reliable_frame(&frame);
                        return ClientWriteEvent::Reliable(frame);
                    }
                    self.reliable_closed = true;
                }
                changed = self.surface_rx.changed(), if !self.surface_closed => {
                    match changed {
                        Ok(()) => return ClientWriteEvent::Surface(self.surface_rx.borrow_and_update().clone()),
                        Err(_) => self.surface_closed = true,
                    }
                }
            }
        }
    }

    fn release_reliable_frame(&self, frame: &ReliableFrame) {
        self.pending_reliable_bytes
            .fetch_sub(frame.bytes.len(), Ordering::AcqRel);
    }
}

pub(crate) async fn run_client_write_task<W, S>(
    mut writer: W,
    mut output_rx: AsyncClientOutputRx,
    surface_source: &mut S,
    state: &mut ClientWriteTaskState,
) -> Result<(), ClientOutputError>
where
    W: AsyncWrite + Unpin,
    S: SurfaceFrameSource,
{
    loop {
        match output_rx.recv_write_event().await {
            ClientWriteEvent::Reliable(frame) => {
                async_write_default_frame(&mut writer, &frame.bytes).await?;
            }
            ClientWriteEvent::Surface(surfaces) => {
                write_surface_signals(&mut writer, surface_source, state, surfaces).await?;
            }
            ClientWriteEvent::Closed => return Ok(()),
        }
    }
}

async fn write_surface_signals<W, S>(
    writer: &mut W,
    surface_source: &mut S,
    state: &mut ClientWriteTaskState,
    surfaces: SurfaceSignalSet,
) -> Result<(), ClientOutputError>
where
    W: AsyncWrite + Unpin,
    S: SurfaceFrameSource,
{
    for signal in surfaces.panes.values() {
        if state.known_surface_versions.get(&signal.pane_id).copied() == Some(signal.version) {
            continue;
        }
        let Some(frame) = surface_frame_for_signal(surface_source, state, signal) else {
            continue;
        };
        async_write_default_frame(writer, &frame).await?;
        state.next_seq = state.next_seq.saturating_add(1);
        state
            .known_surface_versions
            .insert(signal.pane_id.clone(), signal.version);
    }
    Ok(())
}

fn surface_frame_for_signal<S>(
    surface_source: &mut S,
    state: &ClientWriteTaskState,
    signal: &PendingSurfaceSignal,
) -> Option<Vec<u8>>
where
    S: SurfaceFrameSource,
{
    let known = state.known_surface_versions.get(&signal.pane_id).copied();
    if !signal.snapshot_required
        && let Some(known_version) = known
        && known_version.checked_add(1) == Some(signal.version)
        && surface_source.surface_patch_kind(&signal.pane_id)
            != protocol::PatchKind::FullRefreshRequired
    {
        return surface_source.surface_patch_frame(&signal.pane_id, known_version, state.next_seq);
    }
    surface_source.surface_snapshot_frame(&signal.pane_id, state.next_seq)
}

async fn async_write_default_frame<W>(writer: &mut W, frame: &[u8]) -> Result<(), ClientOutputError>
where
    W: AsyncWrite + Unpin,
{
    let mut validated = Vec::with_capacity(frame.len());
    wire::write_default_frame(&mut validated, frame)
        .map_err(|err| ClientOutputError::Wire(err.to_string()))?;
    writer
        .write_all(&validated)
        .await
        .map_err(|err| ClientOutputError::Io(err.to_string()))
}

fn io_cursor(bytes: Vec<u8>) -> io::Cursor<Vec<u8>> {
    io::Cursor::new(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbuffers::FlatBufferBuilder;
    use nmux_proto::PROTOCOL_VERSION;

    #[derive(Default)]
    struct MockSurfaceFrameSource {
        patch_kind: protocol::PatchKind,
        snapshots: Vec<(String, u64)>,
        patches: Vec<(String, u64, u64)>,
    }

    impl SurfaceFrameSource for MockSurfaceFrameSource {
        fn surface_patch_kind(&self, _pane_id: &str) -> protocol::PatchKind {
            self.patch_kind
        }

        fn surface_snapshot_frame(&mut self, pane_id: &str, seq: u64) -> Option<Vec<u8>> {
            self.snapshots.push((pane_id.to_owned(), seq));
            Some(test_frame(seq))
        }

        fn surface_patch_frame(
            &mut self,
            pane_id: &str,
            base_version: u64,
            seq: u64,
        ) -> Option<Vec<u8>> {
            self.patches.push((pane_id.to_owned(), base_version, seq));
            Some(test_frame(seq))
        }
    }

    fn test_frame(seq: u64) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let session_id = builder.create_string("local");
        let connection_id = builder.create_string("test-client");
        let envelope = protocol::Envelope::create(
            &mut builder,
            &protocol::EnvelopeArgs {
                protocol_version: PROTOCOL_VERSION,
                session_id: Some(session_id),
                connection_id: Some(connection_id),
                seq,
                ack: 0,
                sent_at_mono_ms: 0,
                body_type: protocol::EnvelopeBody::NONE,
                body: None,
            },
        );
        protocol::finish_size_prefixed_envelope_buffer(&mut builder, envelope);
        builder.finished_data().to_vec()
    }

    fn decoded_frame_count(bytes: Vec<u8>) -> usize {
        let mut cursor = io_cursor(bytes);
        let mut count = 0;
        while (cursor.position() as usize) < cursor.get_ref().len() {
            wire::read_default_frame(&mut cursor).expect("valid test frame");
            count += 1;
        }
        count
    }

    #[tokio::test]
    async fn reliable_frames_are_bounded() {
        let (output, mut rx) = AsyncClientOutput::new(1, 1024);

        output
            .try_send_reliable(ReliableFrame::new(vec![1, 2, 3]))
            .expect("first reliable frame fits");
        assert_eq!(
            output.try_send_reliable(ReliableFrame::new(vec![4, 5])),
            Err(ClientOutputError::ReliableFrameQueueFull {
                cap: 1,
                frame_len: 2
            })
        );
        assert_eq!(
            rx.recv_reliable().await,
            Some(ReliableFrame::new(vec![1, 2, 3]))
        );
    }

    #[tokio::test]
    async fn reliable_frames_are_byte_bounded() {
        let (output, mut rx) = AsyncClientOutput::new(8, 3);

        output
            .try_send_reliable(ReliableFrame::new(vec![1, 2]))
            .expect("first reliable frame fits byte cap");
        assert_eq!(
            output.try_send_reliable(ReliableFrame::new(vec![3, 4])),
            Err(ClientOutputError::ReliableByteQueueFull {
                cap: 3,
                pending: 2,
                frame_len: 2
            })
        );
        assert_eq!(
            rx.recv_reliable().await,
            Some(ReliableFrame::new(vec![1, 2]))
        );
        output
            .try_send_reliable(ReliableFrame::new(vec![3, 4]))
            .expect("byte cap is released after receive");
    }

    #[tokio::test]
    async fn surface_signals_are_latest_wins() {
        let (mut output, mut rx) = AsyncClientOutput::new(8, 1024);

        output
            .signal_surface_changed("pane-1", 1)
            .expect("signal v1");
        output
            .signal_surface_changed("pane-1", 2)
            .expect("signal v2");
        output
            .signal_surface_changed("pane-1", 3)
            .expect("signal v3");

        let surfaces = rx.changed_surfaces().await.expect("surface signal");
        assert_eq!(surfaces.generation, 3);
        assert_eq!(
            surfaces.panes.get("pane-1"),
            Some(&PendingSurfaceSignal {
                pane_id: "pane-1".to_owned(),
                version: 3,
                snapshot_required: false,
            })
        );
    }

    #[tokio::test]
    async fn snapshot_requirement_survives_later_surface_versions() {
        let (mut output, mut rx) = AsyncClientOutput::new(8, 1024);

        output
            .require_surface_snapshot("pane-1", 4)
            .expect("snapshot required");
        output
            .signal_surface_changed("pane-1", 5)
            .expect("newer surface");

        let surfaces = rx.changed_surfaces().await.expect("surface signal");
        assert_eq!(
            surfaces.panes.get("pane-1"),
            Some(&PendingSurfaceSignal {
                pane_id: "pane-1".to_owned(),
                version: 5,
                snapshot_required: true,
            })
        );
    }

    #[tokio::test]
    async fn write_task_receive_prioritizes_reliable_frames() {
        let (mut output, mut rx) = AsyncClientOutput::new(8, 1024);

        output
            .signal_surface_changed("pane-1", 1)
            .expect("surface signal");
        output
            .try_send_reliable(ReliableFrame::new(vec![9]))
            .expect("reliable frame");

        assert_eq!(
            rx.recv_write_event().await,
            ClientWriteEvent::Reliable(ReliableFrame::new(vec![9]))
        );
        match rx.recv_write_event().await {
            ClientWriteEvent::Surface(surfaces) => {
                assert_eq!(surfaces.panes["pane-1"].version, 1);
            }
            other => panic!("expected surface signal after reliable frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_task_writes_reliable_frames_without_surface_state() {
        let (output, rx) = AsyncClientOutput::new(8, 1024);
        output
            .try_send_reliable(ReliableFrame::new(test_frame(7)))
            .expect("reliable frame");
        drop(output);

        let mut writer = Vec::new();
        let mut source = MockSurfaceFrameSource::default();
        let mut state = ClientWriteTaskState::new(8, BTreeMap::new());
        run_client_write_task(&mut writer, rx, &mut source, &mut state)
            .await
            .expect("write task");

        assert_eq!(decoded_frame_count(writer), 1);
        assert!(source.snapshots.is_empty());
        assert!(source.patches.is_empty());
        assert_eq!(state.next_seq(), 8);
    }

    #[tokio::test]
    async fn write_task_sends_patch_for_adjacent_surface_version() {
        let (mut output, rx) = AsyncClientOutput::new(8, 1024);
        output
            .signal_surface_changed("pane-1", 2)
            .expect("surface signal");
        drop(output);

        let mut writer = Vec::new();
        let mut source = MockSurfaceFrameSource {
            patch_kind: protocol::PatchKind::ReplaceRows,
            ..MockSurfaceFrameSource::default()
        };
        let mut state = ClientWriteTaskState::new(10, BTreeMap::from([("pane-1".to_owned(), 1)]));
        run_client_write_task(&mut writer, rx, &mut source, &mut state)
            .await
            .expect("write task");

        assert_eq!(source.patches, vec![("pane-1".to_owned(), 1, 10)]);
        assert!(source.snapshots.is_empty());
        assert_eq!(state.known_surface_version("pane-1"), Some(2));
        assert_eq!(state.next_seq(), 11);
        assert_eq!(decoded_frame_count(writer), 1);
    }

    #[tokio::test]
    async fn write_task_skips_stale_patches_and_catches_up_with_snapshot() {
        let (mut output, rx) = AsyncClientOutput::new(8, 1024);
        output
            .signal_surface_changed("pane-1", 2)
            .expect("surface v2");
        output
            .signal_surface_changed("pane-1", 4)
            .expect("surface v4");
        drop(output);

        let mut writer = Vec::new();
        let mut source = MockSurfaceFrameSource {
            patch_kind: protocol::PatchKind::ReplaceRows,
            ..MockSurfaceFrameSource::default()
        };
        let mut state = ClientWriteTaskState::new(10, BTreeMap::from([("pane-1".to_owned(), 1)]));
        run_client_write_task(&mut writer, rx, &mut source, &mut state)
            .await
            .expect("write task");

        assert!(source.patches.is_empty());
        assert_eq!(source.snapshots, vec![("pane-1".to_owned(), 10)]);
        assert_eq!(state.known_surface_version("pane-1"), Some(4));
        assert_eq!(decoded_frame_count(writer), 1);
    }

    #[tokio::test]
    async fn write_task_honors_snapshot_required_even_for_adjacent_version() {
        let (mut output, rx) = AsyncClientOutput::new(8, 1024);
        output
            .require_surface_snapshot("pane-1", 2)
            .expect("snapshot required");
        drop(output);

        let mut writer = Vec::new();
        let mut source = MockSurfaceFrameSource {
            patch_kind: protocol::PatchKind::ReplaceRows,
            ..MockSurfaceFrameSource::default()
        };
        let mut state = ClientWriteTaskState::new(10, BTreeMap::from([("pane-1".to_owned(), 1)]));
        run_client_write_task(&mut writer, rx, &mut source, &mut state)
            .await
            .expect("write task");

        assert!(source.patches.is_empty());
        assert_eq!(source.snapshots, vec![("pane-1".to_owned(), 10)]);
        assert_eq!(state.known_surface_version("pane-1"), Some(2));
    }
}
