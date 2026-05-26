use std::collections::BTreeMap;

use nmux_cli::local;
use nmux_proto::protocol;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Widget};

const TREE_MIN_WIDTH: u16 = 18;
const TREE_MAX_WIDTH: u16 = 28;
const MENU_HEIGHT: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiFrame {
    pub text: String,
    pub hits: Vec<HitRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiOverlay {
    pub title: String,
    pub lines: Vec<TuiOverlayLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiOverlayLine {
    pub text: String,
    pub action: Option<OverlayAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayAction {
    SwitchTab(String),
    SwitchSession(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitRegion {
    pub rect: Rect,
    pub target: HitTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitTarget {
    Menu(MenuAction),
    Pane(String),
    PaneContent(String),
    WindowTreePane(String),
    Overlay(OverlayAction),
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Sessions,
    NewSession,
    Windows,
    Clipboard,
}

pub struct WorkspaceFrameInput<'a> {
    pub workspace: &'a local::WorkspaceSummary,
    pub active_surface_text: &'a str,
    pub pane_surfaces: Option<&'a BTreeMap<String, String>>,
    pub overlay: Option<&'a TuiOverlay>,
}

pub fn render_workspace_frame(input: WorkspaceFrameInput<'_>, cols: u16, rows: u16) -> TuiFrame {
    let area = Rect::new(0, 0, cols, rows);
    let mut buffer = Buffer::empty(area);
    let hits = render_workspace_to_buffer(&mut buffer, area, input);

    TuiFrame {
        text: buffer_to_string(&buffer, area),
        hits,
    }
}

pub fn render_workspace_to_buffer(
    buffer: &mut Buffer,
    area: Rect,
    input: WorkspaceFrameInput<'_>,
) -> Vec<HitRegion> {
    paint_background(buffer, area);

    let mut hits = Vec::new();
    hits.push(HitRegion {
        rect: area,
        target: HitTarget::Background,
    });
    if area.width == 0 || area.height == 0 {
        return hits;
    }

    let menu = Rect::new(area.x, area.y, area.width, MENU_HEIGHT.min(area.height));
    render_menu(buffer, menu, &mut hits);

    if area.height <= MENU_HEIGHT {
        return hits;
    }

    let body = Rect::new(
        area.x,
        area.y + MENU_HEIGHT,
        area.width,
        area.height - MENU_HEIGHT,
    );
    let tree_width = tree_width_for(body.width, input.workspace.pane_tree.as_ref());
    let (tree_area, pane_area) = if tree_width == 0 {
        (Rect::new(body.x, body.y, 0, body.height), body)
    } else {
        let tree_area = Rect::new(body.x, body.y, tree_width, body.height);
        let pane_x = body.x.saturating_add(tree_width).saturating_add(1);
        let pane_width = body.width.saturating_sub(tree_width.saturating_add(1));
        (
            tree_area,
            Rect::new(pane_x, body.y, pane_width, body.height),
        )
    };

    if tree_area.width > 0 {
        render_window_tree(buffer, tree_area, input.workspace, &mut hits);
        draw_vertical_rule(
            buffer,
            tree_area.x + tree_area.width,
            tree_area.y,
            tree_area.height,
        );
    }

    if let Some(root) = input.workspace.pane_tree.as_ref() {
        render_pane_node(
            buffer,
            pane_area,
            root,
            input.workspace,
            input.active_surface_text,
            input.pane_surfaces,
            &mut hits,
        );
    } else {
        render_leaf_pane(
            buffer,
            pane_area,
            &input.workspace.pane_id,
            input.workspace.cols,
            input.workspace.rows,
            input.workspace.pane_id == input.workspace.pane_id,
            input.active_surface_text,
            &mut hits,
        );
    }

    if let Some(overlay) = input.overlay {
        render_overlay(buffer, area, overlay, &mut hits);
    }

    hits
}

#[allow(dead_code)]
pub fn hit_test(hits: &[HitRegion], x: u16, y: u16) -> Option<&HitTarget> {
    hit_test_region(hits, x, y).map(|hit| &hit.target)
}

#[allow(dead_code)]
pub fn hit_test_region(hits: &[HitRegion], x: u16, y: u16) -> Option<&HitRegion> {
    hits.iter().rev().find(|hit| rect_contains(hit.rect, x, y))
}

fn tree_width_for(width: u16, root: Option<&local::WorkspacePaneSummary>) -> u16 {
    if root.is_none() || width < 64 {
        return 0;
    }
    width
        .saturating_div(4)
        .clamp(TREE_MIN_WIDTH, TREE_MAX_WIDTH)
}

fn render_menu(buffer: &mut Buffer, area: Rect, hits: &mut Vec<HitRegion>) {
    let items = [
        (" Sessions ", MenuAction::Sessions),
        (" New Session ", MenuAction::NewSession),
        (" Windows ", MenuAction::Windows),
        (" Clipboard ", MenuAction::Clipboard),
    ];
    let menu_style = Style::default()
        .bg(Color::Rgb(32, 36, 42))
        .fg(Color::Rgb(226, 232, 240));
    fill_rect(buffer, area, " ", menu_style);
    let mut x = area.x;
    for (label, action) in items {
        if x >= area.x + area.width {
            break;
        }
        let item_width = (label.chars().count() as u16).min(area.x + area.width - x);
        write_text(
            buffer,
            x,
            area.y,
            area.x + area.width - x,
            label,
            menu_style.add_modifier(Modifier::BOLD),
        );
        hits.push(HitRegion {
            rect: Rect::new(x, area.y, item_width, area.height),
            target: HitTarget::Menu(action),
        });
        x = x.saturating_add(item_width);
    }
}

fn render_window_tree(
    buffer: &mut Buffer,
    area: Rect,
    workspace: &local::WorkspaceSummary,
    hits: &mut Vec<HitRegion>,
) {
    draw_box(buffer, area, "windows", false);
    let Some(inner) = inset(area, 1) else {
        return;
    };
    write_text(
        buffer,
        inner.x,
        inner.y,
        inner.width,
        &workspace.tab_id,
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    if let Some(root) = workspace.pane_tree.as_ref() {
        let mut row = inner.y.saturating_add(1);
        render_tree_node(buffer, inner, root, workspace, 0, &mut row, hits);
    }
}

fn render_tree_node(
    buffer: &mut Buffer,
    area: Rect,
    pane: &local::WorkspacePaneSummary,
    workspace: &local::WorkspaceSummary,
    depth: u16,
    row: &mut u16,
    hits: &mut Vec<HitRegion>,
) {
    if *row >= area.y + area.height {
        return;
    }
    let active = pane.pane_id == workspace.pane_id;
    let prefix = if pane.children.is_empty() { "-" } else { "+" };
    let label = format!(
        "{:indent$}{prefix} {} {}x{}",
        "",
        pane.pane_id,
        pane.cols,
        pane.rows,
        indent = usize::from(depth.saturating_mul(2))
    );
    let style = if active {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::White)
    };
    write_text(buffer, area.x, *row, area.width, &label, style);
    hits.push(HitRegion {
        rect: Rect::new(area.x, *row, area.width, 1),
        target: HitTarget::WindowTreePane(pane.pane_id.clone()),
    });
    *row = row.saturating_add(1);
    for child in &pane.children {
        render_tree_node(buffer, area, child, workspace, depth + 1, row, hits);
    }
}

fn render_pane_node(
    buffer: &mut Buffer,
    area: Rect,
    pane: &local::WorkspacePaneSummary,
    workspace: &local::WorkspaceSummary,
    active_surface_text: &str,
    pane_surfaces: Option<&BTreeMap<String, String>>,
    hits: &mut Vec<HitRegion>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if pane.children.is_empty() {
        let active = pane.pane_id == workspace.pane_id;
        let body = if active {
            active_surface_text
        } else {
            pane_surfaces
                .and_then(|surfaces| surfaces.get(&pane.pane_id).map(String::as_str))
                .unwrap_or("(surface not cached)")
        };
        render_leaf_pane(
            buffer,
            area,
            &pane.pane_id,
            pane.cols,
            pane.rows,
            active,
            body,
            hits,
        );
        return;
    }

    let count = pane.children.len().max(1) as u16;
    match pane.split_axis {
        protocol::SplitAxis::Vertical => {
            let mut x = area.x;
            let mut remaining_width = area.width;
            for (index, child) in pane.children.iter().enumerate() {
                let remaining_children = count.saturating_sub(index as u16);
                let width = if remaining_children <= 1 {
                    remaining_width
                } else {
                    remaining_width / remaining_children
                };
                let child_area = Rect::new(x, area.y, width, area.height);
                render_pane_node(
                    buffer,
                    child_area,
                    child,
                    workspace,
                    active_surface_text,
                    pane_surfaces,
                    hits,
                );
                x = x.saturating_add(width);
                remaining_width = remaining_width.saturating_sub(width);
            }
        }
        protocol::SplitAxis::Horizontal => {
            let mut y = area.y;
            let mut remaining_height = area.height;
            for (index, child) in pane.children.iter().enumerate() {
                let remaining_children = count.saturating_sub(index as u16);
                let height = if remaining_children <= 1 {
                    remaining_height
                } else {
                    remaining_height / remaining_children
                };
                let child_area = Rect::new(area.x, y, area.width, height);
                render_pane_node(
                    buffer,
                    child_area,
                    child,
                    workspace,
                    active_surface_text,
                    pane_surfaces,
                    hits,
                );
                y = y.saturating_add(height);
                remaining_height = remaining_height.saturating_sub(height);
            }
        }
        _ => {
            for child in &pane.children {
                render_pane_node(
                    buffer,
                    area,
                    child,
                    workspace,
                    active_surface_text,
                    pane_surfaces,
                    hits,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_leaf_pane(
    buffer: &mut Buffer,
    area: Rect,
    pane_id: &str,
    _pane_cols: u32,
    _pane_rows: u32,
    active: bool,
    surface_text: &str,
    hits: &mut Vec<HitRegion>,
) {
    let chrome = area;
    draw_box(buffer, chrome, pane_id, active);
    hits.push(HitRegion {
        rect: chrome,
        target: HitTarget::Pane(pane_id.to_owned()),
    });
    let Some(inner) = inset(chrome, 1) else {
        return;
    };
    hits.push(HitRegion {
        rect: inner,
        target: HitTarget::PaneContent(pane_id.to_owned()),
    });
    for (offset, line) in surface_text
        .lines()
        .take(usize::from(inner.height))
        .enumerate()
    {
        let y = inner.y + offset as u16;
        write_text(
            buffer,
            inner.x,
            y,
            inner.width,
            line,
            Style::default().fg(Color::White),
        );
    }
}

fn paint_background(buffer: &mut Buffer, area: Rect) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            set_cell(
                buffer,
                x,
                y,
                " ",
                Style::default()
                    .fg(Color::Rgb(203, 213, 225))
                    .bg(Color::Rgb(17, 19, 24)),
            );
        }
    }
}

fn render_overlay(
    buffer: &mut Buffer,
    area: Rect,
    overlay: &TuiOverlay,
    hits: &mut Vec<HitRegion>,
) {
    if area.width < 12 || area.height < 5 {
        return;
    }
    let max_line_width = overlay
        .lines
        .iter()
        .map(|line| line.text.chars().count() as u16)
        .max()
        .unwrap_or(0)
        .max(overlay.title.chars().count() as u16);
    let width = max_line_width
        .saturating_add(4)
        .clamp(12, area.width.saturating_sub(2).max(12));
    let height = (overlay.lines.len() as u16)
        .saturating_add(4)
        .clamp(5, area.height.saturating_sub(2).max(5));
    let overlay_area = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    fill_rect(
        buffer,
        overlay_area,
        " ",
        Style::default().fg(Color::White).bg(Color::Black),
    );
    draw_box(buffer, overlay_area, &overlay.title, true);
    let Some(inner) = inset(overlay_area, 1) else {
        return;
    };
    for (offset, line) in overlay
        .lines
        .iter()
        .take(usize::from(inner.height))
        .enumerate()
    {
        let row = inner.y + offset as u16;
        write_text(
            buffer,
            inner.x,
            row,
            inner.width,
            &line.text,
            Style::default().fg(Color::White).bg(Color::Black),
        );
        if let Some(action) = line.action.clone() {
            hits.push(HitRegion {
                rect: Rect::new(inner.x, row, inner.width, 1),
                target: HitTarget::Overlay(action),
            });
        }
    }
}

fn draw_box(buffer: &mut Buffer, area: Rect, title: &str, active: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let border_style = if active {
        Style::default()
            .fg(Color::Rgb(94, 234, 212))
            .bg(Color::Rgb(17, 19, 24))
    } else {
        Style::default()
            .fg(Color::Rgb(100, 116, 139))
            .bg(Color::Rgb(17, 19, 24))
    };

    if area.width > 4 {
        let title = if active {
            format!(" {} active ", title)
        } else {
            format!(" {} ", title)
        };
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title_style(border_style.add_modifier(Modifier::BOLD))
            .title(title)
            .render(area, buffer);
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .render(area, buffer);
    }
}

fn draw_vertical_rule(buffer: &mut Buffer, x: u16, y: u16, height: u16) {
    for row in y..y.saturating_add(height) {
        set_cell(
            buffer,
            x,
            row,
            "│",
            Style::default()
                .fg(Color::Rgb(71, 85, 105))
                .bg(Color::Rgb(17, 19, 24)),
        );
    }
}

fn fill_rect(buffer: &mut Buffer, area: Rect, symbol: &str, style: Style) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            set_cell(buffer, x, y, symbol, style);
        }
    }
}

fn write_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    for (offset, ch) in text.chars().take(usize::from(width)).enumerate() {
        set_cell(buffer, x + offset as u16, y, &ch.to_string(), style);
    }
}

fn set_cell(buffer: &mut Buffer, x: u16, y: u16, symbol: &str, style: Style) {
    if let Some(cell) = buffer.cell_mut((x, y)) {
        cell.set_symbol(symbol);
        cell.set_style(style);
    }
}

fn inset(area: Rect, margin: u16) -> Option<Rect> {
    let double = margin.saturating_mul(2);
    if area.width <= double || area.height <= double {
        return None;
    }
    Some(Rect::new(
        area.x + margin,
        area.y + margin,
        area.width - double,
        area.height - double,
    ))
}

#[allow(dead_code)]
fn rect_contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && y >= rect.y
        && x < rect.x.saturating_add(rect.width)
        && y < rect.y.saturating_add(rect.height)
}

fn buffer_to_string(buffer: &Buffer, area: Rect) -> String {
    let mut text = String::new();
    for y in area.y..area.y + area.height {
        let mut row = String::new();
        for x in area.x..area.x + area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                row.push_str(cell.symbol());
            }
        }
        text.push_str(row.trim_end());
        if y + 1 < area.y + area.height {
            text.push('\n');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split_workspace() -> local::WorkspaceSummary {
        local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-2".to_owned(),
            cols: 80,
            rows: 24,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: Some(local::WorkspacePaneSummary {
                pane_id: "pane-1".to_owned(),
                cols: 80,
                rows: 24,
                resize_policy: protocol::ResizePolicy::Fixed,
                split_axis: protocol::SplitAxis::Vertical,
                children: vec![
                    local::WorkspacePaneSummary {
                        pane_id: "pane-1".to_owned(),
                        cols: 40,
                        rows: 24,
                        resize_policy: protocol::ResizePolicy::Fixed,
                        split_axis: protocol::SplitAxis::None,
                        children: Vec::new(),
                    },
                    local::WorkspacePaneSummary {
                        pane_id: "pane-2".to_owned(),
                        cols: 40,
                        rows: 24,
                        resize_policy: protocol::ResizePolicy::Fixed,
                        split_axis: protocol::SplitAxis::None,
                        children: Vec::new(),
                    },
                ],
            }),
            tabs: Vec::new(),
        }
    }

    #[test]
    fn renders_menu_tree_pane_chrome_and_background() {
        let workspace = split_workspace();
        let mut surfaces = BTreeMap::new();
        surfaces.insert("pane-1".to_owned(), "left cached".to_owned());
        let frame = render_workspace_frame(
            WorkspaceFrameInput {
                workspace: &workspace,
                active_surface_text: "right active",
                pane_surfaces: Some(&surfaces),
                overlay: None,
            },
            100,
            20,
        );

        assert!(
            frame
                .text
                .lines()
                .next()
                .is_some_and(|line| line.contains("Sessions") && line.contains("New Session")),
            "menu should occupy the first visible row: {:?}",
            frame.text
        );
        assert!(frame.text.contains("Sessions"));
        assert!(frame.text.contains("New Session"));
        assert!(frame.text.contains("windows"));
        assert!(frame.text.contains("pane-1"));
        assert!(frame.text.contains("pane-2 active"));
        assert!(frame.text.contains("left cached"));
        assert!(frame.text.contains("right active"));
        assert!(
            frame.text.contains("╭") || frame.text.contains("╰"),
            "pane chrome should use rounded ratatui borders"
        );
    }

    #[test]
    fn hit_test_prefers_frontmost_regions() {
        let workspace = split_workspace();
        let frame = render_workspace_frame(
            WorkspaceFrameInput {
                workspace: &workspace,
                active_surface_text: "right active",
                pane_surfaces: None,
                overlay: None,
            },
            100,
            20,
        );

        assert_eq!(
            hit_test(&frame.hits, 1, 0),
            Some(&HitTarget::Menu(MenuAction::Sessions))
        );
        assert_eq!(
            hit_test(&frame.hits, 2, 3),
            Some(&HitTarget::WindowTreePane("pane-1".to_owned()))
        );
        assert!(matches!(
            hit_test(&frame.hits, 50, 3),
            Some(HitTarget::PaneContent(_))
        ));
    }

    #[test]
    fn renders_menu_overlay() {
        let workspace = split_workspace();
        let frame = render_workspace_frame(
            WorkspaceFrameInput {
                workspace: &workspace,
                active_surface_text: "right active",
                pane_surfaces: None,
                overlay: Some(&TuiOverlay {
                    title: "sessions".to_owned(),
                    lines: vec![
                        TuiOverlayLine {
                            text: "* local".to_owned(),
                            action: None,
                        },
                        TuiOverlayLine {
                            text: "click a menu item".to_owned(),
                            action: None,
                        },
                    ],
                }),
            },
            100,
            20,
        );

        assert!(frame.text.contains("sessions active"), "{:?}", frame.text);
        assert!(frame.text.contains("* local"), "{:?}", frame.text);
    }
}
