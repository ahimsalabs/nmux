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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalUpdate {
    pub patch_kind: protocol::PatchKind,
    pub surface: protocol::SurfaceKind,
    pub cursor: TerminalCursor,
    pub styles: Vec<PaneStyle>,
    pub surface_lines: Vec<String>,
    pub surface_row_runs: Vec<Vec<CellRun>>,
    pub scrollback_lines: Vec<String>,
    pub scrollback_row_runs: Vec<Vec<CellRun>>,
}

impl TerminalUpdate {
    pub fn plain(
        patch_kind: protocol::PatchKind,
        surface: protocol::SurfaceKind,
        cursor: TerminalCursor,
        surface_lines: Vec<String>,
        scrollback_lines: Vec<String>,
    ) -> Self {
        Self {
            patch_kind,
            surface,
            cursor,
            styles: vec![PaneStyle::default()],
            surface_row_runs: plain_row_runs(&surface_lines),
            scrollback_row_runs: plain_row_runs(&scrollback_lines),
            surface_lines,
            scrollback_lines,
        }
    }
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

    TerminalUpdate::plain(
        protocol::PatchKind::ReplaceRows,
        surface,
        cursor,
        surface_lines,
        scrollback_lines,
    )
}

pub(crate) fn plain_row_runs(lines: &[String]) -> Vec<Vec<CellRun>> {
    lines
        .iter()
        .map(|line| vec![CellRun::plain(line.clone())])
        .collect()
}

