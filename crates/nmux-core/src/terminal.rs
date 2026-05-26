use std::collections::HashMap;

use nmux_proto::protocol;

#[derive(Debug, Clone)]
pub struct TerminalInput<'a> {
    pub pane_id: &'a str,
    pub cols: u32,
    pub rows: u32,
    pub surface: protocol::SurfaceKind,
    pub cursor: TerminalCursor,
    pub modes: TerminalModes,
    pub title: &'a str,
    pub working_directory: &'a str,
    pub colors: TerminalColors,
    pub styles: &'a [PaneStyle],
    pub surface_lines: &'a [String],
    pub surface_row_runs: &'a [Vec<CellRun>],
    pub surface_semantic_prompts: &'a [protocol::RowSemanticPrompt],
    pub surface_dirty_rows: &'a [bool],
    pub surface_kitty_placeholders: &'a [bool],
    pub scrollback_lines: &'a [String],
    pub scrollback_row_runs: &'a [Vec<CellRun>],
    pub scrollback_semantic_prompts: &'a [protocol::RowSemanticPrompt],
    pub scrollback_dirty_rows: &'a [bool],
    pub scrollback_kitty_placeholders: &'a [bool],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCursor {
    pub row: u32,
    pub col: u32,
    pub visible: bool,
    pub shape: protocol::CursorShape,
    pub blinking: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalModes {
    pub bracketed_paste: bool,
    pub mouse_tracking: bool,
    pub focus_reporting: bool,
    pub application_keypad: bool,
    pub application_cursor: bool,
    pub origin: bool,
    pub wraparound: bool,
    pub mouse_tracking_mode: protocol::MouseTrackingMode,
    pub mouse_format: protocol::MouseFormat,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalColors {
    pub default_fg_rgba: u32,
    pub default_bg_rgba: u32,
    pub cursor_rgba: u32,
    pub cursor_rgba_set: bool,
    pub palette_rgba: Vec<u32>,
}

impl Default for TerminalModes {
    fn default() -> Self {
        Self {
            bracketed_paste: false,
            mouse_tracking: false,
            focus_reporting: false,
            application_keypad: false,
            application_cursor: false,
            origin: false,
            wraparound: true,
            mouse_tracking_mode: protocol::MouseTrackingMode::None,
            mouse_format: protocol::MouseFormat::X10,
        }
    }
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
    pub semantic_content: protocol::CellSemanticContent,
}

pub const CELL_RUN_FLAG_HYPERLINK_PRESENT: u32 = 1 << 0;

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
            semantic_content: protocol::CellSemanticContent::Output,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalUpdate {
    pub patch_kind: protocol::PatchKind,
    pub surface: protocol::SurfaceKind,
    pub preserve_scrollback: bool,
    pub cursor: TerminalCursor,
    pub modes: TerminalModes,
    pub title: String,
    pub working_directory: String,
    pub colors: TerminalColors,
    pub styles: Vec<PaneStyle>,
    pub surface_lines: Vec<String>,
    pub surface_row_runs: Vec<Vec<CellRun>>,
    pub surface_semantic_prompts: Vec<protocol::RowSemanticPrompt>,
    pub surface_dirty_rows: Vec<bool>,
    pub surface_kitty_placeholders: Vec<bool>,
    pub scrollback_lines: Vec<String>,
    pub scrollback_row_runs: Vec<Vec<CellRun>>,
    pub scrollback_semantic_prompts: Vec<protocol::RowSemanticPrompt>,
    pub scrollback_dirty_rows: Vec<bool>,
    pub scrollback_kitty_placeholders: Vec<bool>,
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
            preserve_scrollback: false,
            cursor,
            modes: TerminalModes::default(),
            title: String::new(),
            working_directory: String::new(),
            colors: TerminalColors::default(),
            styles: vec![PaneStyle::default()],
            surface_row_runs: plain_row_runs(&surface_lines),
            surface_semantic_prompts: plain_row_semantic_prompts(&surface_lines),
            surface_dirty_rows: plain_row_dirty_flags(&surface_lines),
            surface_kitty_placeholders: plain_row_kitty_placeholders(&surface_lines),
            scrollback_row_runs: plain_row_runs(&scrollback_lines),
            scrollback_semantic_prompts: plain_row_semantic_prompts(&scrollback_lines),
            scrollback_dirty_rows: plain_row_dirty_flags(&scrollback_lines),
            scrollback_kitty_placeholders: plain_row_kitty_placeholders(&scrollback_lines),
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

    fn drain_pty_writes(&mut self) -> Vec<Vec<u8>> {
        Vec::new()
    }

    fn encode_key_input(&mut self, _input: KeyTerminalInput<'_>) -> Option<Vec<u8>> {
        None
    }

    fn encode_mouse_input(&mut self, _input: MouseTerminalInput) -> Option<Vec<u8>> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyTerminalInput<'a> {
    pub key_name: &'a str,
    pub modifiers: u32,
    pub application_keypad: bool,
    pub application_cursor: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseTerminalInput {
    pub row: u32,
    pub col: u32,
    pub pixel_x: Option<u32>,
    pub pixel_y: Option<u32>,
    pub button: MouseButton,
    pub action: MouseAction,
    pub modifiers: u32,
    pub mouse_format: protocol::MouseFormat,
    pub cols: u32,
    pub rows: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    None,
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
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
        #[cfg(feature = "libghostty-vt")]
        {
            Self::LibghosttyVt
        }
        #[cfg(not(feature = "libghostty-vt"))]
        {
            Self::InterimText
        }
    }
}

fn terminal_engine_for_kind(kind: TerminalEngineKind) -> Box<dyn TerminalEngine> {
    match kind {
        TerminalEngineKind::InterimText => Box::new(InterimTextTerminalEngine::default()),
        #[cfg(feature = "libghostty-vt")]
        TerminalEngineKind::LibghosttyVt => Box::new(ghostty_vt::LibghosttyVtTerminalEngine::new()),
    }
}

#[cfg(feature = "libghostty-vt")]
pub fn libghostty_vt_supports_kitty_graphics() -> bool {
    libghostty_vt::build_info::supports_kitty_graphics().unwrap_or(false)
}

#[derive(Debug, Default)]
pub(crate) struct InterimTextTerminalEngine {
    parser: InterimAnsiParser,
}

impl TerminalEngine for InterimTextTerminalEngine {
    fn apply_output(&mut self, input: TerminalInput<'_>, output: &[u8]) -> Option<TerminalUpdate> {
        let parser_output = self.parser.consume(output, input.modes);
        let (scrollback_lines, cursor_col) = merge_interim_pty_output(
            input.scrollback_lines,
            input.cursor.col > 0,
            &parser_output.text,
        );

        Some(interim_text_update(InterimTextUpdateInput {
            surface: input.surface,
            previous_cursor: input.cursor,
            modes: parser_output.modes,
            title: input.title,
            working_directory: input.working_directory,
            colors: input.colors,
            rows: input.rows,
            scrollback_lines,
            cursor_col,
        }))
    }

    fn resize(
        &mut self,
        input: TerminalInput<'_>,
        _cols: u32,
        rows: u32,
    ) -> Option<TerminalUpdate> {
        Some(interim_text_update(InterimTextUpdateInput {
            surface: input.surface,
            previous_cursor: input.cursor,
            modes: input.modes,
            title: input.title,
            working_directory: input.working_directory,
            colors: input.colors,
            rows,
            scrollback_lines: input.scrollback_lines.to_vec(),
            cursor_col: input.cursor.col,
        }))
    }

    fn encode_key_input(&mut self, input: KeyTerminalInput<'_>) -> Option<Vec<u8>> {
        if input.modifiers != 0 {
            return modified_named_key_bytes(input.key_name, input.modifiers);
        }
        named_key_bytes(
            input.key_name,
            input.application_keypad,
            input.application_cursor,
        )
    }

    fn encode_mouse_input(&mut self, input: MouseTerminalInput) -> Option<Vec<u8>> {
        encode_sgr_mouse_input(input)
    }
}

#[derive(Debug, Default)]
struct InterimAnsiParser {
    state: InterimAnsiState,
}

#[derive(Debug, Default)]
enum InterimAnsiState {
    #[default]
    Ground,
    Escape,
    Csi(Vec<u8>),
    Osc {
        escape_pending: bool,
    },
    StringControl {
        escape_pending: bool,
    },
}

struct InterimAnsiOutput {
    text: Vec<u8>,
    modes: TerminalModes,
}

impl InterimAnsiParser {
    fn consume(&mut self, bytes: &[u8], mut modes: TerminalModes) -> InterimAnsiOutput {
        let mut text = Vec::with_capacity(bytes.len());
        for byte in bytes.iter().copied() {
            match &mut self.state {
                InterimAnsiState::Ground => match byte {
                    0x1b => self.state = InterimAnsiState::Escape,
                    0x9b => self.state = InterimAnsiState::Csi(Vec::new()),
                    _ => text.push(byte),
                },
                InterimAnsiState::Escape => match byte {
                    b'[' => self.state = InterimAnsiState::Csi(Vec::new()),
                    b']' => {
                        self.state = InterimAnsiState::Osc {
                            escape_pending: false,
                        }
                    }
                    b'P' | b'X' | b'^' | b'_' => {
                        self.state = InterimAnsiState::StringControl {
                            escape_pending: false,
                        };
                    }
                    b'=' => {
                        modes.application_keypad = true;
                        self.state = InterimAnsiState::Ground;
                    }
                    b'>' => {
                        modes.application_keypad = false;
                        self.state = InterimAnsiState::Ground;
                    }
                    0x1b => self.state = InterimAnsiState::Escape,
                    0x30..=0x7e => self.state = InterimAnsiState::Ground,
                    _ => self.state = InterimAnsiState::Ground,
                },
                InterimAnsiState::Csi(buffer) => {
                    if byte == 0x1b {
                        self.state = InterimAnsiState::Escape;
                    } else if (0x40..=0x7e).contains(&byte) {
                        let params = std::mem::take(buffer);
                        apply_interim_csi(&mut modes, &params, byte);
                        self.state = InterimAnsiState::Ground;
                    } else {
                        buffer.push(byte);
                    }
                }
                InterimAnsiState::Osc { escape_pending } => {
                    if *escape_pending {
                        if byte == b'\\' {
                            self.state = InterimAnsiState::Ground;
                        } else {
                            *escape_pending = byte == 0x1b;
                        }
                    } else if byte == 0x07 {
                        self.state = InterimAnsiState::Ground;
                    } else if byte == 0x1b {
                        *escape_pending = true;
                    }
                }
                InterimAnsiState::StringControl { escape_pending } => {
                    if *escape_pending {
                        if byte == b'\\' {
                            self.state = InterimAnsiState::Ground;
                        } else {
                            *escape_pending = byte == 0x1b;
                        }
                    } else if byte == 0x1b {
                        *escape_pending = true;
                    }
                }
            }
        }
        InterimAnsiOutput { text, modes }
    }
}

fn apply_interim_csi(modes: &mut TerminalModes, params: &[u8], final_byte: u8) {
    match final_byte {
        b'h' => apply_interim_mode_set(modes, params, true),
        b'l' => apply_interim_mode_set(modes, params, false),
        _ => {}
    }
}

fn apply_interim_mode_set(modes: &mut TerminalModes, params: &[u8], enabled: bool) {
    let private = params.first() == Some(&b'?');
    let params = if private { &params[1..] } else { params };
    for param in params.split(|byte| *byte == b';') {
        let Ok(param) = std::str::from_utf8(param) else {
            continue;
        };
        let Ok(code) = param.parse::<u32>() else {
            continue;
        };
        match (private, code) {
            (true, 1) => modes.application_cursor = enabled,
            (true, 6) => modes.origin = enabled,
            (true, 7) => modes.wraparound = enabled,
            (true, 1000) => {
                set_interim_mouse_tracking(modes, protocol::MouseTrackingMode::Normal, enabled)
            }
            (true, 1002) => {
                set_interim_mouse_tracking(modes, protocol::MouseTrackingMode::Button, enabled)
            }
            (true, 1003) => {
                set_interim_mouse_tracking(modes, protocol::MouseTrackingMode::Any, enabled)
            }
            (true, 1004) => modes.focus_reporting = enabled,
            (true, 1006) => set_interim_mouse_format(modes, protocol::MouseFormat::Sgr, enabled),
            (true, 1016) => {
                set_interim_mouse_format(modes, protocol::MouseFormat::SgrPixels, enabled)
            }
            (true, 2004) => modes.bracketed_paste = enabled,
            _ => {}
        }
    }
}

fn set_interim_mouse_tracking(
    modes: &mut TerminalModes,
    mode: protocol::MouseTrackingMode,
    enabled: bool,
) {
    if enabled {
        modes.mouse_tracking = true;
        modes.mouse_tracking_mode = mode;
    } else if modes.mouse_tracking_mode == mode {
        modes.mouse_tracking = false;
        modes.mouse_tracking_mode = protocol::MouseTrackingMode::None;
    }
}

fn set_interim_mouse_format(
    modes: &mut TerminalModes,
    format: protocol::MouseFormat,
    enabled: bool,
) {
    if enabled {
        modes.mouse_format = format;
    } else if modes.mouse_format == format {
        modes.mouse_format = protocol::MouseFormat::X10;
    }
}

pub fn named_key_bytes(
    key_name: &str,
    application_keypad: bool,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    let bytes: &[u8] = match (key_name, application_keypad, application_cursor) {
        ("numpad-enter", false, _) => b"\r",
        ("numpad-enter", true, _) => b"\x1bOM",
        ("numpad-0", false, _) => b"0",
        ("numpad-0", true, _) => b"\x1bOp",
        ("numpad-1", false, _) => b"1",
        ("numpad-1", true, _) => b"\x1bOq",
        ("numpad-2", false, _) => b"2",
        ("numpad-2", true, _) => b"\x1bOr",
        ("numpad-3", false, _) => b"3",
        ("numpad-3", true, _) => b"\x1bOs",
        ("numpad-4", false, _) => b"4",
        ("numpad-4", true, _) => b"\x1bOt",
        ("numpad-5", false, _) => b"5",
        ("numpad-5", true, _) => b"\x1bOu",
        ("numpad-6", false, _) => b"6",
        ("numpad-6", true, _) => b"\x1bOv",
        ("numpad-7", false, _) => b"7",
        ("numpad-7", true, _) => b"\x1bOw",
        ("numpad-8", false, _) => b"8",
        ("numpad-8", true, _) => b"\x1bOx",
        ("numpad-9", false, _) => b"9",
        ("numpad-9", true, _) => b"\x1bOy",
        ("arrow-up", _, false) => b"\x1b[A",
        ("arrow-up", _, true) => b"\x1bOA",
        ("arrow-down", _, false) => b"\x1b[B",
        ("arrow-down", _, true) => b"\x1bOB",
        ("arrow-right", _, false) => b"\x1b[C",
        ("arrow-right", _, true) => b"\x1bOC",
        ("arrow-left", _, false) => b"\x1b[D",
        ("arrow-left", _, true) => b"\x1bOD",
        ("enter", _, _) => b"\r",
        ("tab", _, _) => b"\t",
        ("space", _, _) => b" ",
        ("backspace", _, _) => b"\x7f",
        ("escape", _, _) => b"\x1b",
        ("insert", _, _) => b"\x1b[2~",
        ("delete", _, _) => b"\x1b[3~",
        ("home", _, _) => b"\x1b[H",
        ("end", _, _) => b"\x1b[F",
        ("page-up", _, _) => b"\x1b[5~",
        ("page-down", _, _) => b"\x1b[6~",
        ("f1", _, _) => b"\x1bOP",
        ("f2", _, _) => b"\x1bOQ",
        ("f3", _, _) => b"\x1bOR",
        ("f4", _, _) => b"\x1bOS",
        ("f5", _, _) => b"\x1b[15~",
        ("f6", _, _) => b"\x1b[17~",
        ("f7", _, _) => b"\x1b[18~",
        ("f8", _, _) => b"\x1b[19~",
        ("f9", _, _) => b"\x1b[20~",
        ("f10", _, _) => b"\x1b[21~",
        ("f11", _, _) => b"\x1b[23~",
        ("f12", _, _) => b"\x1b[24~",
        _ => return None,
    };
    Some(bytes.to_vec())
}

pub fn modified_named_key_bytes(key_name: &str, modifiers: u32) -> Option<Vec<u8>> {
    if modifiers == 0 || modifiers > 0x0f {
        return None;
    }
    let modifier_param = xterm_modifier_param(modifiers)?;
    let sequence = match key_name {
        "arrow-up" => format!("\x1b[1;{modifier_param}A"),
        "arrow-down" => format!("\x1b[1;{modifier_param}B"),
        "arrow-right" => format!("\x1b[1;{modifier_param}C"),
        "arrow-left" => format!("\x1b[1;{modifier_param}D"),
        "home" => format!("\x1b[1;{modifier_param}H"),
        "end" => format!("\x1b[1;{modifier_param}F"),
        "insert" => format!("\x1b[2;{modifier_param}~"),
        "delete" => format!("\x1b[3;{modifier_param}~"),
        "page-up" => format!("\x1b[5;{modifier_param}~"),
        "page-down" => format!("\x1b[6;{modifier_param}~"),
        "f1" => format!("\x1b[1;{modifier_param}P"),
        "f2" => format!("\x1b[1;{modifier_param}Q"),
        "f3" => format!("\x1b[1;{modifier_param}R"),
        "f4" => format!("\x1b[1;{modifier_param}S"),
        "f5" => format!("\x1b[15;{modifier_param}~"),
        "f6" => format!("\x1b[17;{modifier_param}~"),
        "f7" => format!("\x1b[18;{modifier_param}~"),
        "f8" => format!("\x1b[19;{modifier_param}~"),
        "f9" => format!("\x1b[20;{modifier_param}~"),
        "f10" => format!("\x1b[21;{modifier_param}~"),
        "f11" => format!("\x1b[23;{modifier_param}~"),
        "f12" => format!("\x1b[24;{modifier_param}~"),
        "tab" if modifiers == 1 => "\x1b[Z".to_owned(),
        _ => return None,
    };
    Some(sequence.into_bytes())
}

fn xterm_modifier_param(modifiers: u32) -> Option<u32> {
    let mut param = 1;
    if modifiers & 1 != 0 {
        param += 1;
    }
    if modifiers & 4 != 0 {
        param += 2;
    }
    if modifiers & 2 != 0 {
        param += 4;
    }
    if modifiers & 8 != 0 {
        param += 8;
    }
    Some(param)
}

pub fn encode_sgr_mouse_input(input: MouseTerminalInput) -> Option<Vec<u8>> {
    match input.mouse_format {
        protocol::MouseFormat::Sgr | protocol::MouseFormat::SgrPixels => {}
        _ => return None,
    }
    if input.row >= input.rows || input.col >= input.cols {
        return None;
    }
    let mut code = match input.button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    if input.action == MouseAction::Motion {
        code += 32;
    }
    if input.modifiers & 1 != 0 {
        code += 4;
    }
    if input.modifiers & 4 != 0 {
        code += 8;
    }
    if input.modifiers & 2 != 0 {
        code += 16;
    }
    let (x, y) = if input.mouse_format == protocol::MouseFormat::SgrPixels {
        (input.pixel_x?, input.pixel_y?)
    } else {
        (input.col.checked_add(1)?, input.row.checked_add(1)?)
    };
    let final_byte = if input.action == MouseAction::Release {
        'm'
    } else {
        'M'
    };
    Some(format!("\x1b[<{code};{x};{y}{final_byte}").into_bytes())
}

struct InterimTextUpdateInput<'a> {
    surface: protocol::SurfaceKind,
    previous_cursor: TerminalCursor,
    modes: TerminalModes,
    title: &'a str,
    working_directory: &'a str,
    colors: TerminalColors,
    rows: u32,
    scrollback_lines: Vec<String>,
    cursor_col: u32,
}

