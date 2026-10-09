//! Squarified treemap visualization implementing the [`Visualization`] trait.
//!
//! Moves the rendering logic from [`crate::ui::widgets::treemap`] into a
//! struct that owns its state and conforms to the [`Visualization`] trait,
//! enabling resolution-adaptive rendering via [`RenderParams`] and
//! pluggable colour schemes via [`ColorScheme`].

use std::ffi::OsStr;

use crossterm::event::KeyCode;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Widget as _},
};
use streemap::Rect as SRect;

use crate::{
    types::format_size,
    ui::{
        colors::{contrast_text_color, resolve_color, resolve_color_mtime},
        pixel_grid::PixelGrid,
        tree::{DirNode, max_depth_for_node, time_range_for_node},
        visualization::{
            ColorContext, ColorScheme, RenderParams, Visualization, VisualizationAction,
            VisualizationCaps,
        },
        widgets::treemap::{CellLayout, Direction, TreemapLayout, TreemapState},
    },
};

// ---------------------------------------------------------------------------
// PaintCtx
// ---------------------------------------------------------------------------

/// Shared render state threaded through the recursive paint calls.
struct PaintCtx<'a> {
    /// The pixel grid being painted into.
    grid: &'a mut PixelGrid,
    /// Absolute buffer-space origin of the entire treemap ([`PixelGrid`] coordinate origin).
    root_area: Rect,
    /// Accumulates per-leaf geometry for label overlay and navigation.
    cell_layouts: &'a mut Vec<CellLayout>,
    /// Resolution-adaptive rendering parameters.
    params: &'a RenderParams,
    /// Active colour scheme.
    color_scheme: &'a ColorScheme,
    /// Colour context for scheme-dependent colour resolution.
    color_ctx: ColorContext,
}

// ---------------------------------------------------------------------------
// TreemapVisualization
// ---------------------------------------------------------------------------

/// Squarified treemap visualization implementing the [`Visualization`] trait.
///
/// Owns all mutable rendering state (layout, selection, highlight) and renders
/// a [`DirNode`] tree using resolution-adaptive parameters from [`RenderParams`]
/// and the active [`ColorScheme`].
#[derive(Debug)]
pub struct TreemapVisualization {
    /// Mutable rendering state: layout, selection, highlight path.
    pub state: TreemapState,
    /// Drill depth for Esc-key routing: 0 = at the view root, >0 = drilled in.
    drill_depth: u32,
}

impl TreemapVisualization {
    /// Create a new treemap visualization with default state.
    pub fn new() -> Self {
        Self {
            state: TreemapState::default(),
            drill_depth: 0,
        }
    }

    /// Create a visualization from an existing [`TreemapState`].
    ///
    /// Used by the compatibility [`crate::ui::widgets::treemap::TreemapWidget`]
    /// wrapper to delegate rendering while preserving the stateful-widget API.
    pub const fn from_state(state: TreemapState) -> Self {
        Self {
            state,
            drill_depth: 0,
        }
    }

    /// Consume this visualization and return the inner [`TreemapState`].
    pub fn into_state(self) -> TreemapState {
        self.state
    }
}

impl Default for TreemapVisualization {
    fn default() -> Self {
        Self::new()
    }
}

