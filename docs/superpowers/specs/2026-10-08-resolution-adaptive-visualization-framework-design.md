# Resolution-Adaptive Rendering and Visualization Framework

**Issue:** #83
**Date:** 2026-10-08
**Status:** Design

## Summary

The current treemap rendering uses hardcoded visual constants tuned for ~80×24
terminals. On high-resolution terminals (300+ columns, 80+ rows), labels,
vignettes, indents, and recurse thresholds produce suboptimal results. This
design introduces a trait-based visualization framework that decouples
visualization modes from the explorer shell, makes all rendering constants
resolution-adaptive, adds orthogonal color mode switching, and provides an
overview+detail split for large terminals.

The framework is designed for extensibility: adding a new visualization mode
means implementing a trait and registering it — zero changes to the explorer
shell, event routing, or render dispatch. Downstream issues #64 (flame graph,
sunburst, histograms, heatmaps) and #62 (table view, bar chart) build on this
foundation.

## 1. RenderParams — Resolution-Adaptive Constants

All visual constants currently hardcoded in the treemap become functions of the
terminal area. `RenderParams` is computed once per frame and threaded through to
all visualization modes.

### Struct

```rust
/// Resolution-adaptive rendering parameters computed from terminal dimensions.
///
/// All thresholds scale linearly between a minimum (tuned for 80×24) and
/// maximum (tuned for 300×80+). The scaling function is:
///   value = min + (max - min) * factor
/// where factor = clamp((dimension - ref_min) / (ref_max - ref_min), 0, 1)
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
    /// Minimum cell dimensions to apply the nesting indent.
    pub dir_nesting_min: u16,

    // --- Vignette ---
    /// Number of pixel rings for the outer vignette.
    pub vignette_outer_rings: u16,
    /// Number of pixel rings for the inner vignette (0 = disabled).
    pub vignette_inner_rings: u16,
    /// Minimum pixel dimensions to apply any vignette.
    pub vignette_min_size: u16,
}
```

### Scaling Logic

Linear interpolation between reference points:

| Parameter | At 80 cols | At 300 cols | Scale dimension |
|-----------|-----------|-------------|-----------------|
| `label_min_width` | 8 | 16 | width |
| `label_min_height` | 2 | 3 | height |
| `label_detail_min_height` | 4 | 6 | height |
| `dir_indent` | 1 | 4 | width |
| `dir_recurse_threshold` | 2 | 8 | area (w×h) |
| `dir_nesting_min` | 6 | 16 | min(w, h) |
| `vignette_outer_rings` | 1 | 3 | min(w, h) |
| `vignette_inner_rings` | 0 | 2 | min(w, h) |
| `vignette_min_size` | 6 | 12 | min(w, h) |

The reference points are tunable constants in the constructor, not magic numbers
scattered through rendering code.

### Tiered Label Density

Three tiers based on cell dimensions and `RenderParams`:

- **Large cells** (width ≥ `label_min_width * 2`, height ≥ `label_detail_min_height`):
  name on first line, formatted size on second line.
- **Medium cells** (width ≥ `label_min_width`, height ≥ `label_min_height`):
  name only (current behavior).
- **Small cells**: no label.

### Vignette Scaling

Instead of the fixed 1px outer + 1px inner ring:

- Outer ring count scales from 1 (small terminals) to 3 (large terminals).
- Inner ring count scales from 0 to 2.
- Darkening amount per ring decreases with ring count (ring 1 = 30%, ring 2 =
  15%, ring 3 = 8%).
- `vignette_min_size` prevents vignette on cells too small to benefit.

### Creation Point

`RenderParams::from_area(area)` is called in `render_visualization_section`
before dispatching to the active visualization mode. The mode receives
`&RenderParams` in both `render()` and `handle_key()`.

## 2. ColorScheme — Orthogonal Color Modes

Color mode is independent of visualization mode. Every visualization receives a
`ColorScheme` and applies it however makes sense for that rendering backend.

### Enum

```rust
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
```

### Color Resolution Functions

Each scheme is a function
`(node: &DirNode, depth: u16, time_range: &TimeRange) -> Color`:

- **FileType**: existing `file_color()` / `category_color()` — unchanged.
- **Mtime**: maps `node.mtime` into a sequential gradient (blue→white→red)
  relative to the scan's min/max mtime range. `TimeRange` holds the global
  min/max, computed once when entering explorer state.