fn interim_text_update(input: InterimTextUpdateInput<'_>) -> TerminalUpdate {
    let visible_start = input
        .scrollback_lines
        .len()
        .saturating_sub(input.rows as usize);
    let surface_lines = input.scrollback_lines[visible_start..].to_vec();
    let cursor = TerminalCursor {
        row: surface_lines.len().saturating_sub(1) as u32,
        col: input.cursor_col,
        visible: input.previous_cursor.visible,
        shape: input.previous_cursor.shape,
        blinking: input.previous_cursor.blinking,
    };

    let mut update = TerminalUpdate::plain(
        protocol::PatchKind::ReplaceRows,
        input.surface,
        cursor,
        surface_lines,
        input.scrollback_lines,
    );
    update.modes = input.modes;
    update.title = input.title.to_owned();
    update.working_directory = input.working_directory.to_owned();
    update.colors = input.colors;
    update
}

pub(crate) fn plain_row_runs(lines: &[String]) -> Vec<Vec<CellRun>> {
    lines
        .iter()
        .map(|line| vec![CellRun::plain(line.clone())])
        .collect()
}

pub(crate) fn plain_row_semantic_prompts(lines: &[String]) -> Vec<protocol::RowSemanticPrompt> {
    vec![protocol::RowSemanticPrompt::None; lines.len()]
}

pub(crate) fn plain_row_dirty_flags(lines: &[String]) -> Vec<bool> {
    vec![false; lines.len()]
}

pub(crate) fn plain_row_kitty_placeholders(lines: &[String]) -> Vec<bool> {
    vec![false; lines.len()]
}

pub(crate) fn cell_runs_text(runs: &[CellRun]) -> String {
    let mut text = String::new();
    for run in runs {
        text.push_str(&run.text);
    }
    text
}

fn merge_interim_pty_output(
    previous_lines: &[String],
    append_to_previous_line: bool,
    output: &[u8],
) -> (Vec<String>, u32) {
    let mut lines = previous_lines.to_vec();
    let text = String::from_utf8_lossy(output);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut current = String::new();
    let mut appending = append_to_previous_line && !lines.is_empty();
    let mut cursor_col = if appending {
        line_width(lines.last().expect("line exists"))
    } else {
        0
    };
    let mut saw_output = false;
    let mut ended_with_newline = false;

    for ch in text.chars() {
        saw_output = true;
        if ch == '\n' {
            if appending {
                appending = false;
            } else {
                lines.push(std::mem::take(&mut current));
            }
            current.clear();
            cursor_col = 0;
            ended_with_newline = true;
            continue;
        }

        ended_with_newline = false;
        if ch != '\t' && ch.is_control() {
            continue;
        }

        if appending {
            let line = lines.last_mut().expect("line exists");
            line.push(ch);
            cursor_col = line_width(line);
        } else {
            current.push(ch);
            cursor_col = line_width(&current);
        }
    }

    if !appending && (!current.is_empty() || (!saw_output && lines.is_empty())) {
        lines.push(current);
    }
    if ended_with_newline {
        cursor_col = 0;
    }

    (lines, cursor_col)
}

fn line_width(line: &str) -> u32 {
    line.chars().count().try_into().unwrap_or(u32::MAX)
}

#[cfg(feature = "libghostty-vt")]
mod ghostty_vt {
    use std::{cell::RefCell, rc::Rc};

    use libghostty_vt::{
        RenderState, Terminal, TerminalOptions, key, mouse,
        render::{CellIterator, CursorVisualStyle, RowIterator, Snapshot as RenderSnapshot},
        screen::{CellSemanticContent, CellWide},
        style::{RgbColor, Style, StyleColor, Underline},
        terminal::{Mode, ScrollViewport},
    };
    use nmux_proto::protocol;

    use super::{
        CellRun, KeyTerminalInput, MouseAction, MouseButton, MouseTerminalInput, PaneStyle,
        TerminalColors, TerminalCursor, TerminalEngine, TerminalInput, TerminalModes,
        TerminalUpdate,
    };

    pub struct LibghosttyVtTerminalEngine {
        state: Option<GhosttyVtState>,
    }

    const OSC_BUFFER_LIMIT: usize = 4096;

    struct GhosttyVtState {
        terminal: Box<Terminal<'static, 'static>>,
        render_state: RenderState<'static>,
        row_iterator: RowIterator<'static>,
        cell_iterator: CellIterator<'static>,
        osc7: Osc7Tracker,
        pending_decrqm: Vec<u8>,
        pty_writes: Rc<RefCell<Vec<Vec<u8>>>>,
    }

    #[derive(Default)]
    struct Osc7Tracker {
        buffer: Vec<u8>,
        active: bool,
        escape_pending: bool,
        working_directory: Option<String>,
    }

    impl Osc7Tracker {
        fn ingest(&mut self, bytes: &[u8]) {
            for byte in bytes.iter().copied() {
                if self.active {
                    self.ingest_osc_byte(byte);
                } else if self.escape_pending {
                    if byte == b']' {
                        self.active = true;
                        self.escape_pending = false;
                        self.buffer.clear();
                    } else {
                        self.escape_pending = byte == 0x1b;
                    }
                } else if byte == 0x1b {
                    self.escape_pending = true;
                }
            }
        }

        fn working_directory(&self) -> Option<&str> {
            self.working_directory.as_deref()
        }

        fn ingest_osc_byte(&mut self, byte: u8) {
            if self.escape_pending {
                if byte == b'\\' {
                    self.complete();
                } else {
                    self.push_osc_byte(0x1b);
                    self.escape_pending = false;
                    self.ingest_osc_byte(byte);
                }
            } else if byte == 0x07 {
                self.complete();
            } else if byte == 0x1b {
                self.escape_pending = true;
            } else {
                self.push_osc_byte(byte);
            }
        }

        fn push_osc_byte(&mut self, byte: u8) {
            self.buffer.push(byte);
            if self.buffer.len() > OSC_BUFFER_LIMIT {
                self.reset_sequence();
            }
        }

        fn complete(&mut self) {
            if let Some(payload) = self.buffer.strip_prefix(b"7;")
                && let Ok(working_directory) = std::str::from_utf8(payload)
            {
                self.working_directory = Some(working_directory.to_owned());
            }
            self.reset_sequence();
        }

        fn reset_sequence(&mut self) {
            self.buffer.clear();
            self.active = false;
            self.escape_pending = false;
        }
    }

    impl LibghosttyVtTerminalEngine {
        pub fn new() -> Self {
            Self { state: None }
        }

        fn state_mut(&mut self, input: &TerminalInput<'_>) -> Option<&mut GhosttyVtState> {
            if self.state.is_none() {
                self.state = Some(GhosttyVtState::new(input.cols, input.rows)?);
            }
            self.state.as_mut()
        }
    }

