use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use nmux_proto::protocol;

use crate::session::{
    AcceptedSessionEvent, SessionEffect, SessionEvent, SessionEventLane, SessionTraceRecord,
};

const CONTAINER_MAGIC: &[u8; 10] = b"NMUXTRACE\0";
const SEGMENT_MAGIC: &[u8; 4] = b"NMS1";
const CONTAINER_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSegmentKind {
    AcceptedEvent = 1,
    OutputTrace = 2,
    Snapshot = 3,
    Metadata = 4,
}

pub struct TraceContainerWriter<W> {
    writer: W,
}

pub struct BackgroundTraceWriter {
    tx: SyncSender<BackgroundTraceCommand>,
    join: Option<JoinHandle<io::Result<()>>>,
}

enum BackgroundTraceCommand {
    Record(SessionTraceRecord),
    Finish,
}

impl<W: Write> TraceContainerWriter<W> {
    pub fn new(mut writer: W) -> io::Result<Self> {
        writer.write_all(CONTAINER_MAGIC)?;
        writer.write_all(&CONTAINER_VERSION.to_le_bytes())?;
        Ok(Self { writer })
    }

    pub fn write_segment(&mut self, kind: TraceSegmentKind, payload: &[u8]) -> io::Result<()> {
        self.writer.write_all(SEGMENT_MAGIC)?;
        self.writer.write_all(&[kind as u8])?;
        self.writer
            .write_all(&(payload.len() as u64).to_le_bytes())?;
        self.writer.write_all(payload)
    }

    pub fn write_record(&mut self, record: &SessionTraceRecord) -> io::Result<()> {
        let event = encode_accepted_event(&record.accepted);
        self.write_segment(TraceSegmentKind::AcceptedEvent, event.as_bytes())?;
        let effects = encode_effects(&record.effects);
        self.write_segment(TraceSegmentKind::OutputTrace, effects.as_bytes())
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl BackgroundTraceWriter {
    pub fn start(path: impl AsRef<Path>, queue_cap: usize) -> io::Result<Self> {
        let file = File::create(path)?;
        let (tx, rx) = mpsc::sync_channel(queue_cap);
        let join = thread::spawn(move || {
            let mut writer = TraceContainerWriter::new(file)?;
            while let Ok(command) = rx.recv() {
                match command {
                    BackgroundTraceCommand::Record(record) => writer.write_record(&record)?,
                    BackgroundTraceCommand::Finish => break,
                }
            }
            Ok(())
        });
        Ok(Self {
            tx,
            join: Some(join),
        })
    }

    pub fn try_record(&self, record: SessionTraceRecord) -> io::Result<()> {
        self.tx
            .try_send(BackgroundTraceCommand::Record(record))
            .map_err(|err| match err {
                TrySendError::Full(_) => {
                    io::Error::new(io::ErrorKind::WouldBlock, "trace writer queue is full")
                }
                TrySendError::Disconnected(_) => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "trace writer stopped")
                }
            })
    }

    pub fn finish(mut self) -> io::Result<()> {
        let _ = self.tx.send(BackgroundTraceCommand::Finish);
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        join.join()
            .map_err(|_| io::Error::other("trace writer panicked"))?
    }
}

fn encode_accepted_event(accepted: &AcceptedSessionEvent) -> String {
    format!(
        "event_index={}\nsession_mono_ms={}\nsource_id={}\nlane={}\nevent={}\n",
        accepted.metadata.event_index,
        accepted.metadata.session_mono_ms,
        escape_field(&accepted.metadata.source_id),
        lane_name(accepted.metadata.lane),
        event_name(&accepted.event),
    )
}

fn encode_effects(effects: &[SessionEffect]) -> String {
    let mut encoded = String::new();
    for effect in effects {
        match effect {
            SessionEffect::WorkspaceChanged {
                event_index,
                version,
            } => {
                encoded.push_str(&format!(
                    "effect=workspace_changed event_index={event_index} version={version}\n"
                ));
            }
            SessionEffect::PaneSurfaceChanged {
                event_index,
                pane_id,
                surface_version,
                scrollback_version,
                patch_kind,
            } => {
                encoded.push_str(&format!(
                    "effect=pane_surface_changed event_index={event_index} pane_id={} surface_version={surface_version} scrollback_version={scrollback_version} patch_kind={}\n",
                    escape_field(pane_id),
                    patch_kind_name(*patch_kind),
                ));
            }
            SessionEffect::PaneResized {
                event_index,
                pane_id,
                cols,
                rows,
            } => {
                encoded.push_str(&format!(
                    "effect=pane_resized event_index={event_index} pane_id={} cols={cols} rows={rows}\n",
                    escape_field(pane_id),
                ));
            }
        }
    }
    encoded
}