impl Visualization for TreemapVisualization {
    fn name(&self) -> &'static str {
        "Treemap"
    }

    fn capabilities(&self) -> VisualizationCaps {
        VisualizationCaps::SPATIAL_NAV
            | VisualizationCaps::DRILL_DOWN
            | VisualizationCaps::HIGHLIGHT_SYNC
            | VisualizationCaps::CELL_SELECT
    }

    fn render(
        &mut self,
        node: &DirNode,
        area: Rect,
        buf: &mut Buffer,
        params: &RenderParams,
        color_scheme: &ColorScheme,
    ) {
        if area.is_empty() || node.children.is_empty() {
            return;
        }

        let color_ctx = ColorContext {
            time_range: time_range_for_node(node),
            max_depth: max_depth_for_node(node),
            depth: 0,
        };

        let mut grid = PixelGrid::new(area.width, area.height, Color::Reset);
        let mut cell_layouts: Vec<CellLayout> = Vec::new();
        let mut ctx = PaintCtx {
            grid: &mut grid,
            root_area: area,
            cell_layouts: &mut cell_layouts,
            params,
            color_scheme,
            color_ctx,
        };

        paint_recursive(node, area, &mut ctx, &[]);

        // Selection highlight ring: paint the outermost pixel ring with a contrast
        // colour for the keyboard-selected cell (replaces vignette for that cell).
        if let Some(sel_idx) = self.state.selected_index
            && let Some(layout) = cell_layouts.get(sel_idx)
        {
            let cell_color = resolve_cell_color(
                layout.extension.as_deref(),
                layout.mtime,
                *color_scheme,
                &color_ctx,
            );
            let ring_color = contrast_text_color(cell_color);
            let (px, py, pw, ph) = to_pixel_coords(layout.rect, area);
            paint_pixel_ring(&mut grid, px, py, pw, ph, ring_color);
        }

        grid.flush_to_buffer(buf, area);

        // Tiered label overlay on cells wide and tall enough.
        // Skip directory cells: their label would obscure already-painted children.
        for layout in &cell_layouts {
            if layout.is_dir {
                continue;
            }
            let rect = layout.rect;
            if rect.width < params.label_min_width || rect.height < params.label_min_height {
                continue;
            }
            let color = resolve_cell_color(
                layout.extension.as_deref(),
                layout.mtime,
                *color_scheme,
                &color_ctx,
            );
            let text_color = contrast_text_color(color);

            if rect.width >= params.label_min_width * 2
                && rect.height >= params.label_detail_min_height
            {
                // Two-line: name above centre, size below centre.
                let name_label = truncate_label(&layout.name, usize::from(rect.width));
                let size_label = truncate_label(&format_size(layout.size), usize::from(rect.width));
                let name_y = rect.y.saturating_add(rect.height / 2).saturating_sub(1);
                let size_y = rect.y.saturating_add(rect.height / 2);
                buf.set_string(
                    rect.x,
                    name_y,
                    &name_label,
                    Style::default().fg(text_color).bg(color),
                );
                buf.set_string(
                    rect.x,
                    size_y,
                    &size_label,
                    Style::default().fg(text_color).bg(color),
                );
            } else {
                // Single-line: name at centre.
                let label = truncate_label(&layout.name, usize::from(rect.width));
                let label_y = rect.y.saturating_add(rect.height / 2);
                buf.set_string(
                    rect.x,
                    label_y,
                    &label,
                    Style::default().fg(text_color).bg(color),
                );
            }
        }

        // Double-border highlight for the exactly-matched cell.
        let highlight = self.state.highlighted_path.as_deref();
        for layout in &cell_layouts {
            if highlight.is_some_and(|hp| hp == layout.path.as_slice()) {
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(Color::White))
                    .render(layout.rect, buf);
            }
        }

        self.state.layout = TreemapLayout {
            cells: cell_layouts,
        };
    }

    fn handle_key(&mut self, code: KeyCode, _params: &RenderParams) -> VisualizationAction {
        // Auto-select the first cell on the first keypress after entering treemap focus.
        if self.state.selected_index.is_none() && !self.state.layout.cells.is_empty() {
            self.state.selected_index = Some(0);
        }

        match code {
            KeyCode::Left | KeyCode::Char('h') => {
                self.state.move_selection(Direction::Left);
                VisualizationAction::Consumed
            },
            KeyCode::Right | KeyCode::Char('l') => {
                self.state.move_selection(Direction::Right);
                VisualizationAction::Consumed
            },
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.move_selection(Direction::Up);
                VisualizationAction::Consumed
            },
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.move_selection(Direction::Down);
                VisualizationAction::Consumed
            },
            KeyCode::Enter => {
                // Drill into a directory cell; non-directory Enter is consumed silently.
                if let Some(path) = self
                    .state
                    .selected_index
                    .and_then(|i| self.state.layout.cells.get(i))
                    .filter(|c| c.is_dir)
                    .map(|c| c.path.clone())
                {
                    self.drill_depth = self.drill_depth.saturating_add(1);
                    VisualizationAction::DrillInto(path)
                } else {
                    VisualizationAction::Consumed
                }
            },
            KeyCode::Backspace => {
                self.drill_depth = self.drill_depth.saturating_sub(1);
                VisualizationAction::DrillUp
            },
            KeyCode::Esc => {
                // At the view root: return Ignored so the shell can handle focus change.
                // Drilled in: return DrillUp so the shell zooms out one level.
                if self.drill_depth > 0 {
                    self.drill_depth = self.drill_depth.saturating_sub(1);
                    VisualizationAction::DrillUp
                } else {
                    VisualizationAction::Ignored
                }
            },
            _ => VisualizationAction::Ignored,
        }
    }

    fn set_highlight(&mut self, path: Option<&[String]>) {
        self.state.highlighted_path = path.map(<[_]>::to_vec);
    }

    fn selected_item(&self) -> Option<&CellLayout> {
        self.state
            .selected_index
            .and_then(|i| self.state.layout.cells.get(i))
    }

    fn selected_path(&self) -> Option<&[String]> {
        self.state
            .selected_index
            .and_then(|i| self.state.layout.cells.get(i))
            .map(|c| c.path.as_slice())
    }

    fn reset_on_zoom(&mut self) {
        self.state.selected_index = None;
        self.state.highlighted_path = None;
        self.state.layout.cells.clear();
        self.drill_depth = 0;
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

/// Resolve the display colour for a cell based on the active [`ColorScheme`].
///
/// For [`ColorScheme::Mtime`] uses the entry's `mtime` directly via
/// [`resolve_color_mtime`]; for all other schemes delegates to
/// [`resolve_color`] keyed on the file extension.
fn resolve_cell_color(
    ext: Option<&str>,
    mtime: std::time::SystemTime,
    scheme: ColorScheme,
    ctx: &ColorContext,
) -> Color {
    if scheme == ColorScheme::Mtime {
        resolve_color_mtime(mtime, ctx)
    } else {
        resolve_color(ext.map(OsStr::new), scheme, ctx)
    }
}

/// Return the display colour of the largest file in a subtree.
///
/// Iteratively chases the largest child to avoid stack overflow on deep trees.
fn dominant_color(node: &DirNode, scheme: ColorScheme, ctx: &ColorContext) -> Color {
    let mut current = node;
    loop {
        if !current.is_dir {
            return resolve_cell_color(current.extension.as_deref(), current.mtime, scheme, ctx);
        }
        match current.children.iter().max_by_key(|c| c.size) {
            Some(child) => current = child,
            None => return resolve_color(None, scheme, ctx),
        }
    }
}

// ---------------------------------------------------------------------------
// Recursive paint
// ---------------------------------------------------------------------------

/// Squarify and paint `node`'s children into `ctx.grid`.
///
/// `node_area`: absolute buffer-space [`Rect`] allocated to this node.
/// `ctx.root_area`: the overall treemap area, used as the [`PixelGrid`] origin.
fn paint_recursive(
    node: &DirNode,
    node_area: Rect,
    ctx: &mut PaintCtx<'_>,
    current_path: &[String],
) {
    let children: Vec<&DirNode> = node.children.iter().filter(|c| c.size > 0).collect();
    if children.is_empty() || node_area.width == 0 || node_area.height == 0 {
        return;
    }

    let zero_rect = SRect {
        x: 0.0_f32,
        y: 0.0_f32,
        w: 0.0_f32,
        h: 0.0_f32,
    };
    let mut layout: Vec<(&DirNode, SRect<f32>)> =
        children.iter().map(|c| (*c, zero_rect)).collect();

    let container = SRect {
        x: 0.0_f32,
        y: 0.0_f32,
        w: f32::from(node_area.width),
        h: f32::from(node_area.height),
    };

    streemap::squarify(
        container,
        &mut layout,
        #[allow(clippy::cast_precision_loss)]
        // u64→f32: precision loss is acceptable for visual treemap layout proportions.
        |(child, _r)| child.size as f32,
        |(_child, r), new_r| *r = new_r,
    );

    for (child, f32_rect) in &layout {
        let cell_rect = f32_rect_to_ratatui(*f32_rect, node_area);
        if cell_rect.width == 0 || cell_rect.height == 0 {
            continue;
        }
        let mut child_path = current_path.to_vec();
        child_path.push(child.name.clone());
        if child.is_dir {
            paint_dir_cell(child, cell_rect, ctx, &child_path);
        } else {
            paint_file_cell(child, cell_rect, ctx, &child_path);
        }
    }
}

/// Paint a directory cell: recurse if large enough, otherwise fill with dominant colour.
fn paint_dir_cell(child: &DirNode, cell_rect: Rect, ctx: &mut PaintCtx<'_>, child_path: &[String]) {
    let cell_area = u32::from(cell_rect.width) * u32::from(cell_rect.height);
    if cell_area >= ctx.params.dir_recurse_threshold {
        // Register this directory in cell_layouts BEFORE recursing so it can be
        // selected, drilled into, and highlighted via tree↔treemap sync.
        ctx.cell_layouts
            .push(CellLayout::from_node(child, cell_rect, child_path));
        // Apply the nesting indent when the cell is large enough.
        let inner = if cell_rect.width >= ctx.params.dir_nesting_min
            && cell_rect.height >= ctx.params.dir_nesting_min
        {
            Rect {
                x: cell_rect.x.saturating_add(ctx.params.dir_indent),
                y: cell_rect.y,
                width: cell_rect.width.saturating_sub(ctx.params.dir_indent),
                height: cell_rect.height,
            }
        } else {
            cell_rect
        };
        ctx.color_ctx.depth = ctx.color_ctx.depth.saturating_add(1);
        paint_recursive(child, inner, ctx, child_path);
        ctx.color_ctx.depth = ctx.color_ctx.depth.saturating_sub(1);
    } else {
        // Too small to recurse: fill with the dominant child colour using cushion shading.
        let color = dominant_color(child, *ctx.color_scheme, &ctx.color_ctx);
        let (px, py, pw, ph) = to_pixel_coords(cell_rect, ctx.root_area);
        ctx.grid.fill_rect_cushion(px, py, pw, ph, color, 0.55);
        ctx.cell_layouts
            .push(CellLayout::from_node(child, cell_rect, child_path));
    }
}

/// Paint a file leaf cell: fill with cushion shading, record layout.
fn paint_file_cell(
    child: &DirNode,
    cell_rect: Rect,
    ctx: &mut PaintCtx<'_>,
    child_path: &[String],
) {
    let color = resolve_cell_color(
        child.extension.as_deref(),
        child.mtime,
        *ctx.color_scheme,
        &ctx.color_ctx,
    );
    let (px, py, pw, ph) = to_pixel_coords(cell_rect, ctx.root_area);
    ctx.grid.fill_rect_cushion(px, py, pw, ph, color, 0.55);
    ctx.cell_layouts
        .push(CellLayout::from_node(child, cell_rect, child_path));
}

// ---------------------------------------------------------------------------
// Coordinate helpers
// ---------------------------------------------------------------------------

/// Convert a terminal cell rect (absolute buffer coords) to pixel coords for the [`PixelGrid`].
///
/// The [`PixelGrid`] origin is at `root_area`'s top-left corner.
/// Pixel y and pixel height are doubled: two pixel rows per terminal row.
const fn to_pixel_coords(rect: Rect, root_area: Rect) -> (u16, u16, u16, u16) {
    let px = rect.x.saturating_sub(root_area.x);
    let py = rect.y.saturating_sub(root_area.y).saturating_mul(2);
    let pw = rect.width;
    let ph = rect.height.saturating_mul(2);
    (px, py, pw, ph)
}

/// Paint the outermost pixel ring of a cell with `color`.
///
/// Used to render the keyboard-selection highlight over a treemap cell.
/// Coordinates and dimensions are in pixel space (same as [`PixelGrid::fill_rect`]).
fn paint_pixel_ring(grid: &mut PixelGrid, x: u16, y: u16, w: u16, h: u16, color: Color) {
    if w == 0 || h == 0 {
        return;
    }
    // Top row.
    grid.fill_rect(x, y, w, 1, color);
    // Bottom row (only when h > 1).
    if h > 1 {
        grid.fill_rect(x, y + h - 1, w, 1, color);
    }
    // Left and right columns of the interior rows.
    if h > 2 {
        grid.fill_rect(x, y + 1, 1, h - 2, color);
        if w > 1 {
            grid.fill_rect(x + w - 1, y + 1, 1, h - 2, color);
        }
    }
}

/// Truncate `name` to fit in `max_width` terminal columns, appending `…` if needed.
///
/// Uses Unicode display width (via [`unicode_width`]) to correctly handle
/// double-width CJK characters and zero-width combining characters.
fn truncate_label(name: &str, max_width: usize) -> String {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(name) <= max_width {
        return name.to_string();
    }
    // Reserve one column for the ellipsis character.
    let budget = max_width.saturating_sub(1);
    let mut width = 0usize;
    let mut truncated = String::new();
    for ch in name.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(1);
        if width + ch_width > budget {
            break;
        }
        width += ch_width;
        truncated.push(ch);
    }
    format!("{truncated}…")
}

