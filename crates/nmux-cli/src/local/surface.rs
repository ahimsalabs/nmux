use flatbuffers::{self, ForwardsUOffset, Vector};
use nmux_proto::protocol;

pub(super) fn decoded_surface_rows<F>(len: usize, mut row: F) -> Vec<SurfaceRowUpdate>
where
    F: FnMut(usize) -> SurfaceRowUpdate,
{
    (0..len).map(&mut row).collect()
}

pub(super) fn decoded_surface_row(
    row: u32,
    runs: Option<Vector<'_, ForwardsUOffset<protocol::CellRun<'_>>>>,
    dirty_hash: u64,
    row_state_hash: u64,
    semantic_prompt: protocol::RowSemanticPrompt,
    dirty: bool,
    kitty_virtual_placeholder: bool,
) -> SurfaceRowUpdate {
    let runs = runs.map(decoded_cell_runs).unwrap_or_default();
    SurfaceRowUpdate {
        row,
        text: render_run_summaries(&runs),
        runs,
        dirty_hash,
        row_state_hash,
        semantic_prompt,
        dirty,
        kitty_virtual_placeholder,
    }
}

pub(super) fn decoded_cell_runs(
    runs: Vector<'_, ForwardsUOffset<protocol::CellRun<'_>>>,
) -> Vec<CellRunSummary> {
    let mut decoded = Vec::with_capacity(runs.len());
    for run_index in 0..runs.len() {
        let run = runs.get(run_index);
        let cell_widths = run
            .cell_widths()
            .map(|widths| (0..widths.len()).map(|index| widths.get(index)).collect())
            .unwrap_or_default();
        decoded.push(CellRunSummary {
            text: run.text_utf8().unwrap_or_default().to_owned(),
            cell_widths,
            style_id: run.style_id(),
            flags: run.flags(),
            hyperlink_id: run.hyperlink_id(),
            semantic_content: run.semantic_content(),
        });
    }
    decoded
}

pub(super) fn decoded_styles(
    styles: Vector<'_, ForwardsUOffset<protocol::Style<'_>>>,
) -> Vec<StyleSummary> {
    let mut decoded = Vec::with_capacity(styles.len());
    for style_index in 0..styles.len() {
        let style = styles.get(style_index);
        decoded.push(StyleSummary {
            fg_rgba: style.fg_rgba(),
            bg_rgba: style.bg_rgba(),
            underline_rgba: style.underline_rgba(),
            flags: style.flags(),
        });
    }
    decoded
}

pub(super) fn decoded_hyperlinks(
    hyperlinks: Vector<'_, ForwardsUOffset<protocol::Hyperlink<'_>>>,
) -> Result<Vec<HyperlinkSummary>, Box<dyn std::error::Error>> {
    let mut decoded = Vec::with_capacity(hyperlinks.len());
    for hyperlink_index in 0..hyperlinks.len() {
        let hyperlink = hyperlinks.get(hyperlink_index);
        let uri = hyperlink.uri().unwrap_or_default().to_owned();
        decoded.push(HyperlinkSummary {
            id: hyperlink.id(),
            uri,
            osc8_id: hyperlink.osc8_id().unwrap_or_default().to_owned(),
            params: hyperlink.params().unwrap_or_default().to_owned(),
        });
    }
    validate_hyperlink_table(&decoded)?;
    Ok(decoded)
}

pub(super) fn decoded_terminal_colors(
    colors: Option<protocol::TerminalColorState<'_>>,
) -> Option<TerminalColorSummary> {
    let colors = colors?;
    let palette_diff_rgba: Vec<u32> = colors
        .palette_diff_rgba()
        .map(|palette| (0..palette.len()).map(|index| palette.get(index)).collect())
        .unwrap_or_default();
    let palette_diff_start = if palette_diff_rgba.is_empty() {
        None
    } else {
        Some(colors.palette_diff_start())
    };
    Some(TerminalColorSummary {
        default_fg_rgba: colors.default_fg_rgba(),
        default_bg_rgba: colors.default_bg_rgba(),
        cursor_rgba: colors.cursor_rgba(),
        cursor_rgba_set: colors.cursor_rgba_set(),
        palette_rgba: colors
            .palette_rgba()
            .map(|palette| (0..palette.len()).map(|index| palette.get(index)).collect())
            .unwrap_or_default(),
        palette_diff_start,
        palette_diff_rgba,
    })
}

