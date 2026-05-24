use nmux_proto::protocol;

use crate::local::{ClientPaneSurface, SurfaceUpdate, SurfaceUpdateKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeculativeEchoOverlay {
    pub(crate) prediction: Option<SpeculativeEchoPrediction>,
    consecutive_misses: u8,
    suppressed_predictable_keys: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeculativeEchoPrediction {
    pub pane_id: String,
    pub base_version: u64,
    pub input_seq: u64,
    pub row: u32,
    pub col: u32,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeculativeEchoReconcile {
    NoPrediction,
    Pending,
    Confirmed,
    Mismatched,
}

impl SpeculativeEchoOverlay {
    const MAX_CONSECUTIVE_MISSES: u8 = 2;
    const RECOVERY_PREDICTABLE_KEYS: u8 = 3;
    const UNDERLINE_START: &'static str = "\x1b[4m";
    const UNDERLINE_END: &'static str = "\x1b[24m";

    pub fn predict_printable_key(
        &mut self,
        surface: &ClientPaneSurface,
        input_seq: u64,
        text: &str,
    ) -> Option<String> {
        if self.prediction.is_some() {
            return None;
        }
        let ch = single_predictable_char(text)?;
        let cursor = surface.cursor?;
        if surface.surface != protocol::SurfaceKind::Main
            || !cursor.visible
            || cursor.row >= surface.rows
            || cursor.col >= surface.cols
        {
            return None;
        }
        let row_index = usize::try_from(cursor.row).ok()?;
        let row_text = surface.row_text().get(row_index)?;
        if cursor.col as usize != row_text.chars().count() {
            return None;
        }
        if cursor.col + 1 > surface.cols {
            return None;
        }
        if self.prediction_suppressed() {
            self.record_suppressed_predictable_key();
            return None;
        }

        self.prediction = Some(SpeculativeEchoPrediction {
            pane_id: surface.pane_id.clone(),
            base_version: surface.version,
            input_seq,
            row: cursor.row,
            col: cursor.col,
            text: ch.to_string(),
        });
        self.render(surface)
    }

    pub fn render(&self, surface: &ClientPaneSurface) -> Option<String> {
        self.render_with_speculative_style(
            surface,
            PredictionDecoration::Plain,
            SpeculativeSurfaceStyle::PlainText,
        )
    }

    pub fn render_underlined(&self, surface: &ClientPaneSurface) -> Option<String> {
        self.render_with_speculative_style(
            surface,
            PredictionDecoration::Underlined,
            SpeculativeSurfaceStyle::PlainText,
        )
    }

    pub fn render_underlined_styled(&self, surface: &ClientPaneSurface) -> Option<String> {
        self.render_with_speculative_style(
            surface,
            PredictionDecoration::Underlined,
            SpeculativeSurfaceStyle::StructuredSgr,
        )
    }

    fn render_with_speculative_style(
        &self,
        surface: &ClientPaneSurface,
        decoration: PredictionDecoration,
        surface_style: SpeculativeSurfaceStyle,
    ) -> Option<String> {
        let prediction = self.prediction.as_ref()?;
        if prediction.pane_id != surface.pane_id || prediction.base_version != surface.version {
            return None;
        }
        let row_index = usize::try_from(prediction.row).ok()?;
        let row = surface.row_text().get(row_index)?;
        if prediction.col as usize != row.chars().count() {
            return None;
        }
        let visible_rows = surface.visible_row_count().max(row_index + 1);
        let mut rows = Vec::with_capacity(visible_rows);
        for current_row in 0..visible_rows {
            let mut rendered_row = match surface_style {
                SpeculativeSurfaceStyle::PlainText => surface.row_text()[current_row].clone(),
                SpeculativeSurfaceStyle::StructuredSgr => surface.render_styled_row(current_row),
            };
            if current_row == row_index {
                match decoration {
                    PredictionDecoration::Plain => rendered_row.push_str(&prediction.text),
                    PredictionDecoration::Underlined => {
                        rendered_row.push_str(Self::UNDERLINE_START);
                        rendered_row.push_str(&prediction.text);
                        rendered_row.push_str(Self::UNDERLINE_END);
                    }
                }
            }
            rows.push(rendered_row);
        }
        Some(rows.join("\n"))
    }

    fn rebase_pending_prediction(&mut self, update: &SurfaceUpdate) -> SpeculativeEchoReconcile {
        if let Some(prediction) = self.prediction.as_mut() {
            prediction.base_version = update.version;
        }
        SpeculativeEchoReconcile::Pending
    }

    fn clear_confirmed(&mut self) -> SpeculativeEchoReconcile {
        self.prediction = None;
        self.consecutive_misses = 0;
        self.suppressed_predictable_keys = 0;
        SpeculativeEchoReconcile::Confirmed
    }

    pub fn reconcile_update(&mut self, update: &SurfaceUpdate) -> SpeculativeEchoReconcile {
        let Some(prediction) = self.prediction.clone() else {
            return SpeculativeEchoReconcile::NoPrediction;
        };
        if prediction.pane_id != update.pane_id {
            return SpeculativeEchoReconcile::Pending;
        }
        if update.kind == SurfaceUpdateKind::Snapshot
            || update
                .surface
                .is_some_and(|surface| surface != protocol::SurfaceKind::Main)
        {
            return self.clear_mismatched();
        }
        if update.version <= prediction.base_version {
            return SpeculativeEchoReconcile::Pending;
        }
        if update
            .cursor
            .is_some_and(|cursor| cursor.row != prediction.row || cursor.col < prediction.col)
        {
            return self.clear_mismatched();
        }
        let Some(row) = update
            .row_updates
            .iter()
            .find(|row| row.row == prediction.row)
        else {
            return self.rebase_pending_prediction(update);
        };
        let confirmed = row
            .text
            .chars()
            .nth(prediction.col as usize)
            .is_some_and(|ch| ch.to_string() == prediction.text);
        if confirmed {
            self.clear_confirmed()
        } else {
            self.clear_mismatched()
        }
    }

    pub fn prediction_allowed(&self) -> bool {
        !self.prediction_suppressed()
    }

    fn clear_mismatched(&mut self) -> SpeculativeEchoReconcile {
        self.prediction = None;
        self.consecutive_misses = self.consecutive_misses.saturating_add(1);
        self.suppressed_predictable_keys = 0;
        SpeculativeEchoReconcile::Mismatched
    }

    fn prediction_suppressed(&self) -> bool {
        self.consecutive_misses >= Self::MAX_CONSECUTIVE_MISSES
    }

    fn record_suppressed_predictable_key(&mut self) {
        self.suppressed_predictable_keys = self.suppressed_predictable_keys.saturating_add(1);
        if self.suppressed_predictable_keys >= Self::RECOVERY_PREDICTABLE_KEYS {
            self.consecutive_misses = 0;
            self.suppressed_predictable_keys = 0;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PredictionDecoration {
    Plain,
    Underlined,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpeculativeSurfaceStyle {
    PlainText,
    StructuredSgr,
}

fn single_predictable_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() || ch.is_control() || !ch.is_ascii() {
        return None;
    }
    Some(ch)
}