- **Depth**: maps the node's absolute nesting depth into a sequential gradient
  (light→dark). Max depth is computed once from the tree.

### Integration

- `ExplorerState` gains a `color_scheme: ColorScheme` field (default `FileType`).
- Keybinding `c` cycles through schemes: FileType → Mtime → Depth → FileType.
- The `Visualization` trait's `render()` receives `&ColorScheme`. Each mode
  calls a shared `resolve_color(node, scheme, context)` utility rather than
  directly calling `file_color()`.
- The extension legend panel adapts: in `FileType` mode it shows the existing
  category legend; in `Mtime`/`Depth` mode it shows a gradient scale with labels.

### NO_COLOR Support

Mtime and Depth gradients have grayscale fallback palettes, following the same
`is_color_enabled()` check as the existing category palette.

## 3. Visualization Trait — The Core Abstraction

All visualization modes implement this trait. Modes are fully self-contained.
The explorer shell adapts to mode capabilities rather than hardcoding per-mode
behavior. The split-borrow problem is solved by returning actions instead of
mutating shared state.

### Capabilities

```rust
bitflags::bitflags! {
    /// What a visualization mode can do. The explorer shell inspects these
    /// to decide which UI elements to show and which sync behaviors to enable.
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
```

### Actions

```rust
/// Action returned by a visualization's key handler, telling the explorer
/// shell what to do without the mode needing access to ExplorerState.
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
```

### Trait Definition

```rust
/// A pluggable visualization mode for the bottom panel.
///
/// Each mode owns its state, renders itself, and handles its own keys.
/// The explorer shell orchestrates lifecycle (creation, switching, sync)
/// and applies returned actions to ExplorerState.
pub trait Visualization: std::fmt::Debug {
    /// Human-readable name for panel title and mode-switching UI.
    fn name(&self) -> &str;

    /// Declare what this mode supports.
    fn capabilities(&self) -> VisualizationCaps;

    /// Render the visualization into `area`.
    ///
    /// Receives the DirNode subtree at the current zoom level (not the
    /// full tree), resolution-adaptive parameters, and the active color
    /// scheme. Writes directly into the ratatui Buffer.
    fn render(
        &mut self,
        node: &DirNode,
        area: Rect,
        buf: &mut Buffer,
        params: &RenderParams,
        color_scheme: &ColorScheme,
    );

    /// Handle a key press. Returns an action for the shell to apply.
    fn handle_key(
        &mut self,
        code: KeyCode,
        params: &RenderParams,
    ) -> VisualizationAction;

    /// Set the highlight path from tree selection (called by the shell
    /// when HIGHLIGHT_SYNC is declared). Default is no-op.
    fn set_highlight(&mut self, _path: Option<&[String]>) {}

    /// Return info about the currently selected cell for the status bar
    /// (called by the shell when CELL_SELECT is declared). Default is None.
    fn selected_item(&self) -> Option<&CellLayout> { None }

    /// Return the path of the currently selected item, for tree↔viz sync.
    /// Default is None.
    fn selected_path(&self) -> Option<&[String]> { None }

    /// Reset mode-specific state after a zoom operation.
    fn reset_on_zoom(&mut self) {}
}
```

### Shell Dispatch Pattern

```
render_visualization_section:
    1. Compute RenderParams from area
    2. Resolve DirNode at current treemap_root
    3. If params.use_overview_detail():
         Split area 30/70
         Render overview with full zoom-root node
         Render detail with selected subtree
       Else:
         Render visualization into full area
    4. If caps.contains(CELL_SELECT) and selected_item() is Some:
         Render status bar
    5. Render breadcrumb bar (unchanged)

handle_explorer_event (when viz panel focused):
    1. Call visualization.handle_key(code, params)
    2. Match on returned VisualizationAction:
       - Consumed → done
       - DrillInto(path) → state.zoom_into_path(path)
       - DrillUp → state.zoom_out()
       - Ignored → fall through to global keybindings (Tab, ?, w, c, v, R, etc.)

sync highlight:
    If caps.contains(HIGHLIGHT_SYNC):
        visualization.set_highlight(selected_path)

sync tree to viz selection:
    If caps.contains(CELL_SELECT):
        Use selected_path() to drive tree_state.select()
```

### Mode Switching

- Keybindings `1`–`6` switch visualization mode. For this issue, only `1`
  (Treemap) is registered — pressing `2`–`6` is a no-op. The keybindings and
  the `switch_visualization` method exist so that #64 and #62 can register
  their modes without touching event handling.