pub(super) fn validate_palette_diff_scope(
    kind: SurfaceUpdateKind,
    patch_kind: Option<protocol::PatchKind>,
    colors: Option<&TerminalColorSummary>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(colors) = colors else {
        return Ok(());
    };
    if !terminal_colors_have_palette_diff(colors) {
        return Ok(());
    }
    match kind {
        SurfaceUpdateKind::Snapshot => Err("surface snapshot cannot carry palette diff".into()),
        SurfaceUpdateKind::Patch if patch_kind == Some(protocol::PatchKind::ColorOnly) => Ok(()),
        SurfaceUpdateKind::Patch => Err("non-color surface patch cannot carry palette diff".into()),
    }
}

pub(super) fn terminal_colors_have_palette_diff(colors: &TerminalColorSummary) -> bool {
    colors.palette_diff_start.is_some() || !colors.palette_diff_rgba.is_empty()
}

pub(super) fn validate_attach_surface_update(
    status: &super::AttachStatusSummary,
    update: &SurfaceUpdate,
) -> Result<(), Box<dyn std::error::Error>> {
    if update.pane_id != status.pane_id {
        return Err(format!(
            "attach surface pane_id {} does not match attach status pane_id {}",
            update.pane_id, status.pane_id
        )
        .into());
    }

    match (status.surface_state, update.kind) {
        (protocol::AttachSurfaceState::Snapshot, SurfaceUpdateKind::Snapshot)
        | (protocol::AttachSurfaceState::Patch, SurfaceUpdateKind::Patch) => Ok(()),
        (protocol::AttachSurfaceState::Current, _) => {
            Err("attach status Current cannot include a surface frame".into())
        }
        (state, kind) => {
            Err(format!("attach status {state:?} does not match surface update {kind:?}").into())
        }
    }
}

pub(super) fn required_string(
    value: Option<&str>,
    field: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let value = value.ok_or_else(|| format!("missing {field}"))?;
    if value.is_empty() {
        return Err(format!("empty {field}").into());
    }
    Ok(value.to_owned())
}

pub(super) fn default_style_summaries() -> Vec<StyleSummary> {
    vec![StyleSummary {
        fg_rgba: 0,
        bg_rgba: 0,
        underline_rgba: 0,
        flags: 0,
    }]
}

pub(super) fn render_decoded_rows(rows: &[SurfaceRowUpdate]) -> String {
    let mut rendered = String::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        rendered.push_str(&row.text);
    }
    rendered
}

pub(super) fn render_run_summaries(runs: &[CellRunSummary]) -> String {
    let mut rendered = String::new();
    for run in runs {
        rendered.push_str(&run.text);
    }
    rendered
}

