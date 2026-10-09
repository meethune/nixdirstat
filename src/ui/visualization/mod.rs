//! Resolution-adaptive visualization framework.
//!
//! Provides the [`RenderParams`] struct for resolution-adaptive rendering
//! constants, the [`Visualization`] trait for pluggable visualization modes,
//! [`VisualizationAction`] for shell dispatch, and [`VisualizationCaps`]
//! bitflags for mode capability declaration.

pub mod treemap;

use std::time::SystemTime;

use crossterm::event::KeyCode;
use ratatui::{buffer::Buffer, layout::Rect};

use crate::ui::{tree::DirNode, widgets::treemap::CellLayout};

// ---------------------------------------------------------------------------
// ColorScheme
// ---------------------------------------------------------------------------

/// Which dimension of the data drives cell coloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    /// Color by file category (Okabe-Ito palette). Current behavior.
    #[default]
    FileType,
    /// Heatmap by modification time (cold = old, hot = recent).
    Mtime,
    /// Color by nesting depth (gradient from root to deepest leaf).
    Depth,
}

// ---------------------------------------------------------------------------
// TimeRange / ColorContext
// ---------------------------------------------------------------------------

/// The time span `[min, max]` of modification times in the current view.
///
/// Used by [`crate::ui::colors::resolve_color_mtime`] to map each entry's
/// mtime to a normalised position on the blue→white→red gradient.
#[derive(Debug, Clone, Copy)]
pub struct TimeRange {
    /// Oldest modification time in the view (maps to the cold/blue end).
    pub min: SystemTime,
    /// Most recent modification time in the view (maps to the hot/red end).
    pub max: SystemTime,
}

/// Context passed to [`crate::ui::colors::resolve_color`] beyond the file extension.
///
/// Groups the per-view data that the active [`ColorScheme`] needs to resolve a
/// single entry's color.
#[derive(Debug, Clone, Copy)]
pub struct ColorContext {
    /// Modification-time range for the view; `None` when mtime data is unavailable.
    pub time_range: Option<TimeRange>,
    /// Maximum nesting depth in the current view (0 = only the root).
    pub max_depth: u16,
    /// Nesting depth of the entry being colored (0 = root of the current view).
    pub depth: u16,
}

// ---------------------------------------------------------------------------
// Scaling helpers
// ---------------------------------------------------------------------------

/// Linearly interpolate a `u16` parameter over an integer dimension.
///
/// Returns `val_min` when `dim <= ref_min`, `val_max` when `dim >= ref_max`,
/// and rounds to the nearest integer for intermediate values.
fn scale_u16(dim: u64, ref_min: u64, ref_max: u64, val_min: u16, val_max: u16) -> u16 {
    if dim <= ref_min {
        return val_min;
    }
    if dim >= ref_max {
        return val_max;
    }
    let range = ref_max - ref_min;
    let offset = dim - ref_min;
    let val_range = u64::from(val_max) - u64::from(val_min);
    // Round to nearest: add range/2 before integer division.
    let result = u64::from(val_min) + (val_range * offset + range / 2) / range;
    // result is bounded by [val_min, val_max] ⊆ [0, u16::MAX].
    u16::try_from(result).unwrap_or(val_max)
}

/// Linearly interpolate a `u32` parameter over an integer dimension.
///
/// Returns `val_min` when `dim <= ref_min`, `val_max` when `dim >= ref_max`,
/// and rounds to the nearest integer for intermediate values.
fn scale_u32(dim: u64, ref_min: u64, ref_max: u64, val_min: u32, val_max: u32) -> u32 {
    if dim <= ref_min {
        return val_min;
    }
    if dim >= ref_max {
        return val_max;
    }
    let range = ref_max - ref_min;
    let offset = dim - ref_min;
    let val_range = u64::from(val_max) - u64::from(val_min);
    let result = u64::from(val_min) + (val_range * offset + range / 2) / range;
    // result is bounded by [val_min, val_max] ⊆ [0, u32::MAX].
    u32::try_from(result).unwrap_or(val_max)
}

// ---------------------------------------------------------------------------
// RenderParams
// ---------------------------------------------------------------------------

/// Resolution-adaptive rendering parameters computed from terminal dimensions.
///
/// All thresholds scale linearly between a minimum (tuned for 80×24) and a
/// maximum (tuned for 300×80+). The scaling function is:
///   `value = min + (max − min) × factor`
/// where `factor = clamp((dimension − ref_min) / (ref_max − ref_min), 0, 1)`.
///
/// Computed once per frame by [`RenderParams::from_area`] and threaded
/// through to every visualization mode's [`Visualization::render`] and
/// [`Visualization::handle_key`] calls.
#[derive(Debug, Clone)]
pub struct RenderParams {
    /// Terminal area these params were computed from.
    pub area: Rect,

