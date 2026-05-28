use std::path::Path;
use std::{fs, io};

use nmux_proto::protocol;

use super::state_file::{
    decode_palette_rgba, decode_state_hex_field, hex_decode, hex_encode, parse_state_bool,
    parse_state_cell_semantic_content, parse_state_cursor_shape, parse_state_i64,
    parse_state_mouse_format, parse_state_mouse_tracking_mode, parse_state_row_semantic_prompt,
    parse_state_surface_kind, parse_state_u32, parse_state_u64, parse_state_usize, state_hex_field,
    state_save_tmp_path,
};
use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientAttachState {
    pub(super) scope: Option<SocketIdentity>,
    pub(super) surfaces: Vec<ClientPaneSurface>,
    pub(super) scrollbacks: Vec<ClientPaneScrollback>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPaneScrollback {
    pub pane_id: String,
    pub version: u64,
    pub start_line: u64,
    pub line_count: u32,
    pub total_lines: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientStateSummary {
    pub scope: Option<SocketIdentitySummary>,
    pub surfaces: Vec<ClientStateSurfaceSummary>,
    pub scrollbacks: Vec<ClientPaneScrollback>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketIdentitySummary {
    pub dev: u64,
    pub ino: u64,
    pub ctime: i64,
    pub ctime_nsec: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientStateSurfaceSummary {
    pub pane_id: String,
    pub version: u64,
    pub cols: u32,
    pub rows: u32,
    pub surface_kind: protocol::SurfaceKind,
    pub title: String,
    pub working_directory: String,
    pub cursor: Option<CursorSummary>,
    pub modes: TerminalModeSummary,
}

impl ClientAttachState {
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(encoded) => Self::decode(&encoded),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = state_save_tmp_path(path);
        fs::write(&tmp_path, self.encode())?;
        match fs::rename(&tmp_path, path) {
            Ok(()) => Ok(()),
            Err(err) => {
                let _ = fs::remove_file(&tmp_path);
                Err(err)
            }
        }
    }

    pub fn summary(&self) -> ClientStateSummary {
        ClientStateSummary {
            scope: self.scope.map(SocketIdentitySummary::from),
            surfaces: self
                .surfaces
                .iter()
                .map(|surface| ClientStateSurfaceSummary {
                    pane_id: surface.pane_id.clone(),
                    version: surface.version,
                    cols: surface.cols,
                    rows: surface.rows,
                    surface_kind: surface.surface,
                    title: surface.title.clone(),
                    working_directory: surface.working_directory.clone(),
                    cursor: surface.cursor,
                    modes: surface.modes,
                })
                .collect(),
            scrollbacks: self.scrollbacks.clone(),
        }
    }

    pub fn known_viewports(&self) -> Vec<KnownViewportVersion> {
        self.surfaces
            .iter()
            .map(|surface| KnownViewportVersion {
                pane_id: surface.pane_id.clone(),
                version: surface.version,
            })
            .collect()
    }

    pub fn known_viewports_for_scope(
        &self,
        scope: Option<SocketIdentity>,
    ) -> Vec<KnownViewportVersion> {
        if self.scope == scope {
            self.known_viewports()
        } else {
            Vec::new()
        }
    }

    pub fn cached_scrollback_version_for_scope(
        &self,
        scope: Option<SocketIdentity>,
        pane_id: &str,
        start_line: u64,
        line_count: u32,
    ) -> Option<u64> {
        if self.scope != scope {
            return None;
        }
        self.cached_scrollback_version(pane_id, start_line, line_count)
    }

    pub fn known_scrollback_versions_for_scope(
        &self,
        scope: Option<SocketIdentity>,
    ) -> Vec<KnownScrollbackVersion> {
        if self.scope != scope {
            return Vec::new();
        }
        self.scrollbacks
            .iter()
            .map(|scrollback| KnownScrollbackVersion {
                pane_id: scrollback.pane_id.clone(),
                start_line: scrollback.start_line,
                line_count: scrollback.line_count,
                version: scrollback.version,
            })
            .collect()
    }

    pub fn apply_scope(&mut self, scope: Option<SocketIdentity>) {
        if self.scope != scope {
            self.surfaces.clear();
            self.scrollbacks.clear();
        }
        self.scope = scope;
    }

    pub fn render_attach(
        &mut self,
        snapshot: AttachSnapshot,
    ) -> Result<RenderedAttach, Box<dyn std::error::Error>> {
        let pane_id = snapshot.status.pane_id.clone();
        let surface_text = match snapshot.surface.as_ref() {
            Some(update) => {
                validate_attach_surface_update(&snapshot.status, update)?;
                Some(self.apply_surface_update(update)?)
            }
            None => {
                if snapshot.status.surface_state != protocol::AttachSurfaceState::Current {
                    return Err(format!(
                        "attach status {:?} requires a matching surface frame",
                        snapshot.status.surface_state
                    )
                    .into());
                }
                let surface =
                    self.cached_current_surface(&pane_id, snapshot.status.surface_version)?;
                Some(surface.render_text())
            }
        };
        let surface = self.cached_current_surface(&pane_id, snapshot.status.surface_version)?;
        let surface_metadata = TerminalMetadataSummary::from_surface(surface);
        let surface_kind = surface.surface;
        let cursor = surface.cursor;
        let modes = surface.modes;
        let surface_summary = surface.summary();

        if let Some(scrollback) = snapshot.scrollback.as_ref() {
            self.cache_scrollback_chunk(scrollback);
        }

        Ok(RenderedAttach {
            workspace: snapshot.workspace,
            status: snapshot.status,
            surface_metadata,
            surface_kind,
            cursor,
            modes,
            surface: surface_summary,
            surface_text,
            scrollback: snapshot.scrollback,
        })
    }

    pub fn render_surface_update(
        &mut self,
        update: &SurfaceUpdate,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.apply_surface_update(update)
    }

    pub fn render_speculative_echo(
        &self,
        overlay: &mut SpeculativeEchoOverlay,
        pane_id: &str,
        input_seq: u64,
        text: &str,
        styled: bool,
    ) -> Option<String> {
        let surface = self
            .surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)?;
        overlay.predict_printable_key(surface, input_seq, text)?;
        if styled {
            overlay.render_underlined_styled(surface)
        } else {
            overlay.render_underlined(surface)
        }
    }

    /// Render a surface update with optional ANSI SGR styling from structured
    /// cell runs. When `styled` is true and the engine produced style data,
    /// the output contains SGR escape sequences reconstructed from protocol
    /// objects — the client never parses raw VT bytes.
    pub fn render_surface_update_styled(
        &mut self,
        update: &SurfaceUpdate,
        styled: bool,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.apply_surface_update_styled(update, styled)
    }

    pub fn cached_surface_text(&self, pane_id: &str) -> Option<String> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(ClientPaneSurface::render_text)
    }

    /// Like `cached_surface_text` but with optional ANSI SGR styling.
    pub fn cached_surface_text_styled(&self, pane_id: &str, styled: bool) -> Option<String> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(|surface| {
                if styled && surface.has_styled_runs() {
                    surface.render_styled_text()
                } else {
                    surface.render_text()
                }
            })
    }

    pub fn cached_surface_metadata(&self, pane_id: &str) -> Option<TerminalMetadataSummary> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(TerminalMetadataSummary::from_surface)
    }

    pub fn cached_surface_modes(&self, pane_id: &str) -> Option<TerminalModeSummary> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(|surface| surface.modes)
    }

    pub fn cached_surface_summary(&self, pane_id: &str) -> Option<CachedSurfaceSummary> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(|surface| CachedSurfaceSummary {
                surface_kind: surface.surface,
                cursor: surface.cursor,
                modes: surface.modes,
            })
    }

    pub fn cached_rendered_surface_summary(&self, pane_id: &str) -> Option<RenderedSurfaceSummary> {
        self.surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
            .map(ClientPaneSurface::summary)
    }

    fn cached_current_surface(
        &self,
        pane_id: &str,
        version: u64,
    ) -> Result<&ClientPaneSurface, Box<dyn std::error::Error>> {
        let Some(surface) = self
            .surfaces
            .iter()
            .find(|surface| surface.pane_id == pane_id)
        else {
            return Err(format!("current attach has no cached surface for pane {pane_id}").into());
        };
        if surface.version != version {
            return Err(format!(
                "current attach surface version mismatch for pane {pane_id}: cached {}, status {}",
                surface.version, version
            )
            .into());
        }
        Ok(surface)
    }

    pub fn cached_scrollback_version(
        &self,
        pane_id: &str,
        start_line: u64,
        line_count: u32,
    ) -> Option<u64> {
        self.scrollbacks
            .iter()
            .find(|scrollback| {
                scrollback.pane_id == pane_id
                    && scrollback.start_line == start_line
                    && scrollback.line_count == line_count
            })
            .map(|scrollback| scrollback.version)
    }

    fn apply_surface_update(
        &mut self,
        update: &SurfaceUpdate,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.apply_surface_update_styled(update, false)
    }

    fn apply_surface_update_styled(
        &mut self,
        update: &SurfaceUpdate,
        styled: bool,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if let Some(surface) = self
            .surfaces
            .iter_mut()
            .find(|surface| surface.pane_id == update.pane_id)
        {
            if update.version <= surface.version {
                return Ok(if styled && surface.has_styled_runs() {
                    surface.render_styled_text()
                } else {
                    surface.render_text()
                });
            }
            if update.kind == SurfaceUpdateKind::Patch
                && update.base_version != Some(surface.version)
            {
                return Ok(if styled && surface.has_styled_runs() {
                    surface.render_styled_text()
                } else {
                    surface.render_text()
                });
            }
            surface.apply_update(update)?;
            return Ok(if styled && surface.has_styled_runs() {
                surface.render_styled_text()
            } else {
                surface.render_text()
            });
        }

        if update.kind != SurfaceUpdateKind::Snapshot {
            return Err(format!(
                "cannot apply pane {} patch without a cached snapshot",
                update.pane_id
            )
            .into());
        }

        let surface = ClientPaneSurface::from_snapshot(update)?;
        let rendered = if styled && surface.has_styled_runs() {
            surface.render_styled_text()
        } else {
            surface.render_text()
        };
        self.surfaces.push(surface);
        Ok(rendered)
    }

    pub fn cache_scrollback_chunk(&mut self, chunk: &ScrollbackChunkSummary) {
        if chunk.lines.is_empty() {
            return;
        }
        let line_count = u32::try_from(chunk.lines.len()).unwrap_or(u32::MAX);
        let cached = ClientPaneScrollback {
            pane_id: chunk.pane_id.clone(),
            version: chunk.scrollback_version,
            start_line: chunk.start_line,
            line_count,
            total_lines: chunk.total_lines,
        };
        if let Some(existing) = self.scrollbacks.iter_mut().find(|scrollback| {
            scrollback.pane_id == cached.pane_id
                && scrollback.start_line == cached.start_line
                && scrollback.line_count == cached.line_count
        }) {
            *existing = cached;
        } else {
            self.scrollbacks.push(cached);
        }
    }

    pub(super) fn encode(&self) -> String {
        let mut encoded = String::from("NMUX_CLIENT_STATE 8\n");
        if let Some(scope) = self.scope {
            encoded.push_str("scope socket ");
            encoded.push_str(&scope.dev.to_string());
            encoded.push(' ');
            encoded.push_str(&scope.ino.to_string());
            encoded.push(' ');
            encoded.push_str(&scope.ctime.to_string());
            encoded.push(' ');
            encoded.push_str(&scope.ctime_nsec.to_string());
            encoded.push('\n');
        }
        for scrollback in &self.scrollbacks {
            encoded.push_str("scrollback ");
            encoded.push_str(&hex_encode(scrollback.pane_id.as_bytes()));
            encoded.push(' ');
            encoded.push_str(&scrollback.version.to_string());
            encoded.push(' ');
            encoded.push_str(&scrollback.start_line.to_string());
            encoded.push(' ');
            encoded.push_str(&scrollback.line_count.to_string());
            encoded.push(' ');
            encoded.push_str(&scrollback.total_lines.to_string());
            encoded.push('\n');
        }
        for surface in &self.surfaces {
            encoded.push_str("surface ");
            encoded.push_str(&hex_encode(surface.pane_id.as_bytes()));
            encoded.push(' ');
            encoded.push_str(&surface.version.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.cols.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.rows.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.surface.0.to_string());
            encoded.push('\n');
            match surface.cursor {
                Some(cursor) => {
                    encoded.push_str("cursor ");
                    encoded.push_str(&cursor.row.to_string());
                    encoded.push(' ');
                    encoded.push_str(&cursor.col.to_string());
                    encoded.push(' ');
                    encoded.push_str(if cursor.visible { "1" } else { "0" });
                    encoded.push(' ');
                    encoded.push_str(&cursor.shape.0.to_string());
                    encoded.push(' ');
                    encoded.push_str(if cursor.blinking { "1" } else { "0" });
                    encoded.push('\n');
                }
                None => encoded.push_str("cursor none\n"),
            }
            encoded.push_str("modes ");
            encoded.push_str(if surface.modes.bracketed_paste {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(if surface.modes.mouse_tracking {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(if surface.modes.focus_reporting {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(if surface.modes.application_keypad {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(if surface.modes.application_cursor {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(if surface.modes.origin { "1" } else { "0" });
            encoded.push(' ');
            encoded.push_str(if surface.modes.wraparound { "1" } else { "0" });
            encoded.push(' ');
            encoded.push_str(&surface.modes.mouse_tracking_mode.0.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.modes.mouse_format.0.to_string());
            encoded.push('\n');
            encoded.push_str("title ");
            encoded.push_str(&hex_encode(surface.title.as_bytes()));
            encoded.push('\n');
            encoded.push_str("pwd ");
            encoded.push_str(&hex_encode(surface.working_directory.as_bytes()));
            encoded.push('\n');
            encoded.push_str("colors ");
            encoded.push_str(&surface.colors.default_fg_rgba.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.colors.default_bg_rgba.to_string());
            encoded.push(' ');
            encoded.push_str(&surface.colors.cursor_rgba.to_string());
            encoded.push(' ');
            encoded.push_str(if surface.colors.cursor_rgba_set {
                "1"
            } else {
                "0"
            });
            encoded.push(' ');
            encoded.push_str(&hex_encode(
                &surface
                    .colors
                    .palette_rgba
                    .iter()
                    .flat_map(|color| color.to_be_bytes())
                    .collect::<Vec<_>>(),
            ));
            encoded.push('\n');
            for style in &surface.styles {
                encoded.push_str("style ");
                encoded.push_str(&style.fg_rgba.to_string());
                encoded.push(' ');
                encoded.push_str(&style.bg_rgba.to_string());
                encoded.push(' ');
                encoded.push_str(&style.underline_rgba.to_string());
                encoded.push(' ');
                encoded.push_str(&style.flags.to_string());
                encoded.push('\n');
            }
            for hyperlink in &surface.hyperlinks {
                encoded.push_str("hyperlink ");
                encoded.push_str(&hyperlink.id.to_string());
                encoded.push(' ');
                encoded.push_str(&hex_encode(hyperlink.uri.as_bytes()));
                encoded.push(' ');
                encoded.push_str(&state_hex_field(hyperlink.osc8_id.as_bytes()));
                encoded.push(' ');
                encoded.push_str(&state_hex_field(hyperlink.params.as_bytes()));
                encoded.push('\n');
            }
            for (index, row) in surface.row_text.iter().enumerate() {
                encoded.push_str("row ");
                encoded.push_str(&index.to_string());
                encoded.push(' ');
                encoded.push_str(&hex_encode(row.as_bytes()));
                encoded.push('\n');
                encoded.push_str("rowmeta ");
                encoded.push_str(&index.to_string());
                encoded.push(' ');
                encoded.push_str(&surface.row_semantic_prompts[index].0.to_string());
                encoded.push(' ');
                encoded.push_str(if surface.row_dirty[index] { "1" } else { "0" });
                encoded.push(' ');
                encoded.push_str(if surface.row_kitty_placeholders[index] {
                    "1"
                } else {
                    "0"
                });
                encoded.push(' ');
                encoded.push_str(&surface.row_state_hashes[index].to_string());
                encoded.push(' ');
                encoded.push_str(&surface.row_dirty_hashes[index].to_string());
                encoded.push('\n');
                for run in &surface.row_runs[index] {
                    encoded.push_str("run ");
                    encoded.push_str(&index.to_string());
                    encoded.push(' ');
                    encoded.push_str(&hex_encode(run.text.as_bytes()));
                    encoded.push(' ');
                    encoded.push_str(&hex_encode(&run.cell_widths));
                    encoded.push(' ');
                    encoded.push_str(&run.style_id.to_string());
                    encoded.push(' ');
                    encoded.push_str(&run.flags.to_string());
                    encoded.push(' ');
                    encoded.push_str(&run.hyperlink_id.to_string());
                    encoded.push(' ');
                    encoded.push_str(&run.semantic_content.0.to_string());
                    encoded.push('\n');
                }
            }
            encoded.push_str("end\n");
        }
        encoded
    }

    pub(super) fn decode(encoded: &str) -> io::Result<Self> {
        let mut lines = encoded.lines();
        let Some(header) = lines.next() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid nmux client state header",
            ));
        };
        if header != "NMUX_CLIENT_STATE 1"
            && header != "NMUX_CLIENT_STATE 2"
            && header != "NMUX_CLIENT_STATE 3"
            && header != "NMUX_CLIENT_STATE 4"
            && header != "NMUX_CLIENT_STATE 5"
            && header != "NMUX_CLIENT_STATE 6"
            && header != "NMUX_CLIENT_STATE 7"
            && header != "NMUX_CLIENT_STATE 8"
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid nmux client state header",
            ));
        }

        let mut scope = None;
        let mut surfaces = Vec::new();
        let mut scrollbacks = Vec::new();
        while let Some(line) = lines.next() {
            let scope_parts = line.split(' ').collect::<Vec<_>>();
            if let ["scope", "socket", dev, ino] = scope_parts.as_slice() {
                scope = Some(SocketIdentity {
                    dev: parse_state_u64(dev)?,
                    ino: parse_state_u64(ino)?,
                    ctime: 0,
                    ctime_nsec: 0,
                });
                continue;
            }
            if let ["scope", "socket", dev, ino, ctime, ctime_nsec] = scope_parts.as_slice() {
                scope = Some(SocketIdentity {
                    dev: parse_state_u64(dev)?,
                    ino: parse_state_u64(ino)?,
                    ctime: parse_state_i64(ctime)?,
                    ctime_nsec: parse_state_i64(ctime_nsec)?,
                });
                continue;
            }
            if let [
                "scrollback",
                pane_id,
                version,
                start_line,
                line_count,
                total_lines,
            ] = scope_parts.as_slice()
            {
                let start_line = parse_state_u64(start_line)?;
                let line_count = parse_state_u32(line_count)?;
                if start_line == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "cached scrollback start_line must be 1-based",
                    ));
                }
                if line_count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "cached scrollback line_count must be nonzero",
                    ));
                }
                scrollbacks.push(ClientPaneScrollback {
                    pane_id: String::from_utf8(hex_decode(pane_id)?)
                        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                    version: parse_state_u64(version)?,
                    start_line,
                    line_count,
                    total_lines: parse_state_u64(total_lines)?,
                });
                continue;
            }

            let mut parts = line.split(' ');
            let (
                Some("surface"),
                Some(pane_id),
                Some(version),
                Some(cols),
                Some(rows),
                surface,
                None,
            ) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            )
            else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid nmux client state surface line",
                ));
            };

            let pane_id = String::from_utf8(hex_decode(pane_id)?)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            let version = parse_state_u64(version)?;
            let cols = parse_state_u32(cols)?;
            let rows = parse_state_u32(rows)?;
            let surface = surface
                .map(parse_state_surface_kind)
                .transpose()?
                .unwrap_or(protocol::SurfaceKind::Main);
            let row_count = usize::try_from(rows).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "surface rows too large")
            })?;
            let mut cursor = None;
            let mut modes = TerminalModeSummary::default();
            let mut title = String::new();
            let mut working_directory = String::new();
            let mut colors = TerminalColorSummary::default();
            let mut styles = Vec::new();
            let mut hyperlinks = Vec::new();
            let mut row_text = vec![String::new(); row_count];
            let mut row_runs = vec![Vec::new(); row_count];
            let mut row_dirty_hashes = vec![0; row_count];
            let mut row_semantic_prompts = vec![protocol::RowSemanticPrompt::None; row_count];
            let mut row_dirty = vec![false; row_count];
            let mut row_kitty_placeholders = vec![false; row_count];
            let mut row_state_hashes = vec![0; row_count];

            loop {
                let Some(line) = lines.next() else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unterminated nmux client state surface",
                    ));
                };
                if line == "end" {
                    break;
                }

                let parts = line.split(' ').collect::<Vec<_>>();
                match parts.as_slice() {
                    ["cursor", "none"] => cursor = None,
                    ["cursor", row, col, visible, shape] => {
                        cursor = Some(CursorSummary {
                            row: parse_state_u32(row)?,
                            col: parse_state_u32(col)?,
                            visible: parse_state_bool(visible)?,
                            shape: parse_state_cursor_shape(shape)?,
                            blinking: true,
                        });
                    }
                    ["cursor", row, col, visible, shape, blinking] => {
                        cursor = Some(CursorSummary {
                            row: parse_state_u32(row)?,
                            col: parse_state_u32(col)?,
                            visible: parse_state_bool(visible)?,
                            shape: parse_state_cursor_shape(shape)?,
                            blinking: parse_state_bool(blinking)?,
                        });
                    }
                    [
                        "modes",
                        bracketed_paste,
                        mouse_tracking,
                        focus_reporting,
                        application_keypad,
                        application_cursor,
                        origin,
                        wraparound,
                    ] => {
                        modes = TerminalModeSummary {
                            bracketed_paste: parse_state_bool(bracketed_paste)?,
                            mouse_tracking: parse_state_bool(mouse_tracking)?,
                            focus_reporting: parse_state_bool(focus_reporting)?,
                            application_keypad: parse_state_bool(application_keypad)?,
                            application_cursor: parse_state_bool(application_cursor)?,
                            origin: parse_state_bool(origin)?,
                            wraparound: parse_state_bool(wraparound)?,
                            ..TerminalModeSummary::default()
                        };
                    }
                    [
                        "modes",
                        bracketed_paste,
                        mouse_tracking,
                        focus_reporting,
                        application_keypad,
                        application_cursor,
                        origin,
                        wraparound,
                        mouse_tracking_mode,
                        mouse_format,
                    ] => {
                        modes = TerminalModeSummary {
                            bracketed_paste: parse_state_bool(bracketed_paste)?,
                            mouse_tracking: parse_state_bool(mouse_tracking)?,
                            focus_reporting: parse_state_bool(focus_reporting)?,
                            application_keypad: parse_state_bool(application_keypad)?,
                            application_cursor: parse_state_bool(application_cursor)?,
                            origin: parse_state_bool(origin)?,
                            wraparound: parse_state_bool(wraparound)?,
                            mouse_tracking_mode: parse_state_mouse_tracking_mode(
                                mouse_tracking_mode,
                            )?,
                            mouse_format: parse_state_mouse_format(mouse_format)?,
                        };
                    }
                    ["title", value] => {
                        title = String::from_utf8(hex_decode(value)?)
                            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    }
                    ["pwd", value] => {
                        working_directory = String::from_utf8(hex_decode(value)?)
                            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    }
                    [
                        "colors",
                        default_fg_rgba,
                        default_bg_rgba,
                        cursor_rgba,
                        palette_rgba,
                    ] => {
                        let cursor_rgba = parse_state_u32(cursor_rgba)?;
                        colors = TerminalColorSummary {
                            default_fg_rgba: parse_state_u32(default_fg_rgba)?,
                            default_bg_rgba: parse_state_u32(default_bg_rgba)?,
                            cursor_rgba,
                            cursor_rgba_set: cursor_rgba != 0,
                            palette_rgba: decode_palette_rgba(palette_rgba)?,
                            palette_diff_start: None,
                            palette_diff_rgba: Vec::new(),
                        };
                    }
                    [
                        "colors",
                        default_fg_rgba,
                        default_bg_rgba,
                        cursor_rgba,
                        cursor_rgba_set,
                        palette_rgba,
                    ] => {
                        colors = TerminalColorSummary {
                            default_fg_rgba: parse_state_u32(default_fg_rgba)?,
                            default_bg_rgba: parse_state_u32(default_bg_rgba)?,
                            cursor_rgba: parse_state_u32(cursor_rgba)?,
                            cursor_rgba_set: parse_state_bool(cursor_rgba_set)?,
                            palette_rgba: decode_palette_rgba(palette_rgba)?,
                            palette_diff_start: None,
                            palette_diff_rgba: Vec::new(),
                        };
                    }
                    ["style", fg_rgba, bg_rgba, underline_rgba, flags] => {
                        styles.push(StyleSummary {
                            fg_rgba: parse_state_u32(fg_rgba)?,
                            bg_rgba: parse_state_u32(bg_rgba)?,
                            underline_rgba: parse_state_u32(underline_rgba)?,
                            flags: parse_state_u32(flags)?,
                        });
                    }
                    ["hyperlink", id, uri, osc8_id, params] => {
                        hyperlinks.push(HyperlinkSummary {
                            id: parse_state_u32(id)?,
                            uri: String::from_utf8(hex_decode(uri)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                            osc8_id: String::from_utf8(decode_state_hex_field(osc8_id)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                            params: String::from_utf8(decode_state_hex_field(params)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                        });
                    }
                    ["row", row, text] => {
                        let row = parse_state_usize(row)?;
                        let Some(target) = row_text.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row index outside surface",
                            ));
                        };
                        *target = String::from_utf8(hex_decode(text)?)
                            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    }
                    ["rowmeta", row, semantic_prompt] => {
                        let row = parse_state_usize(row)?;
                        let Some(target) = row_semantic_prompts.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        *target = parse_state_row_semantic_prompt(semantic_prompt)?;
                    }
                    ["rowmeta", row, semantic_prompt, dirty] => {
                        let row = parse_state_usize(row)?;
                        let Some(semantic_target) = row_semantic_prompts.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(dirty_target) = row_dirty.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        *semantic_target = parse_state_row_semantic_prompt(semantic_prompt)?;
                        *dirty_target = parse_state_bool(dirty)?;
                    }
                    ["rowmeta", row, semantic_prompt, dirty, kitty_placeholder] => {
                        let row = parse_state_usize(row)?;
                        let Some(semantic_target) = row_semantic_prompts.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(dirty_target) = row_dirty.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(kitty_target) = row_kitty_placeholders.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        *semantic_target = parse_state_row_semantic_prompt(semantic_prompt)?;
                        *dirty_target = parse_state_bool(dirty)?;
                        *kitty_target = parse_state_bool(kitty_placeholder)?;
                    }
                    [
                        "rowmeta",
                        row,
                        semantic_prompt,
                        dirty,
                        kitty_placeholder,
                        row_state_hash,
                    ] => {
                        let row = parse_state_usize(row)?;
                        let Some(semantic_target) = row_semantic_prompts.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(dirty_target) = row_dirty.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(kitty_target) = row_kitty_placeholders.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(hash_target) = row_state_hashes.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        *semantic_target = parse_state_row_semantic_prompt(semantic_prompt)?;
                        *dirty_target = parse_state_bool(dirty)?;
                        *kitty_target = parse_state_bool(kitty_placeholder)?;
                        *hash_target = parse_state_u64(row_state_hash)?;
                    }
                    [
                        "rowmeta",
                        row,
                        semantic_prompt,
                        dirty,
                        kitty_placeholder,
                        row_state_hash,
                        dirty_hash,
                    ] => {
                        let row = parse_state_usize(row)?;
                        let Some(semantic_target) = row_semantic_prompts.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(dirty_target) = row_dirty.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(kitty_target) = row_kitty_placeholders.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(hash_target) = row_state_hashes.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        let Some(dirty_hash_target) = row_dirty_hashes.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state row metadata index outside surface",
                            ));
                        };
                        *semantic_target = parse_state_row_semantic_prompt(semantic_prompt)?;
                        *dirty_target = parse_state_bool(dirty)?;
                        *kitty_target = parse_state_bool(kitty_placeholder)?;
                        *hash_target = parse_state_u64(row_state_hash)?;
                        *dirty_hash_target = parse_state_u64(dirty_hash)?;
                    }
                    ["run", row, text, cell_widths, style_id, flags] => {
                        let row = parse_state_usize(row)?;
                        let Some(target) = row_runs.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state run row index outside surface",
                            ));
                        };
                        target.push(CellRunSummary {
                            text: String::from_utf8(hex_decode(text)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                            cell_widths: hex_decode(cell_widths)?,
                            style_id: parse_state_u32(style_id)?,
                            flags: parse_state_u32(flags)?,
                            hyperlink_id: 0,
                            semantic_content: protocol::CellSemanticContent::Output,
                        });
                    }
                    ["run", row, text, cell_widths, style_id, flags, hyperlink_id] => {
                        let row = parse_state_usize(row)?;
                        let Some(target) = row_runs.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state run row index outside surface",
                            ));
                        };
                        target.push(CellRunSummary {
                            text: String::from_utf8(hex_decode(text)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                            cell_widths: hex_decode(cell_widths)?,
                            style_id: parse_state_u32(style_id)?,
                            flags: parse_state_u32(flags)?,
                            hyperlink_id: parse_state_u32(hyperlink_id)?,
                            semantic_content: protocol::CellSemanticContent::Output,
                        });
                    }
                    [
                        "run",
                        row,
                        text,
                        cell_widths,
                        style_id,
                        flags,
                        hyperlink_id,
                        semantic_content,
                    ] => {
                        let row = parse_state_usize(row)?;
                        let Some(target) = row_runs.get_mut(row) else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "client state run row index outside surface",
                            ));
                        };
                        target.push(CellRunSummary {
                            text: String::from_utf8(hex_decode(text)?)
                                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
                            cell_widths: hex_decode(cell_widths)?,
                            style_id: parse_state_u32(style_id)?,
                            flags: parse_state_u32(flags)?,
                            hyperlink_id: parse_state_u32(hyperlink_id)?,
                            semantic_content: parse_state_cell_semantic_content(semantic_content)?,
                        });
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid nmux client state surface body line",
                        ));
                    }
                }
            }

            validate_hyperlink_table(&hyperlinks)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
            let row_runs = row_runs_for_text(&row_text, row_runs);
            let styles = if styles.is_empty() {
                default_style_summaries()
            } else {
                styles
            };
            validate_cached_row_style_ids(&row_runs, &styles)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
            validate_cached_row_hyperlink_ids(&row_runs, &hyperlinks)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

            surfaces.push(ClientPaneSurface {
                pane_id,
                version,
                scrollback_version: 0,
                scrollback_total_lines: 0,
                cols,
                rows,
                surface,
                cursor,
                modes,
                title,
                working_directory,
                colors,
                styles,
                hyperlinks,
                row_runs,
                row_dirty_hashes,
                row_semantic_prompts,
                row_dirty,
                row_kitty_placeholders,
                row_state_hashes,
                row_text,
            });
        }

        Ok(Self {
            scope,
            surfaces,
            scrollbacks,
        })
    }
}

impl From<SocketIdentity> for SocketIdentitySummary {
    fn from(identity: SocketIdentity) -> Self {
        Self {
            dev: identity.dev,
            ino: identity.ino,
            ctime: identity.ctime,
            ctime_nsec: identity.ctime_nsec,
        }
    }
}