/// Convert a structured style into an ANSI SGR escape sequence.
/// Style flags: bit 0=bold, 1=italic, 2=faint, 3=blink, 4=inverse,
/// 5=invisible, 6=strikethrough, 7=overline, 8..12=underline variants.
/// RGBA colors are packed as `[R, G, B, 0xFF]` in big-endian u32.
pub(super) fn style_to_sgr(style: &StyleSummary) -> String {
    let mut params = Vec::new();
    let flags = style.flags;
    if flags & (1 << 0) != 0 {
        params.push("1".to_owned());
    }
    if flags & (1 << 2) != 0 {
        params.push("2".to_owned());
    }
    if flags & (1 << 1) != 0 {
        params.push("3".to_owned());
    }
    // Underline variants: bit 8=single, 9=double, 10=curly, 11=dotted, 12=dashed
    if flags & (1 << 8) != 0 {
        params.push("4".to_owned());
    } else if flags & (1 << 9) != 0 {
        params.push("21".to_owned());
    } else if flags & (0x1f << 10) != 0 {
        // Curly/dotted/dashed — use SGR 4:3/4:4/4:5 if terminal supports it,
        // fall back to single underline for broad compatibility.
        params.push("4".to_owned());
    }
    if flags & (1 << 3) != 0 {
        params.push("5".to_owned());
    }
    if flags & (1 << 4) != 0 {
        params.push("7".to_owned());
    }
    if flags & (1 << 5) != 0 {
        params.push("8".to_owned());
    }
    if flags & (1 << 6) != 0 {
        params.push("9".to_owned());
    }
    if flags & (1 << 7) != 0 {
        params.push("53".to_owned());
    }
    if style.fg_rgba != 0 {
        let [r, g, b, _] = style.fg_rgba.to_be_bytes();
        params.push(format!("38;2;{r};{g};{b}"));
    }
    if style.bg_rgba != 0 {
        let [r, g, b, _] = style.bg_rgba.to_be_bytes();
        params.push(format!("48;2;{r};{g};{b}"));
    }
    if params.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", params.join(";"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedSurfaceSummary {
    pub pane_id: String,
    pub version: u64,
    pub scrollback_version: u64,
    pub scrollback_total_lines: u64,
    pub cols: u32,
    pub rows: u32,
    pub colors: TerminalColorSummary,
    pub styles: Vec<StyleSummary>,
    pub hyperlinks: Vec<HyperlinkSummary>,
    pub row_updates: Vec<SurfaceRowUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalMetadataSummary {
    pub title: String,
    pub working_directory: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedSurfaceSummary {
    pub surface_kind: protocol::SurfaceKind,
    pub cursor: Option<CursorSummary>,
    pub modes: TerminalModeSummary,
}

impl TerminalMetadataSummary {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.working_directory.is_empty()
    }

    pub fn display_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.title.is_empty() {
            lines.push(format!("title={}", sanitized_metadata_value(&self.title)));
        }
        if !self.working_directory.is_empty() {
            lines.push(format!(
                "working-directory={}",
                sanitized_metadata_value(&self.working_directory)
            ));
        }
        lines
    }

    pub(super) fn from_surface(surface: &ClientPaneSurface) -> Self {
        Self {
            title: surface.title.clone(),
            working_directory: surface.working_directory.clone(),
        }
    }
}

