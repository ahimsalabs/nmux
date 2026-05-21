#[derive(Debug, Clone, Copy)]
pub struct TerminalInput<'a> {
    pub pane_id: &'a str,
    pub cols: u32,
    pub rows: u32,
    pub cursor: TerminalCursor,
    pub surface_lines: &'a [String],
    pub scrollback_lines: &'a [String],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCursor {
    pub row: u32,
    pub col: u32,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalUpdate {
    pub cursor: TerminalCursor,
    pub surface_lines: Vec<String>,
    pub scrollback_lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalEngineKind {
    InterimText,
}

pub trait TerminalEngine {
    fn apply_output(&mut self, input: TerminalInput<'_>, output: &[u8]) -> Option<TerminalUpdate>;
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
    }
}

#[derive(Debug, Default)]
pub struct InterimTextTerminalEngine;

impl TerminalEngine for InterimTextTerminalEngine {
    fn apply_output(&mut self, input: TerminalInput<'_>, output: &[u8]) -> Option<TerminalUpdate> {
        let mut scrollback_lines = input.scrollback_lines.to_vec();
        scrollback_lines.extend(text_lines_from_pty_output(output));

        let visible_start = scrollback_lines.len().saturating_sub(input.rows as usize);
        let surface_lines = scrollback_lines[visible_start..].to_vec();
        let cursor = TerminalCursor {
            row: surface_lines.len().saturating_sub(1) as u32,
            col: 0,
            visible: input.cursor.visible,
        };

        Some(TerminalUpdate {
            cursor,
            surface_lines,
            scrollback_lines,
        })
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

#[cfg(test)]
mod tests {
    use super::{
        InterimTextTerminalEngine, PaneTerminalEngines, TerminalCursor, TerminalEngine,
        TerminalEngineKind, TerminalInput,
    };

    #[test]
    fn interim_text_engine_normalizes_process_output() {
        let mut engine = InterimTextTerminalEngine;
        let scrollback_lines = vec!["existing".to_owned()];
        let input = TerminalInput {
            pane_id: "pane-1",
            cols: 80,
            rows: 24,
            cursor: TerminalCursor {
                row: 0,
                col: 0,
                visible: true,
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
                visible: true
            }
        );
    }

    #[test]
    fn interim_text_engine_keeps_visible_tail() {
        let mut engine = InterimTextTerminalEngine;
        let scrollback_lines = vec!["one".to_owned(), "two".to_owned()];
        let input = TerminalInput {
            pane_id: "pane-1",
            cols: 80,
            rows: 2,
            cursor: TerminalCursor {
                row: 0,
                col: 0,
                visible: false,
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
                visible: false
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
}
use std::collections::HashMap;
