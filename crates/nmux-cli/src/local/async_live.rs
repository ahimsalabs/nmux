use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
        tokio::select! {
            biased;
            reliable = self.reliable_rx.recv() => {
                if let Some(frame) = reliable {
                    self.release_reliable_frame(&frame);
                    ClientWriteEvent::Reliable(frame)
                } else {
                    ClientWriteEvent::Closed
                }
            }
            changed = self.surface_rx.changed() => {
                match changed {
                    Ok(()) => ClientWriteEvent::Surface(self.surface_rx.borrow_and_update().clone()),
                    Err(_) => ClientWriteEvent::Closed,
                }
            }
        }
    }

    fn release_reliable_frame(&self, frame: &ReliableFrame) {
        self.pending_reliable_bytes
            .fetch_sub(frame.bytes.len(), Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(rx.recv_reliable().await, Some(ReliableFrame::new(vec![1, 2])));
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
}