fn sanitized_metadata_value(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch == '\t' || !ch.is_control() {
                ch
            } else {
                '\u{fffd}'
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceUpdate {
    pub kind: SurfaceUpdateKind,
    pub pane_id: String,
    pub version: u64,
    pub scrollback_version: u64,
    pub scrollback_total_lines: u64,
    pub base_version: Option<u64>,
    pub patch_kind: Option<protocol::PatchKind>,
    pub cols: Option<u32>,
    pub rows: Option<u32>,
    pub surface: Option<protocol::SurfaceKind>,
    pub cursor: Option<CursorSummary>,
    pub modes: TerminalModeSummary,
    pub title: String,
    pub working_directory: String,
    pub colors: Option<TerminalColorSummary>,
    pub row_updates: Vec<SurfaceRowUpdate>,
    pub styles: Vec<StyleSummary>,
    pub hyperlinks: Vec<HyperlinkSummary>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceUpdateKind {
    Snapshot,
    Patch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorSummary {
    pub row: u32,
    pub col: u32,
    pub visible: bool,
    pub shape: protocol::CursorShape,
    pub blinking: bool,
}

impl CursorSummary {
    pub(super) fn from_protocol(cursor: protocol::CursorState<'_>) -> Self {
        Self {
            row: cursor.row(),
            col: cursor.col(),
            visible: cursor.visible(),
            shape: cursor.shape(),
            blinking: cursor.blinking(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalModeSummary {
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalColorSummary {
    pub default_fg_rgba: u32,
    pub default_bg_rgba: u32,
    pub cursor_rgba: u32,
    pub cursor_rgba_set: bool,
    pub palette_rgba: Vec<u32>,
    pub palette_diff_start: Option<u32>,
    pub palette_diff_rgba: Vec<u32>,
}

impl TerminalColorSummary {
    fn materialize(&self, base: &Self) -> Result<Self, Box<dyn std::error::Error>> {
        let mut colors = self.clone();
        if let Some(start) = self.palette_diff_start {
            let start = usize::try_from(start)?;
            if start > base.palette_rgba.len() {
                return Err(format!(
                    "palette diff start {start} exceeds cached palette length {}",
                    base.palette_rgba.len()
                )
                .into());
            }
            colors.palette_rgba = base.palette_rgba[..start].to_vec();
            colors
                .palette_rgba
                .extend(self.palette_diff_rgba.iter().copied());
        } else if self.palette_rgba.is_empty() {
            colors.palette_rgba = base.palette_rgba.clone();
        }
        colors.palette_diff_start = None;
        colors.palette_diff_rgba.clear();
        Ok(colors)
    }
}

impl Default for TerminalModeSummary {
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

impl TerminalModeSummary {
    pub(super) fn from_protocol(modes: protocol::TerminalModeState<'_>) -> Self {
        Self {
            bracketed_paste: modes.bracketed_paste(),
            mouse_tracking: modes.mouse_tracking(),
            focus_reporting: modes.focus_reporting(),
            application_keypad: modes.application_keypad(),
            application_cursor: modes.application_cursor(),
            origin: modes.origin(),
            wraparound: modes.wraparound(),
            mouse_tracking_mode: modes.mouse_tracking_mode(),
            mouse_format: modes.mouse_format(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceRowUpdate {
    pub row: u32,
    pub text: String,
    pub runs: Vec<CellRunSummary>,
    pub dirty_hash: u64,
    pub row_state_hash: u64,
    pub semantic_prompt: protocol::RowSemanticPrompt,
    pub dirty: bool,
    pub kitty_virtual_placeholder: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRunSummary {
    pub text: String,
    pub cell_widths: Vec<u8>,
    pub style_id: u32,
    pub flags: u32,
    pub hyperlink_id: u32,
    pub semantic_content: protocol::CellSemanticContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleSummary {
    pub fg_rgba: u32,
    pub bg_rgba: u32,
    pub underline_rgba: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperlinkSummary {
    pub id: u32,
    pub uri: String,
    pub osc8_id: String,
    pub params: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPaneSurface {
    pub pane_id: String,
    pub version: u64,
    pub scrollback_version: u64,
    pub scrollback_total_lines: u64,
    pub cols: u32,
    pub rows: u32,
    pub surface: protocol::SurfaceKind,
    pub cursor: Option<CursorSummary>,
    pub modes: TerminalModeSummary,
    pub title: String,
    pub working_directory: String,
    pub colors: TerminalColorSummary,
    pub(super) styles: Vec<StyleSummary>,
    pub(super) hyperlinks: Vec<HyperlinkSummary>,
    pub(super) row_text: Vec<String>,
    pub(super) row_runs: Vec<Vec<CellRunSummary>>,
    pub(super) row_dirty_hashes: Vec<u64>,
    pub(super) row_semantic_prompts: Vec<protocol::RowSemanticPrompt>,
    pub(super) row_dirty: Vec<bool>,
    pub(super) row_kitty_placeholders: Vec<bool>,
    pub(super) row_state_hashes: Vec<u64>,
}

impl ClientPaneSurface {
    pub fn from_snapshot(update: &SurfaceUpdate) -> Result<Self, Box<dyn std::error::Error>> {
        validate_palette_diff_scope(SurfaceUpdateKind::Snapshot, None, update.colors.as_ref())?;
        let mut surface = Self {
            pane_id: update.pane_id.clone(),
            version: update.version,
            scrollback_version: update.scrollback_version,
            scrollback_total_lines: update.scrollback_total_lines,
            cols: update.cols.ok_or("surface snapshot missing cols")?,
            rows: update.rows.ok_or("surface snapshot missing rows")?,
            surface: update.surface.unwrap_or(protocol::SurfaceKind::Main),
            cursor: update.cursor,
            modes: update.modes,
            title: update.title.clone(),
            working_directory: update.working_directory.clone(),
            colors: update.colors.clone().unwrap_or_default(),
            styles: if update.styles.is_empty() {
                default_style_summaries()
            } else {
                update.styles.clone()
            },
            hyperlinks: update.hyperlinks.clone(),
            row_text: Vec::new(),
            row_runs: Vec::new(),
            row_dirty_hashes: Vec::new(),
            row_semantic_prompts: Vec::new(),
            row_dirty: Vec::new(),
            row_kitty_placeholders: Vec::new(),
            row_state_hashes: Vec::new(),
        };
        validate_surface_kind(surface.surface)?;
        validate_cursor_summary(surface.cursor)?;
        validate_terminal_mode_summary(surface.modes)?;
        validate_hyperlink_table(&surface.hyperlinks)?;
        let row_count =
            usize::try_from(surface.rows).map_err(|_| "surface row count does not fit in usize")?;
        surface.row_text.resize(row_count, String::new());
        surface.row_runs.resize(row_count, Vec::new());
        surface.row_dirty_hashes.resize(row_count, 0);
        surface
            .row_semantic_prompts
            .resize(row_count, protocol::RowSemanticPrompt::None);
        surface.row_dirty.resize(row_count, false);
        surface.row_kitty_placeholders.resize(row_count, false);
        surface.row_state_hashes.resize(row_count, 0);
        validate_row_update_indices(&update.row_updates, row_count)?;
        validate_row_update_style_ids(&update.row_updates, &surface.styles)?;
        validate_row_update_hyperlink_ids(&update.row_updates, &surface.hyperlinks)?;
        surface.apply_rows(&update.row_updates)?;
        Ok(surface)
    }

    pub fn apply_update(
        &mut self,
        update: &SurfaceUpdate,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match update.kind {
            SurfaceUpdateKind::Snapshot => {
                *self = Self::from_snapshot(update)?;
                Ok(())
            }
            SurfaceUpdateKind::Patch => self.apply_patch(update),
        }
    }

    pub fn apply_patch(
        &mut self,
        update: &SurfaceUpdate,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if update.pane_id != self.pane_id {
            return Err(format!(
                "surface patch pane mismatch: expected {}, got {}",
                self.pane_id, update.pane_id
            )
            .into());
        }
        if update.base_version != Some(self.version) {
            return Err(format!(
                "surface patch base version mismatch: expected {}, got {:?}",
                self.version, update.base_version
            )
            .into());
        }
        if let Some(patch_kind) = update.patch_kind {
            validate_patch_kind(patch_kind)?;
        }
        validate_palette_diff_scope(update.kind, update.patch_kind, update.colors.as_ref())?;
        validate_cursor_summary(update.cursor)?;
        validate_terminal_mode_summary(update.modes)?;
        if !update.hyperlinks.is_empty() {
            return Err("surface patch cannot change hyperlink table".into());
        }
        if let Some(patch_kind) = update.patch_kind {
            validate_no_row_patch_payload(patch_kind, &update.row_updates)?;
        }
        if update.patch_kind == Some(protocol::PatchKind::FullRefreshRequired) {
            return Err("surface patch requires full refresh".into());
        }
        if update.patch_kind == Some(protocol::PatchKind::CursorOnly) {
            if let Some(colors) = update.colors.as_ref()
                && colors != &self.colors
            {
                return Err("cursor-only patch changes terminal colors".into());
            }
            self.cursor = update.cursor;
            self.title = update.title.clone();
            self.working_directory = update.working_directory.clone();
            self.version = update.version;
            self.scrollback_version = update.scrollback_version;
            self.scrollback_total_lines = update.scrollback_total_lines;
            return Ok(());
        }
        if update.patch_kind == Some(protocol::PatchKind::ModeOnly) {
            if let Some(colors) = update.colors.as_ref()
                && colors != &self.colors
            {
                return Err("mode-only patch changes terminal colors".into());
            }
            self.cursor = update.cursor;
            self.modes = update.modes;
            self.title = update.title.clone();
            self.working_directory = update.working_directory.clone();
            self.version = update.version;
            self.scrollback_version = update.scrollback_version;
            self.scrollback_total_lines = update.scrollback_total_lines;
            return Ok(());
        }
        if update.patch_kind == Some(protocol::PatchKind::ColorOnly) {
            let Some(colors) = update.colors.as_ref() else {
                return Err("color-only patch is missing terminal colors".into());
            };
            let colors = colors.materialize(&self.colors)?;
            self.cursor = update.cursor;
            self.colors = colors;
            self.modes = update.modes;
            self.title = update.title.clone();
            self.working_directory = update.working_directory.clone();
            self.version = update.version;
            self.scrollback_version = update.scrollback_version;
            self.scrollback_total_lines = update.scrollback_total_lines;
            return Ok(());
        }
        if update.patch_kind != Some(protocol::PatchKind::ReplaceRows) {
            return Err(format!("unsupported surface patch kind: {:?}", update.patch_kind).into());
        }
        if let Some(colors) = update.colors.as_ref()
            && colors != &self.colors
        {
            return Err("replace-rows patch changes terminal colors".into());
        }
        validate_row_update_terminal_enums(&update.row_updates)?;
        validate_row_update_indices(&update.row_updates, self.row_text.len())?;
        validate_row_update_style_ids(&update.row_updates, &self.styles)?;
        validate_row_update_hyperlink_ids(&update.row_updates, &self.hyperlinks)?;
        self.apply_rows(&update.row_updates)?;
        self.cursor = update.cursor;
        self.modes = update.modes;
        self.title = update.title.clone();
        self.working_directory = update.working_directory.clone();
        self.version = update.version;
        self.scrollback_version = update.scrollback_version;
        self.scrollback_total_lines = update.scrollback_total_lines;
        Ok(())
    }

    pub fn render_text(&self) -> String {
        let visible_rows = self.visible_row_count();
        self.row_text[..visible_rows].join("\n")
    }

    /// Render visible rows with ANSI SGR styling derived from structured cell
    /// runs and the style table. The client never parses raw VT bytes; it
    /// reconstructs styled output from the protocol's structured style objects.
    pub fn render_styled_text(&self) -> String {
        let visible_rows = self.visible_row_count();
        let mut output = String::new();
        for row_index in 0..visible_rows {
            if row_index > 0 {
                output.push('\n');
            }
            output.push_str(&self.render_styled_row(row_index));
        }
        output
    }

    pub(crate) fn render_styled_row(&self, row_index: usize) -> String {
        let runs = &self.row_runs[row_index];
        if runs.is_empty() {
            return self.row_text[row_index].clone();
        }
        let mut output = String::new();
        for run in runs {
            let style = self.styles.get(run.style_id as usize);
            let needs_sgr = style.is_some_and(|s| s.fg_rgba != 0 || s.bg_rgba != 0 || s.flags != 0);
            if needs_sgr && let Some(style) = style {
                output.push_str(&style_to_sgr(style));
            }
            output.push_str(&run.text);
            if needs_sgr {
                output.push_str("\x1b[0m");
            }
        }
        output
    }

    /// Returns true when the style table contains any non-default styles,
    /// indicating that the terminal engine produced structured style data.
    pub fn has_styled_runs(&self) -> bool {
        self.styles.len() > 1
            || self
                .styles
                .first()
                .is_some_and(|s| s.fg_rgba != 0 || s.bg_rgba != 0 || s.flags != 0)
    }

    pub(crate) fn visible_row_count(&self) -> usize {
        self.row_text
            .iter()
            .rposition(|row| !row.is_empty())
            .map(|index| index + 1)
            .unwrap_or(0)
    }

    pub(crate) fn row_text(&self) -> &[String] {
        &self.row_text
    }

    pub(super) fn summary(&self) -> RenderedSurfaceSummary {
        let row_updates = (0..self.visible_row_count())
            .map(|index| SurfaceRowUpdate {
                row: u32::try_from(index).expect("surface row index fits in u32"),
                text: self.row_text[index].clone(),
                runs: self.row_runs[index].clone(),
                dirty_hash: self.row_dirty_hashes[index],
                row_state_hash: self.row_state_hashes[index],
                semantic_prompt: self.row_semantic_prompts[index],
                dirty: self.row_dirty[index],
                kitty_virtual_placeholder: self.row_kitty_placeholders[index],
            })
            .collect();
        RenderedSurfaceSummary {
            pane_id: self.pane_id.clone(),
            version: self.version,
            scrollback_version: self.scrollback_version,
            scrollback_total_lines: self.scrollback_total_lines,
            cols: self.cols,
            rows: self.rows,
            colors: self.colors.clone(),
            styles: self.styles.clone(),
            hyperlinks: self.hyperlinks.clone(),
            row_updates,
        }
    }

    fn apply_rows(&mut self, rows: &[SurfaceRowUpdate]) -> Result<(), Box<dyn std::error::Error>> {
        for row in rows {
            let index =
                usize::try_from(row.row).map_err(|_| "surface row index does not fit in usize")?;
            let Some(target) = self.row_text.get_mut(index) else {
                return Err(format!(
                    "surface row {} is outside {} row surface",
                    row.row, self.rows
                )
                .into());
            };
            *target = row.text.clone();
            self.row_runs[index] = if row.runs.is_empty() {
                vec![CellRunSummary::plain(&row.text)]
            } else {
                row.runs.clone()
            };
            self.row_dirty_hashes[index] = row.dirty_hash;
            self.row_semantic_prompts[index] = row.semantic_prompt;
            self.row_dirty[index] = row.dirty;
            self.row_kitty_placeholders[index] = row.kitty_virtual_placeholder;
            self.row_state_hashes[index] = row.row_state_hash;
        }
        Ok(())
    }
}

impl CellRunSummary {
    pub(super) fn plain(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            cell_widths: text.chars().map(|_| 1).collect(),
            style_id: 0,
            flags: 0,
            hyperlink_id: 0,
            semantic_content: protocol::CellSemanticContent::Output,
        }
    }
}

pub(super) fn row_runs_for_text(
    row_text: &[String],
    mut row_runs: Vec<Vec<CellRunSummary>>,
) -> Vec<Vec<CellRunSummary>> {
    for (index, text) in row_text.iter().enumerate() {
        if row_runs[index].is_empty() || render_run_summaries(&row_runs[index]) != *text {
            row_runs[index] = vec![CellRunSummary::plain(text)];
        }
    }
    row_runs
}

pub(super) fn validate_row_update_hyperlink_ids(
    rows: &[SurfaceRowUpdate],
    hyperlinks: &[HyperlinkSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for row in rows {
        validate_cell_run_hyperlink_ids(&row.runs, hyperlinks)?;
    }
    Ok(())
}

pub(super) fn validate_no_row_patch_payload(
    patch_kind: protocol::PatchKind,
    rows: &[SurfaceRowUpdate],
) -> Result<(), Box<dyn std::error::Error>> {
    if !rows.is_empty()
        && matches!(
            patch_kind,
            protocol::PatchKind::CursorOnly
                | protocol::PatchKind::ModeOnly
                | protocol::PatchKind::ColorOnly
                | protocol::PatchKind::FullRefreshRequired
        )
    {
        return Err(format!("{patch_kind:?} patch cannot carry row updates").into());
    }
    Ok(())
}

pub(super) fn validate_patch_kind(
    patch_kind: protocol::PatchKind,
) -> Result<(), Box<dyn std::error::Error>> {
    if patch_kind.variant_name().is_some() {
        Ok(())
    } else {
        Err(format!("unknown surface patch kind {}", patch_kind.0).into())
    }
}

pub(super) fn validate_surface_kind(
    surface: protocol::SurfaceKind,
) -> Result<(), Box<dyn std::error::Error>> {
    if surface.variant_name().is_some() {
        Ok(())
    } else {
        Err(format!("unknown surface kind {}", surface.0).into())
    }
}

pub(super) fn validate_cursor_summary(
    cursor: Option<CursorSummary>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(cursor) = cursor
        && cursor.shape.variant_name().is_none()
    {
        return Err(format!("unknown cursor shape {}", cursor.shape.0).into());
    }
    Ok(())
}

pub(super) fn validate_terminal_mode_summary(
    modes: TerminalModeSummary,
) -> Result<(), Box<dyn std::error::Error>> {
    if modes.mouse_tracking_mode.variant_name().is_none() {
        return Err(format!(
            "unknown mouse tracking mode {}",
            modes.mouse_tracking_mode.0
        )
        .into());
    }
    if modes.mouse_format.variant_name().is_none() {
        return Err(format!("unknown mouse format {}", modes.mouse_format.0).into());
    }
    Ok(())
}

pub(super) fn validate_row_update_indices(
    rows: &[SurfaceRowUpdate],
    row_count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut seen = vec![false; row_count];
    for row in rows {
        let index =
            usize::try_from(row.row).map_err(|_| "surface row index does not fit in usize")?;
        let Some(seen_row) = seen.get_mut(index) else {
            return Err(
                format!("surface row {} is outside {row_count} row surface", row.row).into(),
            );
        };
        if *seen_row {
            return Err(format!("surface row {} is repeated in one update", row.row).into());
        }
        *seen_row = true;
    }
    Ok(())
}

pub(super) fn validate_row_update_terminal_enums(
    rows: &[SurfaceRowUpdate],
) -> Result<(), Box<dyn std::error::Error>> {
    for row in rows {
        validate_row_semantic_prompt(row.semantic_prompt)?;
        validate_cell_run_semantic_content(&row.runs)?;
    }
    Ok(())
}

pub(super) fn validate_row_semantic_prompt(
    prompt: protocol::RowSemanticPrompt,
) -> Result<(), Box<dyn std::error::Error>> {
    if prompt.variant_name().is_some() {
        Ok(())
    } else {
        Err(format!("unknown row semantic prompt {}", prompt.0).into())
    }
}

pub(super) fn validate_cell_run_semantic_content(
    runs: &[CellRunSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for run in runs {
        if run.semantic_content.variant_name().is_none() {
            return Err(format!(
                "cell run references unknown semantic content {}",
                run.semantic_content.0
            )
            .into());
        }
    }
    Ok(())
}

pub(super) fn validate_row_update_style_ids(
    rows: &[SurfaceRowUpdate],
    styles: &[StyleSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for row in rows {
        validate_cell_run_style_ids(&row.runs, styles)?;
    }
    Ok(())
}

pub(super) fn validate_cell_run_style_ids(
    runs: &[CellRunSummary],
    styles: &[StyleSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    let style_count = styles.len().max(1);
    for run in runs {
        let style_id = usize::try_from(run.style_id)
            .map_err(|_| format!("cell run style_id {} is too large", run.style_id))?;
        if style_id >= style_count {
            return Err(format!("cell run references unknown style_id {}", run.style_id).into());
        }
    }
    Ok(())
}

pub(super) fn validate_cell_run_hyperlink_ids(
    runs: &[CellRunSummary],
    hyperlinks: &[HyperlinkSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for run in runs {
        if run.hyperlink_id != 0
            && !hyperlinks
                .iter()
                .any(|hyperlink| hyperlink.id == run.hyperlink_id)
        {
            return Err(format!(
                "cell run references unknown hyperlink_id {}",
                run.hyperlink_id
            )
            .into());
        }
    }
    Ok(())
}

pub(super) fn validate_hyperlink_table(
    hyperlinks: &[HyperlinkSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for (index, hyperlink) in hyperlinks.iter().enumerate() {
        if hyperlink.id == 0 {
            return Err("hyperlink table entry uses reserved id 0".into());
        }
        if hyperlink.uri.is_empty() {
            return Err(format!("hyperlink {} is missing target uri", hyperlink.id).into());
        }
        if hyperlinks[..index]
            .iter()
            .any(|existing| existing.id == hyperlink.id)
        {
            return Err(format!("duplicate hyperlink id {}", hyperlink.id).into());
        }
    }
    Ok(())
}

pub(super) fn validate_cached_row_hyperlink_ids(
    row_runs: &[Vec<CellRunSummary>],
    hyperlinks: &[HyperlinkSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for runs in row_runs {
        validate_cell_run_hyperlink_ids(runs, hyperlinks)?;
    }
    Ok(())
}

pub(super) fn validate_cached_row_style_ids(
    row_runs: &[Vec<CellRunSummary>],
    styles: &[StyleSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    for runs in row_runs {
        validate_cell_run_style_ids(runs, styles)?;
    }
    Ok(())
}