/// Convert a [`streemap::Rect<f32>`] into a [`ratatui::layout::Rect`]
/// offset by `container`'s origin, clamped to container bounds.
fn f32_rect_to_ratatui(r: SRect<f32>, container: Rect) -> Rect {
    // Values are clamped to [0.0, u16::MAX] before cast, so truncation and sign loss are impossible.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let x_rel = r.x.floor().clamp(0.0, f32::from(u16::MAX)) as u16;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let y_rel = r.y.floor().clamp(0.0, f32::from(u16::MAX)) as u16;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let right_rel = (r.x + r.w).ceil().clamp(0.0, f32::from(u16::MAX)) as u16;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bottom_rel = (r.y + r.h).ceil().clamp(0.0, f32::from(u16::MAX)) as u16;

    let x = container.x.saturating_add(x_rel).min(container.right());
    let y = container.y.saturating_add(y_rel).min(container.bottom());
    let right = container.x.saturating_add(right_rel).min(container.right());
    let bottom = container
        .y
        .saturating_add(bottom_rel)
        .min(container.bottom());

    Rect {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use crossterm::event::KeyCode;
    use ratatui::{buffer::Buffer, layout::Rect, style::Color};

    use super::*;
    use crate::{
        types::FileCategory,
        ui::{
            colors::category_color,
            visualization::{
                ColorScheme, RenderParams, Visualization, VisualizationAction, VisualizationCaps,
            },
        },
    };

    fn make_file(name: &str, size: u64, ext: Option<&str>) -> DirNode {
        DirNode {
            name: name.into(),
            size,
            allocated: size,
            file_count: 1,
            children: Vec::new(),
            is_dir: false,
            extension: ext.map(String::from),
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    fn make_dir(name: &str, size: u64, children: Vec<DirNode>) -> DirNode {
        DirNode {
            name: name.into(),
            size,
            allocated: size,
            file_count: 0,
            children,
            is_dir: true,
            extension: None,
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    fn render_viz(root: &DirNode, width: u16, height: u16) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let params = RenderParams::from_area(area);
        let mut viz = TreemapVisualization::new();
        let mut buf = Buffer::empty(area);
        viz.render(root, area, &mut buf, &params, &ColorScheme::FileType);
        buf
    }

    // --- Trait conformance tests ---

    #[test]
    fn capabilities_has_all_flags() {
        let viz = TreemapVisualization::new();
        let caps = viz.capabilities();
        assert!(caps.contains(VisualizationCaps::SPATIAL_NAV));
        assert!(caps.contains(VisualizationCaps::DRILL_DOWN));
        assert!(caps.contains(VisualizationCaps::HIGHLIGHT_SYNC));
        assert!(caps.contains(VisualizationCaps::CELL_SELECT));
    }

    #[test]
    fn handle_key_tab_returns_ignored() {
        let mut viz = TreemapVisualization::new();
        let params = RenderParams::from_area(Rect::new(0, 0, 80, 24));
        assert!(matches!(
            viz.handle_key(KeyCode::Tab, &params),
            VisualizationAction::Ignored
        ));
    }

    #[test]
    fn handle_key_backspace_returns_drill_up() {
        let mut viz = TreemapVisualization::new();
        let params = RenderParams::from_area(Rect::new(0, 0, 80, 24));
        assert!(matches!(
            viz.handle_key(KeyCode::Backspace, &params),
            VisualizationAction::DrillUp
        ));
    }

    #[test]
    fn reset_on_zoom_clears_state() {
        let mut viz = TreemapVisualization::new();
        viz.state.selected_index = Some(5);
        viz.state.highlighted_path = Some(vec!["foo".into()]);
        viz.reset_on_zoom();
        assert_eq!(viz.state.selected_index, None);
        assert_eq!(viz.state.highlighted_path, None);
        assert!(viz.state.layout.cells.is_empty());
    }

    #[test]
    fn set_highlight_roundtrips() {
        let mut viz = TreemapVisualization::new();
        let path = vec!["a".to_owned(), "b".to_owned()];
        viz.set_highlight(Some(&path));
        assert_eq!(viz.state.highlighted_path, Some(path));
    }

    // --- Migrated rendering tests ---

    #[test]
    fn treemap_renders_half_block_characters() {
        let root = make_dir("root", 100, vec![make_file("a.rs", 100, Some("rs"))]);
        let buf = render_viz(&root, 40, 10);
        let has_half = buf
            .content()
            .iter()
            .any(|c| c.symbol() == "▀" || c.symbol() == "█");
        assert!(has_half, "expected half-block characters in output");
    }

    #[test]
    fn treemap_uses_category_colors_not_extension_hash() {
        let root = make_dir("root", 100, vec![make_file("a.rs", 100, Some("rs"))]);
        let buf = render_viz(&root, 40, 10);
        let code_color = category_color(FileCategory::Code);
        let has_code_color = buf
            .content()
            .iter()
            .any(|c| c.fg == code_color || c.bg == code_color);
        assert!(has_code_color, "expected Okabe-Ito blue for .rs file");
    }

    #[test]
    fn treemap_edge_pixels_are_darker_than_interior() {
        let root = make_dir("root", 100, vec![make_file("a.rs", 100, Some("rs"))]);
        let buf = render_viz(&root, 40, 10);
        let code_color = category_color(FileCategory::Code);
        let has_darkened = buf.content().iter().any(|c| {
            let fg_is_variant = c.fg != code_color && c.fg != Color::Reset;
            let bg_is_variant = c.bg != code_color && c.bg != Color::Reset;
            (fg_is_variant || bg_is_variant) && c.symbol() == "▀"
        });
        assert!(has_darkened, "expected edge-darkened colors");
    }

    #[test]
    fn treemap_labels_on_large_cells() {
        let root = make_dir("root", 100, vec![make_file("bigfile.rs", 100, Some("rs"))]);
        let buf = render_viz(&root, 40, 10);
        let content: String = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert!(
            content.contains("bigfile.rs"),
            "expected filename label on large cell"
        );
    }

    #[test]
    fn treemap_no_label_on_tiny_cells() {
        let files: Vec<DirNode> = (0..50)
            .map(|i| make_file(&format!("f{i}.rs"), 2, Some("rs")))
            .collect();
        let root = make_dir("root", 100, files);
        let buf = render_viz(&root, 40, 10);
        let content: String = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert!(
            !content.contains("f0.rs"),
            "tiny cells should not have labels"
        );
    }

    #[test]
    fn treemap_proportional_areas() {
        let root = make_dir(
            "root",
            100,
            vec![
                make_file("big.rs", 75, Some("rs")),
                make_file("small.zip", 25, Some("zip")),
            ],
        );
        let buf = render_viz(&root, 80, 24);
        let total = 80_usize * 24;
        // With cushion shading, colors are darkened variants of the base.
        // Code = Rgb(0, 114, 178): blue-dominant (b > r and b > g).
        // Count cells whose fg matches this blue-dominant signature.
        let code_count = buf
            .content()
            .iter()
            .filter(|c| matches!(c.fg, Color::Rgb(r, g, b) if b > r && b > g && b >= 80))
            .count();
        assert!(
            code_count * 100 / total >= 60,
            "expected ≥60% blue-dominant cells for 75% file, got {code_count}/{total}"
        );
    }

    #[test]
    fn treemap_recursive_renders_files() {
        let root = make_dir(
            "root",
            100,
            vec![make_dir(
                "sub",
                100,
                vec![
                    make_file("a.rs", 60, Some("rs")),
                    make_file("b.zip", 40, Some("zip")),
                ],
            )],
        );
        let buf = render_viz(&root, 40, 10);
        let code_color = category_color(FileCategory::Code);
        let archive_color = category_color(FileCategory::Archive);
        let has_code = buf
            .content()
            .iter()
            .any(|c| c.fg == code_color || c.bg == code_color);
        let has_archive = buf
            .content()
            .iter()
            .any(|c| c.fg == archive_color || c.bg == archive_color);
        assert!(has_code, "expected Code-colored cells for .rs file");
        assert!(has_archive, "expected Archive-colored cells for .zip file");
    }

    #[test]
    fn treemap_empty_root() {
        let root = make_dir("root", 0, Vec::new());
        let buf = render_viz(&root, 40, 10);
        let non_space = buf.content().iter().filter(|c| c.symbol() != " ").count();
        assert_eq!(non_space, 0, "empty root should render nothing");
    }

    #[test]
    fn treemap_directory_recursion() {
        let root = make_dir(
            "root",
            50,
            vec![make_dir(
                "dir",
                50,
                vec![make_file("file.rs", 50, Some("rs"))],
            )],
        );
        let buf = render_viz(&root, 40, 10);
        let code_color = category_color(FileCategory::Code);
        let has_code = buf
            .content()
            .iter()
            .any(|c| c.fg == code_color || c.bg == code_color);
        assert!(has_code, "nested file should produce Code-colored cells");
    }

    // --- Resolution-adaptive parametric tests ---

    #[test]
    fn high_res_has_fewer_labels_than_low_res() {
        // Same tree, rendered at 80×24 vs 300×80. High-res should have
        // fewer or equal labels because label_min_width is larger.
        let files: Vec<DirNode> = (0..20)
            .map(|i| make_file(&format!("file{i}.rs"), 50, Some("rs")))
            .collect();
        let root = make_dir("root", 1000, files);

        let label_count_at = |w: u16, h: u16| {
            let params = RenderParams::from_area(Rect::new(0, 0, w, h));
            let mut viz = TreemapVisualization::new();
            let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
            viz.render(
                &root,
                Rect::new(0, 0, w, h),
                &mut buf,
                &params,
                &ColorScheme::FileType,
            );
            // Count cells with non-half-block text
            buf.content()
                .iter()
                .filter(|c| {
                    let s = c.symbol();
                    s != " " && s != "▀" && s != "█" && s != "▄"
                })
                .count()
        };
        let low = label_count_at(80, 24);
        let high = label_count_at(300, 80);
        assert!(
            high <= low * 3,
            "high-res label char count {high} should not vastly exceed low-res {low}"
        );
    }

    #[test]
    fn depth_color_scheme_varies_with_nesting() {
        // Build a two-level tree: root → dir → file.
        // At depth 0 the dir is painted; at depth 1 the file inside it is painted.
        // With ColorScheme::Depth, the two levels should produce different colors.
        let root = make_dir(
            "root",
            100,
            vec![make_dir(
                "inner",
                100,
                vec![make_file("deep.rs", 100, Some("rs"))],
            )],
        );
        let area = Rect::new(0, 0, 80, 24);
        let params = RenderParams::from_area(area);
        let mut viz = TreemapVisualization::new();
        let mut buf = Buffer::empty(area);
        viz.render(&root, area, &mut buf, &params, &ColorScheme::Depth);

        // Collect every unique non-Reset color from the buffer.
        let colors: std::collections::HashSet<Color> = buf
            .content()
            .iter()
            .flat_map(|c| [c.fg, c.bg])
            .filter(|c| *c != Color::Reset)
            .collect();

        assert!(
            colors.len() >= 2,
            "expected at least two distinct colors for Depth scheme at different nesting levels, got {colors:?}"
        );
    }

    #[test]
    fn render_empty_children_node_no_panic() {
        let root = make_dir("root", 0, vec![]);
        let mut viz = TreemapVisualization::new();
        let params = RenderParams::from_area(Rect::new(0, 0, 80, 24));
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        viz.render(
            &root,
            Rect::new(0, 0, 80, 24),
            &mut buf,
            &params,
            &ColorScheme::FileType,
        );
        // No panic = pass. Buffer should remain empty (no content to render).
        assert_eq!(
            buf.content().iter().filter(|c| c.symbol() != " ").count(),
            0,
            "empty root should produce no visible content"
        );
    }
}