    // --- Label thresholds ---
    /// Minimum cell width (in terminal columns) to show any label.
    pub label_min_width: u16,
    /// Minimum cell height (in terminal rows) to show any label.
    pub label_min_height: u16,
    /// Minimum cell height to show a second line (size metadata).
    pub label_detail_min_height: u16,

    // --- Directory rendering ---
    /// Directory indent width in columns (scales with terminal width).
    pub dir_indent: u16,
    /// Minimum cell area (in terminal cells) to recurse into a directory.
    pub dir_recurse_threshold: u32,
    /// Minimum cell dimension to apply the nesting indent.
    pub dir_nesting_min: u16,

    // --- Vignette ---
    /// Number of pixel rings for the outer vignette.
    pub vignette_outer_rings: u16,
    /// Number of pixel rings for the inner vignette (0 = disabled).
    pub vignette_inner_rings: u16,
    /// Minimum pixel dimension to apply any vignette.
    pub vignette_min_size: u16,
}

impl RenderParams {
    /// Compute resolution-adaptive rendering parameters from a terminal area.
    ///
    /// Each parameter is linearly interpolated between its minimum value
    /// (tuned for an 80×24 terminal) and its maximum value (tuned for a
    /// 300×80 terminal), then clamped to that range.
    pub fn from_area(area: Rect) -> Self {
        let w = u64::from(area.width);
        let h = u64::from(area.height);
        let area_px = w.saturating_mul(h);
        let min_dim = w.min(h);

        Self {
            area,
            label_min_width: scale_u16(w, 80, 300, 8, 16),
            label_min_height: scale_u16(h, 24, 80, 2, 3),
            label_detail_min_height: scale_u16(h, 24, 80, 4, 6),
            dir_indent: scale_u16(w, 80, 300, 1, 4),
            dir_recurse_threshold: scale_u32(area_px, 1920, 24000, 2, 8),
            dir_nesting_min: scale_u16(min_dim, 24, 80, 6, 16),
            vignette_outer_rings: scale_u16(min_dim, 24, 80, 1, 3),
            vignette_inner_rings: scale_u16(min_dim, 24, 80, 0, 2),
            vignette_min_size: scale_u16(min_dim, 24, 80, 6, 12),
        }
    }

    /// Returns `true` when the terminal is large enough to render an
    /// overview+detail split layout (≥200 columns, ≥50 rows).
    pub const fn use_overview_detail(&self) -> bool {
        self.area.width >= 200 && self.area.height >= 50
    }
}

// ---------------------------------------------------------------------------
// VisualizationCaps
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    /// What a visualization mode can do.
    ///
    /// The explorer shell inspects these flags to decide which UI elements
    /// to show and which sync behaviors to enable.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct VisualizationCaps: u8 {
        /// Mode supports spatial navigation (arrow keys move between cells).
        const SPATIAL_NAV    = 0b0000_0001;
        /// Mode supports drill-down into directories.
        const DRILL_DOWN     = 0b0000_0010;
        /// Mode supports highlight sync from tree selection.
        const HIGHLIGHT_SYNC = 0b0000_0100;
        /// Mode has a selectable cell that can be shown in the status bar.
        const CELL_SELECT    = 0b0000_1000;
    }
}

// ---------------------------------------------------------------------------
// VisualizationAction
// ---------------------------------------------------------------------------

/// Action returned by a visualization's key handler, telling the explorer
/// shell what to do without the mode needing access to `ExplorerState`.
#[derive(Debug, Clone)]
pub enum VisualizationAction {
    /// Key was consumed by the mode. No shell action needed.
    Consumed,
    /// Mode wants to drill into a directory at this relative path.
    DrillInto(Vec<String>),
    /// Mode wants to drill up one level.
    DrillUp,
    /// Mode didn't handle this key. Shell should try global bindings.
    Ignored,
}

// ---------------------------------------------------------------------------
// Visualization trait
// ---------------------------------------------------------------------------