pub(crate) fn cell_runs_text(runs: &[CellRun]) -> String {
    let mut text = String::new();
    for run in runs {
        text.push_str(&run.text);
    }
    text
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
        style::{RgbColor, Style, StyleColor, Underline},
        terminal::{Mode, ScrollViewport},
    };
    use nmux_proto::protocol;

    use super::{
        CellRun, PaneStyle, TerminalCursor, TerminalEngine, TerminalInput, TerminalUpdate,
    };

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
            let mut styles = vec![PaneStyle::default()];
            let scrollback_rows = if surface == protocol::SurfaceKind::Main {
                self.scrollback_rows(&mut styles)?
            } else {
                ExtractedRows {
                    lines: input.scrollback_lines.to_vec(),
                    row_runs: super::plain_row_runs(input.scrollback_lines),
                }
            };
            self.terminal.scroll_viewport(ScrollViewport::Bottom);
            let snapshot = self.render_state.update(&self.terminal).ok()?;
            let surface_rows = extract_rows(
                &snapshot,
                &mut self.row_iterator,
                &mut self.cell_iterator,
                &mut styles,
            )?;
            let surface_lines = surface_rows.lines.clone();
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
                styles,
                surface_row_runs: surface_rows.row_runs,
                scrollback_row_runs: scrollback_rows.row_runs,
                surface_lines,
                scrollback_lines: scrollback_rows.lines,
            })
        }

        fn scrollback_rows(&mut self, styles: &mut Vec<PaneStyle>) -> Option<ExtractedRows> {
            let total_rows = self.terminal.total_rows().ok()?;
            if total_rows == 0 {
                return Some(ExtractedRows {
                    lines: Vec::new(),
                    row_runs: Vec::new(),
                });
            }

            self.terminal.scroll_viewport(ScrollViewport::Top);
            let snapshot = self.render_state.update(&self.terminal).ok()?;
            let mut rows = extract_rows(
                &snapshot,
                &mut self.row_iterator,
                &mut self.cell_iterator,
                styles,
            )?;
            rows.lines.truncate(total_rows);
            rows.row_runs.truncate(total_rows);

            while rows.lines.len() < total_rows {
                self.terminal.scroll_viewport(ScrollViewport::Delta(1));
                let snapshot = self.render_state.update(&self.terminal).ok()?;
                let viewport_lines = extract_rows(
                    &snapshot,
                    &mut self.row_iterator,
                    &mut self.cell_iterator,
                    styles,
                )?;
                let Some(next_line) = viewport_lines.lines.last() else {
                    break;
                };
                let Some(next_runs) = viewport_lines.row_runs.last() else {
                    break;
                };
                rows.lines.push(next_line.clone());
                rows.row_runs.push(next_runs.clone());
            }

            Some(rows)
        }
    }

    struct ExtractedRows {
        lines: Vec<String>,
        row_runs: Vec<Vec<CellRun>>,
    }

    fn extract_rows<'alloc>(
        snapshot: &RenderSnapshot<'alloc, '_>,
        row_iterator: &mut RowIterator<'alloc>,
        cell_iterator: &mut CellIterator<'alloc>,
        styles: &mut Vec<PaneStyle>,
    ) -> Option<ExtractedRows> {
        let mut rows = row_iterator.update(snapshot).ok()?;
        let mut row_runs = Vec::new();
        let mut lines = Vec::new();
        while let Some(row) = rows.next() {
            let mut cells = cell_iterator.update(row).ok()?;
            let mut runs: Vec<CellRun> = Vec::new();
            while cells.next().is_some() {
                let raw_cell = cells.raw_cell().ok()?;
                let width = match raw_cell.wide().ok()? {
                    CellWide::Narrow => 1,
                    CellWide::Wide => 2,
                    CellWide::SpacerTail | CellWide::SpacerHead => continue,
                };

                let text = cell_text(&cells)?;
                let style_id = style_id(styles, pane_style(&cells)?);
                if let Some(last) = runs.last_mut()
                    && last.style_id == style_id
                    && last.flags == 0
                    && last.hyperlink_id == 0
                {
                    last.text.push_str(&text);
                    last.cell_widths.push(width);
                } else {
                    runs.push(CellRun {
                        text,
                        cell_widths: vec![width],
                        style_id,
                        flags: 0,
                        hyperlink_id: 0,
                    });
                }
            }
            trim_trailing_spaces(&mut runs);
            lines.push(super::cell_runs_text(&runs));
            row_runs.push(runs);
        }
        Some(ExtractedRows { lines, row_runs })
    }

    fn cell_text(cells: &libghostty_vt::render::CellIteration<'_, '_>) -> Option<String> {
        let graphemes = cells.graphemes().ok()?;
        if graphemes.is_empty() {
            Some(" ".to_owned())
        } else {
            Some(graphemes.into_iter().collect())
        }
    }

    fn pane_style(cells: &libghostty_vt::render::CellIteration<'_, '_>) -> Option<PaneStyle> {
        let style = cells.style().ok()?;
        Some(PaneStyle {
            fg_rgba: cells.fg_color().ok().flatten().map_or(0, rgba),
            bg_rgba: cells.bg_color().ok().flatten().map_or(0, rgba),
            underline_rgba: style_color_rgba(style.underline_color),
            flags: style_flags(style),
        })
    }

    fn style_id(styles: &mut Vec<PaneStyle>, style: PaneStyle) -> u32 {
        if let Some(index) = styles.iter().position(|known| *known == style) {
            index as u32
        } else {
            styles.push(style);
            (styles.len() - 1) as u32
        }
    }

    fn rgba(color: RgbColor) -> u32 {
        u32::from_be_bytes([color.r, color.g, color.b, 0xff])
    }

    fn style_color_rgba(color: StyleColor) -> u32 {
        match color {
            StyleColor::Rgb(rgb) => rgba(rgb),
            _ => 0,
        }
    }

    fn style_flags(style: Style) -> u32 {
        let mut flags = 0;
        if style.bold {
            flags |= 1 << 0;
        }
        if style.italic {
            flags |= 1 << 1;
        }
        if style.faint {
            flags |= 1 << 2;
        }
        if style.blink {
            flags |= 1 << 3;
        }
        if style.inverse {
            flags |= 1 << 4;
        }
        if style.invisible {
            flags |= 1 << 5;
        }
        if style.strikethrough {
            flags |= 1 << 6;
        }
        if style.overline {
            flags |= 1 << 7;
        }
        flags | underline_flags(style.underline)
    }

    fn underline_flags(underline: Underline) -> u32 {
        match underline {
            Underline::Single => 1 << 8,
            Underline::Double => 1 << 9,
            Underline::Curly => 1 << 10,
            Underline::Dotted => 1 << 11,
            Underline::Dashed => 1 << 12,
            _ => 0,
        }
    }

    fn trim_trailing_spaces(runs: &mut Vec<CellRun>) {
        while let Some(last) = runs.last_mut() {
            while last.text.ends_with(' ') {
                last.text.pop();
                last.cell_widths.pop();
            }
            if last.text.is_empty() {
                runs.pop();
            } else {
                break;
            }
        }
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

    #[cfg(feature = "libghostty-vt")]
    fn terminal_input_from_update<'a>(update: &'a super::TerminalUpdate) -> TerminalInput<'a> {
        TerminalInput {
            pane_id: "pane-1",
            cols: 80,
            rows: update.surface_lines.len() as u32,
            surface: update.surface,
            cursor: update.cursor,
            surface_lines: &update.surface_lines,
            scrollback_lines: &update.scrollback_lines,
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
    fn libghostty_vt_engine_extracts_cell_runs_and_style_ids() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input(2, &empty, &empty),
                b"\x1b[31mred\x1b[0m plain\r\nwide:\xe4\xb8\xad",
            )
            .expect("terminal update");

        assert!(
            update.styles.len() > 1,
            "styled output did not add a non-default style: {:?}",
            update.styles
        );
        let styled_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("red"))
            .expect("styled red run");
        assert_ne!(styled_run.style_id, 0);

        let plain_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains(" plain"))
            .expect("plain run");
        assert_eq!(plain_run.style_id, 0);

        let wide_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains('中'))
            .expect("wide run");
        let wide_index = wide_run
            .text
            .chars()
            .position(|ch| ch == '中')
            .expect("wide char");
        assert_eq!(wide_run.cell_widths[wide_index], 2);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_resolves_palette_indexed_styles() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input(2, &empty, &empty),
                b"\x1b[38;5;196;48;5;21mpalette\x1b[0m",
            )
            .expect("terminal update");

        let palette_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("palette"))
            .expect("palette-styled run");
        assert_ne!(palette_run.style_id, 0);

        let style = update
            .styles
            .get(palette_run.style_id as usize)
            .expect("palette style");
        assert_ne!(
            style.fg_rgba, 0,
            "palette foreground should resolve to RGBA"
        );
        assert_ne!(
            style.bg_rgba, 0,
            "palette background should resolve to RGBA"
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_basic_sgr_style_flags() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input(2, &empty, &empty),
                b"\x1b[1;3;4;9mflags\x1b[0m plain",
            )
            .expect("terminal update");

        let flags_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("flags"))
            .expect("flag-styled run");
        let style = update
            .styles
            .get(flags_run.style_id as usize)
            .expect("flag style");

        assert_ne!(flags_run.style_id, 0);
        assert_ne!(style.flags & (1 << 0), 0, "bold flag missing");
        assert_ne!(style.flags & (1 << 1), 0, "italic flag missing");
        assert_ne!(style.flags & (1 << 6), 0, "strikethrough flag missing");
        assert_ne!(style.flags & (1 << 8), 0, "underline flag missing");

        let plain_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains(" plain"))
            .expect("plain run");
        assert_eq!(plain_run.style_id, 0);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_preserves_combining_mark_cell_widths() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input(2, &empty, &empty),
                b"accent:e\xcc\x81\r\nplain:e",
            )
            .expect("terminal update");

        let accent_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("e\u{301}"))
            .expect("combining mark run");
        assert!(
            accent_run.text.contains("accent:e\u{301}"),
            "combining mark was not preserved in run text: {:?}",
            accent_run
        );
        let accent_index = accent_run
            .text
            .chars()
            .position(|ch| ch == 'e')
            .expect("accent base char");
        assert_eq!(
            accent_run.cell_widths[accent_index], 1,
            "combining mark should not add a second cell width: {:?}",
            accent_run
        );
        assert!(
            accent_run.cell_widths.len() < accent_run.text.chars().count(),
            "run widths should be per rendered cell, not per scalar: {:?}",
            accent_run
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_preserves_hyperlink_text_without_ids() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input(2, &empty, &empty),
                b"\x1b]8;;https://example.com\x1b\\linked\x1b]8;;\x1b\\ text",
            )
            .expect("terminal update");

        let link_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("linked"))
            .expect("hyperlink text run");
        assert!(
            link_run.text.contains("linked text"),
            "hyperlink text was not preserved in rendered row: {:?}",
            link_run
        );
        assert_eq!(
            link_run.hyperlink_id, 0,
            "nmux must not invent hyperlink IDs before a hyperlink table exists"
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_safe_api_tracks_bracketed_paste_mode() {
        use libghostty_vt::{Terminal, TerminalOptions, terminal::Mode};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");

        assert!(!terminal.mode(Mode::BRACKETED_PASTE).expect("mode"));

        terminal.vt_write(b"\x1b[?2004h");
        assert!(terminal.mode(Mode::BRACKETED_PASTE).expect("mode"));

        terminal.vt_write(b"\x1b[?2004l");
        assert!(!terminal.mode(Mode::BRACKETED_PASTE).expect("mode"));
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_key_encoder_uses_application_cursor_mode() {
        use libghostty_vt::{
            Terminal, TerminalOptions,
            key::{Action, Encoder, Event, Key},
            terminal::Mode,
        };

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut event = Event::new().expect("event");
        event.set_action(Action::Press).set_key(Key::ArrowUp);

        let mut normal_encoder = Encoder::new().expect("encoder");
        normal_encoder.set_options_from_terminal(&terminal);
        let mut normal = Vec::new();
        normal_encoder
            .encode_to_vec(&event, &mut normal)
            .expect("normal arrow");

        terminal.set_mode(Mode::DECCKM, true).expect("set mode");
        let mut application_encoder = Encoder::new().expect("encoder");
        application_encoder.set_options_from_terminal(&terminal);
        let mut application = Vec::new();
        application_encoder
            .encode_to_vec(&event, &mut application)
            .expect("application arrow");

        assert_eq!(normal, b"\x1b[A");
        assert_eq!(application, b"\x1bOA");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_safe_api_tracks_mouse_tracking_modes() {
        use libghostty_vt::{Terminal, TerminalOptions};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");

        assert!(!terminal.is_mouse_tracking().expect("mouse tracking"));

        terminal.vt_write(b"\x1b[?1000h");
        assert!(terminal.is_mouse_tracking().expect("mouse tracking"));
        terminal.vt_write(b"\x1b[?1000l");
        assert!(!terminal.is_mouse_tracking().expect("mouse tracking"));

        terminal.vt_write(b"\x1b[?1002h");
        assert!(terminal.is_mouse_tracking().expect("mouse tracking"));
        terminal.vt_write(b"\x1b[?1002l");
        assert!(!terminal.is_mouse_tracking().expect("mouse tracking"));

        terminal.vt_write(b"\x1b[?1003h");
        assert!(terminal.is_mouse_tracking().expect("mouse tracking"));
        terminal.vt_write(b"\x1b[?1003l");
        assert!(!terminal.is_mouse_tracking().expect("mouse tracking"));
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
    fn libghostty_vt_engine_emits_cursor_only_patch_for_cursor_visibility_and_shape() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        let first = engine
            .apply_output(terminal_input(2, &empty, &empty), b"alpha\r\nbeta")
            .expect("initial terminal update");

        let hidden_beam = engine
            .apply_output(terminal_input_from_update(&first), b"\x1b[?25l\x1b[6 q")
            .expect("hidden beam cursor update");

        assert_eq!(hidden_beam.patch_kind, protocol::PatchKind::CursorOnly);
        assert_eq!(hidden_beam.surface_lines, first.surface_lines);
        assert_eq!(hidden_beam.scrollback_lines, first.scrollback_lines);
        assert_eq!(
            hidden_beam.cursor,
            TerminalCursor {
                row: first.cursor.row,
                col: first.cursor.col,
                visible: false,
                shape: protocol::CursorShape::Beam,
            }
        );

        let visible_underline = engine
            .apply_output(
                terminal_input_from_update(&hidden_beam),
                b"\x1b[?25h\x1b[4 q",
            )
            .expect("visible underline cursor update");

        assert_eq!(
            visible_underline.patch_kind,
            protocol::PatchKind::CursorOnly
        );
        assert_eq!(visible_underline.surface_lines, first.surface_lines);
        assert_eq!(visible_underline.scrollback_lines, first.scrollback_lines);
        assert_eq!(
            visible_underline.cursor,
            TerminalCursor {
                row: first.cursor.row,
                col: first.cursor.col,
                visible: true,
                shape: protocol::CursorShape::Underline,
            }
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_render_state_tracks_cursor_blinking_without_protocol_fields() {
        use libghostty_vt::{RenderState, Terminal, TerminalOptions};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut render_state = RenderState::new().expect("render state");

        terminal.vt_write(b"\x1b[?12l");
        let snapshot = render_state.update(&terminal).expect("snapshot");
        assert!(!snapshot.cursor_blinking().expect("cursor blink off"));
        drop(snapshot);

        terminal.vt_write(b"\x1b[?12h");
        let snapshot = render_state.update(&terminal).expect("snapshot");
        assert!(snapshot.cursor_blinking().expect("cursor blink on"));
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
    fn libghostty_vt_engine_restores_main_screen_after_alternate_screen() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let primary = engine
            .apply_output(terminal_input(2, &empty, &empty), b"primary")
            .expect("primary update");
        assert_eq!(primary.surface, protocol::SurfaceKind::Main);
        assert!(
            primary
                .surface_lines
                .iter()
                .any(|line| line.contains("primary")),
            "primary surface did not contain text: {:?}",
            primary.surface_lines
        );

        let alternate = engine
            .apply_output(
                terminal_input(2, &primary.surface_lines, &primary.scrollback_lines),
                b"\x1b[?1049halternate",
            )
            .expect("alternate update");
        assert_eq!(alternate.surface, protocol::SurfaceKind::Alternate);
        assert!(
            alternate
                .surface_lines
                .iter()
                .any(|line| line.contains("alternate")),
            "alternate surface did not contain text: {:?}",
            alternate.surface_lines
        );

        let restored = engine
            .apply_output(
                terminal_input(2, &alternate.surface_lines, &alternate.scrollback_lines),
                b"\x1b[?1049l",
            )
            .expect("restore update");
        assert_eq!(restored.surface, protocol::SurfaceKind::Main);
        assert!(
            restored
                .surface_lines
                .iter()
                .any(|line| line.contains("primary")),
            "main surface was not restored: {:?}",
            restored.surface_lines
        );
        assert!(
            restored
                .surface_lines
                .iter()
                .all(|line| !line.contains("alternate")),
            "alternate text leaked into restored main surface: {:?}",
            restored.surface_lines
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
                b"\x1b[31mone\x1b[0m\r\ntwo\r\nthree\r\nfour",
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

        let styled_history_run = update
            .scrollback_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("one"))
            .expect("styled scrollback run");
        assert_ne!(
            styled_history_run.style_id, 0,
            "styled scrollback should preserve Ghostty style IDs: {:?}",
            update.scrollback_row_runs
        );
        let style = update
            .styles
            .get(styled_history_run.style_id as usize)
            .expect("scrollback style");
        assert_ne!(
            style.fg_rgba, 0,
            "styled scrollback should reference a resolved style table entry"
        );
    }
}
use std::collections::HashMap;
