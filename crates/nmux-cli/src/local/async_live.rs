use std::collections::BTreeMap;
use std::io;
use std::io::Write as _;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{
    InputSummary, PaneViewportIntentSummary, PingSummary, ResizeIntentSummary,
    ScrollbackFetchSummary, input_summary_from_frame, pane_viewport_intent_from_frame,
    ping_from_frame, resize_intent_from_frame,
};
use nmux_proto::{protocol, wire};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
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
    pub snapshot_frame: Option<Vec<u8>>,
    pub patch_frame: Option<SurfacePatchFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SurfacePatchFrame {
    pub base_version: u64,
    pub bytes: Vec<u8>,
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
    SurfaceByteQueueFull {
        cap: usize,
        pending: usize,
    },
    ReliableQueueClosed,
    SurfaceQueueClosed,
    Wire(String),
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClientInputError {
    EventQueueClosed,
    Io(String),
    Wire(String),
    UnexpectedFrame(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClientInputEvent {
    Resize(ResizeIntentSummary),
    Scrollback(ScrollbackFetchSummary),
    Viewport(PaneViewportIntentSummary),
    Input(InputSummary),
    Ping(PingSummary),
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClientWriteEvent {
    Reliable(ReliableFrame),
    Surface(SurfaceSignalSet),
    Closed,
}

#[derive(Clone)]
pub(crate) struct AsyncClientInputTx {
    tx: mpsc::Sender<ClientInputEvent>,
    wake: Option<AsyncInputWake>,
}

pub(crate) struct AsyncClientInputRx {
    rx: mpsc::Receiver<ClientInputEvent>,
}

#[derive(Clone)]
pub(crate) struct AsyncInputWake {
    writer: Arc<Mutex<StdUnixStream>>,
}

pub(crate) struct AsyncClientOutput {
    reliable_tx: mpsc::Sender<ReliableFrame>,
    surface_tx: watch::Sender<SurfaceSignalSet>,
    latest_surfaces: SurfaceSignalSet,
    reliable_frame_cap: usize,
    reliable_byte_cap: usize,
    surface_byte_cap: usize,
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

pub(crate) fn async_client_input_channel(cap: usize) -> (AsyncClientInputTx, AsyncClientInputRx) {
    let (tx, rx) = mpsc::channel(cap);
    (
        AsyncClientInputTx { tx, wake: None },
        AsyncClientInputRx { rx },
    )
}

pub(crate) fn async_client_input_channel_with_wake(
    cap: usize,
    wake_writer: StdUnixStream,
) -> (AsyncClientInputTx, AsyncClientInputRx) {
    let (tx, rx) = mpsc::channel(cap);
    (
        AsyncClientInputTx {
            tx,
            wake: Some(AsyncInputWake {
                writer: Arc::new(Mutex::new(wake_writer)),
            }),
        },
        AsyncClientInputRx { rx },
    )
}

impl AsyncClientInputTx {
    async fn send(&self, event: ClientInputEvent) -> Result<(), ClientInputError> {
        self.tx
            .send(event)
            .await
            .map_err(|_| ClientInputError::EventQueueClosed)?;
        if let Some(wake) = self.wake.as_ref() {
            wake.notify();
        }
        Ok(())
    }
}

impl AsyncInputWake {
    fn notify(&self) {
        let Ok(mut writer) = self.writer.lock() else {
            return;
        };
        match writer.write(&[1]) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => {}
        }
    }
}

impl AsyncClientInputRx {
    pub(crate) async fn recv(&mut self) -> Option<ClientInputEvent> {
        self.rx.recv().await
    }

    pub(crate) fn try_recv(&mut self) -> Result<ClientInputEvent, mpsc::error::TryRecvError> {
        self.rx.try_recv()
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
                surface_byte_cap: reliable_byte_cap,
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
        self.update_surface_signal(pane_id.into(), version, false, None, None)
    }

    pub(crate) fn signal_surface_frame_bundle(
        &mut self,
        pane_id: impl Into<String>,
        version: u64,
        snapshot_frame: Vec<u8>,
        patch_frame: Option<SurfacePatchFrame>,
    ) -> Result<(), ClientOutputError> {
        self.update_surface_signal(
            pane_id.into(),
            version,
            false,
            Some(snapshot_frame),
            patch_frame,
        )
    }

    pub(crate) fn require_surface_snapshot(
        &mut self,
        pane_id: impl Into<String>,
        version: u64,
    ) -> Result<(), ClientOutputError> {
        self.update_surface_signal(pane_id.into(), version, true, None, None)
    }

    pub(crate) fn require_surface_snapshot_frame(
        &mut self,
        pane_id: impl Into<String>,
        version: u64,
        snapshot_frame: Vec<u8>,
    ) -> Result<(), ClientOutputError> {
        self.update_surface_signal(pane_id.into(), version, true, Some(snapshot_frame), None)
    }

    fn update_surface_signal(
        &mut self,
        pane_id: String,
        version: u64,
        snapshot_required: bool,
        snapshot_frame: Option<Vec<u8>>,
        patch_frame: Option<SurfacePatchFrame>,
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
                snapshot_frame,
                patch_frame,
            },
        );
        self.enforce_surface_budget()?;
        self.surface_tx
            .send(self.latest_surfaces.clone())
            .map_err(|_| ClientOutputError::SurfaceQueueClosed)
    }

    fn enforce_surface_budget(&mut self) -> Result<(), ClientOutputError> {
        let pending = surface_signal_set_bytes(&self.latest_surfaces);
        if pending <= self.surface_byte_cap {
            return Ok(());
        }

        for signal in self.latest_surfaces.panes.values_mut() {
            signal.snapshot_required = true;
            signal.patch_frame = None;
        }
        let pending = surface_signal_set_bytes(&self.latest_surfaces);
        if pending <= self.surface_byte_cap {
            return Ok(());
        }
        Err(ClientOutputError::SurfaceByteQueueFull {
            cap: self.surface_byte_cap,
            pending,
        })
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

fn surface_signal_set_bytes(surfaces: &SurfaceSignalSet) -> usize {
    surfaces
        .panes
        .values()
        .map(|signal| {
            signal.snapshot_frame.as_ref().map_or(0, Vec::len)
                + signal
                    .patch_frame
                    .as_ref()
                    .map_or(0, |patch| patch.bytes.len())
        })
        .sum()
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

pub(crate) async fn run_client_read_task<R>(
    mut reader: R,
    events: AsyncClientInputTx,
) -> Result<(), ClientInputError>
where
    R: AsyncRead + Unpin,
{
    loop {
        match async_read_default_frame(&mut reader).await {
            Ok(frame) => events.send(client_input_event_from_frame(&frame)?).await?,
            Err(ClientInputError::Io(err)) if async_read_closed_error(&err) => {
                let _ = events.send(ClientInputEvent::Closed).await;
                return Ok(());
            }
            Err(err) => return Err(err),
        }
    }
}

fn client_input_event_from_frame(frame: &[u8]) -> Result<ClientInputEvent, ClientInputError> {
    let envelope = protocol::size_prefixed_root_as_envelope(frame)
        .map_err(|err| ClientInputError::Wire(err.to_string()))?;
    match envelope.body_type() {
        protocol::EnvelopeBody::ResizeIntent => resize_intent_from_frame(frame)
            .map(ClientInputEvent::Resize)
            .map_err(|err| ClientInputError::Wire(err.to_string())),
        protocol::EnvelopeBody::PaneViewportIntent => pane_viewport_intent_from_frame(frame)
            .map(ClientInputEvent::Viewport)
            .map_err(|err| ClientInputError::Wire(err.to_string())),
        protocol::EnvelopeBody::InputEvent => input_summary_from_frame(frame)
            .map(ClientInputEvent::Input)
            .map_err(|err| ClientInputError::Wire(err.to_string())),
        protocol::EnvelopeBody::Ping => ping_from_frame(frame)
            .map(ClientInputEvent::Ping)
            .map_err(|err| ClientInputError::Wire(err.to_string())),
        other => Err(ClientInputError::UnexpectedFrame(format!("{other:?}"))),
    }
}

async fn async_read_default_frame<R>(reader: &mut R) -> Result<Vec<u8>, ClientInputError>
where
    R: AsyncRead + Unpin,
{
    let mut prefix = [0_u8; 4];
    reader
        .read_exact(&mut prefix)
        .await
        .map_err(|err| ClientInputError::Io(format!("{:?}: {err}", err.kind())))?;
    let payload_len = u32::from_le_bytes(prefix) as usize;
    if payload_len > wire::DEFAULT_MAX_FRAME_LEN {
        return Err(ClientInputError::Wire(format!(
            "wire frame too large: {} bytes exceeds {} byte limit",
            payload_len,
            wire::DEFAULT_MAX_FRAME_LEN
        )));
    }

    let mut frame = Vec::with_capacity(4 + payload_len);
    frame.extend_from_slice(&prefix);
    frame.resize(4 + payload_len, 0);
    reader
        .read_exact(&mut frame[4..])
        .await
        .map_err(|err| ClientInputError::Io(format!("{:?}: {err}", err.kind())))?;

    protocol::size_prefixed_root_as_envelope(&frame)
        .map_err(|err| ClientInputError::Wire(err.to_string()))?;
    Ok(frame)
}

fn async_read_closed_error(error: &str) -> bool {
    error.starts_with("UnexpectedEof")
        || error.starts_with("ConnectionReset")
        || error.starts_with("BrokenPipe")
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
    if let Some(frame) = bundled_surface_frame_for_signal(state, signal) {
        return Some(frame);
    }
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

fn bundled_surface_frame_for_signal(
    state: &ClientWriteTaskState,
    signal: &PendingSurfaceSignal,
) -> Option<Vec<u8>> {
    if !signal.snapshot_required
        && let Some(known_version) = state.known_surface_versions.get(&signal.pane_id).copied()
        && let Some(patch) = signal.patch_frame.as_ref()
        && patch.base_version == known_version
        && known_version.checked_add(1) == Some(signal.version)
    {
        return Some(patch.bytes.clone());
    }
    signal.snapshot_frame.clone()
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
    use nmux_core::session::{InputFrameContext, ScrollbackFetchSpec, ScrollbackRange, Session};
    use nmux_proto::PROTOCOL_VERSION;
    use std::io::Read as _;

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

    fn joined_frames(frames: &[Vec<u8>]) -> Vec<u8> {
        let mut joined = Vec::new();
        for frame in frames {
            joined.extend_from_slice(frame);
        }
        joined
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
                snapshot_frame: None,
                patch_frame: None,
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
                snapshot_frame: None,
                patch_frame: None,
            })
        );
    }

    #[tokio::test]
    async fn surface_budget_drops_patch_and_marks_snapshot_required() {
        let (mut output, mut rx) = AsyncClientOutput::new(8, 10);

        output
            .signal_surface_frame_bundle(
                "pane-1",
                2,
                vec![1, 2, 3, 4, 5, 6],
                Some(SurfacePatchFrame {
                    base_version: 1,
                    bytes: vec![7, 8, 9, 10, 11, 12],
                }),
            )
            .expect("surface signal fits after dropping patch");

        match rx.recv_write_event().await {
            ClientWriteEvent::Surface(surfaces) => {
                let signal = surfaces.panes.get("pane-1").expect("surface signal");
                assert!(signal.snapshot_required);
                assert_eq!(
                    signal.snapshot_frame.as_deref(),
                    Some(&[1, 2, 3, 4, 5, 6][..])
                );
                assert_eq!(signal.patch_frame, None);
            }
            other => panic!("expected surface signal, got {other:?}"),
        }
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

    #[tokio::test]
    async fn write_task_can_use_bundled_surface_patch_without_source_access() {
        let (mut output, rx) = AsyncClientOutput::new(8, 1024);
        output
            .signal_surface_frame_bundle(
                "pane-1",
                2,
                test_frame(50),
                Some(SurfacePatchFrame {
                    base_version: 1,
                    bytes: test_frame(51),
                }),
            )
            .expect("surface bundle");
        drop(output);

        let mut writer = Vec::new();
        let mut source = MockSurfaceFrameSource::default();
        let mut state = ClientWriteTaskState::new(10, BTreeMap::from([("pane-1".to_owned(), 1)]));
        run_client_write_task(&mut writer, rx, &mut source, &mut state)
            .await
            .expect("write task");

        assert!(source.patches.is_empty());
        assert!(source.snapshots.is_empty());
        assert_eq!(state.known_surface_version("pane-1"), Some(2));
        assert_eq!(decoded_frame_count(writer), 1);
    }

    #[tokio::test]
    async fn write_task_uses_bundled_snapshot_when_patch_base_is_stale() {
        let (mut output, rx) = AsyncClientOutput::new(8, 1024);
        output
            .signal_surface_frame_bundle(
                "pane-1",
                4,
                test_frame(50),
                Some(SurfacePatchFrame {
                    base_version: 3,
                    bytes: test_frame(51),
                }),
            )
            .expect("surface bundle");
        drop(output);

        let mut writer = Vec::new();
        let mut source = MockSurfaceFrameSource::default();
        let mut state = ClientWriteTaskState::new(10, BTreeMap::from([("pane-1".to_owned(), 1)]));
        run_client_write_task(&mut writer, rx, &mut source, &mut state)
            .await
            .expect("write task");

        assert!(source.patches.is_empty());
        assert!(source.snapshots.is_empty());
        assert_eq!(state.known_surface_version("pane-1"), Some(4));
        assert_eq!(decoded_frame_count(writer), 1);
    }

    #[tokio::test]
    async fn client_read_task_forwards_input_events_through_bounded_queue() {
        let frame =
            Session::initial().key_input_frame("local-client", 3, "actor-1", "pane-1", 2, "x");
        let (tx, mut rx) = async_client_input_channel(2);

        run_client_read_task(frame.as_slice(), tx)
            .await
            .expect("read task");

        match rx.recv().await {
            Some(ClientInputEvent::Input(input)) => {
                assert_eq!(input.pane_id, "pane-1");
                assert_eq!(input.actor_id, "actor-1");
                assert_eq!(input.input_seq, 2);
                assert_eq!(input.bytes, b"x");
            }
            other => panic!("expected input event, got {other:?}"),
        }
        assert_eq!(rx.recv().await, Some(ClientInputEvent::Closed));
    }

    #[tokio::test]
    async fn client_read_task_wakes_session_loop_after_enqueue() {
        let frame =
            Session::initial().key_input_frame("local-client", 3, "actor-1", "pane-1", 2, "x");
        let (mut wake_reader, wake_writer) = StdUnixStream::pair().expect("wake socket pair");
        wake_reader
            .set_nonblocking(true)
            .expect("nonblocking wake reader");
        wake_writer
            .set_nonblocking(true)
            .expect("nonblocking wake writer");
        let (tx, mut rx) = async_client_input_channel_with_wake(2, wake_writer);

        run_client_read_task(frame.as_slice(), tx)
            .await
            .expect("read task");

        assert!(matches!(rx.recv().await, Some(ClientInputEvent::Input(_))));
        let mut buf = [0_u8; 8];
        let wake_count = wake_reader.read(&mut buf).expect("wake byte");
        assert!(wake_count > 0);
    }

    #[tokio::test]
    async fn client_read_task_forwards_control_plane_live_events_in_order() {
        let session = Session::initial();
        let resize = session.resize_intent_frame(
            "local-client",
            1,
            "actor-1",
            "pane-1",
            100,
            40,
            protocol::ResizeReason::FrontendViewport,
        );
        let fetch = session.scrollback_fetch_frame(
            InputFrameContext {
                connection_id: "local-client",
                seq: 2,
                actor_id: "actor-1",
                pane_id: "pane-1",
                input_seq: 0,
            },
            ScrollbackFetchSpec {
                range: ScrollbackRange {
                    start_line: 1,
                    line_count: 10,
                },
                known_scrollback_version: 3,
            },
        );
        let ping = PingSummary {
            actor_id: "actor-1".to_owned(),
            ping_seq: 9,
        }
        .frame("local", "local-client", 3, protocol::EnvelopeBody::Ping);
        let frames = joined_frames(&[resize, fetch, ping]);
        let (tx, mut rx) = async_client_input_channel(4);

        run_client_read_task(frames.as_slice(), tx)
            .await
            .expect("read task");

        match rx.recv().await {
            Some(ClientInputEvent::Resize(resize)) => {
                assert_eq!(resize.pane_id, "pane-1");
                assert_eq!(resize.cols, 100);
                assert_eq!(resize.rows, 40);
            }
            other => panic!("expected resize event, got {other:?}"),
        }
        match rx.recv().await {
            Some(ClientInputEvent::Viewport(intent)) => {
                assert_eq!(intent.pane_id, "pane-1");
                assert_eq!(intent.viewport, protocol::PaneViewportKind::Pinned);
                assert_eq!(intent.top_line, 1);
                assert_eq!(intent.visible_rows, 10);
                assert_eq!(intent.known_viewport_version, 3);
            }
            other => panic!("expected viewport event, got {other:?}"),
        }
        assert_eq!(
            rx.recv().await,
            Some(ClientInputEvent::Ping(PingSummary {
                actor_id: "actor-1".to_owned(),
                ping_seq: 9,
            }))
        );
        assert_eq!(rx.recv().await, Some(ClientInputEvent::Closed));
    }

    #[tokio::test]
    async fn client_read_task_reports_unexpected_server_surface_frames() {
        let frame = Session::initial().pane_surface_frame("local-client", 1);
        let (tx, _rx) = async_client_input_channel(1);

        let err = run_client_read_task(frame.as_slice(), tx)
            .await
            .expect_err("surface frames are not client input events");

        assert!(matches!(err, ClientInputError::UnexpectedFrame(_)));
    }
}
