use nmux_proto::protocol;

#[derive(Debug, Clone, Copy)]
pub struct TerminalInput<'a> {
    pub pane_id: &'a str,
    pub cols: u32,
    pub rows: u32,
    pub surface: protocol::SurfaceKind,
    pub cursor: TerminalCursor,
    pub surface_lines: &'a [String],
    pub scrollback_lines: &'a [String],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCursor {
    pub row: u32,
    pub col: u32,
    pub visible: bool,
    pub shape: protocol::CursorShape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalUpdate {
    pub patch_kind: protocol::PatchKind,
    pub surface: protocol::SurfaceKind,
    pub cursor: TerminalCursor,
    pub surface_lines: Vec<String>,
    pub scrollback_lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalEngineKind {
    InterimText,
    #[cfg(feature = "libghostty-vt")]
    LibghosttyVt,
}

pub trait TerminalEngine {
    fn apply_output(&mut self, input: TerminalInput<'_>, output: &[u8]) -> Option<TerminalUpdate>;
    fn resize(&mut self, input: TerminalInput<'_>, cols: u32, rows: u32) -> Option<TerminalUpdate>;
}

#[derive(Default)]
pub struct PaneTerminalEngines {
    kind: TerminalEngineKind,
    engines: HashMap<String, Box<dyn TerminalEngine>>,
}

impl PaneTerminalEngines {
    pub fn interim() -> Self {
        Self::new(TerminalEngineKind::InterimText)
    }

    pub fn new(kind: TerminalEngineKind) -> Self {
        Self {
            kind,
            engines: HashMap::new(),
        }
    }

    pub fn engine_mut(&mut self, pane_id: &str) -> &mut dyn TerminalEngine {
        let kind = self.kind;
        self.engines
            .entry(pane_id.to_owned())
            .or_insert_with(|| terminal_engine_for_kind(kind))
            .as_mut()
    }

    pub fn pane_count(&self) -> usize {
        self.engines.len()
    }

    pub fn kind(&self) -> TerminalEngineKind {
        self.kind
    }
}

impl Default for TerminalEngineKind {
    fn default() -> Self {
        Self::InterimText
    }
}

fn terminal_engine_for_kind(kind: TerminalEngineKind) -> Box<dyn TerminalEngine> {
    match kind {
        TerminalEngineKind::InterimText => Box::new(InterimTextTerminalEngine),
        #[cfg(feature = "libghostty-vt")]
        TerminalEngineKind::LibghosttyVt => Box::new(ghostty_vt::LibghosttyVtTerminalEngine::new()),
    }
}

#[derive(Debug, Default)]
pub struct InterimTextTerminalEngine;

impl TerminalEngine for InterimTextTerminalEngine {
    fn apply_output(&mut self, input: TerminalInput<'_>, output: &[u8]) -> Option<TerminalUpdate> {
        let mut scrollback_lines = input.scrollback_lines.to_vec();
        scrollback_lines.extend(text_lines_from_pty_output(output));

        Some(interim_text_update(
            input.surface,
            input.cursor,
            input.rows,
            scrollback_lines,
        ))
    }

    fn resize(
        &mut self,
        input: TerminalInput<'_>,
        _cols: u32,
        rows: u32,
    ) -> Option<TerminalUpdate> {
        Some(interim_text_update(
            input.surface,
            input.cursor,
            rows,
            input.scrollback_lines.to_vec(),
        ))
    }
}

fn interim_text_update(
    surface: protocol::SurfaceKind,
    previous_cursor: TerminalCursor,
    rows: u32,
    scrollback_lines: Vec<String>,
) -> TerminalUpdate {
    let visible_start = scrollback_lines.len().saturating_sub(rows as usize);
    let surface_lines = scrollback_lines[visible_start..].to_vec();
    let cursor = TerminalCursor {
        row: surface_lines.len().saturating_sub(1) as u32,
        col: 0,
        visible: previous_cursor.visible,
        shape: previous_cursor.shape,
    };

    TerminalUpdate {
        patch_kind: protocol::PatchKind::ReplaceRows,
        surface,
        cursor,
        surface_lines,
        scrollback_lines,
    }
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

#[cfg(feature = "libghostty-vt")]
mod ghostty_vt {
    use libghostty_vt::{
        RenderState, Terminal, TerminalOptions,
        render::{CellIterator, CursorVisualStyle, RowIterator, Snapshot as RenderSnapshot},
        screen::CellWide,
        terminal::{Mode, ScrollViewport},
    };
    use nmux_proto::protocol;

    use super::{TerminalCursor, TerminalEngine, TerminalInput, TerminalUpdate};

    pub struct LibghosttyVtTerminalEngine {
        state: Option<GhosttyVtState>,
    }

    struct GhosttyVtState {
        terminal: Terminal<'static, 'static>,
        render_state: RenderState<'static>,
        row_iterator: RowIterator<'static>,
        cell_iterator: CellIterator<'static>,
    }

    impl LibghosttyVtTerminalEngine {
        pub fn new() -> Self {
            Self { state: None }
        }

        fn state_mut(&mut self, input: TerminalInput<'_>) -> Option<&mut GhosttyVtState> {
            if self.state.is_none() {
                self.state = Some(GhosttyVtState::new(input.cols, input.rows)?);
            }
            self.state.as_mut()
        }
    }

    impl TerminalEngine for LibghosttyVtTerminalEngine {
        fn apply_output(
            &mut self,
            input: TerminalInput<'_>,
            output: &[u8],
        ) -> Option<TerminalUpdate> {
            let state = self.state_mut(input)?;
            state.terminal.vt_write(output);
            state.extract_update(input, false)
        }

        fn resize(
            &mut self,
            input: TerminalInput<'_>,
            cols: u32,
            rows: u32,
        ) -> Option<TerminalUpdate> {
            let state = self.state_mut(input)?;
            let cols = u16::try_from(cols).ok()?;
            let rows = u16::try_from(rows).ok()?;
            state.terminal.resize(cols, rows, 8, 16).ok()?;
            state.extract_update(input, true)
        }
    }

    impl GhosttyVtState {
        fn new(cols: u32, rows: u32) -> Option<Self> {
            Some(Self {
                terminal: Terminal::new(TerminalOptions {
                    cols: u16::try_from(cols).ok()?,
                    rows: u16::try_from(rows).ok()?,
                    max_scrollback: 10000,
                })
                .ok()?,
                render_state: RenderState::new().ok()?,
                row_iterator: RowIterator::new().ok()?,
                cell_iterator: CellIterator::new().ok()?,
            })
        }

        fn extract_update(
            &mut self,
            input: TerminalInput<'_>,
            force_rows: bool,
        ) -> Option<TerminalUpdate> {
            let surface = surface_kind(&self.terminal)?;
            let scrollback_lines = if surface == protocol::SurfaceKind::Main {
                self.scrollback_lines()?
            } else {
                input.scrollback_lines.to_vec()
            };
            self.terminal.scroll_viewport(ScrollViewport::Bottom);
            let snapshot = self.render_state.update(&self.terminal).ok()?;
            let surface_lines =
                surface_lines(&snapshot, &mut self.row_iterator, &mut self.cell_iterator)?;
            let cursor = cursor(&snapshot, input.cursor)?;
            let patch_kind = if !force_rows
                && surface == input.surface
                && surface_lines == input.surface_lines
                && cursor != input.cursor
            {
                protocol::PatchKind::CursorOnly
            } else {
                protocol::PatchKind::ReplaceRows
            };

            Some(TerminalUpdate {
                patch_kind,
                surface,
                cursor,
                scrollback_lines,
                surface_lines,
            })
        }

        fn scrollback_lines(&mut self) -> Option<Vec<String>> {
            let total_rows = self.terminal.total_rows().ok()?;
            if total_rows == 0 {
                return Some(Vec::new());
            }

            self.terminal.scroll_viewport(ScrollViewport::Top);
            let snapshot = self.render_state.update(&self.terminal).ok()?;
            let mut lines =
                surface_lines(&snapshot, &mut self.row_iterator, &mut self.cell_iterator)?;
            lines.truncate(total_rows);

            while lines.len() < total_rows {
                self.terminal.scroll_viewport(ScrollViewport::Delta(1));
                let snapshot = self.render_state.update(&self.terminal).ok()?;
                let viewport_lines =
                    surface_lines(&snapshot, &mut self.row_iterator, &mut self.cell_iterator)?;
                let Some(next_line) = viewport_lines.last() else {
                    break;
                };
                lines.push(next_line.clone());
            }

            Some(lines)
        }
    }

    fn surface_lines<'alloc>(
        snapshot: &RenderSnapshot<'alloc, '_>,
        row_iterator: &mut RowIterator<'alloc>,
        cell_iterator: &mut CellIterator<'alloc>,
    ) -> Option<Vec<String>> {
        let mut rows = row_iterator.update(snapshot).ok()?;
        let mut lines = Vec::new();
        while let Some(row) = rows.next() {
            let mut cells = cell_iterator.update(row).ok()?;
            let mut line = String::new();
            while cells.next().is_some() {
                let raw_cell = cells.raw_cell().ok()?;
                if matches!(
                    raw_cell.wide().ok()?,
                    CellWide::SpacerTail | CellWide::SpacerHead
                ) {
                    continue;
                }

                let graphemes = cells.graphemes().ok()?;
                if graphemes.is_empty() {
                    line.push(' ');
                } else {
                    line.extend(graphemes);
                }
            }
            lines.push(line.trim_end().to_owned());
        }
        Some(lines)
    }

    fn cursor(
        snapshot: &RenderSnapshot<'_, '_>,
        previous: TerminalCursor,
    ) -> Option<TerminalCursor> {
        let shape = match snapshot.cursor_visual_style().ok()? {
            CursorVisualStyle::Bar => protocol::CursorShape::Beam,
            CursorVisualStyle::Underline => protocol::CursorShape::Underline,
            CursorVisualStyle::Block | CursorVisualStyle::BlockHollow => {
                protocol::CursorShape::Block
            }
            _ => protocol::CursorShape::Block,
        };
        let Some(viewport) = snapshot.cursor_viewport().ok()? else {
            return Some(TerminalCursor {
                visible: false,
                shape,
                ..previous
            });
        };

        Some(TerminalCursor {
            row: u32::from(viewport.y),
            col: u32::from(viewport.x),
            visible: snapshot.cursor_visible().ok()?,
            shape,
        })
    }

    fn surface_kind(terminal: &Terminal<'_, '_>) -> Option<protocol::SurfaceKind> {
        if terminal.mode(Mode::ALT_SCREEN).ok()?
            || terminal.mode(Mode::ALT_SCREEN_SAVE).ok()?
            || terminal.mode(Mode::ALT_SCREEN_LEGACY).ok()?
        {
            Some(protocol::SurfaceKind::Alternate)
        } else {
            Some(protocol::SurfaceKind::Main)
        }
    }
}

#[cfg(test)]
mod tests {
    use nmux_proto::protocol;

    use super::{
        InterimTextTerminalEngine, PaneTerminalEngines, TerminalCursor, TerminalEngine,
        TerminalEngineKind, TerminalInput,
    };

    #[cfg(feature = "libghostty-vt")]
    fn terminal_input<'a>(
        rows: u32,
        surface_lines: &'a [String],
        scrollback_lines: &'a [String],
    ) -> TerminalInput<'a> {
        terminal_input_with_size(80, rows, surface_lines, scrollback_lines)
    }

    #[cfg(feature = "libghostty-vt")]
    fn terminal_input_with_size<'a>(
        cols: u32,
        rows: u32,
        surface_lines: &'a [String],
        scrollback_lines: &'a [String],
    ) -> TerminalInput<'a> {
        TerminalInput {
            pane_id: "pane-1",
            cols,
            rows,
            surface: protocol::SurfaceKind::Main,
            cursor: TerminalCursor {
                row: 0,
                col: 0,
                visible: true,
                shape: protocol::CursorShape::Block,
            },
            surface_lines,
            scrollback_lines,
        }
    }

    #[test]
    fn interim_text_engine_normalizes_process_output() {
        let mut engine = InterimTextTerminalEngine;
        let scrollback_lines = vec!["existing".to_owned()];
        let input = TerminalInput {
            pane_id: "pane-1",
            cols: 80,
            rows: 24,
            surface: protocol::SurfaceKind::Main,
            cursor: TerminalCursor {
                row: 0,
                col: 0,
                visible: true,
                shape: protocol::CursorShape::Block,
            },
            surface_lines: &[],
            scrollback_lines: &scrollback_lines,
        };

        let update = engine
            .apply_output(input, b"hello\r\nsecond\x1b[31m line\n")
            .expect("terminal update");

        assert_eq!(
            update.scrollback_lines,
            vec![
                "existing".to_owned(),
                "hello".to_owned(),
                "second[31m line".to_owned()
            ]
        );
        assert_eq!(update.surface_lines, update.scrollback_lines);
        assert_eq!(
            update.cursor,
            TerminalCursor {
                row: 2,
                col: 0,
                visible: true,
                shape: protocol::CursorShape::Block
            }
        );
        assert_eq!(update.surface, protocol::SurfaceKind::Main);
    }

    #[test]
    fn interim_text_engine_keeps_visible_tail() {
        let mut engine = InterimTextTerminalEngine;
        let scrollback_lines = vec!["one".to_owned(), "two".to_owned()];
        let input = TerminalInput {
            pane_id: "pane-1",
            cols: 80,
            rows: 2,
            surface: protocol::SurfaceKind::Main,
            cursor: TerminalCursor {
                row: 0,
                col: 0,
                visible: false,
                shape: protocol::CursorShape::Beam,
            },
            surface_lines: &[],
            scrollback_lines: &scrollback_lines,
        };

        let update = engine
            .apply_output(input, b"three\n")
            .expect("terminal update");

        assert_eq!(
            update.scrollback_lines,
            vec!["one".to_owned(), "two".to_owned(), "three".to_owned()]
        );
        assert_eq!(
            update.surface_lines,
            vec!["two".to_owned(), "three".to_owned()]
        );
        assert_eq!(
            update.cursor,
            TerminalCursor {
                row: 1,
                col: 0,
                visible: false,
                shape: protocol::CursorShape::Beam
            }
        );
    }

    #[test]
    fn interim_text_engine_resizes_visible_tail() {
        let mut engine = InterimTextTerminalEngine;
        let scrollback_lines = vec!["one".to_owned(), "two".to_owned(), "three".to_owned()];
        let input = TerminalInput {
            pane_id: "pane-1",
            cols: 80,
            rows: 3,
            surface: protocol::SurfaceKind::Alternate,
            cursor: TerminalCursor {
                row: 2,
                col: 0,
                visible: true,
                shape: protocol::CursorShape::Underline,
            },
            surface_lines: &scrollback_lines,
            scrollback_lines: &scrollback_lines,
        };

        let update = engine.resize(input, 100, 2).expect("terminal update");

        assert_eq!(
            update.surface_lines,
            vec!["two".to_owned(), "three".to_owned()]
        );
        assert_eq!(update.scrollback_lines, scrollback_lines);
        assert_eq!(update.surface, protocol::SurfaceKind::Alternate);
        assert_eq!(
            update.cursor,
            TerminalCursor {
                row: 1,
                col: 0,
                visible: true,
                shape: protocol::CursorShape::Underline
            }
        );
    }

    #[test]
    fn pane_terminal_engines_reuses_engine_per_pane() {
        let mut engines = PaneTerminalEngines::new(TerminalEngineKind::InterimText);
        assert_eq!(engines.kind(), TerminalEngineKind::InterimText);

        let _ = engines.engine_mut("pane-1");
        let _ = engines.engine_mut("pane-1");
        assert_eq!(engines.pane_count(), 1);

        let _ = engines.engine_mut("pane-2");
        assert_eq!(engines.pane_count(), 2);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_consumes_ansi_sequences() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input(2, &empty, &empty),
                b"\x1b[31mred\x1b[0m\r\nnext",
            )
            .expect("terminal update");

        assert_eq!(update.surface, protocol::SurfaceKind::Main);
        assert!(
            update.surface_lines.iter().any(|line| line.contains("red")),
            "surface did not contain red text: {:?}",
            update.surface_lines
        );
        assert!(
            update
                .surface_lines
                .iter()
                .all(|line| !line.contains("[31m") && !line.contains("[0m")),
            "ansi sequences leaked into surface text: {:?}",
            update.surface_lines
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_emits_cursor_only_patch_for_cursor_movement() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        let first = engine
            .apply_output(terminal_input(2, &empty, &empty), b"alpha\r\nbeta")
            .expect("initial terminal update");

        let cursor_move = engine
            .apply_output(
                terminal_input(2, &first.surface_lines, &first.scrollback_lines),
                b"\x1b[1;3H",
            )
            .expect("cursor movement update");

        assert_eq!(cursor_move.patch_kind, protocol::PatchKind::CursorOnly);
        assert_eq!(cursor_move.surface_lines, first.surface_lines);
        assert_eq!(cursor_move.scrollback_lines, first.scrollback_lines);
        assert_eq!(
            cursor_move.cursor,
            TerminalCursor {
                row: 0,
                col: 2,
                visible: true,
                shape: protocol::CursorShape::Block
            }
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_reports_alternate_screen() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(terminal_input(2, &empty, &empty), b"\x1b[?1049halternate")
            .expect("alternate screen update");

        assert_eq!(update.surface, protocol::SurfaceKind::Alternate);
        assert!(
            update
                .surface_lines
                .iter()
                .any(|line| line.contains("alternate")),
            "surface did not contain alternate text: {:?}",
            update.surface_lines
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_resizes_after_wrapped_output() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        let first = engine
            .apply_output(
                terminal_input_with_size(6, 2, &empty, &empty),
                b"abcdef ghijkl mnopqr",
            )
            .expect("initial wrapped terminal update");

        let resized = engine
            .resize(
                terminal_input_with_size(6, 2, &first.surface_lines, &first.scrollback_lines),
                12,
                3,
            )
            .expect("resize update");

        assert_eq!(resized.patch_kind, protocol::PatchKind::ReplaceRows);
        assert_eq!(resized.surface, protocol::SurfaceKind::Main);
        assert!(
            resized.surface_lines.len() <= 3,
            "resize returned more rows than viewport: {:?}",
            resized.surface_lines
        );
        assert!(
            resized
                .surface_lines
                .iter()
                .any(|line| line.contains("abcdef")),
            "resize lost wrapped output: {:?}",
            resized.surface_lines
        );
        assert_ne!(resized.surface_lines, first.surface_lines);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_backend_owned_scrollback() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input_with_size(20, 2, &empty, &empty),
                b"one\r\ntwo\r\nthree\r\nfour",
            )
            .expect("terminal update");

        assert!(
            update.scrollback_lines.len() > update.surface_lines.len(),
            "scrollback did not include history beyond the viewport: {:?}",
            update.scrollback_lines
        );
        for expected in ["one", "two", "three", "four"] {
            assert!(
                update
                    .scrollback_lines
                    .iter()
                    .any(|line| line.contains(expected)),
                "scrollback missing {expected}: {:?}",
                update.scrollback_lines
            );
        }
    }
}
use std::collections::HashMap;