/// A pluggable visualization mode for the bottom panel.
///
/// Each mode owns its state, renders itself, and handles its own keys.
/// The explorer shell orchestrates lifecycle (creation, switching, sync)
/// and applies returned [`VisualizationAction`]s to `ExplorerState`.
///
/// # Implementing a new mode
///
/// 1. Create a struct that holds the mode's state and `#[derive(Debug)]`.
/// 2. Implement this trait, providing at least `name`, `capabilities`,
///    `render`, and `handle_key`.
/// 3. Override the optional methods (`set_highlight`, `selected_item`,
///    `selected_path`, `reset_on_zoom`) that match the declared capabilities.
pub trait Visualization: std::fmt::Debug + Send {
    /// Human-readable name used in panel titles and mode-switching UI.
    fn name(&self) -> &'static str;

    /// Declare what this mode supports.
    fn capabilities(&self) -> VisualizationCaps;

    /// Render the visualization into `area`.
    ///
    /// Receives the [`DirNode`] subtree at the current zoom level (not the
    /// full tree), resolution-adaptive parameters, and the active color
    /// scheme. Writes directly into the ratatui [`Buffer`].
    fn render(
        &mut self,
        node: &DirNode,
        area: Rect,
        buf: &mut Buffer,
        params: &RenderParams,
        color_scheme: &ColorScheme,
    );

    /// Handle a key press. Returns an action for the shell to apply.
    fn handle_key(&mut self, code: KeyCode, params: &RenderParams) -> VisualizationAction;

    /// Update the highlight path from tree selection.
    ///
    /// Called by the shell when [`VisualizationCaps::HIGHLIGHT_SYNC`] is
    /// declared. Default implementation is a no-op.
    fn set_highlight(&mut self, _path: Option<&[String]>) {}

    /// Return info about the currently selected cell for the status bar.
    ///
    /// Called by the shell when [`VisualizationCaps::CELL_SELECT`] is
    /// declared. Default returns `None`.
    fn selected_item(&self) -> Option<&CellLayout> {
        None
    }

    /// Return the path of the currently selected item for tree↔visualization sync.
    ///
    /// Default returns `None`.
    fn selected_path(&self) -> Option<&[String]> {
        None
    }

    /// Reset mode-specific state after a zoom operation.
    ///
    /// Default implementation is a no-op.
    fn reset_on_zoom(&mut self) {}

    /// Returns a mutable reference to `self` as `dyn Any`.
    ///
    /// Implementations must return `self`. Used in tests to downcast to the
    /// concrete visualization type for state inspection and injection.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ratatui::layout::Rect;

    use super::*;

    #[test]
    fn render_params_at_80x24() {
        let p = RenderParams::from_area(Rect::new(0, 0, 80, 24));
        assert_eq!(p.label_min_width, 8);
        assert_eq!(p.label_min_height, 2);
        assert_eq!(p.dir_indent, 1);
        assert_eq!(p.dir_recurse_threshold, 2);
        assert_eq!(p.vignette_outer_rings, 1);
        assert_eq!(p.vignette_inner_rings, 0);
    }

    #[test]
    fn render_params_at_300x80() {
        let p = RenderParams::from_area(Rect::new(0, 0, 300, 80));
        assert_eq!(p.label_min_width, 16);
        assert_eq!(p.label_min_height, 3);
        assert_eq!(p.dir_indent, 4);
        assert_eq!(p.dir_recurse_threshold, 8);
        assert_eq!(p.vignette_outer_rings, 3);
        assert_eq!(p.vignette_inner_rings, 2);
    }

    #[test]
    fn use_overview_detail_at_200x50() {
        let p = RenderParams::from_area(Rect::new(0, 0, 200, 50));
        assert!(p.use_overview_detail());
    }

    #[test]
    fn no_overview_detail_below_threshold() {
        let p = RenderParams::from_area(Rect::new(0, 0, 199, 50));
        assert!(!p.use_overview_detail());
    }

    proptest! {
        #[test]
        fn render_params_all_within_bounds(
            w in 1u16..=500,
            h in 1u16..=200,
        ) {
            let p = RenderParams::from_area(Rect::new(0, 0, w, h));
            prop_assert!(p.label_min_width >= 4);
            prop_assert!(p.label_min_height >= 1);
            prop_assert!(p.dir_indent >= 1);
            prop_assert!(p.dir_recurse_threshold >= 1);
            prop_assert!(p.vignette_outer_rings >= 1);
        }
    }

    #[test]
    fn render_params_monotonic_with_width() {
        let small = RenderParams::from_area(Rect::new(0, 0, 80, 24));
        let large = RenderParams::from_area(Rect::new(0, 0, 300, 80));
        assert!(large.label_min_width >= small.label_min_width);
        assert!(large.dir_indent >= small.dir_indent);
        assert!(large.dir_recurse_threshold >= small.dir_recurse_threshold);
        assert!(large.vignette_outer_rings >= small.vignette_outer_rings);
    }
}