fn lane_name(lane: SessionEventLane) -> &'static str {
    match lane {
        SessionEventLane::Control => "control",
        SessionEventLane::Client => "client",
        SessionEventLane::Pane => "pane",
        SessionEventLane::Timer => "timer",
        SessionEventLane::Lifecycle => "lifecycle",
    }
}

fn event_name(event: &SessionEvent) -> &'static str {
    match event {
        SessionEvent::AddTab { .. } => "add_tab",
        SessionEvent::SwitchTab { .. } => "switch_tab",
        SessionEvent::CloseTab { .. } => "close_tab",
        SessionEvent::FocusPane { .. } => "focus_pane",
        SessionEvent::SplitPane { .. } => "split_pane",
        SessionEvent::SetPaneResizePolicy { .. } => "set_pane_resize_policy",
        SessionEvent::PaneOutput { .. } => "pane_output",
        SessionEvent::CommitPaneResize { .. } => "commit_pane_resize",
    }
}

fn patch_kind_name(kind: protocol::PatchKind) -> &'static str {
    match kind {
        protocol::PatchKind::ReplaceRows => "replace_rows",
        protocol::PatchKind::FullRefreshRequired => "full_refresh_required",
        protocol::PatchKind::ColorOnly => "color_only",
        protocol::PatchKind::ModeOnly => "mode_only",
        protocol::PatchKind::CursorOnly => "cursor_only",
        _ => "unknown",
    }
}

fn escape_field(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            ' ' => "\\s".chars().collect(),
            _ => vec![ch],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use nmux_proto::protocol;

    use crate::replay::{BackgroundTraceWriter, TraceContainerWriter};
    use crate::session::{
        Session, SessionCore, SessionEvent, SessionEventLane, SessionTraceRecord,
    };

    static NEXT_TRACE_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn trace_container_writes_segmented_records() {
        let mut core = SessionCore::initial();
        let transition = core.accept(
            "client-1",
            SessionEventLane::Client,
            7,
            SessionEvent::CommitPaneResize {
                pane_id: "pane-1".to_owned(),
                cols: 100,
                rows: 30,
            },
        );
        let mut writer = TraceContainerWriter::new(Vec::new()).expect("writer");

        writer
            .write_record(&SessionTraceRecord::from_transition(transition))
            .expect("write record");
        let bytes = writer.into_inner();

        assert!(bytes.starts_with(b"NMUXTRACE\0\x01\0\0\0"));
        assert!(String::from_utf8_lossy(&bytes).contains("event=commit_pane_resize"));
        assert!(String::from_utf8_lossy(&bytes).contains("effect=pane_resized"));
    }

    #[test]
    fn background_trace_writer_flushes_without_timing_dependencies() {
        let mut core = SessionCore::initial();
        let record = SessionTraceRecord::from_transition(core.accept(
            "control",
            SessionEventLane::Control,
            3,
            SessionEvent::SetPaneResizePolicy {
                pane_id: "pane-1".to_owned(),
                policy: protocol::ResizePolicy::Manual,
            },
        ));
        let path = std::env::temp_dir().join(format!(
            "nmux-trace-{}-{}.nmt",
            std::process::id(),
            NEXT_TRACE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let writer = BackgroundTraceWriter::start(&path, 4).expect("start writer");

        writer.try_record(record).expect("queue record");
        writer.finish().expect("finish writer");

        let bytes = fs::read(&path).expect("read trace");
        let _ = fs::remove_file(&path);
        assert!(bytes.starts_with(b"NMUXTRACE\0\x01\0\0\0"));
        assert!(String::from_utf8_lossy(&bytes).contains("event=set_pane_resize_policy"));
        assert!(String::from_utf8_lossy(&bytes).contains("effect=workspace_changed"));
    }

    #[test]
    fn replay_trace_records_remain_independent_of_live_handles() {
        let mut core = SessionCore::initial();
        let record = SessionTraceRecord::from_transition(core.accept(
            "pane-1",
            SessionEventLane::Pane,
            5,
            SessionEvent::PaneOutput {
                pane_id: "pane-1".to_owned(),
                bytes: b"durable replay\n".to_vec(),
            },
        ));

        let (replayed, records) = SessionCore::replay_accepted_events(
            Session::initial(),
            crate::terminal::TerminalEngineKind::InterimText,
            [record.accepted],
        );

        assert_eq!(records.len(), 1);
        assert_eq!(
            replayed
                .session()
                .pane_surface("pane-1")
                .expect("pane")
                .lines
                .last()
                .map(String::as_str),
            Some("durable replay")
        );
    }
}