    fn vt_output_may_change_rows(output: &[u8]) -> bool {
        enum State {
            Ground,
            Escape,
            Csi,
            Osc,
            OscEscape,
        }

        let mut state = State::Ground;
        for byte in output.iter().copied() {
            match state {
                State::Ground => match byte {
                    0x1b => state = State::Escape,
                    b'\n' | b'\r' | b'\t' | 0x08 => return true,
                    0x20..=0x7e | 0x80..=0xff => return true,
                    _ => {}
                },
                State::Escape => match byte {
                    b'[' => state = State::Csi,
                    b']' => state = State::Osc,
                    _ => return true,
                },
                State::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        match byte {
                            b'm' | b'h' | b'l' | b'A' | b'B' | b'C' | b'D' | b'E' | b'F' | b'G'
                            | b'H' | b'f' | b'd' => state = State::Ground,
                            _ => return true,
                        }
                    }
                }
                State::Osc => match byte {
                    0x07 => state = State::Ground,
                    0x1b => state = State::OscEscape,
                    _ => {}
                },
                State::OscEscape => {
                    state = if byte == b'\\' {
                        State::Ground
                    } else {
                        State::Osc
                    };
                }
            }
        }
        false
    }

    impl TerminalEngine for LibghosttyVtTerminalEngine {
        fn apply_output(
            &mut self,
            input: TerminalInput<'_>,
            output: &[u8],
        ) -> Option<TerminalUpdate> {
            let state = self.state_mut(&input)?;
            state.osc7.ingest(output);
            let pty_write_count = state.pty_writes.borrow().len();
            let saw_wraparound_query = state.ingest_decrqm_query(output);
            let vt_write_span = tracing::trace_span!("terminal.libghostty.vt_write");
            vt_write_span.in_scope(|| state.terminal.vt_write(output));
            if saw_wraparound_query && state.pty_writes.borrow().len() == pty_write_count {
                state.pty_writes.borrow_mut().push(b"\x1b[?7;1$y".to_vec());
            }
            state.extract_update(input, false, !vt_output_may_change_rows(output))
        }

        fn resize(
            &mut self,
            input: TerminalInput<'_>,
            cols: u32,
            rows: u32,
        ) -> Option<TerminalUpdate> {
            let state = self.state_mut(&input)?;
            let cols = u16::try_from(cols).ok()?;
            let rows = u16::try_from(rows).ok()?;
            state.terminal.resize(cols, rows, 8, 16).ok()?;
            state.extract_update(input, true, false)
        }

        fn encode_mouse_input(&mut self, input: MouseTerminalInput) -> Option<Vec<u8>> {
            let state = self.state.as_ref()?;
            encode_mouse_input(&state.terminal, input)
        }

        fn encode_key_input(&mut self, input: KeyTerminalInput<'_>) -> Option<Vec<u8>> {
            let state = self.state.as_ref()?;
            encode_key_input(&state.terminal, input)
        }

        fn drain_pty_writes(&mut self) -> Vec<Vec<u8>> {
            let Some(state) = self.state.as_mut() else {
                return Vec::new();
            };
            state.drain_pty_writes()
        }
    }

    impl GhosttyVtState {
        fn new(cols: u32, rows: u32) -> Option<Self> {
            let pty_writes = Rc::new(RefCell::new(Vec::new()));
            let mut terminal = Box::new(
                Terminal::new(TerminalOptions {
                    cols: u16::try_from(cols).ok()?,
                    rows: u16::try_from(rows).ok()?,
                    max_scrollback: 10000,
                })
                .ok()?,
            );
            // libghostty-vt callback registration stores userdata pointing
            // into the Terminal value, so register callbacks only after the
            // terminal is in a stable heap allocation.
            terminal
                .on_pty_write({
                    let pty_writes = Rc::clone(&pty_writes);
                    move |_terminal, bytes| {
                        pty_writes.borrow_mut().push(bytes.to_vec());
                    }
                })
                .ok()?;
            Some(Self {
                terminal,
                render_state: RenderState::new().ok()?,
                row_iterator: RowIterator::new().ok()?,
                cell_iterator: CellIterator::new().ok()?,
                osc7: Osc7Tracker::default(),
                pending_decrqm: Vec::new(),
                pty_writes,
            })
        }

        fn drain_pty_writes(&mut self) -> Vec<Vec<u8>> {
            self.pty_writes.borrow_mut().drain(..).collect()
        }

        fn ingest_decrqm_query(&mut self, output: &[u8]) -> bool {
            const QUERY: &[u8] = b"\x1b[?7$p";
            self.pending_decrqm.extend_from_slice(output);
            if self
                .pending_decrqm
                .windows(QUERY.len())
                .any(|window| window == QUERY)
            {
                self.pending_decrqm.clear();
                return true;
            }
            let keep = self.pending_decrqm.len().min(QUERY.len().saturating_sub(1));
            if keep < self.pending_decrqm.len() {
                self.pending_decrqm =
                    self.pending_decrqm[self.pending_decrqm.len() - keep..].to_vec();
            }
            false
        }

        fn extract_update(
            &mut self,
            input: TerminalInput<'_>,
            force_rows: bool,
            preserve_input_rows: bool,
        ) -> Option<TerminalUpdate> {
            let surface = surface_kind(&self.terminal)?;
            let mut styles = if surface == protocol::SurfaceKind::Main || input.styles.is_empty() {
                vec![PaneStyle::default()]
            } else {
                input.styles.to_vec()
            };
            let total_main_rows = if surface == protocol::SurfaceKind::Main {
                Some(self.terminal.total_rows().ok()?)
            } else {
                None
            };
            self.terminal.scroll_viewport(ScrollViewport::Bottom);
            let surface_span = tracing::trace_span!("terminal.libghostty.extract_surface_rows");
            let surface_guard = surface_span.enter();
            let snapshot = self.render_state.update(&self.terminal).ok()?;
            let mut surface_rows = extract_rows(
                &snapshot,
                &mut self.row_iterator,
                &mut self.cell_iterator,
                &mut styles,
            )?;
            drop(surface_guard);
            if preserve_input_rows && surface == input.surface {
                surface_rows = ExtractedRows {
                    lines: input.surface_lines.to_vec(),
                    row_runs: input.surface_row_runs.to_vec(),
                    semantic_prompts: input.surface_semantic_prompts.to_vec(),
                    dirty_rows: input.surface_dirty_rows.to_vec(),
                    kitty_placeholders: input.surface_kitty_placeholders.to_vec(),
                };
            }
            let surface_lines = surface_rows.lines.clone();
            let surface_row_runs = surface_rows.row_runs.clone();
            let surface_semantic_prompts = surface_rows.semantic_prompts.clone();
            let surface_dirty_rows = surface_rows.dirty_rows.clone();
            let surface_kitty_placeholders = surface_rows.kitty_placeholders.clone();
            let cursor = cursor(&snapshot, input.cursor)?;
            let colors = terminal_colors(&snapshot)?;
            let mut preserve_scrollback = false;
            let scrollback_rows = if preserve_input_rows && surface == input.surface {
                ExtractedRows {
                    lines: input.scrollback_lines.to_vec(),
                    row_runs: input.scrollback_row_runs.to_vec(),
                    semantic_prompts: input.scrollback_semantic_prompts.to_vec(),
                    dirty_rows: input.scrollback_dirty_rows.to_vec(),
                    kitty_placeholders: input.scrollback_kitty_placeholders.to_vec(),
                }
            } else if let Some(total_rows) = total_main_rows {
                if total_rows <= surface_rows.lines.len() {
                    surface_rows.truncated(total_rows)
                } else if let Some(scrollback_rows) =
                    cached_main_scrollback_rows(&input, total_rows, &surface_rows)
                {
                    let cached_span = tracing::trace_span!(
                        "terminal.libghostty.cached_scrollback_rows",
                        total_rows,
                        surface_rows = surface_rows.lines.len(),
                        input_scrollback_rows = input.scrollback_lines.len()
                    );
                    cached_span.in_scope(|| scrollback_rows)
                } else if input.scrollback_lines.is_empty() {
                    let full_span = tracing::trace_span!(
                        "terminal.libghostty.extract_full_scrollback_rows",
                        total_rows,
                        surface_rows = surface_rows.lines.len(),
                        input_scrollback_rows = input.scrollback_lines.len()
                    );
                    full_span.in_scope(|| self.scrollback_rows(total_rows, &mut styles))?
                } else {
                    let full_span = tracing::trace_span!(
                        "terminal.libghostty.defer_full_scrollback_rows",
                        total_rows,
                        surface_rows = surface_rows.lines.len(),
                        input_scrollback_rows = input.scrollback_lines.len()
                    );
                    full_span.in_scope(|| {
                        preserve_scrollback = true;
                        ExtractedRows {
                            lines: input.scrollback_lines.to_vec(),
                            row_runs: row_runs_prefix_or_plain(
                                input.scrollback_lines,
                                input.scrollback_row_runs,
                                input.scrollback_lines.len(),
                            ),
                            semantic_prompts: row_values_prefix_or_default(
                                input.scrollback_semantic_prompts,
                                input.scrollback_lines.len(),
                                protocol::RowSemanticPrompt::None,
                            ),
                            dirty_rows: row_values_prefix_or_default(
                                input.scrollback_dirty_rows,
                                input.scrollback_lines.len(),
                                false,
                            ),
                            kitty_placeholders: row_values_prefix_or_default(
                                input.scrollback_kitty_placeholders,
                                input.scrollback_lines.len(),
                                false,
                            ),
                        }
                    })
                }
            } else {
                ExtractedRows {
                    lines: input.scrollback_lines.to_vec(),
                    row_runs: if input.scrollback_row_runs.len() == input.scrollback_lines.len() {
                        input.scrollback_row_runs.to_vec()
                    } else {
                        super::plain_row_runs(input.scrollback_lines)
                    },
                    semantic_prompts: input.scrollback_semantic_prompts.to_vec(),
                    dirty_rows: input.scrollback_dirty_rows.to_vec(),
                    kitty_placeholders: input.scrollback_kitty_placeholders.to_vec(),
                }
            };
            let modes = modes(&self.terminal)?;
            let title = self.terminal.title().ok()?;
            let terminal_working_directory = self.terminal.pwd().ok()?.to_owned();
            let working_directory = self
                .osc7
                .working_directory()
                .unwrap_or(&terminal_working_directory);
            let patch_kind = if !force_rows
                && surface == input.surface
                && surface_lines == input.surface_lines
                && surface_row_runs == input.surface_row_runs
                && surface_semantic_prompts == input.surface_semantic_prompts
                && surface_dirty_rows == input.surface_dirty_rows
                && surface_kitty_placeholders == input.surface_kitty_placeholders
                && colors != input.colors
            {
                protocol::PatchKind::ColorOnly
            } else if !force_rows
                && surface == input.surface
                && surface_lines == input.surface_lines
                && surface_row_runs == input.surface_row_runs
                && surface_semantic_prompts == input.surface_semantic_prompts
                && surface_dirty_rows == input.surface_dirty_rows
                && surface_kitty_placeholders == input.surface_kitty_placeholders
                && colors == input.colors
                && modes != input.modes
            {
                protocol::PatchKind::ModeOnly
            } else if !force_rows
                && surface == input.surface
                && surface_lines == input.surface_lines
                && surface_row_runs == input.surface_row_runs
                && surface_semantic_prompts == input.surface_semantic_prompts
                && surface_dirty_rows == input.surface_dirty_rows
                && surface_kitty_placeholders == input.surface_kitty_placeholders
                && colors == input.colors
                && (cursor != input.cursor
                    || title != input.title
                    || working_directory != input.working_directory)
            {
                protocol::PatchKind::CursorOnly
            } else {
                protocol::PatchKind::ReplaceRows
            };

            Some(TerminalUpdate {
                patch_kind,
                surface,
                preserve_scrollback,
                cursor,
                modes,
                title: title.to_owned(),
                working_directory: working_directory.to_owned(),
                colors,
                styles,
                surface_row_runs,
                scrollback_row_runs: scrollback_rows.row_runs,
                surface_semantic_prompts,
                scrollback_semantic_prompts: scrollback_rows.semantic_prompts,
                surface_dirty_rows,
                scrollback_dirty_rows: scrollback_rows.dirty_rows,
                surface_kitty_placeholders,
                scrollback_kitty_placeholders: scrollback_rows.kitty_placeholders,
                surface_lines,
                scrollback_lines: scrollback_rows.lines,
            })
        }

        fn scrollback_rows(
            &mut self,
            total_rows: usize,
            styles: &mut Vec<PaneStyle>,
        ) -> Option<ExtractedRows> {
            if total_rows == 0 {
                return Some(ExtractedRows {
                    lines: Vec::new(),
                    row_runs: Vec::new(),
                    semantic_prompts: Vec::new(),
                    dirty_rows: Vec::new(),
                    kitty_placeholders: Vec::new(),
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
            rows.truncate(total_rows);

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
                let Some(next_semantic_prompt) = viewport_lines.semantic_prompts.last() else {
                    break;
                };
                let Some(next_dirty) = viewport_lines.dirty_rows.last() else {
                    break;
                };
                let Some(next_kitty_placeholder) = viewport_lines.kitty_placeholders.last() else {
                    break;
                };
                rows.lines.push(next_line.clone());
                rows.row_runs.push(next_runs.clone());
                rows.semantic_prompts.push(*next_semantic_prompt);
                rows.dirty_rows.push(*next_dirty);
                rows.kitty_placeholders.push(*next_kitty_placeholder);
            }

            Some(rows)
        }
    }

    #[derive(Clone)]
    struct ExtractedRows {
        lines: Vec<String>,
        row_runs: Vec<Vec<CellRun>>,
        semantic_prompts: Vec<protocol::RowSemanticPrompt>,
        dirty_rows: Vec<bool>,
        kitty_placeholders: Vec<bool>,
    }

    impl ExtractedRows {
        fn truncate(&mut self, len: usize) {
            self.lines.truncate(len);
            self.row_runs.truncate(len);
            self.semantic_prompts.truncate(len);
            self.dirty_rows.truncate(len);
            self.kitty_placeholders.truncate(len);
        }

        fn truncated(mut self, len: usize) -> Self {
            self.truncate(len);
            self
        }
    }

    fn cached_main_scrollback_rows(
        input: &TerminalInput<'_>,
        total_rows: usize,
        surface_rows: &ExtractedRows,
    ) -> Option<ExtractedRows> {
        let history_len = total_rows.checked_sub(surface_rows.lines.len())?;
        if history_len > input.scrollback_lines.len() {
            return None;
        }

        let history_lines = input.scrollback_lines[..history_len].to_vec();
        let mut rows = ExtractedRows {
            row_runs: row_runs_prefix_or_plain(
                &history_lines,
                input.scrollback_row_runs,
                history_len,
            ),
            semantic_prompts: row_values_prefix_or_default(
                input.scrollback_semantic_prompts,
                history_len,
                protocol::RowSemanticPrompt::None,
            ),
            dirty_rows: row_values_prefix_or_default(
                input.scrollback_dirty_rows,
                history_len,
                false,
            ),
            kitty_placeholders: row_values_prefix_or_default(
                input.scrollback_kitty_placeholders,
                history_len,
                false,
            ),
            lines: history_lines,
        };
        rows.lines.extend(surface_rows.lines.iter().cloned());
        rows.row_runs.extend(surface_rows.row_runs.iter().cloned());
        rows.semantic_prompts
            .extend(surface_rows.semantic_prompts.iter().copied());
        rows.dirty_rows
            .extend(surface_rows.dirty_rows.iter().copied());
        rows.kitty_placeholders
            .extend(surface_rows.kitty_placeholders.iter().copied());
        Some(rows)
    }

    fn row_runs_prefix_or_plain(
        lines: &[String],
        runs: &[Vec<CellRun>],
        len: usize,
    ) -> Vec<Vec<CellRun>> {
        if runs.len() >= len {
            runs[..len].to_vec()
        } else {
            super::plain_row_runs(lines)
        }
    }

    fn row_values_prefix_or_default<T: Copy>(values: &[T], len: usize, default: T) -> Vec<T> {
        if values.len() >= len {
            values[..len].to_vec()
        } else {
            vec![default; len]
        }
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
        let mut semantic_prompts = Vec::new();
        let mut dirty_rows = Vec::new();
        let mut kitty_placeholders = Vec::new();
        while let Some(row) = rows.next() {
            let raw_row = row.raw_row().ok()?;
            let semantic_prompt = row_semantic_prompt(raw_row.semantic_prompt().ok()?);
            let kitty_placeholder = raw_row.has_kitty_virtual_placeholder().ok()?;
            let dirty = row.dirty().ok()?;
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
                let semantic_content = cell_semantic_content(raw_cell.semantic_content().ok()?);
                let flags = cell_run_flags(raw_cell)?;
                if let Some(last) = runs.last_mut()
                    && last.style_id == style_id
                    && last.flags == flags
                    && last.hyperlink_id == 0
                    && last.semantic_content == semantic_content
                {
                    last.text.push_str(&text);
                    last.cell_widths.push(width);
                } else {
                    runs.push(CellRun {
                        text,
                        cell_widths: vec![width],
                        style_id,
                        flags,
                        hyperlink_id: 0,
                        semantic_content,
                    });
                }
            }
            trim_trailing_spaces(&mut runs);
            lines.push(super::cell_runs_text(&runs));
            row_runs.push(runs);
            semantic_prompts.push(semantic_prompt);
            dirty_rows.push(dirty);
            kitty_placeholders.push(kitty_placeholder);
        }
        Some(ExtractedRows {
            lines,
            row_runs,
            semantic_prompts,
            dirty_rows,
            kitty_placeholders,
        })
    }

    fn cell_run_flags(cell: libghostty_vt::screen::Cell) -> Option<u32> {
        let mut flags = 0;
        if cell.has_hyperlink().ok()? {
            flags |= super::CELL_RUN_FLAG_HYPERLINK_PRESENT;
        }
        Some(flags)
    }

    fn row_semantic_prompt(
        prompt: libghostty_vt::screen::RowSemanticPrompt,
    ) -> protocol::RowSemanticPrompt {
        match prompt {
            libghostty_vt::screen::RowSemanticPrompt::None => protocol::RowSemanticPrompt::None,
            libghostty_vt::screen::RowSemanticPrompt::Prompt => protocol::RowSemanticPrompt::Prompt,
            libghostty_vt::screen::RowSemanticPrompt::Continuation => {
                protocol::RowSemanticPrompt::Continuation
            }
        }
    }

    fn cell_semantic_content(content: CellSemanticContent) -> protocol::CellSemanticContent {
        match content {
            CellSemanticContent::Output => protocol::CellSemanticContent::Output,
            CellSemanticContent::Input => protocol::CellSemanticContent::Input,
            CellSemanticContent::Prompt => protocol::CellSemanticContent::Prompt,
        }
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
            if !trimmable_trailing_run(last) {
                break;
            }
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

    fn trimmable_trailing_run(run: &CellRun) -> bool {
        run.style_id == 0
            && run.flags == 0
            && run.hyperlink_id == 0
            && run.semantic_content == protocol::CellSemanticContent::Output
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
                blinking: snapshot.cursor_blinking().ok()?,
                ..previous
            });
        };

        Some(TerminalCursor {
            row: u32::from(viewport.y),
            col: u32::from(viewport.x),
            visible: snapshot.cursor_visible().ok()?,
            shape,
            blinking: snapshot.cursor_blinking().ok()?,
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

    fn modes(terminal: &Terminal<'_, '_>) -> Option<TerminalModes> {
        Some(TerminalModes {
            bracketed_paste: terminal.mode(Mode::BRACKETED_PASTE).ok()?,
            mouse_tracking: terminal.is_mouse_tracking().ok()?,
            focus_reporting: terminal.mode(Mode::FOCUS_EVENT).ok()?,
            application_keypad: terminal.mode(Mode::KEYPAD_KEYS).ok()?,
            application_cursor: terminal.mode(Mode::DECCKM).ok()?,
            origin: terminal.mode(Mode::ORIGIN).ok()?,
            wraparound: terminal.mode(Mode::WRAPAROUND).ok()?,
            mouse_tracking_mode: mouse_tracking_mode(terminal)?,
            mouse_format: mouse_format(terminal)?,
        })
    }

    fn mouse_tracking_mode(terminal: &Terminal<'_, '_>) -> Option<protocol::MouseTrackingMode> {
        if terminal.mode(Mode::ANY_MOUSE).ok()? {
            Some(protocol::MouseTrackingMode::Any)
        } else if terminal.mode(Mode::BUTTON_MOUSE).ok()? {
            Some(protocol::MouseTrackingMode::Button)
        } else if terminal.mode(Mode::NORMAL_MOUSE).ok()? {
            Some(protocol::MouseTrackingMode::Normal)
        } else if terminal.mode(Mode::X10_MOUSE).ok()? {
            Some(protocol::MouseTrackingMode::X10)
        } else {
            Some(protocol::MouseTrackingMode::None)
        }
    }

    fn mouse_format(terminal: &Terminal<'_, '_>) -> Option<protocol::MouseFormat> {
        if terminal.mode(Mode::SGR_PIXELS_MOUSE).ok()? {
            Some(protocol::MouseFormat::SgrPixels)
        } else if terminal.mode(Mode::URXVT_MOUSE).ok()? {
            Some(protocol::MouseFormat::Urxvt)
        } else if terminal.mode(Mode::SGR_MOUSE).ok()? {
            Some(protocol::MouseFormat::Sgr)
        } else if terminal.mode(Mode::UTF8_MOUSE).ok()? {
            Some(protocol::MouseFormat::Utf8)
        } else {
            Some(protocol::MouseFormat::X10)
        }
    }

    fn terminal_colors(snapshot: &RenderSnapshot<'_, '_>) -> Option<TerminalColors> {
        let colors = snapshot.colors().ok()?;
        Some(TerminalColors {
            default_fg_rgba: rgba(colors.foreground),
            default_bg_rgba: rgba(colors.background),
            cursor_rgba: colors.cursor.map_or(0, rgba),
            cursor_rgba_set: colors.cursor.is_some(),
            palette_rgba: colors.palette.iter().copied().map(rgba).collect(),
        })
    }

    fn encode_mouse_input(
        terminal: &Terminal<'_, '_>,
        input: MouseTerminalInput,
    ) -> Option<Vec<u8>> {
        let mut encoder = mouse::Encoder::new().ok()?;
        let fallback_width = input.cols.checked_mul(8)?;
        let fallback_height = input.rows.checked_mul(16)?;
        let screen_width = match input.pixel_x {
            Some(pixel_x) => fallback_width.max(pixel_x.checked_add(1)?),
            None => fallback_width,
        };
        let screen_height = match input.pixel_y {
            Some(pixel_y) => fallback_height.max(pixel_y.checked_add(1)?),
            None => fallback_height,
        };
        encoder
            .set_options_from_terminal(terminal)
            .set_size(mouse::EncoderSize {
                screen_width,
                screen_height,
                cell_width: 8,
                cell_height: 16,
                padding_top: 0,
                padding_bottom: 0,
                padding_right: 0,
                padding_left: 0,
            });
        let mut event = mouse::Event::new().ok()?;
        event
            .set_action(mouse_action(input.action))
            .set_button(mouse_button(input.button))
            .set_mods(libghostty_vt::key::Mods::from_bits_retain(
                u16::try_from(input.modifiers).ok()?,
            ))
            .set_position(mouse::Position {
                x: input.pixel_x.unwrap_or(input.col.checked_mul(8)?) as f32,
                y: input.pixel_y.unwrap_or(input.row.checked_mul(16)?) as f32,
            });
        let mut bytes = Vec::new();
        encoder.encode_to_vec(&event, &mut bytes).ok()?;
        Some(bytes)
    }

    fn encode_key_input(
        terminal: &Terminal<'_, '_>,
        input: KeyTerminalInput<'_>,
    ) -> Option<Vec<u8>> {
        if input.modifiers == 0 && input.application_keypad && is_keypad_name(input.key_name) {
            return super::named_key_bytes(
                input.key_name,
                input.application_keypad,
                input.application_cursor,
            );
        }
        let mut encoder = key::Encoder::new().ok()?;
        encoder.set_options_from_terminal(terminal);
        let mut event = key::Event::new().ok()?;
        event
            .set_action(key::Action::Press)
            .set_key(key_from_name(input.key_name)?)
            .set_mods(key::Mods::from_bits_retain(
                u16::try_from(input.modifiers).ok()?,
            ));
        let mut bytes = Vec::new();
        encoder.encode_to_vec(&event, &mut bytes).ok()?;
        if bytes.is_empty() && input.modifiers == 0 {
            return super::named_key_bytes(
                input.key_name,
                input.application_keypad,
                input.application_cursor,
            );
        }
        Some(bytes)
    }

    fn is_keypad_name(key_name: &str) -> bool {
        matches!(
            key_name,
            "numpad-enter"
                | "numpad-0"
                | "numpad-1"
                | "numpad-2"
                | "numpad-3"
                | "numpad-4"
                | "numpad-5"
                | "numpad-6"
                | "numpad-7"
                | "numpad-8"
                | "numpad-9"
        )
    }

    fn key_from_name(key_name: &str) -> Option<key::Key> {
        Some(match key_name {
            "numpad-enter" => key::Key::NumpadEnter,
            "numpad-0" => key::Key::Numpad0,
            "numpad-1" => key::Key::Numpad1,
            "numpad-2" => key::Key::Numpad2,
            "numpad-3" => key::Key::Numpad3,
            "numpad-4" => key::Key::Numpad4,
            "numpad-5" => key::Key::Numpad5,
            "numpad-6" => key::Key::Numpad6,
            "numpad-7" => key::Key::Numpad7,
            "numpad-8" => key::Key::Numpad8,
            "numpad-9" => key::Key::Numpad9,
            "arrow-up" => key::Key::ArrowUp,
            "arrow-down" => key::Key::ArrowDown,
            "arrow-right" => key::Key::ArrowRight,
            "arrow-left" => key::Key::ArrowLeft,
            "enter" => key::Key::Enter,
            "tab" => key::Key::Tab,
            "space" => key::Key::Space,
            "backspace" => key::Key::Backspace,
            "escape" => key::Key::Escape,
            "insert" => key::Key::Insert,
            "delete" => key::Key::Delete,
            "home" => key::Key::Home,
            "end" => key::Key::End,
            "page-up" => key::Key::PageUp,
            "page-down" => key::Key::PageDown,
            "f1" => key::Key::F1,
            "f2" => key::Key::F2,
            "f3" => key::Key::F3,
            "f4" => key::Key::F4,
            "f5" => key::Key::F5,
            "f6" => key::Key::F6,
            "f7" => key::Key::F7,
            "f8" => key::Key::F8,
            "f9" => key::Key::F9,
            "f10" => key::Key::F10,
            "f11" => key::Key::F11,
            "f12" => key::Key::F12,
            _ => return None,
        })
    }

    fn mouse_action(action: MouseAction) -> mouse::Action {
        match action {
            MouseAction::Press => mouse::Action::Press,
            MouseAction::Release => mouse::Action::Release,
            MouseAction::Motion => mouse::Action::Motion,
        }
    }

    fn mouse_button(button: MouseButton) -> Option<mouse::Button> {
        match button {
            MouseButton::None => None,
            MouseButton::Left => Some(mouse::Button::Left),
            MouseButton::Middle => Some(mouse::Button::Middle),
            MouseButton::Right => Some(mouse::Button::Right),
            MouseButton::WheelUp => Some(mouse::Button::Four),
            MouseButton::WheelDown => Some(mouse::Button::Five),
        }
    }
}

#[cfg(test)]
mod tests {
    use nmux_proto::protocol;
    #[cfg(feature = "libghostty-vt")]
    use serde_json::{Value, json};
    #[cfg(feature = "libghostty-vt")]
    use std::fs;
    #[cfg(feature = "libghostty-vt")]
    use std::hash::{Hash, Hasher};
    #[cfg(feature = "libghostty-vt")]
    use std::path::{Path, PathBuf};

    use super::{
        InterimTextTerminalEngine, MouseAction, MouseButton, MouseTerminalInput,
        PaneTerminalEngines, TerminalColors, TerminalCursor, TerminalEngine, TerminalEngineKind,
        TerminalInput, TerminalModes,
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
                blinking: true,
            },
            modes: TerminalModes::default(),
            title: "",
            working_directory: "",
            colors: TerminalColors::default(),
            styles: &[],
            surface_lines,
            surface_row_runs: &[],
            surface_semantic_prompts: &[],
            surface_dirty_rows: &[],
            surface_kitty_placeholders: &[],
            scrollback_lines,
            scrollback_row_runs: &[],
            scrollback_semantic_prompts: &[],
            scrollback_dirty_rows: &[],
            scrollback_kitty_placeholders: &[],
        }
    }

    fn terminal_input_from_update<'a>(update: &'a super::TerminalUpdate) -> TerminalInput<'a> {
        TerminalInput {
            pane_id: "pane-1",
            cols: 80,
            rows: update.surface_lines.len() as u32,
            surface: update.surface,
            cursor: update.cursor,
            modes: update.modes,
            title: &update.title,
            working_directory: &update.working_directory,
            colors: update.colors.clone(),
            styles: &update.styles,
            surface_lines: &update.surface_lines,
            surface_row_runs: &update.surface_row_runs,
            surface_semantic_prompts: &update.surface_semantic_prompts,
            surface_dirty_rows: &update.surface_dirty_rows,
            surface_kitty_placeholders: &update.surface_kitty_placeholders,
            scrollback_lines: &update.scrollback_lines,
            scrollback_row_runs: &update.scrollback_row_runs,
            scrollback_semantic_prompts: &update.scrollback_semantic_prompts,
            scrollback_dirty_rows: &update.scrollback_dirty_rows,
            scrollback_kitty_placeholders: &update.scrollback_kitty_placeholders,
        }
    }

    #[cfg(feature = "libghostty-vt")]
    struct CoreRendererFixture {
        name: String,
        requires_kitty_graphics: bool,
        cols: u32,
        rows: u32,
        resize: Option<RendererFixtureSize>,
        terminal_output: Vec<String>,
        expected_terminal: Value,
        expected_surface: Value,
        expected_scrollback: Value,
    }

    #[cfg(feature = "libghostty-vt")]
    struct RendererFixtureSize {
        cols: u32,
        rows: u32,
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn renderer_equivalence_fixture_corpus_projects_server_owned_surface() {
        for fixture in load_core_renderer_fixtures() {
            if fixture.requires_kitty_graphics && !super::libghostty_vt_supports_kitty_graphics() {
                continue;
            }
            let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
            let empty = Vec::new();
            let mut update = None;
            for output in &fixture.terminal_output {
                let input = match &update {
                    Some(previous) => {
                        terminal_input_from_update_with_size(fixture.cols, fixture.rows, previous)
                    }
                    None => terminal_input_with_size(fixture.cols, fixture.rows, &empty, &empty),
                };
                update = engine.apply_output(input, output.as_bytes());
            }
            let update = update
                .unwrap_or_else(|| panic!("{} did not produce a terminal update", fixture.name));
            let update = match fixture.resize {
                Some(size) => engine
                    .resize(
                        terminal_input_from_update_with_size(fixture.cols, fixture.rows, &update),
                        size.cols,
                        size.rows,
                    )
                    .unwrap_or_else(|| panic!("{} resize did not produce an update", fixture.name)),
                None => update,
            };

            let actual_terminal = renderer_terminal_json(&update);
            if fixture.name == "libghostty-vt-smoke" {
                // The real PTY-backed CLI fixture observes the final main-screen
                // cursor column after alternate-screen restoration differently
                // from direct chunk replay. Keep core coverage on the shared
                // rows, styles, modes, title, working directory, surface kind,
                // cursor row/visibility/shape/blink, and let the CLI fixture pin
                // the exact user-facing cursor column.
                assert_eq!(
                    terminal_without_cursor_col(actual_terminal),
                    terminal_without_cursor_col(fixture.expected_terminal),
                    "{} terminal projection mismatch",
                    fixture.name
                );
            } else {
                assert_eq!(
                    actual_terminal, fixture.expected_terminal,
                    "{} terminal projection mismatch",
                    fixture.name
                );
            }

            let actual_surface = renderer_surface_json(&update);
            assert_eq!(
                actual_surface, fixture.expected_surface,
                "{} surface projection mismatch",
                fixture.name
            );

            let actual_scrollback = renderer_scrollback_json(&update);
            assert_eq!(
                actual_scrollback, fixture.expected_scrollback,
                "{} scrollback projection mismatch",
                fixture.name
            );
        }
    }

    #[cfg(feature = "libghostty-vt")]
    fn terminal_without_cursor_col(mut terminal: Value) -> Value {
        terminal
            .get_mut("cursor")
            .and_then(Value::as_object_mut)
            .expect("terminal cursor object")
            .remove("col");
        terminal
    }

    #[cfg(feature = "libghostty-vt")]
    fn load_core_renderer_fixtures() -> Vec<CoreRendererFixture> {
        let dir = workspace_path(PathBuf::from("fixtures/renderer-equivalence"));
        let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("read renderer fixture dir {}: {error}", dir.display()))
            .map(|entry| {
                entry
                    .unwrap_or_else(|error| panic!("read renderer fixture dir entry: {error}"))
                    .path()
            })
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        paths.sort();
        assert!(
            !paths.is_empty(),
            "renderer fixture dir {} has no json fixtures",
            dir.display()
        );
        paths
            .into_iter()
            .map(|path| load_core_renderer_fixture(&path))
            .collect()
    }

    #[cfg(feature = "libghostty-vt")]
    fn load_core_renderer_fixture(path: &Path) -> CoreRendererFixture {
        let json = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read renderer fixture {}: {error}", path.display()));
        let decoded: Value = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("decode renderer fixture {}: {error}", path.display()));
        let expected = decoded.get("expected").expect("expected fixture object");
        let expected_core = decoded.get("expected_core").unwrap_or(expected);
        let workspace = expected
            .get("workspace")
            .expect("expected workspace object");
        let size = decoded.get("initial_size").unwrap_or(workspace);
        CoreRendererFixture {
            name: path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_else(|| panic!("fixture path has no utf-8 stem: {}", path.display()))
                .to_owned(),
            requires_kitty_graphics: decoded
                .get("requires_kitty_graphics")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            cols: renderer_numeric_field(size, "cols")
                .try_into()
                .expect("fixture cols fit u32"),
            rows: renderer_numeric_field(size, "rows")
                .try_into()
                .expect("fixture rows fit u32"),
            resize: decoded.get("resize").map(materialize_renderer_fixture_size),
            terminal_output: renderer_string_or_array_field(&decoded, "terminal_output"),
            expected_terminal: expected_core
                .get("terminal")
                .or_else(|| expected.get("terminal"))
                .expect("expected terminal object")
                .clone(),
            expected_surface: expected_core
                .get("surface")
                .or_else(|| expected.get("surface"))
                .expect("expected surface object")
                .clone(),
            expected_scrollback: expected_core
                .get("scrollback")
                .or_else(|| expected.get("scrollback"))
                .expect("expected scrollback rows")
                .clone(),
        }
    }

    #[cfg(feature = "libghostty-vt")]
    fn materialize_renderer_fixture_size(size: &Value) -> RendererFixtureSize {
        RendererFixtureSize {
            cols: renderer_numeric_field(size, "cols")
                .try_into()
                .expect("fixture cols fit u32"),
            rows: renderer_numeric_field(size, "rows")
                .try_into()
                .expect("fixture rows fit u32"),
        }
    }

    #[cfg(feature = "libghostty-vt")]
    fn terminal_input_from_update_with_size<'a>(
        cols: u32,
        rows: u32,
        update: &'a super::TerminalUpdate,
    ) -> TerminalInput<'a> {
        TerminalInput {
            pane_id: "pane-1",
            cols,
            rows,
            surface: update.surface,
            cursor: update.cursor,
            modes: update.modes,
            title: &update.title,
            working_directory: &update.working_directory,
            colors: update.colors.clone(),
            styles: &update.styles,
            surface_lines: &update.surface_lines,
            surface_row_runs: &update.surface_row_runs,
            surface_semantic_prompts: &update.surface_semantic_prompts,
            surface_dirty_rows: &update.surface_dirty_rows,
            surface_kitty_placeholders: &update.surface_kitty_placeholders,
            scrollback_lines: &update.scrollback_lines,
            scrollback_row_runs: &update.scrollback_row_runs,
            scrollback_semantic_prompts: &update.scrollback_semantic_prompts,
            scrollback_dirty_rows: &update.scrollback_dirty_rows,
            scrollback_kitty_placeholders: &update.scrollback_kitty_placeholders,
        }
    }

    #[cfg(feature = "libghostty-vt")]
    fn renderer_terminal_json(update: &super::TerminalUpdate) -> Value {
        json!({
            "title": update.title,
            "working_directory": update.working_directory,
            "surface_kind": surface_kind_name(update.surface),
            "cursor": {
                "row": update.cursor.row,
                "col": update.cursor.col,
                "visible": update.cursor.visible,
                "shape": cursor_shape_name(update.cursor.shape),
                "blinking": update.cursor.blinking,
            },
            "modes": {
                "bracketed_paste": update.modes.bracketed_paste,
                "mouse_tracking": update.modes.mouse_tracking,
                "focus_reporting": update.modes.focus_reporting,
                "application_keypad": update.modes.application_keypad,
                "application_cursor": update.modes.application_cursor,
                "origin": update.modes.origin,
                "wraparound": update.modes.wraparound,
                "mouse_tracking_mode": mouse_tracking_mode_name(update.modes.mouse_tracking_mode),
                "mouse_format": mouse_format_name(update.modes.mouse_format),
            },
        })
    }

    #[cfg(feature = "libghostty-vt")]
    fn renderer_surface_json(update: &super::TerminalUpdate) -> Value {
        json!({
            "colors": renderer_colors_json(&update.colors),
            "styles": update
                .styles
                .iter()
                .map(renderer_style_json)
                .collect::<Vec<_>>(),
            "row_updates": renderer_rows_json(
                "row",
                0,
                &update.surface_lines,
                &update.surface_row_runs,
                &update.surface_semantic_prompts,
                &update.surface_dirty_rows,
                &update.surface_kitty_placeholders,
            ),
        })
    }

    #[cfg(feature = "libghostty-vt")]
    fn renderer_scrollback_json(update: &super::TerminalUpdate) -> Value {
        renderer_rows_json(
            "line",
            1,
            &update.scrollback_lines,
            &update.scrollback_row_runs,
            &update.scrollback_semantic_prompts,
            &update.scrollback_dirty_rows,
            &update.scrollback_kitty_placeholders,
        )
    }

    #[cfg(feature = "libghostty-vt")]
    fn renderer_colors_json(colors: &TerminalColors) -> Value {
        json!({
            "default_fg_rgba": colors.default_fg_rgba,
            "default_bg_rgba": colors.default_bg_rgba,
            "cursor_rgba": colors.cursor_rgba,
            "cursor_rgba_set": colors.cursor_rgba_set,
            "palette_len": colors.palette_rgba.len(),
            "palette_prefix": colors
                .palette_rgba
                .iter()
                .take(16)
                .copied()
                .collect::<Vec<_>>(),
        })
    }

    #[cfg(feature = "libghostty-vt")]
    fn renderer_style_json(style: &super::PaneStyle) -> Value {
        json!({
            "fg_rgba": style.fg_rgba,
            "bg_rgba": style.bg_rgba,
            "underline_rgba": style.underline_rgba,
            "flags": style.flags,
        })
    }

    #[cfg(feature = "libghostty-vt")]
    fn renderer_rows_json(
        index_field: &str,
        index_base: usize,
        lines: &[String],
        row_runs: &[Vec<super::CellRun>],
        semantic_prompts: &[protocol::RowSemanticPrompt],
        dirty_rows: &[bool],
        kitty_placeholders: &[bool],
    ) -> Value {
        Value::Array(
            lines
                .iter()
                .enumerate()
                .filter(|(_, text)| !text.is_empty())
                .map(|(row, text)| {
                    let semantic_prompt = semantic_prompts
                        .get(row)
                        .copied()
                        .unwrap_or(protocol::RowSemanticPrompt::None);
                    let dirty = dirty_rows.get(row).copied().unwrap_or(false);
                    let kitty_virtual_placeholder =
                        kitty_placeholders.get(row).copied().unwrap_or(false);
                    let runs = row_runs
                        .get(row)
                        .unwrap_or_else(|| panic!("missing row runs for row {row}"));
                    let mut row_json = json!({
                        "text": text,
                        "dirty_hash": stable_row_hash(text),
                        "row_state_hash": row_state_hash(
                            runs,
                            semantic_prompt,
                            dirty,
                            kitty_virtual_placeholder
                        ),
                        "semantic_prompt": row_semantic_prompt_name(semantic_prompt),
                        "dirty": dirty,
                        "kitty_virtual_placeholder": kitty_virtual_placeholder,
                        "runs": row_runs
                            .get(row)
                            .unwrap_or_else(|| panic!("missing row runs for row {row}"))
                            .iter()
                            .map(renderer_run_json)
                            .collect::<Vec<_>>(),
                    });
                    row_json
                        .as_object_mut()
                        .expect("renderer row json object")
                        .insert(index_field.to_owned(), json!(row + index_base));
                    row_json
                })
                .collect(),
        )
    }

    #[cfg(feature = "libghostty-vt")]
    fn renderer_run_json(run: &super::CellRun) -> Value {
        json!({
            "text": run.text,
            "cell_widths": run.cell_widths,
            "style_id": run.style_id,
            "flags": run.flags,
            "hyperlink_id": run.hyperlink_id,
            "semantic_content": cell_semantic_content_name(run.semantic_content),
        })
    }

    #[cfg(feature = "libghostty-vt")]
    fn stable_row_hash(line: &str) -> u64 {
        let mut hasher = StableHasher::new();
        hasher.write(line.as_bytes());
        hasher.finish()
    }

    #[cfg(feature = "libghostty-vt")]
    fn row_state_hash(
        runs: &[super::CellRun],
        semantic_prompt: protocol::RowSemanticPrompt,
        dirty: bool,
        kitty_virtual_placeholder: bool,
    ) -> u64 {
        let mut hasher = StableHasher::new();
        for run in runs {
            run.text.hash(&mut hasher);
            run.cell_widths.hash(&mut hasher);
            run.style_id.hash(&mut hasher);
            run.flags.hash(&mut hasher);
            run.hyperlink_id.hash(&mut hasher);
            run.semantic_content.0.hash(&mut hasher);
        }
        semantic_prompt.0.hash(&mut hasher);
        dirty.hash(&mut hasher);
        kitty_virtual_placeholder.hash(&mut hasher);
        hasher.finish()
    }

    #[cfg(feature = "libghostty-vt")]
    struct StableHasher(u64);

    #[cfg(feature = "libghostty-vt")]
    impl StableHasher {
        fn new() -> Self {
            Self(0xcbf2_9ce4_8422_2325_u64)
        }
    }

    #[cfg(feature = "libghostty-vt")]
    impl Hasher for StableHasher {
        fn finish(&self) -> u64 {
            self.0
        }

        fn write(&mut self, bytes: &[u8]) {
            for byte in bytes {
                self.0 ^= u64::from(*byte);
                self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }

    #[cfg(feature = "libghostty-vt")]
    fn renderer_string_or_array_field(value: &Value, name: &str) -> Vec<String> {
        let field = value
            .get(name)
            .unwrap_or_else(|| panic!("missing string or array field {name} in {value:?}"));
        if let Some(text) = field.as_str() {
            return vec![text.to_owned()];
        }
        field
            .as_array()
            .unwrap_or_else(|| panic!("field {name} is not a string or array in {value:?}"))
            .iter()
            .map(|chunk| {
                chunk
                    .as_str()
                    .unwrap_or_else(|| panic!("field {name} contains non-string chunk: {chunk:?}"))
                    .to_owned()
            })
            .collect()
    }

    #[cfg(feature = "libghostty-vt")]
    fn renderer_numeric_field(value: &Value, name: &str) -> u64 {
        value
            .get(name)
            .and_then(Value::as_u64)
            .unwrap_or_else(|| panic!("missing numeric field {name} in {value:?}"))
    }

    #[cfg(feature = "libghostty-vt")]
    fn workspace_path(path: PathBuf) -> PathBuf {
        if path.is_absolute() {
            return path;
        }
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    #[cfg(feature = "libghostty-vt")]
    fn surface_kind_name(kind: protocol::SurfaceKind) -> &'static str {
        match kind {
            protocol::SurfaceKind::Main => "main",
            protocol::SurfaceKind::Alternate => "alternate",
            _ => "unknown",
        }
    }

    #[cfg(feature = "libghostty-vt")]
    fn cursor_shape_name(shape: protocol::CursorShape) -> &'static str {
        match shape {
            protocol::CursorShape::Block => "block",
            protocol::CursorShape::Beam => "beam",
            protocol::CursorShape::Underline => "underline",
            _ => "unknown",
        }
    }

    #[cfg(feature = "libghostty-vt")]
    fn mouse_tracking_mode_name(mode: protocol::MouseTrackingMode) -> &'static str {
        match mode {
            protocol::MouseTrackingMode::None => "none",
            protocol::MouseTrackingMode::X10 => "x10",
            protocol::MouseTrackingMode::Normal => "normal",
            protocol::MouseTrackingMode::Button => "button",
            protocol::MouseTrackingMode::Any => "any",
            _ => "unknown",
        }
    }

    #[cfg(feature = "libghostty-vt")]
    fn mouse_format_name(format: protocol::MouseFormat) -> &'static str {
        match format {
            protocol::MouseFormat::X10 => "x10",
            protocol::MouseFormat::Utf8 => "utf8",
            protocol::MouseFormat::Sgr => "sgr",
            protocol::MouseFormat::Urxvt => "urxvt",
            protocol::MouseFormat::SgrPixels => "sgr-pixels",
            _ => "unknown",
        }
    }

    #[cfg(feature = "libghostty-vt")]
    fn row_semantic_prompt_name(prompt: protocol::RowSemanticPrompt) -> &'static str {
        match prompt {
            protocol::RowSemanticPrompt::None => "none",
            protocol::RowSemanticPrompt::Prompt => "prompt",
            protocol::RowSemanticPrompt::Continuation => "continuation",
            _ => "unknown",
        }
    }

    #[cfg(feature = "libghostty-vt")]
    fn cell_semantic_content_name(content: protocol::CellSemanticContent) -> &'static str {
        match content {
            protocol::CellSemanticContent::Output => "output",
            protocol::CellSemanticContent::Prompt => "prompt",
            protocol::CellSemanticContent::Input => "input",
            _ => "unknown",
        }
    }

    #[test]
    fn interim_text_engine_normalizes_process_output() {
        let mut engine = InterimTextTerminalEngine::default();
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
                blinking: true,
            },
            modes: TerminalModes::default(),
            title: "existing title",
            working_directory: "file://localhost/existing",
            colors: TerminalColors::default(),
            styles: &[],
            surface_lines: &[],
            surface_row_runs: &[],
            surface_semantic_prompts: &[],
            surface_dirty_rows: &[],
            surface_kitty_placeholders: &[],
            scrollback_lines: &scrollback_lines,
            scrollback_row_runs: &[],
            scrollback_semantic_prompts: &[],
            scrollback_dirty_rows: &[],
            scrollback_kitty_placeholders: &[],
        };

        let update = engine
            .apply_output(input, b"hello\r\nsecond\x1b[31m line\x1b[0m\n")
            .expect("terminal update");

        assert_eq!(
            update.scrollback_lines,
            vec![
                "existing".to_owned(),
                "hello".to_owned(),
                "second line".to_owned()
            ]
        );
        assert_eq!(update.surface_lines, update.scrollback_lines);
        assert_eq!(
            update.cursor,
            TerminalCursor {
                row: 2,
                col: 0,
                visible: true,
                shape: protocol::CursorShape::Block,
                blinking: true
            }
        );
        assert_eq!(update.surface, protocol::SurfaceKind::Main);
        assert_eq!(update.title, "existing title");
    }

    #[test]
    fn interim_text_engine_merges_split_pty_writes_until_newline() {
        let mut engine = InterimTextTerminalEngine::default();
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
                blinking: true,
            },
            modes: TerminalModes::default(),
            title: "",
            working_directory: "",
            colors: TerminalColors::default(),
            styles: &[],
            surface_lines: &[],
            surface_row_runs: &[],
            surface_semantic_prompts: &[],
            surface_dirty_rows: &[],
            surface_kitty_placeholders: &[],
            scrollback_lines: &scrollback_lines,
            scrollback_row_runs: &[],
            scrollback_semantic_prompts: &[],
            scrollback_dirty_rows: &[],
            scrollback_kitty_placeholders: &[],
        };

        let first = engine.apply_output(input, b"a").expect("first update");
        assert_eq!(first.scrollback_lines, vec!["existing", "a"]);
        assert_eq!(first.cursor.col, 1);

        let second = engine
            .apply_output(terminal_input_from_update(&first), b"b")
            .expect("second update");
        assert_eq!(second.scrollback_lines, vec!["existing", "ab"]);
        assert_eq!(second.cursor.col, 2);

        let third = engine
            .apply_output(terminal_input_from_update(&second), b"c\n")
            .expect("third update");
        assert_eq!(third.scrollback_lines, vec!["existing", "abc"]);
        assert_eq!(third.cursor.col, 0);
    }

    #[test]
    fn interim_text_engine_consumes_split_control_sequences() {
        let mut engine = InterimTextTerminalEngine::default();
        let scrollback_lines = Vec::new();
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
                blinking: true,
            },
            modes: TerminalModes::default(),
            title: "",
            working_directory: "",
            colors: TerminalColors::default(),
            styles: &[],
            surface_lines: &[],
            surface_row_runs: &[],
            surface_semantic_prompts: &[],
            surface_dirty_rows: &[],
            surface_kitty_placeholders: &[],
            scrollback_lines: &scrollback_lines,
            scrollback_row_runs: &[],
            scrollback_semantic_prompts: &[],
            scrollback_dirty_rows: &[],
            scrollback_kitty_placeholders: &[],
        };

        let first = engine
            .apply_output(input, b"ready\n\x1b[")
            .expect("first update");
        let second = engine
            .apply_output(
                terminal_input_from_update(&first),
                b"?2004h\x1b[?1000h\x1b[?1006h",
            )
            .expect("second update");
        let third = engine
            .apply_output(
                terminal_input_from_update(&second),
                b"\x1b]0;ignored title\x07done\n",
            )
            .expect("third update");

        assert_eq!(
            third.scrollback_lines,
            vec!["ready".to_owned(), "done".to_owned()]
        );
        assert!(third.modes.bracketed_paste);
        assert!(third.modes.mouse_tracking);
        assert_eq!(
            third.modes.mouse_tracking_mode,
            protocol::MouseTrackingMode::Normal
        );
        assert_eq!(third.modes.mouse_format, protocol::MouseFormat::Sgr);
    }

    #[test]
    fn interim_text_engine_keeps_visible_tail() {
        let mut engine = InterimTextTerminalEngine::default();
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
                blinking: true,
            },
            modes: TerminalModes::default(),
            title: "",
            working_directory: "",
            colors: TerminalColors::default(),
            styles: &[],
            surface_lines: &[],
            surface_row_runs: &[],
            surface_semantic_prompts: &[],
            surface_dirty_rows: &[],
            surface_kitty_placeholders: &[],
            scrollback_lines: &scrollback_lines,
            scrollback_row_runs: &[],
            scrollback_semantic_prompts: &[],
            scrollback_dirty_rows: &[],
            scrollback_kitty_placeholders: &[],
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
                shape: protocol::CursorShape::Beam,
                blinking: true
            }
        );
    }

    #[test]
    fn interim_text_engine_resizes_visible_tail() {
        let mut engine = InterimTextTerminalEngine::default();
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
                blinking: true,
            },
            modes: TerminalModes::default(),
            title: "",
            working_directory: "",
            colors: TerminalColors::default(),
            styles: &[],
            surface_lines: &scrollback_lines,
            surface_row_runs: &[],
            surface_semantic_prompts: &[],
            surface_dirty_rows: &[],
            surface_kitty_placeholders: &[],
            scrollback_lines: &scrollback_lines,
            scrollback_row_runs: &[],
            scrollback_semantic_prompts: &[],
            scrollback_dirty_rows: &[],
            scrollback_kitty_placeholders: &[],
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
                shape: protocol::CursorShape::Underline,
                blinking: true
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
    fn libghostty_vt_engine_preserves_styled_trailing_blank_cells() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input_with_size(12, 1, &empty, &empty),
                b"\x1b[48;2;1;2;3m   \x1b[0m",
            )
            .expect("terminal update");

        let blank_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text == "   ")
            .expect("styled blank run");
        assert_ne!(blank_run.style_id, 0);
        assert_eq!(blank_run.cell_widths, vec![1, 1, 1]);
        let style = update
            .styles
            .get(blank_run.style_id as usize)
            .expect("blank style");
        assert_ne!(
            style.bg_rgba, 0,
            "styled trailing blanks should preserve background color"
        );
        assert!(
            update.surface_lines.iter().any(|line| line == "   "),
            "styled trailing blanks were trimmed from fallback text: {:?}",
            update.surface_lines
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_trims_default_trailing_blank_cells() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(terminal_input_with_size(12, 1, &empty, &empty), b"plain   ")
            .expect("terminal update");

        assert!(
            update.surface_lines.iter().any(|line| line == "plain"),
            "default trailing blanks should still be trimmed: {:?}",
            update.surface_lines
        );
        assert!(
            update
                .surface_row_runs
                .iter()
                .flat_map(|row| row.iter())
                .all(|run| run.text != "plain   "),
            "default trailing blanks leaked into row runs: {:?}",
            update.surface_row_runs
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_render_state_exposes_default_colors_without_protocol_fields() {
        use libghostty_vt::{RenderState, Terminal, TerminalOptions};

        let terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut render_state = RenderState::new().expect("render state");

        let snapshot = render_state.update(&terminal).expect("snapshot");
        let colors = snapshot.colors().expect("render colors");

        assert_ne!(
            colors.foreground, colors.background,
            "default foreground and background should be distinct"
        );
        assert_ne!(
            colors.palette[0], colors.palette[7],
            "default palette should expose indexed colors"
        );
        assert_eq!(
            colors.cursor, None,
            "default cursor color should be absent until explicitly set"
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_terminal_color_state() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let update = engine
            .apply_output(terminal_input_with_size(80, 24, &[], &[]), b"colors")
            .expect("terminal update");

        assert_ne!(update.colors.default_fg_rgba, 0);
        assert_ne!(update.colors.default_bg_rgba, 0);
        assert_ne!(update.colors.default_fg_rgba, update.colors.default_bg_rgba);
        assert!(update.colors.palette_rgba.len() >= 16);
        assert_eq!(update.colors.cursor_rgba, 0);
        assert!(!update.colors.cursor_rgba_set);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_render_state_tracks_explicit_cursor_color_without_protocol_fields() {
        use libghostty_vt::{RenderState, Terminal, TerminalOptions, style::RgbColor};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut render_state = RenderState::new().expect("render state");

        terminal.vt_write(b"\x1b]12;#ff00ff\x1b\\");
        let snapshot = render_state.update(&terminal).expect("snapshot");
        let cursor = Some(RgbColor {
            r: 255,
            g: 0,
            b: 255,
        });

        assert_eq!(snapshot.cursor_color().expect("cursor color"), cursor);
        assert_eq!(snapshot.colors().expect("render colors").cursor, cursor);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_tracks_explicit_cursor_color_in_color_state() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let update = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b]12;#ff00ff\x1b\\cursor",
            )
            .expect("terminal update");

        assert_eq!(update.colors.cursor_rgba, 0xff00_ffff);
        assert!(update.colors.cursor_rgba_set);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_render_state_tracks_palette_override_without_protocol_fields() {
        use libghostty_vt::{RenderState, Terminal, TerminalOptions, style::RgbColor};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut render_state = RenderState::new().expect("render state");

        terminal.vt_write(b"\x1b]4;1;#112233\x1b\\");
        let snapshot = render_state.update(&terminal).expect("snapshot");

        assert_eq!(
            snapshot.colors().expect("render colors").palette[1],
            RgbColor {
                r: 0x11,
                g: 0x22,
                b: 0x33,
            }
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_tracks_palette_override_in_color_state() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let update = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b]4;1;#112233\x1b\\palette",
            )
            .expect("terminal update");

        assert_eq!(update.colors.palette_rgba.get(1).copied(), Some(0x112233ff));
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_updates_palette_indexed_style_after_palette_override() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let styled = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b[38;5;1mpalette-red\x1b[0m",
            )
            .expect("styled update");

        let palette_override = engine
            .apply_output(
                terminal_input_from_update(&styled),
                b"\x1b]4;1;#112233\x1b\\",
            )
            .expect("palette override update");

        assert_eq!(palette_override.surface_lines, styled.surface_lines);
        assert_eq!(
            palette_override.colors.palette_rgba.get(1).copied(),
            Some(0x112233ff)
        );
        let styled_run = palette_override
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("palette-red"))
            .expect("palette styled run");
        let style = palette_override
            .styles
            .get(styled_run.style_id as usize)
            .expect("palette style");
        assert_eq!(
            style.fg_rgba, 0x112233ff,
            "palette-indexed style should resolve through the new palette color"
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_safe_api_has_pwd_accessor_but_does_not_populate_osc7() {
        use libghostty_vt::{Terminal, TerminalOptions};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");

        terminal.vt_write(b"\x1b]2;nmux test title\x1b\\");
        assert_eq!(terminal.title().expect("terminal title"), "nmux test title");

        terminal.vt_write(b"\x1b]7;file://localhost/tmp/nmux\x07");
        assert_eq!(
            terminal.pwd().expect("terminal working directory"),
            "",
            "current backend path exposes a pwd accessor but does not populate it from OSC 7 bytes"
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_title_metadata() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let first = engine
            .apply_output(terminal_input_with_size(80, 24, &[], &[]), b"ready")
            .expect("first update");
        let title = engine
            .apply_output(
                terminal_input_from_update(&first),
                b"\x1b]2;nmux test title\x1b\\",
            )
            .expect("title update");

        assert_eq!(title.title, "nmux test title");
        assert_eq!(title.surface_lines, first.surface_lines);
        assert_eq!(title.patch_kind, protocol::PatchKind::CursorOnly);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_osc7_working_directory_with_bel() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let first = engine
            .apply_output(terminal_input_with_size(80, 24, &[], &[]), b"ready")
            .expect("first update");
        let update = engine
            .apply_output(
                terminal_input_from_update(&first),
                b"\x1b]7;file://localhost/tmp/nmux\x07",
            )
            .expect("working-directory update");

        assert_eq!(update.working_directory, "file://localhost/tmp/nmux");
        assert_eq!(update.surface_lines, first.surface_lines);
        assert_eq!(update.patch_kind, protocol::PatchKind::CursorOnly);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_osc7_working_directory_with_st() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let update = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b]7;file://localhost/tmp/st\x1b\\ready",
            )
            .expect("working-directory update");

        assert_eq!(update.working_directory, "file://localhost/tmp/st");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_split_osc7_working_directory() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let partial = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b]7;file://localhost/tmp/split",
            )
            .expect("partial update");
        assert_eq!(partial.working_directory, "");

        let update = engine
            .apply_output(terminal_input_from_update(&partial), b"\x07")
            .expect("working-directory update");

        assert_eq!(update.working_directory, "file://localhost/tmp/split");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_osc7_when_start_is_split() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let partial = engine
            .apply_output(terminal_input_with_size(80, 24, &[], &[]), b"\x1b")
            .expect("partial update");
        assert_eq!(partial.working_directory, "");

        let update = engine
            .apply_output(
                terminal_input_from_update(&partial),
                b"]7;file://localhost/tmp/start\x07",
            )
            .expect("working-directory update");

        assert_eq!(update.working_directory, "file://localhost/tmp/start");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_ignores_invalid_utf8_osc7() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let first = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b]7;file://localhost/tmp/valid\x07",
            )
            .expect("first update");
        assert_eq!(first.working_directory, "file://localhost/tmp/valid");

        let update = engine
            .apply_output(
                terminal_input_from_update(&first),
                b"\x1b]7;file://localhost/tmp/\xff\x07",
            )
            .expect("terminal update");

        assert_eq!(update.working_directory, "file://localhost/tmp/valid");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_ignores_non_osc7_metadata() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let first = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b]7;file://localhost/tmp/valid\x07",
            )
            .expect("first update");

        let update = engine
            .apply_output(
                terminal_input_from_update(&first),
                b"\x1b]2;pane title\x1b\\",
            )
            .expect("terminal update");

        assert_eq!(update.working_directory, "file://localhost/tmp/valid");
        assert_eq!(update.title, "pane title");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_allows_empty_osc7_to_clear_working_directory() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let first = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b]7;file://localhost/tmp/valid\x07",
            )
            .expect("first update");
        assert_eq!(first.working_directory, "file://localhost/tmp/valid");

        let update = engine
            .apply_output(terminal_input_from_update(&first), b"\x1b]7;\x07")
            .expect("terminal update");

        assert_eq!(update.working_directory, "");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_ignores_oversized_unterminated_osc7() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let mut output = b"\x1b]7;file://localhost/tmp/".to_vec();
        output.extend(std::iter::repeat_n(b'x', 4097));
        output.push(0x07);

        let update = engine
            .apply_output(terminal_input_with_size(80, 24, &[], &[]), &output)
            .expect("terminal update");

        assert_eq!(update.working_directory, "");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_safe_api_tracks_semantic_prompt() {
        use libghostty_vt::{
            RenderState, Terminal, TerminalOptions,
            render::{CellIterator, RowIterator},
            screen::{CellSemanticContent, RowSemanticPrompt},
        };

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut render_state = RenderState::new().expect("render state");
        let mut rows = RowIterator::new().expect("row iterator");
        let mut cells = CellIterator::new().expect("cell iterator");

        terminal.vt_write(b"\x1b]133;A\x1b\\prompt> ");
        let snapshot = render_state.update(&terminal).expect("snapshot");
        let mut row_iter = rows.update(&snapshot).expect("row iteration");
        let first_row = row_iter.next().expect("first row");

        assert_eq!(
            first_row
                .raw_row()
                .expect("raw row")
                .semantic_prompt()
                .expect("semantic prompt"),
            RowSemanticPrompt::Prompt
        );
        let mut cell_iter = cells.update(first_row).expect("cell iteration");
        cell_iter.next().expect("first cell");
        assert_eq!(
            cell_iter
                .raw_cell()
                .expect("raw cell")
                .semantic_content()
                .expect("semantic content"),
            CellSemanticContent::Prompt
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_semantic_prompt_metadata() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let update = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b]133;A\x1b\\prompt> ",
            )
            .expect("prompt update");

        assert_eq!(
            update.surface_lines.first().map(String::as_str),
            Some("prompt> ")
        );
        assert_eq!(
            update.surface_semantic_prompts.first().copied(),
            Some(protocol::RowSemanticPrompt::Prompt)
        );
        assert_eq!(
            update
                .surface_row_runs
                .first()
                .and_then(|runs| runs.first())
                .map(|run| run.semantic_content),
            Some(protocol::CellSemanticContent::Prompt)
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_splits_runs_by_semantic_content() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let update = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b]133;A\x1b\\prompt \x1b]133;B\x1b\\input\x1b]133;C\x1b\\output",
            )
            .expect("semantic content update");

        assert_eq!(
            update.surface_lines.first().map(String::as_str),
            Some("prompt inputoutput")
        );
        let runs = update.surface_row_runs.first().expect("first row runs");
        let semantic_content: Vec<_> = runs.iter().map(|run| run.semantic_content).collect();
        assert_eq!(
            semantic_content,
            vec![
                protocol::CellSemanticContent::Prompt,
                protocol::CellSemanticContent::Input,
                protocol::CellSemanticContent::Output,
            ]
        );
        assert_eq!(
            runs.iter().map(|run| run.style_id).collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert_eq!(
            runs.iter().map(|run| run.text.as_str()).collect::<Vec<_>>(),
            vec!["prompt ", "input", "output"]
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_preserves_semantic_trailing_blank_cells() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let update = engine
            .apply_output(
                terminal_input_with_size(80, 24, &[], &[]),
                b"\x1b]133;A\x1b\\prompt\x1b]133;B\x1b\\   ",
            )
            .expect("semantic trailing blank update");

        assert_eq!(
            update.surface_lines.first().map(String::as_str),
            Some("prompt   ")
        );
        let runs = update.surface_row_runs.first().expect("first row runs");
        let input_run = runs
            .iter()
            .find(|run| run.semantic_content == protocol::CellSemanticContent::Input)
            .expect("input semantic blank run");

        assert_eq!(input_run.text, "   ");
        assert_eq!(input_run.cell_widths, vec![1, 1, 1]);
        assert_eq!(input_run.style_id, 0);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_render_state_exposes_row_dirty() {
        use libghostty_vt::{RenderState, Terminal, TerminalOptions, render::RowIterator};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut render_state = RenderState::new().expect("render state");
        let mut rows = RowIterator::new().expect("row iterator");

        {
            let clean_snapshot = render_state.update(&terminal).expect("initial snapshot");
            let mut clean_row_iter = rows.update(&clean_snapshot).expect("clean row iteration");
            while let Some(row) = clean_row_iter.next() {
                row.set_dirty(false).expect("mark row clean");
            }
        }

        terminal.vt_write(b"dirty row");
        let dirty_snapshot = render_state.update(&terminal).expect("dirty snapshot");

        let mut row_iter = rows.update(&dirty_snapshot).expect("row iteration");
        let mut dirty_rows = 0;
        while let Some(row) = row_iter.next() {
            if row.dirty().expect("row dirty") {
                dirty_rows += 1;
            }
        }

        assert!(
            dirty_rows > 0,
            "render state should expose at least one dirty row"
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_row_dirty_metadata() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let update = engine
            .apply_output(terminal_input_with_size(80, 24, &[], &[]), b"dirty row")
            .expect("dirty update");

        assert_eq!(
            update.surface_lines.first().map(String::as_str),
            Some("dirty row")
        );
        assert_eq!(update.surface_dirty_rows.first().copied(), Some(true));
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_safe_api_tracks_kitty_placeholder_without_image_placement_fields() {
        use libghostty_vt::{
            RenderState, Terminal, TerminalOptions, build_info, render::RowIterator,
        };

        if !build_info::supports_kitty_graphics().expect("kitty graphics support query") {
            return;
        }

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut render_state = RenderState::new().expect("render state");
        let mut rows = RowIterator::new().expect("row iterator");

        let mut seq = Vec::new();
        seq.extend_from_slice(b"\x1b_Ga=T,t=d;");
        seq.extend_from_slice(
            b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNgYAAAAAMAASsJTYQAAAAASUVORK5CYII=",
        );
        seq.extend_from_slice(b"\x1b\\");
        seq.extend_from_slice("\u{10eeee}".as_bytes());

        terminal.vt_write(&seq);
        let snapshot = render_state.update(&terminal).expect("snapshot");
        let mut row_iter = rows.update(&snapshot).expect("row iteration");
        let mut has_placeholder = false;
        while let Some(row) = row_iter.next() {
            if row
                .raw_row()
                .expect("raw row")
                .has_kitty_virtual_placeholder()
                .expect("kitty placeholder")
            {
                has_placeholder = true;
            }
        }

        assert!(
            has_placeholder,
            "kitty graphics placeholder should be visible through row metadata"
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_kitty_placeholder_metadata() {
        use libghostty_vt::build_info;

        if !build_info::supports_kitty_graphics().expect("kitty graphics support query") {
            return;
        }

        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let mut seq = Vec::new();
        seq.extend_from_slice(b"\x1b_Ga=T,t=d;");
        seq.extend_from_slice(
            b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNgYAAAAAMAASsJTYQAAAAASUVORK5CYII=",
        );
        seq.extend_from_slice(b"\x1b\\");
        seq.extend_from_slice("\u{10eeee}".as_bytes());

        let update = engine
            .apply_output(terminal_input_with_size(80, 24, &[], &[]), &seq)
            .expect("kitty placeholder update");

        assert!(
            update
                .surface_kitty_placeholders
                .iter()
                .any(|placeholder| *placeholder),
            "kitty graphics placeholder should be carried through row metadata"
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
                b"\x1b[1;2;3;4;5;7;8;9;53mflags\x1b[0m plain",
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
        assert_ne!(style.flags & (1 << 2), 0, "faint flag missing");
        assert_ne!(style.flags & (1 << 3), 0, "blink flag missing");
        assert_ne!(style.flags & (1 << 4), 0, "inverse flag missing");
        assert_ne!(style.flags & (1 << 5), 0, "invisible flag missing");
        assert_ne!(style.flags & (1 << 6), 0, "strikethrough flag missing");
        assert_ne!(style.flags & (1 << 7), 0, "overline flag missing");
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
    fn libghostty_vt_engine_extracts_underline_variants() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input(2, &empty, &empty),
                b"\x1b[4msingle\x1b[0m \x1b[4:2mdouble\x1b[0m \x1b[4:3mcurly\x1b[0m \x1b[4:4mdotted\x1b[0m \x1b[4:5mdashed\x1b[0m",
            )
            .expect("terminal update");

        for (text, flag) in [
            ("single", 1 << 8),
            ("double", 1 << 9),
            ("curly", 1 << 10),
            ("dotted", 1 << 11),
            ("dashed", 1 << 12),
        ] {
            let run = update
                .surface_row_runs
                .iter()
                .flat_map(|row| row.iter())
                .find(|run| run.text.contains(text))
                .unwrap_or_else(|| panic!("{text} underline run missing"));
            let style = update
                .styles
                .get(run.style_id as usize)
                .unwrap_or_else(|| panic!("{text} underline style missing"));

            assert_ne!(run.style_id, 0, "{text} underline style ID missing");
            assert_ne!(
                style.flags & flag,
                0,
                "{text} underline flag {flag:#x} missing"
            );
        }
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_extracts_underline_color() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input(2, &empty, &empty),
                b"\x1b[4;58;2;255;0;128munder\x1b[0m plain",
            )
            .expect("terminal update");

        let underline_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("under"))
            .expect("underline-styled run");
        let style = update
            .styles
            .get(underline_run.style_id as usize)
            .expect("underline style");

        assert_ne!(underline_run.style_id, 0);
        assert_ne!(style.flags & (1 << 8), 0, "underline flag missing");
        assert_ne!(
            style.underline_rgba, 0,
            "underline color should resolve to RGBA"
        );

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
    fn libghostty_vt_engine_preserves_emoji_cluster_cell_widths() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let update = engine
            .apply_output(
                terminal_input(2, &empty, &empty),
                b"emoji:\xf0\x9f\x91\xa9\xe2\x80\x8d\xf0\x9f\x92\xbb\r\nplain",
            )
            .expect("terminal update");

        let emoji_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("\u{1f469}\u{200d}\u{1f4bb}"))
            .expect("emoji cluster run");
        assert!(
            emoji_run.text.contains("emoji:\u{1f469}\u{200d}\u{1f4bb}"),
            "emoji cluster was not preserved in run text: {:?}",
            emoji_run
        );
        let emoji_index = emoji_run
            .text
            .chars()
            .position(|ch| ch == '\u{1f469}')
            .expect("emoji base char");
        assert_eq!(
            emoji_run.cell_widths[emoji_index], 2,
            "emoji cluster should occupy a double-width rendered cell: {:?}",
            emoji_run
        );
        assert!(
            emoji_run.cell_widths.len() < emoji_run.text.chars().count(),
            "emoji cluster widths should be per rendered cell, not per scalar: {:?}",
            emoji_run
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
        assert_eq!(link_run.text, "linked");
        assert_ne!(
            link_run.flags & super::CELL_RUN_FLAG_HYPERLINK_PRESENT,
            0,
            "hyperlink presence flag missing from linked run: {:?}",
            link_run
        );
        assert_eq!(
            link_run.hyperlink_id, 0,
            "nmux must not invent hyperlink IDs before a hyperlink table exists"
        );
        let plain_run = update
            .surface_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains(" text"))
            .expect("plain text run after hyperlink reset");
        assert_eq!(
            plain_run.flags & super::CELL_RUN_FLAG_HYPERLINK_PRESENT,
            0,
            "hyperlink flag should be cleared after OSC 8 reset: {:?}",
            plain_run
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_safe_api_tracks_hyperlink_presence_without_uri_protocol_fields() {
        use libghostty_vt::{
            RenderState, Terminal, TerminalOptions,
            render::{CellIterator, RowIterator},
        };

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        terminal.vt_write(b"\x1b]8;;https://example.com\x1b\\linked\x1b]8;;\x1b\\ plain");

        let mut render_state = RenderState::new().expect("render state");
        let snapshot = render_state.update(&terminal).expect("snapshot");
        let mut row_iterator = RowIterator::new().expect("row iterator");
        let mut rows = row_iterator.update(&snapshot).expect("rows");
        let row = rows.next().expect("first row");
        assert!(
            row.raw_row()
                .expect("raw row")
                .has_hyperlink()
                .expect("row hyperlink")
        );

        let mut cell_iterator = CellIterator::new().expect("cell iterator");
        let mut cells = cell_iterator.update(row).expect("cells");
        let mut linked_cells = 0;
        let mut plain_cells = 0;
        while cells.next().is_some() {
            let raw_cell = cells.raw_cell().expect("raw cell");
            let graphemes = cells.graphemes().expect("graphemes");
            if graphemes.is_empty() {
                continue;
            }
            if raw_cell.has_hyperlink().expect("cell hyperlink") {
                linked_cells += 1;
            } else {
                plain_cells += 1;
            }
        }

        assert!(linked_cells > 0, "expected linked cells in OSC 8 range");
        assert!(
            plain_cells > 0,
            "expected plain cells after hyperlink reset"
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
    fn libghostty_vt_engine_emits_mode_only_patch_for_mode_changes() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let first = engine
            .apply_output(terminal_input(2, &empty, &empty), b"ready")
            .expect("initial update");
        let mode_change = engine
            .apply_output(
                terminal_input_from_update(&first),
                b"\x1b[?2004h\x1b[?1004h",
            )
            .expect("mode update");

        assert_eq!(mode_change.patch_kind, protocol::PatchKind::ModeOnly);
        assert_eq!(mode_change.surface_lines, first.surface_lines);
        assert_eq!(mode_change.cursor, first.cursor);
        assert!(mode_change.modes.bracketed_paste);
        assert!(mode_change.modes.focus_reporting);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_exposes_terminal_query_pty_writes() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let _ = engine
            .apply_output(terminal_input(2, &empty, &empty), b"\x1b[?7$p")
            .expect("terminal update");

        let writes = engine.drain_pty_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0], b"\x1b[?7;1$y");
        assert!(
            engine.drain_pty_writes().is_empty(),
            "pty writes should drain exactly once"
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_exposes_split_terminal_query_pty_writes() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let first = engine
            .apply_output(terminal_input(2, &empty, &empty), b"\x1b[?7")
            .expect("partial terminal update");
        assert!(engine.drain_pty_writes().is_empty());
        let _ = engine
            .apply_output(terminal_input_from_update(&first), b"$p")
            .expect("completed terminal update");

        let writes = engine.drain_pty_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0], b"\x1b[?7;1$y");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_paste_safety_detects_injection_sequences_without_protocol_fields() {
        use libghostty_vt::paste;

        assert!(paste::is_safe("safe paste text"));
        assert!(!paste::is_safe("unsafe\npaste"));
        assert!(!paste::is_safe("escape bracketed paste \x1b[201~ then run"));
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_safe_api_tracks_focus_reporting_without_protocol_fields() {
        use libghostty_vt::{Terminal, TerminalOptions, focus, terminal::Mode};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");

        assert!(!terminal.mode(Mode::FOCUS_EVENT).expect("focus mode"));

        terminal.vt_write(b"\x1b[?1004h");
        assert!(terminal.mode(Mode::FOCUS_EVENT).expect("focus mode"));

        let mut gained = [0; 8];
        let gained_len = focus::Event::Gained
            .encode(&mut gained)
            .expect("focus gained");
        assert_eq!(&gained[..gained_len], b"\x1b[I");

        let mut lost = [0; 8];
        let lost_len = focus::Event::Lost.encode(&mut lost).expect("focus lost");
        assert_eq!(&lost[..lost_len], b"\x1b[O");

        terminal.vt_write(b"\x1b[?1004l");
        assert!(!terminal.mode(Mode::FOCUS_EVENT).expect("focus mode"));
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
    fn libghostty_vt_safe_api_tracks_application_keypad_mode() {
        use libghostty_vt::{Terminal, TerminalOptions, terminal::Mode};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");

        assert!(!terminal.mode(Mode::KEYPAD_KEYS).expect("keypad mode"));

        terminal.vt_write(b"\x1b=");
        assert!(terminal.mode(Mode::KEYPAD_KEYS).expect("keypad mode"));

        terminal.vt_write(b"\x1b>");
        assert!(!terminal.mode(Mode::KEYPAD_KEYS).expect("keypad mode"));
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_key_encoder_supports_application_keypad_mode_without_protocol_fields() {
        use libghostty_vt::{
            Terminal, TerminalOptions,
            key::{Action, Encoder, Event, Key},
        };

        let terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut event = Event::new().expect("event");
        event.set_action(Action::Press).set_key(Key::NumpadEnter);

        let mut normal_encoder = Encoder::new().expect("encoder");
        normal_encoder.set_options_from_terminal(&terminal);
        let mut normal = Vec::new();
        normal_encoder
            .encode_to_vec(&event, &mut normal)
            .expect("normal keypad");

        let mut application_encoder = Encoder::new().expect("encoder");
        application_encoder.set_keypad_key_application(true);
        let mut application = Vec::new();
        application_encoder
            .encode_to_vec(&event, &mut application)
            .expect("application keypad");

        assert_eq!(normal, b"\r");
        assert_eq!(application, b"\x1bOM");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_safe_api_tracks_origin_and_wrap_modes() {
        use libghostty_vt::{Terminal, TerminalOptions, terminal::Mode};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");

        assert!(!terminal.mode(Mode::ORIGIN).expect("origin mode"));
        assert!(terminal.mode(Mode::WRAPAROUND).expect("wrap mode"));

        terminal.vt_write(b"\x1b[?6h\x1b[?7l");
        assert!(terminal.mode(Mode::ORIGIN).expect("origin mode"));
        assert!(!terminal.mode(Mode::WRAPAROUND).expect("wrap mode"));

        terminal.vt_write(b"\x1b[?6l\x1b[?7h");
        assert!(!terminal.mode(Mode::ORIGIN).expect("origin mode"));
        assert!(terminal.mode(Mode::WRAPAROUND).expect("wrap mode"));
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_safe_api_tracks_mouse_tracking_modes() {
        use libghostty_vt::{Terminal, TerminalOptions, terminal::Mode};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");

        assert!(!terminal.is_mouse_tracking().expect("mouse tracking"));

        terminal.vt_write(b"\x1b[?1000h");
        assert!(terminal.is_mouse_tracking().expect("mouse tracking"));
        assert!(terminal.mode(Mode::NORMAL_MOUSE).expect("normal mouse"));
        terminal.vt_write(b"\x1b[?1000l");
        assert!(!terminal.is_mouse_tracking().expect("mouse tracking"));

        terminal.vt_write(b"\x1b[?1002h");
        assert!(terminal.is_mouse_tracking().expect("mouse tracking"));
        assert!(terminal.mode(Mode::BUTTON_MOUSE).expect("button mouse"));
        terminal.vt_write(b"\x1b[?1002l");
        assert!(!terminal.is_mouse_tracking().expect("mouse tracking"));

        terminal.vt_write(b"\x1b[?1003h");
        assert!(terminal.is_mouse_tracking().expect("mouse tracking"));
        assert!(terminal.mode(Mode::ANY_MOUSE).expect("any mouse"));
        terminal.vt_write(b"\x1b[?1003l");
        assert!(!terminal.is_mouse_tracking().expect("mouse tracking"));
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_encodes_sgr_mouse_press_from_terminal_state() {
        use super::{MouseAction, MouseButton, MouseTerminalInput};

        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        let update = engine
            .apply_output(
                terminal_input(24, &empty, &empty),
                b"\x1b[?1000h\x1b[?1006h",
            )
            .expect("terminal update");
        assert!(update.modes.mouse_tracking);
        assert_eq!(
            update.modes.mouse_tracking_mode,
            protocol::MouseTrackingMode::Normal
        );
        assert_eq!(update.modes.mouse_format, protocol::MouseFormat::Sgr);

        let bytes = engine
            .encode_mouse_input(MouseTerminalInput {
                row: 0,
                col: 0,
                pixel_x: None,
                pixel_y: None,
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 0,
                mouse_format: update.modes.mouse_format,
                cols: 80,
                rows: 24,
            })
            .expect("encoded mouse input");

        assert_eq!(bytes, b"\x1b[<0;1;1M");
    }

    #[test]
    fn interim_engine_encodes_modified_named_keys() {
        let mut engine = super::InterimTextTerminalEngine::default();

        assert_eq!(
            engine.encode_key_input(super::KeyTerminalInput {
                key_name: "arrow-up",
                modifiers: 2,
                application_keypad: false,
                application_cursor: false,
            }),
            Some(b"\x1b[1;5A".to_vec())
        );
        assert_eq!(
            engine.encode_key_input(super::KeyTerminalInput {
                key_name: "delete",
                modifiers: 3,
                application_keypad: false,
                application_cursor: false,
            }),
            Some(b"\x1b[3;6~".to_vec())
        );
        assert_eq!(
            engine.encode_key_input(super::KeyTerminalInput {
                key_name: "tab",
                modifiers: 1,
                application_keypad: false,
                application_cursor: false,
            }),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            engine.encode_key_input(super::KeyTerminalInput {
                key_name: "enter",
                modifiers: 2,
                application_keypad: false,
                application_cursor: false,
            }),
            None
        );
    }

    #[test]
    fn interim_engine_encodes_sgr_mouse_input() {
        let mut engine = super::InterimTextTerminalEngine::default();

        assert_eq!(
            engine.encode_mouse_input(MouseTerminalInput {
                row: 0,
                col: 0,
                pixel_x: None,
                pixel_y: None,
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 0,
                mouse_format: protocol::MouseFormat::Sgr,
                cols: 80,
                rows: 24,
            }),
            Some(b"\x1b[<0;1;1M".to_vec())
        );
        assert_eq!(
            engine.encode_mouse_input(MouseTerminalInput {
                row: 4,
                col: 5,
                pixel_x: None,
                pixel_y: None,
                button: MouseButton::WheelDown,
                action: MouseAction::Press,
                modifiers: 2,
                mouse_format: protocol::MouseFormat::Sgr,
                cols: 80,
                rows: 24,
            }),
            Some(b"\x1b[<81;6;5M".to_vec())
        );
        assert_eq!(
            engine.encode_mouse_input(MouseTerminalInput {
                row: 0,
                col: 0,
                pixel_x: Some(33),
                pixel_y: Some(65),
                button: MouseButton::Left,
                action: MouseAction::Release,
                modifiers: 0,
                mouse_format: protocol::MouseFormat::SgrPixels,
                cols: 80,
                rows: 24,
            }),
            Some(b"\x1b[<0;33;65m".to_vec())
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_encodes_sgr_mouse_modifiers() {
        use super::{MouseAction, MouseButton, MouseTerminalInput};

        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        engine
            .apply_output(
                terminal_input(24, &empty, &empty),
                b"\x1b[?1000h\x1b[?1006h",
            )
            .expect("terminal update");

        let bytes = engine
            .encode_mouse_input(MouseTerminalInput {
                row: 0,
                col: 0,
                pixel_x: None,
                pixel_y: None,
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 2,
                mouse_format: protocol::MouseFormat::Sgr,
                cols: 80,
                rows: 24,
            })
            .expect("encoded mouse input");

        assert_eq!(bytes, b"\x1b[<16;1;1M");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_encodes_sgr_pixel_mouse_coordinates() {
        use super::{MouseAction, MouseButton, MouseTerminalInput};

        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        let update = engine
            .apply_output(
                terminal_input(24, &empty, &empty),
                b"\x1b[?1000h\x1b[?1006h\x1b[?1016h",
            )
            .expect("terminal update");
        assert_eq!(update.modes.mouse_format, protocol::MouseFormat::SgrPixels);

        let bytes = engine
            .encode_mouse_input(MouseTerminalInput {
                row: 0,
                col: 0,
                pixel_x: Some(1000),
                pixel_y: Some(2000),
                button: MouseButton::Left,
                action: MouseAction::Press,
                modifiers: 0,
                mouse_format: update.modes.mouse_format,
                cols: 80,
                rows: 24,
            })
            .expect("encoded mouse input");

        assert_eq!(bytes, b"\x1b[<0;1000;2000M");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_encodes_key_from_application_cursor_state() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        let update = engine
            .apply_output(terminal_input(24, &empty, &empty), b"\x1b[?1h")
            .expect("terminal update");
        assert!(update.modes.application_cursor);

        let bytes = engine
            .encode_key_input(super::KeyTerminalInput {
                key_name: "arrow-up",
                modifiers: 0,
                application_keypad: false,
                application_cursor: false,
            })
            .expect("encoded key input");

        assert_eq!(bytes, b"\x1bOA");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_preserves_keypad_fallback_from_application_state() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        let update = engine
            .apply_output(terminal_input(24, &empty, &empty), b"\x1b=")
            .expect("terminal update");
        assert!(update.modes.application_keypad);

        let bytes = engine
            .encode_key_input(super::KeyTerminalInput {
                key_name: "numpad-enter",
                modifiers: 0,
                application_keypad: update.modes.application_keypad,
                application_cursor: false,
            })
            .expect("encoded keypad input");

        assert_eq!(bytes, b"\x1bOM");
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_encodes_modified_named_key() {
        use libghostty_vt::{
            Terminal, TerminalOptions,
            key::{Action, Encoder, Event, Key, Mods},
        };

        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        engine
            .apply_output(terminal_input(24, &empty, &empty), b"")
            .expect("terminal update");
        let bytes = engine
            .encode_key_input(super::KeyTerminalInput {
                key_name: "arrow-up",
                modifiers: 2,
                application_keypad: false,
                application_cursor: false,
            })
            .expect("encoded modified key input");

        let terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut encoder = Encoder::new().expect("key encoder");
        encoder.set_options_from_terminal(&terminal);
        let mut event = Event::new().expect("key event");
        event
            .set_action(Action::Press)
            .set_key(Key::ArrowUp)
            .set_mods(Mods::CTRL);
        let mut expected = Vec::new();
        encoder
            .encode_to_vec(&event, &mut expected)
            .expect("direct key encoding");

        assert_eq!(bytes, expected);
        assert!(!bytes.is_empty());
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_rejects_unknown_named_key() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        engine
            .apply_output(terminal_input(24, &empty, &empty), b"")
            .expect("terminal update");

        assert_eq!(
            engine.encode_key_input(super::KeyTerminalInput {
                key_name: "f13",
                modifiers: 0,
                application_keypad: false,
                application_cursor: false,
            }),
            None
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
            .apply_output(terminal_input_from_update(&first), b"\x1b[1;3H")
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
                shape: protocol::CursorShape::Block,
                blinking: false
            }
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_uses_replace_rows_for_row_run_change_with_cursor_movement() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        let first = engine
            .apply_output(terminal_input(2, &empty, &empty), b"input")
            .expect("initial terminal update");

        let row_run_change = engine
            .apply_output(
                terminal_input_from_update(&first),
                b"\r\x1b]133;B\x1b\\input\x1b[1;1H",
            )
            .expect("row run and cursor update");

        assert_eq!(row_run_change.patch_kind, protocol::PatchKind::ReplaceRows);
        assert_eq!(row_run_change.surface_lines, first.surface_lines);
        assert_ne!(row_run_change.cursor, first.cursor);
        assert_ne!(row_run_change.surface_row_runs, first.surface_row_runs);
        assert_eq!(
            row_run_change.surface_row_runs[0][0].semantic_content,
            protocol::CellSemanticContent::Input
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
                blinking: first.cursor.blinking,
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
                blinking: first.cursor.blinking,
            }
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_emits_cursor_only_patch_for_cursor_blinking() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();
        let first = engine
            .apply_output(terminal_input(2, &empty, &empty), b"alpha\r\nbeta")
            .expect("initial terminal update");

        assert!(!first.cursor.blinking);

        let blink_on = engine
            .apply_output(terminal_input_from_update(&first), b"\x1b[?12h")
            .expect("cursor blink update");

        assert_eq!(blink_on.patch_kind, protocol::PatchKind::CursorOnly);
        assert_eq!(blink_on.surface_lines, first.surface_lines);
        assert_eq!(blink_on.scrollback_lines, first.scrollback_lines);
        assert_eq!(
            blink_on.cursor,
            TerminalCursor {
                blinking: true,
                ..first.cursor
            }
        );
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_render_state_tracks_cursor_blinking() {
        use libghostty_vt::{RenderState, Terminal, TerminalOptions};

        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut render_state = RenderState::new().expect("render state");

        terminal.vt_write(b"\x1b[?12l");
        {
            let snapshot = render_state.update(&terminal).expect("snapshot");
            assert!(!snapshot.cursor_blinking().expect("cursor blink off"));
        }

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
                terminal_input_from_update(&primary),
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
            .apply_output(terminal_input_from_update(&alternate), b"\x1b[?1049l")
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
    fn libghostty_vt_engine_withholds_alternate_screen_scrollback() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let primary = engine
            .apply_output(
                terminal_input(2, &empty, &empty),
                b"main-0\r\nmain-1\r\nmain-2",
            )
            .expect("primary update");
        assert_eq!(primary.surface, protocol::SurfaceKind::Main);
        assert!(
            primary
                .scrollback_lines
                .iter()
                .any(|line| line.contains("main-0")),
            "primary output should create main scrollback: {:?}",
            primary.scrollback_lines
        );

        let alternate = engine
            .apply_output(
                terminal_input_from_update(&primary),
                b"\x1b[?1049halt-0\r\nalt-1\r\nalt-2",
            )
            .expect("alternate update");

        assert_eq!(alternate.surface, protocol::SurfaceKind::Alternate);
        assert_eq!(alternate.scrollback_lines, primary.scrollback_lines);
        assert!(
            alternate
                .scrollback_lines
                .iter()
                .all(|line| !line.contains("alt-")),
            "alternate output leaked into main scrollback: {:?}",
            alternate.scrollback_lines
        );

        let restored = engine
            .apply_output(terminal_input_from_update(&alternate), b"\x1b[?1049l")
            .expect("restore update");
        assert_eq!(restored.surface, protocol::SurfaceKind::Main);
        assert_eq!(restored.scrollback_lines, primary.scrollback_lines);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn libghostty_vt_engine_preserves_styled_scrollback_while_alternate_screen_is_active() {
        let mut engine = super::ghostty_vt::LibghosttyVtTerminalEngine::new();
        let empty = Vec::new();

        let primary = engine
            .apply_output(
                terminal_input_with_size(20, 2, &empty, &empty),
                b"\x1b[31mmain-red\x1b[0m\r\nmain-plain\r\nmain-tail",
            )
            .expect("primary update");
        let styled_run = primary
            .scrollback_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("main-red"))
            .expect("styled main scrollback run");
        assert_ne!(
            styled_run.style_id, 0,
            "primary scrollback should contain styled runs"
        );

        let alternate = engine
            .apply_output(
                terminal_input_from_update(&primary),
                b"\x1b[?1049halt-red\r\nalt-tail",
            )
            .expect("alternate update");

        assert_eq!(alternate.surface, protocol::SurfaceKind::Alternate);
        assert_eq!(alternate.scrollback_lines, primary.scrollback_lines);
        assert_eq!(
            alternate.scrollback_row_runs, primary.scrollback_row_runs,
            "alternate screen should preserve structured main scrollback runs"
        );
        let preserved_run = alternate
            .scrollback_row_runs
            .iter()
            .flat_map(|row| row.iter())
            .find(|run| run.text.contains("main-red"))
            .expect("preserved styled main scrollback run");
        let preserved_style = alternate
            .styles
            .get(preserved_run.style_id as usize)
            .expect("preserved style table entry");
        assert_ne!(
            preserved_style.fg_rgba, 0,
            "preserved scrollback run should still reference a style table entry"
        );
        assert!(
            alternate
                .scrollback_lines
                .iter()
                .all(|line| !line.contains("alt-")),
            "alternate output leaked into main scrollback: {:?}",
            alternate.scrollback_lines
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
