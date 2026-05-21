#[derive(Debug, Clone, Copy)]
pub struct TerminalInput<'a> {
    pub pane_id: &'a str,
    pub cols: u32,
    pub rows: u32,
    pub surface_lines: &'a [String],
    pub scrollback_lines: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalUpdate {
    pub surface_lines: Vec<String>,
    pub scrollback_lines: Vec<String>,
}

pub trait TerminalEngine {
    fn apply_output(&mut self, input: TerminalInput<'_>, output: &[u8]) -> Option<TerminalUpdate>;
}

#[derive(Debug, Default)]
pub struct InterimTextTerminalEngine;

impl TerminalEngine for InterimTextTerminalEngine {
    fn apply_output(&mut self, input: TerminalInput<'_>, output: &[u8]) -> Option<TerminalUpdate> {
        let mut scrollback_lines = input.scrollback_lines.to_vec();
        scrollback_lines.extend(text_lines_from_pty_output(output));

        let visible_start = scrollback_lines.len().saturating_sub(input.rows as usize);
        let surface_lines = scrollback_lines[visible_start..].to_vec();

        Some(TerminalUpdate {
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
    use super::{InterimTextTerminalEngine, TerminalEngine, TerminalInput};

    #[test]
    fn interim_text_engine_normalizes_process_output() {
        let mut engine = InterimTextTerminalEngine;
        let scrollback_lines = vec!["existing".to_owned()];
        let input = TerminalInput {
            pane_id: "pane-1",
            cols: 80,
            rows: 24,
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
    }

    #[test]
    fn interim_text_engine_keeps_visible_tail() {
        let mut engine = InterimTextTerminalEngine;
        let scrollback_lines = vec!["one".to_owned(), "two".to_owned()];
        let input = TerminalInput {
            pane_id: "pane-1",
            cols: 80,
            rows: 2,
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
    }
}