- Switching swaps `self.visualization` with a new `Box<dyn Visualization>`.
  The old mode's state is dropped; the new mode starts fresh.
- Mode list is a static registry: a `Vec` or array of
  `(VisualizationMode, fn() -> Box<dyn Visualization>)` pairs. Future issues
  add entries to this registry.

## 4. TreemapVisualization — Migrating the Existing Treemap

The existing `TreemapWidget` + `TreemapState` + hardcoded constants are
refactored into a `TreemapVisualization` struct implementing `Visualization`.

### Struct

```rust
pub struct TreemapVisualization {
    /// Spatial layout and selection state (moved from ExplorerState).
    state: TreemapState,
}
```

### Rendering Changes

1. **`paint_recursive`** receives `&RenderParams` instead of using hardcoded
   values:
   - `cell_area >= 2` → `cell_area >= params.dir_recurse_threshold`
   - `cell_rect.width >= 6 && cell_rect.height >= 6` →
     `cell_rect.width >= params.dir_nesting_min && cell_rect.height >= params.dir_nesting_min`
   - Indent `1` column → `params.dir_indent` columns

2. **Label overlay** uses tiered density from `RenderParams`:
   - `rect.width < 8 || rect.height < 2` →
     `rect.width < params.label_min_width || rect.height < params.label_min_height`
   - New: if `rect.height >= params.label_detail_min_height && rect.width >= params.label_min_width * 2`,
     render a second line with `format_size(cell.size)`

3. **`darken_edges`** becomes adaptive:
   - Takes `params.vignette_outer_rings` and `params.vignette_inner_rings`
   - Applies `N` concentric rings with decreasing darkening amounts instead of
     the current fixed 1+1 rings
   - `params.vignette_min_size` gates whether any vignette is applied at all

4. **Color resolution** switches from calling `file_color()` directly to calling
   the shared `resolve_color()` utility with the active `ColorScheme`.

### Trait Implementation

```rust
impl Visualization for TreemapVisualization {
    fn name(&self) -> &str { "Treemap" }

    fn capabilities(&self) -> VisualizationCaps {
        VisualizationCaps::SPATIAL_NAV
            | VisualizationCaps::DRILL_DOWN
            | VisualizationCaps::HIGHLIGHT_SYNC
            | VisualizationCaps::CELL_SELECT
    }

    fn render(&mut self, node: &DirNode, area: Rect, buf: &mut Buffer,
              params: &RenderParams, color_scheme: &ColorScheme) {
        // Existing TreemapWidget logic, using params instead of hardcoded
        // constants and color_scheme instead of file_color()
    }

    fn handle_key(&mut self, code: KeyCode, params: &RenderParams)
        -> VisualizationAction {
        // Existing handle_treemap_keys logic, returning actions instead
        // of mutating ExplorerState directly
    }

    fn set_highlight(&mut self, path: Option<&[String]>) {
        self.state.highlighted_path = path.map(|p| p.to_vec());
    }

    fn selected_item(&self) -> Option<&CellLayout> {
        self.state.selected_index
            .and_then(|i| self.state.layout.cells.get(i))
    }

    fn selected_path(&self) -> Option<&[String]> {
        self.selected_item().map(|c| c.path.as_slice())
    }

    fn reset_on_zoom(&mut self) {
        self.state.selected_index = None;
        self.state.highlighted_path = None;
        self.state.layout = TreemapLayout::default();
    }
}
```

### What Stays Unchanged

- `PixelGrid` — preserved as-is (new adaptive vignette method added alongside
  the existing one)
- `CellLayout`, `TreemapLayout`, `TreemapState`, `Direction`, `move_selection`
  — all preserved
- Squarify algorithm and coordinate helpers — unchanged

### What Goes Away

- `TreemapWidget` as a public `StatefulWidget` — rendering logic moves into
  `TreemapVisualization::render()`. The `Visualization` trait replaces `StatefulWidget`
  dispatch.

## 5. Overview+Detail Split

At large terminal sizes (≥200 columns, ≥50 rows), the bottom panel splits into
two regions.

### Activation Threshold

```rust
impl RenderParams {
    pub fn use_overview_detail(&self) -> bool {
        self.area.width >= 200 && self.area.height >= 50
    }
}
```

### Layout

```
┌─ Disk Usage ──────────────────────────────────────────────────┐
│┌─ Overview (30%) ─────────┐┌─ Detail (70%) ──────────────────┐│
││ Full tree at zoom root    ││ Subtree under selected dir       ││
││ Selected dir highlighted  ││ Full resolution, interactive     ││
│└──────────────────────────┘└─────────────────────────────────┘│
└───────────────────────────────────────────────────────────────┘
```

- **Overview**: a second `TreemapVisualization` instance, rendered
  non-interactively. Shows the full tree at the current `treemap_root` and
  highlights the directory that the detail view is zoomed into.
- **Detail**: the primary interactive `Visualization` instance. Receives
  keyboard focus, spatial navigation, and all key handling.

### Management

The shell owns the split decision, not the visualization.
`render_visualization_section` checks `params.use_overview_detail()`:

- **Yes**: splits the area 30/70 horizontally, renders an overview instance
  with the full zoom-root node, renders the detail instance with the selected
  subtree node.
- **No**: renders the single active visualization in the full area.

The overview instance is stored in `ExplorerState`:

```rust
overview: Option<Box<dyn Visualization>>,
```

Created/destroyed when terminal resize crosses the threshold. The overview
always uses `TreemapVisualization` regardless of what the detail mode is — the
treemap is the natural overview representation even when the detail view uses a
different visualization.

### Sync

When the user selects a directory in the detail view (or navigates the tree
panel), the overview's highlight updates via `set_highlight()`.

## 6. ExplorerState Changes

### Field Changes

```rust
pub struct ExplorerState {
    // --- Removed ---
    // treemap_state: TreemapState,           // moved into TreemapVisualization

    // --- Added ---
    visualization: Box<dyn Visualization>,    // active viz mode (detail)
    overview: Option<Box<dyn Visualization>>, // overview pane (large terminals)
    color_scheme: ColorScheme,                // orthogonal color mode
    time_range: Option<TimeRange>,            // min/max mtime for Mtime scheme
    max_depth: u16,                           // max tree depth for Depth scheme
    last_render_params: Option<RenderParams>, // cached for event handling

    // --- Unchanged ---
    // tree, tree_state, extension_stats, treemap_root, focus,
    // sort_field, sort_ascending, popup, error_message, legend_scroll,
    // scan_root, free_space, warnings, search_*, preview_*, freshness,
    // size_accuracy, filesystem_type
}
```

`TimeRange` and `max_depth` are computed once in `ExplorerState::new()` by
walking the `DirNode` tree.

### API Surface Changes

Removed:
- `treemap_state()` / `treemap_state_mut()`
- `tree_and_treemap_state_mut()`
- `sync_treemap_highlight()` — moves into shell dispatch
- `sync_tree_to_treemap_selection()` — moves into shell dispatch

Added:
- `visualization() -> &dyn Visualization`
- `visualization_mut() -> &mut dyn Visualization`
- `tree_and_visualization_mut() -> (&DirNode, &mut dyn Visualization)`
- `color_scheme() -> ColorScheme`
- `cycle_color_scheme()` — FileType → Mtime → Depth → FileType
- `switch_visualization(mode: VisualizationMode)`
- `time_range() -> Option<&TimeRange>`
- `max_depth() -> u16`

### Event Handling Changes

`handle_treemap_keys` is deleted. Its logic splits between:
- `TreemapVisualization::handle_key()` — spatial nav, Enter, Backspace
- New `handle_viz_global_keys()` — Tab, `?`, `w`, `c`, `v`, `R`, Esc, mode
  switch keys

The match in `handle_explorer_event`:

```rust
PanelFocus::Treemap => {
    let params = last_render_params_or_default();
    match state.visualization_mut().handle_key(key.code, &params) {
        VisualizationAction::Consumed => {},
        VisualizationAction::DrillInto(path) => state.zoom_into_path(path),
        VisualizationAction::DrillUp => state.zoom_out(),
        VisualizationAction::Ignored => {
            handle_viz_global_keys(key.code, state)
        },
    }
}
```

### Zoom Lifecycle

`reset_after_zoom()` calls `self.visualization_mut().reset_on_zoom()` instead of
directly clearing `treemap_state.selected_index`.

### Legend Adaptation

When `color_scheme` is not `FileType`, the legend panel switches to a gradient
scale:

```rust
pub enum LegendContent<'a> {
    Extensions {
        stats: &'a [ExtensionStat],
        total_size: u64,
        scroll_offset: usize,
    },
    Gradient {
        scheme: ColorScheme,
        label_min: String,
        label_max: String,
    },
}
```

### New Keybindings

| Key | Action | Context |
|-----|--------|---------|
| `c` | Cycle color scheme | Viz panel + tree panel |
| `1`–`6` | Switch visualization mode | Viz panel |

## 7. Testing Strategy

### Unit Tests — RenderParams

- **Property tests** (`proptest`): for any `(width, height)` in
  `[1, 500] × [1, 200]`, all computed parameters are within sane bounds
  (e.g. `label_min_width >= 4`, `dir_indent >= 1`, `vignette_outer_rings >= 1`).
- **Boundary tests**: verify exact values at the reference points (80×24,
  300×80) match the tuned constants.
- **Monotonicity tests**: increasing terminal width monotonically increases
  (or holds) every width-dependent parameter.

### Unit Tests — ColorScheme

- `resolve_color` for each scheme returns valid RGB colors.
- Mtime gradient: min mtime → cold color, max mtime → hot color, midpoint →
  midpoint color.
- Depth gradient: depth 0 → lightest, max depth → darkest.
- NO_COLOR: all schemes produce grayscale when `NO_COLOR` is set.

### Unit Tests — Visualization Trait Conformance

- `TreemapVisualization::capabilities()` returns the expected flags.
- `handle_key` returns `Ignored` for keys the mode doesn't handle.
- `handle_key` returns `DrillInto` on Enter with a selected directory cell.
- `handle_key` returns `DrillUp` on Backspace.
- `reset_on_zoom` clears selection and layout.
- `set_highlight` round-trips through `selected_path`.

### Unit Tests — TreemapVisualization Rendering

All existing treemap tests are preserved. They now construct a
`TreemapVisualization`, call `render()` with explicit `RenderParams` and
`ColorScheme::FileType`, and assert the same invariants.

New parametric tests render the same tree at 80×24 vs 300×80, asserting:
- Label count is lower at high res (labels appear on fewer, larger cells only).
- Vignette ring count is higher.
- Directory indent is wider.

### Unit Tests — Explorer Shell

- `VisualizationAction::DrillInto` triggers `zoom_into_path`.
- `VisualizationAction::Ignored` falls through to global keys.
- `cycle_color_scheme` round-trips through all three schemes.
- Overview instance is created when area crosses the threshold, destroyed when
  it shrinks.

### Integration Tests

All existing event-handling tests in `mod.rs::tests` are preserved with minimal
changes — they exercise the shell dispatch, which still works the same way
through the trait indirection.

### VHS Visual Tests

- **`resolution-adaptive.tape`**: renders the same scan at multiple terminal
  sizes (80×24, 120×40, 200×60, 300×80) and screenshots each. Visual
  verification that labels thin out, vignettes scale, and indents widen.
- **`color-modes.tape`**: cycles through FileType → Mtime → Depth and
  screenshots each. Visual verification of gradient rendering and legend
  adaptation.
- **`overview-detail.tape`**: renders at 300×80 to trigger the split view.
  Visual verification of the 30/70 layout with overview highlighting.

## Design Decisions

### Keep PixelGrid for the Treemap

Canvas's higher-resolution markers (Braille/Octant = 2×4 per cell) only support
one foreground color per cell. HalfBlock gets two colors per cell (fg+bg), which
is what treemaps need for sharp color boundaries between adjacent regions. Use
Canvas only for future visualization modes (sunburst, flame graph) where its
float coordinate system and shape primitives add value.

### Trait Objects Over Enum Dispatch

The set of visualization modes is designed to grow (#64, #62, and potentially
community contributions). Trait objects allow adding a mode by implementing the
trait and registering it — zero changes to the shell. The `VisualizationAction`
return pattern solves the split-borrow problem cleanly.

### RenderParams as a Struct, Not Method Parameters

Centralizing all resolution-adaptive constants in one struct computed once per
frame avoids scattering scaling logic across rendering functions. It also makes
testing deterministic — tests construct `RenderParams` with explicit values
rather than depending on terminal size.

### ColorScheme Orthogonal to Visualization Mode

Color mode affects all visualization modes uniformly. Making it orthogonal means
N modes × M color schemes without N×M implementations.

### Overview Always Uses Treemap

The treemap is the natural overview representation — it shows spatial
proportions at a glance regardless of what the detail view is doing. This avoids
the complexity of pairing arbitrary overview+detail mode combinations.
