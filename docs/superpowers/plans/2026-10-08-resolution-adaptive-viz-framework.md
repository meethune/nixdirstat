# Resolution-Adaptive Visualization Framework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace hardcoded treemap visual constants with resolution-adaptive parameters, introduce a trait-based visualization framework for future extensibility, add orthogonal color mode switching (file type / mtime heatmap / depth), and an overview+detail split for large terminals.

**Architecture:** A `Visualization` trait decouples rendering modes from the explorer shell. Each mode owns its state, renders itself, handles its own keys, and returns `VisualizationAction` values for the shell to apply — solving the split-borrow problem. `RenderParams` centralizes all resolution-adaptive constants, computed once per frame from terminal dimensions. `ColorScheme` is orthogonal to visualization mode — every mode receives it and applies it uniformly.

**Tech Stack:** Rust stable (edition 2024, MSRV 1.95), ratatui, crossterm, bitflags, proptest, streemap

**Spec:** `docs/superpowers/specs/2026-10-08-resolution-adaptive-visualization-framework-design.md`

## Global Constraints

- Rust edition 2024, MSRV 1.95, stable toolchain.
- `unsafe` code is forbidden (except `src/platform/btrfs_ioctl.rs`).
- Never `#[allow(...)]` clippy except verified false positives with comment.
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` must pass.
- `just check` (fmt, clippy, test, doc, deny) before every commit.
- Conventional Commits for messages.
- `bitflags` is a new dependency — run `cargo deny check` after adding.
- All new public types need doc comments.
- `NO_COLOR` support for every color path.

## Review Focus

1. **Terminal resize mid-render:** `RenderParams` is cached from last render; if terminal shrinks dramatically between render and key event, `handle_key` receives stale params. Modes must not index out of bounds using cached layout data against new smaller areas. Test: key event with `RenderParams` from 300×80 but layout rendered at 40×10 — no panic.
2. **Empty tree node passed to visualization:** `render()` receives the node at `treemap_root`, which could be a directory with zero children or zero size. Test: `TreemapVisualization::render()` with empty-children node produces no panic and writes nothing.
3. **Mtime range of zero span:** All files have identical mtime → `TimeRange` has `min == max` → division by zero in gradient interpolation. Test: `resolve_color` with `Mtime` scheme and `min == max` returns a valid color (midpoint).
4. **Max depth of zero:** Flat directory with only files → `max_depth == 0` → division by zero in depth gradient. Test: `resolve_color` with `Depth` scheme and `max_depth == 0` returns a valid color.
5. **Overview+detail threshold oscillation:** Terminal hovers at exactly 200×50 → overview is created and destroyed every frame. Test: `RenderParams::from_area` at exactly 200×50 returns `use_overview_detail() == true` (stable, no flicker from boundary equality).

---

## File Structure

| File | Responsibility |
|------|---------------|
| `src/ui/visualization/mod.rs` | **NEW** — `Visualization` trait, `VisualizationAction`, `VisualizationCaps`, `RenderParams`, `ColorScheme`, `TimeRange`, `resolve_color()`, mode registry |
| `src/ui/visualization/treemap.rs` | **NEW** — `TreemapVisualization` implementing `Visualization`, migrated from `widgets/treemap.rs` |
| `src/ui/widgets/treemap.rs` | **MODIFY** — retains `CellLayout`, `TreemapLayout`, `TreemapState`, `Direction`, `move_selection`, coordinate helpers; removes `TreemapWidget` `StatefulWidget` impl and `paint_*` rendering functions |
| `src/ui/pixel_grid.rs` | **MODIFY** — add `darken_edges_adaptive()` method |
| `src/ui/colors.rs` | **MODIFY** — add `resolve_color()`, mtime gradient, depth gradient, grayscale fallbacks |
| `src/ui/app.rs` | **MODIFY** — replace `treemap_state` with `visualization: Box<dyn Visualization>`, add `overview`, `color_scheme`, `time_range`, `max_depth`, `last_render_params`; update API surface |
| `src/ui/views/explorer.rs` | **MODIFY** — `render_treemap_section` → `render_visualization_section` with `RenderParams` + trait dispatch + overview+detail split; adapt legend to `LegendContent` |
| `src/ui/widgets/extension_legend.rs` | **MODIFY** — add `LegendContent` enum, gradient scale rendering |
| `src/ui/mod.rs` | **MODIFY** — add `pub mod visualization;` declaration; rewrite `handle_treemap_keys` → trait dispatch via `handle_viz_global_keys`; update sync functions |
| `Cargo.toml` | **MODIFY** — add `bitflags` dependency |

---

### Task 1: RenderParams and Scaling Logic

**Files:**
- Create: `src/ui/visualization/mod.rs`
- Modify: `src/ui/mod.rs` (add `pub mod visualization;`)
- Modify: `Cargo.toml` (add `bitflags`)

**Interfaces:**
- Consumes: `ratatui::layout::Rect`
- Produces:
  - `RenderParams` struct with all fields from the spec
  - `RenderParams::from_area(area: Rect) -> Self`
  - `RenderParams::use_overview_detail(&self) -> bool`
  - `VisualizationCaps` bitflags type
  - `VisualizationAction` enum
  - `Visualization` trait (with all methods and default impls from spec)

- [ ] **Step 1: Add `bitflags` dependency to `Cargo.toml`**

Add `bitflags = "2"` to `[dependencies]`. Run `cargo deny check` to verify no advisories.

- [ ] **Step 2: Create `src/ui/visualization/mod.rs` with `RenderParams`**

Define `RenderParams` struct with all fields from the spec. Implement `from_area(area: Rect) -> Self` using linear interpolation with these reference points:

| Parameter | ref_min dim | ref_max dim | min_val | max_val | scale_dim |
|-----------|------------|------------|---------|---------|-----------|
| `label_min_width` | 80 | 300 | 8 | 16 | `area.width` |
| `label_min_height` | 24 | 80 | 2 | 3 | `area.height` |
| `label_detail_min_height` | 24 | 80 | 4 | 6 | `area.height` |
| `dir_indent` | 80 | 300 | 1 | 4 | `area.width` |
| `dir_recurse_threshold` | 1920 | 24000 | 2 | 8 | `area.width * area.height` |
| `dir_nesting_min` | 24 | 80 | 6 | 16 | `min(width, height)` |
| `vignette_outer_rings` | 24 | 80 | 1 | 3 | `min(width, height)` |
| `vignette_inner_rings` | 24 | 80 | 0 | 2 | `min(width, height)` |
| `vignette_min_size` | 24 | 80 | 6 | 12 | `min(width, height)` |

Implement `use_overview_detail(&self) -> bool` returning `self.area.width >= 200 && self.area.height >= 50`.

- [ ] **Step 3: Define `VisualizationCaps`, `VisualizationAction`, and `Visualization` trait**

In the same file, define all three exactly as specified in the spec (Section 3). The trait requires `Debug`. Default impls for `set_highlight`, `selected_item`, `selected_path`, `reset_on_zoom`.

- [ ] **Step 4: Add `pub mod visualization;` to `src/ui/mod.rs`**

Add the module declaration after `pub mod widgets;`.

- [ ] **Step 5: Write boundary tests for `RenderParams::from_area`**

In `src/ui/visualization/mod.rs` `#[cfg(test)] mod tests`:

```rust
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
```

- [ ] **Step 6: Write proptest for `RenderParams` bounds and monotonicity**

```rust
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
```

- [ ] **Step 7: Run `just check`**

Expected: all green. The `Visualization` trait is defined but not yet implemented.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/ui/visualization/mod.rs src/ui/mod.rs
git commit -m "feat(ui): add RenderParams, Visualization trait, and VisualizationAction (#83)"
```

---

### Task 2: ColorScheme and resolve_color

**Files:**
- Modify: `src/ui/visualization/mod.rs` (add `ColorScheme`, `TimeRange`, re-export)
- Modify: `src/ui/colors.rs` (add `resolve_color()`, mtime gradient, depth gradient)

**Interfaces:**
- Consumes: `DirNode` (from `ui::tree`), `is_color_enabled()` (from `ui::colors`), `category_color()`, `no_color_fallback()`
- Produces:
  - `ColorScheme` enum: `FileType`, `Mtime`, `Depth` (with `Default` = `FileType`)
  - `TimeRange { min: SystemTime, max: SystemTime }`
  - `ColorContext { time_range: Option<TimeRange>, max_depth: u16, depth: u16 }`
  - `resolve_color(ext: Option<&OsStr>, scheme: ColorScheme, ctx: &ColorContext) -> Color`

- [ ] **Step 1: Define `ColorScheme`, `TimeRange`, `ColorContext` in `src/ui/visualization/mod.rs`**

`ColorScheme` enum exactly as spec. `TimeRange` struct with `min: SystemTime, max: SystemTime`. `ColorContext` groups the data that `resolve_color` needs beyond the extension.

- [ ] **Step 2: Write failing tests for `resolve_color`**

In `src/ui/colors.rs` tests:

```rust
#[test]
fn resolve_color_filetype_matches_category_color() {
    let ctx = ColorContext { time_range: None, max_depth: 10, depth: 0 };
    let color = resolve_color(Some(OsStr::new("rs")), ColorScheme::FileType, &ctx);
    assert_eq!(color, category_color(FileCategory::Code));
}

#[test]
fn resolve_color_mtime_min_is_cold() {
    let min = SystemTime::UNIX_EPOCH;
    let max = SystemTime::UNIX_EPOCH + Duration::from_secs(86400 * 365);
    let ctx = ColorContext {
        time_range: Some(TimeRange { min, max }),
        max_depth: 0, depth: 0,
    };
    // Cold end of blue→white→red gradient: should have high blue, low red
    let color = resolve_color_mtime(min, &ctx);
    if let Color::Rgb(r, _, b) = color { assert!(b > r); }
}

#[test]
fn resolve_color_mtime_equal_range_no_panic() {
    let t = SystemTime::UNIX_EPOCH;
    let ctx = ColorContext {
        time_range: Some(TimeRange { min: t, max: t }),
        max_depth: 0, depth: 0,
    };
    let color = resolve_color_mtime(t, &ctx);
    assert!(matches!(color, Color::Rgb(..)));
}

#[test]
fn resolve_color_depth_zero_max_no_panic() {
    let ctx = ColorContext { time_range: None, max_depth: 0, depth: 0 };
    let color = resolve_color_depth(&ctx);
    assert!(matches!(color, Color::Rgb(..)));
}

#[test]
fn resolve_color_depth_gradient_darker_with_depth() {
    let ctx_shallow = ColorContext { time_range: None, max_depth: 10, depth: 0 };
    let ctx_deep = ColorContext { time_range: None, max_depth: 10, depth: 10 };
    let shallow = resolve_color_depth(&ctx_shallow);
    let deep = resolve_color_depth(&ctx_deep);
    // Shallow should be lighter (higher channel values) than deep
    if let (Color::Rgb(rs, gs, bs), Color::Rgb(rd, gd, bd)) = (shallow, deep) {
        assert!(rs + gs + bs > rd + gd + bd);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib ui::colors::tests`
Expected: compilation errors — `resolve_color` not defined yet.

- [ ] **Step 4: Implement `resolve_color` and helpers in `src/ui/colors.rs`**

- `resolve_color(ext, scheme, ctx)`: matches on `ColorScheme`, delegates to `file_color` (FileType), `resolve_color_mtime` (Mtime), or `resolve_color_depth` (Depth).
- `resolve_color_mtime(mtime, ctx)`: linearly interpolate mtime within `TimeRange` to a blue→white→red gradient. Guard `min == max` by returning the midpoint color.
- `resolve_color_depth(ctx)`: linearly interpolate `ctx.depth / ctx.max_depth` to a light→dark gradient (e.g. `Rgb(220, 220, 240)` → `Rgb(30, 30, 80)`). Guard `max_depth == 0` by returning the light end.
- Each function has a grayscale variant selected via `is_color_enabled()`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib ui::colors::tests`
Expected: all pass.

- [ ] **Step 6: Run `just check`**

Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src/ui/visualization/mod.rs src/ui/colors.rs
git commit -m "feat(ui): add ColorScheme with mtime and depth gradients (#83)"
```

---

### Task 3: Adaptive Vignette in PixelGrid

**Files:**
- Modify: `src/ui/pixel_grid.rs`

**Interfaces:**
- Consumes: `darken()` from `colors.rs`
- Produces: `PixelGrid::darken_edges_adaptive(&mut self, x, y, w, h, outer_rings: u16, inner_rings: u16, min_size: u16)`

- [ ] **Step 1: Write failing tests for `darken_edges_adaptive`**

In `src/ui/pixel_grid.rs` tests:

```rust
#[test]
fn darken_edges_adaptive_three_outer_rings() {
    let red = Color::Rgb(200, 100, 50);
    let mut grid = PixelGrid::new(20, 10, Color::Reset);
    grid.fill_rect(0, 0, 20, 20, red);
    grid.darken_edges_adaptive(0, 0, 20, 20, 3, 0, 6);
    // Ring 0 (outermost): 30% darker
    assert_eq!(get_pixel(&grid, 0, 0), Color::Rgb(140, 70, 35));
    // Ring 1: 15% darker than original
    assert_eq!(get_pixel(&grid, 1, 1), darken(red, 0.15));
    // Ring 2: 8% darker than original
    assert_eq!(get_pixel(&grid, 2, 2), darken(red, 0.08));
    // Interior: unchanged
    assert_eq!(get_pixel(&grid, 5, 5), red);
}

#[test]
fn darken_edges_adaptive_below_min_size_is_noop() {
    let red = Color::Rgb(200, 100, 50);
    let mut grid = PixelGrid::new(5, 3, Color::Reset);
    grid.fill_rect(0, 0, 4, 4, red);
    grid.darken_edges_adaptive(0, 0, 4, 4, 1, 0, 6);
    // 4 < min_size 6 → no darkening applied
    assert_eq!(get_pixel(&grid, 0, 0), red);
}

#[test]
fn darken_edges_adaptive_with_inner_rings() {
    let red = Color::Rgb(200, 100, 50);
    let mut grid = PixelGrid::new(20, 10, Color::Reset);
    grid.fill_rect(0, 0, 20, 20, red);
    grid.darken_edges_adaptive(0, 0, 20, 20, 1, 2, 6);
    // Outer ring: 30%
    assert_eq!(get_pixel(&grid, 0, 0), darken(red, 0.30));
    // Inner ring 1: 15%
    assert_eq!(get_pixel(&grid, 1, 1), darken(red, 0.15));
    // Inner ring 2: 8%
    assert_eq!(get_pixel(&grid, 2, 2), darken(red, 0.08));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib ui::pixel_grid::tests`
Expected: compilation error — method not defined.

- [ ] **Step 3: Implement `darken_edges_adaptive` on `PixelGrid`**

Use the existing `apply_ring` method in a loop. Darkening amounts: `[0.30, 0.15, 0.08]` indexed by ring number. Gate the entire method on `min(w, h) >= min_size` — if below, return immediately. Apply `outer_rings` rings from outside in, then `inner_rings` additional rings continuing inward.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib ui::pixel_grid::tests`
Expected: all pass.

- [ ] **Step 5: Run `just check`**

- [ ] **Step 6: Commit**

```bash
git add src/ui/pixel_grid.rs
git commit -m "feat(ui): add darken_edges_adaptive for scaled vignette rings (#83)"
```

---

### Task 4: TreemapVisualization — Migrate Rendering to Trait

**Files:**
- Create: `src/ui/visualization/treemap.rs`
- Modify: `src/ui/visualization/mod.rs` (add `pub mod treemap;`)
- Modify: `src/ui/widgets/treemap.rs` (remove `TreemapWidget`, `StatefulWidget` impl, `paint_*` functions, `file_color`; retain `CellLayout`, `TreemapLayout`, `TreemapState`, `Direction`, `move_selection`, coordinate helpers, `truncate_label`)

**Interfaces:**
- Consumes: `Visualization` trait, `RenderParams`, `ColorScheme`, `ColorContext`, `resolve_color()` (Task 2), `PixelGrid` + `darken_edges_adaptive` (Task 3), `CellLayout`, `TreemapLayout`, `TreemapState`, `Direction` (from `widgets/treemap`)
- Produces: `TreemapVisualization` struct implementing `Visualization`

- [ ] **Step 1: Create `src/ui/visualization/treemap.rs` with `TreemapVisualization` struct**

```rust
#[derive(Debug)]
pub struct TreemapVisualization {
    state: TreemapState,
}

impl TreemapVisualization {
    pub fn new() -> Self {
        Self { state: TreemapState::default() }
    }
}
```

- [ ] **Step 2: Move rendering logic from `widgets/treemap.rs` into `visualization/treemap.rs`**

Move `paint_recursive`, `paint_dir_cell`, `paint_file_cell`, `PaintCtx`, `dominant_color`, `paint_pixel_ring`, `to_pixel_coords`, `f32_rect_to_ratatui` into the new file. Update `PaintCtx` to carry `&RenderParams` and `&ColorScheme`. Replace every hardcoded constant with `RenderParams` field access. Replace `file_color()` calls with `resolve_color()`.

Specific replacements in `paint_dir_cell`:
- `cell_area >= 2` → `cell_area >= u32::from(params.dir_recurse_threshold)`
- `cell_rect.width >= 6 && cell_rect.height >= 6` → `cell_rect.width >= params.dir_nesting_min && cell_rect.height >= params.dir_nesting_min`
- `x: cell_rect.x.saturating_add(1)` → `x: cell_rect.x.saturating_add(params.dir_indent)`
- `width: cell_rect.width.saturating_sub(1)` → `width: cell_rect.width.saturating_sub(params.dir_indent)`

Replace `darken_edges` with `darken_edges_adaptive(px, py, pw, ph, params.vignette_outer_rings, params.vignette_inner_rings, params.vignette_min_size)`.

- [ ] **Step 3: Implement tiered label overlay in `TreemapVisualization::render`**

After `grid.flush_to_buffer`, iterate `cell_layouts`. For each non-directory cell:
- Skip if `rect.width < params.label_min_width || rect.height < params.label_min_height` (no label).
- If `rect.width >= params.label_min_width * 2 && rect.height >= params.label_detail_min_height`: render name on `rect.y + rect.height / 2 - 1`, render `format_size(cell.size)` on `rect.y + rect.height / 2`.
- Otherwise: render name only on `rect.y + rect.height / 2` (current behavior).

- [ ] **Step 4: Implement `Visualization` trait for `TreemapVisualization`**

- `name()`: return `"Treemap"`.
- `capabilities()`: all four flags.
- `render()`: the logic from steps 2–3.
- `handle_key()`: migrate from `handle_treemap_keys` in `mod.rs:530-587`. Arrow keys/hjkl → `Consumed` after `move_selection`. Enter with selected dir cell → `DrillInto(path)`. Backspace → `DrillUp`. Esc at root → `Ignored` (shell handles). Esc drilled in → `DrillUp`. Unknown keys → `Ignored`. Auto-select first cell on first keypress.
- `set_highlight`, `selected_item`, `selected_path`, `reset_on_zoom`: as specified.

- [ ] **Step 5: Write trait conformance tests**

In `src/ui/visualization/treemap.rs` tests:

```rust
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
```

- [ ] **Step 6: Migrate existing rendering tests from `widgets/treemap.rs`**

Copy the rendering tests (`treemap_renders_half_block_characters`, `treemap_proportional_areas`, etc.) into `visualization/treemap.rs` tests. Adapt them: construct `TreemapVisualization`, call `viz.render(root, area, buf, &params, &ColorScheme::FileType)` instead of using `TreemapWidget` + `StatefulWidget`. Use `RenderParams::from_area(Rect::new(0, 0, width, height))`.

- [ ] **Step 7: Add resolution-adaptive parametric tests**

```rust
#[test]
fn high_res_has_fewer_labels_than_low_res() {
    // Same tree, rendered at 80×24 vs 300×80. High-res should have
    // fewer or equal labels because label_min_width is larger.
    // Build a tree with many medium-sized files.
    let files: Vec<DirNode> = (0..20)
        .map(|i| make_file(&format!("file{i}.rs"), 50, Some("rs")))
        .collect();
    let root = make_dir("root", 1000, files);

    let label_count_at = |w, h| {
        let params = RenderParams::from_area(Rect::new(0, 0, w, h));
        let mut viz = TreemapVisualization::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        viz.render(&root, Rect::new(0, 0, w, h), &mut buf, &params, &ColorScheme::FileType);
        // Count cells with non-half-block text
        buf.content().iter()
            .filter(|c| {
                let s = c.symbol();
                s != " " && s != "▀" && s != "█" && s != "▄"
            })
            .count()
    };
    let low = label_count_at(80, 24);
    let high = label_count_at(300, 80);
    assert!(high <= low * 3, "high-res label char count {high} should not vastly exceed low-res {low}");
}
```

- [ ] **Step 8: Remove `TreemapWidget`, `StatefulWidget` impl, `paint_*`, and `file_color` from `widgets/treemap.rs`**

Retain: `CellLayout`, `CellLayout::from_node`, `Default for CellLayout`, `TreemapLayout`, `Direction`, `TreemapState`, `move_selection`, `truncate_label`, coordinate helpers (`to_pixel_coords`, `f32_rect_to_ratatui`, `dominant_color`), and all tests that exercise `move_selection` / `CellLayout`.

Actually — `to_pixel_coords`, `f32_rect_to_ratatui`, `dominant_color`, and `truncate_label` are used by the rendering code. Move these to `visualization/treemap.rs` since that's where rendering lives now. Keep in `widgets/treemap.rs` only the data types and spatial navigation logic.

- [ ] **Step 9: Run `just check`**

Expected: compilation failures in `explorer.rs` and `mod.rs` (they still reference `TreemapWidget` and old APIs). That's expected — Task 5 fixes those.

- [ ] **Step 10: Commit**

```bash
git add src/ui/visualization/treemap.rs src/ui/visualization/mod.rs src/ui/widgets/treemap.rs
git commit -m "feat(ui): implement TreemapVisualization with Visualization trait (#83)"
```

---

### Task 5: ExplorerState Migration and Shell Rewiring

**Files:**
- Modify: `src/ui/app.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/views/explorer.rs`

**Interfaces:**
- Consumes: `Visualization` trait, `TreemapVisualization` (Task 4), `RenderParams` (Task 1), `ColorScheme`, `TimeRange`, `ColorContext` (Task 2), `VisualizationAction`, `VisualizationCaps`
- Produces:
  - Updated `ExplorerState` with `visualization: Box<dyn Visualization>`, `color_scheme`, `time_range`, `max_depth`, `last_render_params`, `overview`
  - `ExplorerState::visualization() -> &dyn Visualization`
  - `ExplorerState::visualization_mut() -> &mut dyn Visualization`
  - `ExplorerState::cycle_color_scheme()`
  - `ExplorerState::time_range() -> Option<&TimeRange>`
  - `ExplorerState::max_depth() -> u16`
  - `render_visualization_section` (renamed from `render_treemap_section`)
  - `handle_viz_global_keys` (new, handles Tab/`?`/w/c/v/R/Esc/mode-switch)
  - Updated `handle_explorer_event` with trait dispatch

- [ ] **Step 1: Update `ExplorerState` fields in `src/ui/app.rs`**

Remove `treemap_state: TreemapState`. Add:
- `visualization: Box<dyn Visualization>` (initialized to `Box::new(TreemapVisualization::new())`)
- `overview: Option<Box<dyn Visualization>>` (initialized to `None`)
- `color_scheme: ColorScheme` (initialized to `ColorScheme::default()`)
- `time_range: Option<TimeRange>` (computed from tree in `new()`)
- `max_depth: u16` (computed from tree in `new()`)
- `last_render_params: Option<RenderParams>` (initialized to `None`)

Add helper `compute_time_range(node: &DirNode) -> TimeRange` and `compute_max_depth(node: &DirNode) -> u16` — recursive walks over the tree.

- [ ] **Step 2: Update `ExplorerState` API surface**

Remove: `treemap_state()`, `treemap_state_mut()`, `tree_and_treemap_state_mut()`, `sync_treemap_highlight()`, `sync_tree_to_treemap_selection()`.

Add:
- `visualization(&self) -> &dyn Visualization`
- `visualization_mut(&mut self) -> &mut dyn Visualization`
- `tree_and_visualization_mut(&mut self) -> (&DirNode, &mut dyn Visualization)`
- `color_scheme(&self) -> ColorScheme`
- `cycle_color_scheme(&mut self)` — cycles FileType → Mtime → Depth → FileType
- `time_range(&self) -> Option<&TimeRange>`
- `max_depth(&self) -> u16`
- `last_render_params(&self) -> Option<&RenderParams>`
- `set_last_render_params(&mut self, params: RenderParams)`

Update `reset_after_zoom()` to call `self.visualization.reset_on_zoom()`.

- [ ] **Step 3: Rewrite `render_treemap_section` → `render_visualization_section` in `explorer.rs`**

1. Compute `RenderParams::from_area(content_area)`.
2. Store via `state.set_last_render_params(params)`.
3. Resolve node at `treemap_root`.
4. Get `color_scheme` from state.
5. Call `state.tree_and_visualization_mut()` to get `(&DirNode, &mut dyn Visualization)`.
6. Call `viz.render(node, content_area, buf, &params, &color_scheme)`.
7. After render: if `viz.capabilities().contains(CELL_SELECT)` and `viz.selected_item().is_some()`, render status bar.
8. Render breadcrumb bar (unchanged).

Remove the `TreemapWidget` import and the `render_stateful_widget` call.

- [ ] **Step 4: Rewrite event handling in `src/ui/mod.rs`**

Delete `handle_treemap_keys`. Create `handle_viz_global_keys(code: KeyCode, state: &mut ExplorerState) -> bool` handling: Tab, `?`, `w`, `c` (cycle color), `v` (preview), `R` (refresh), Esc (zoom out or set focus to Tree), `1`–`6` (mode switch — only `1` active for now, others no-op).

Update `handle_explorer_event`:
- In `PanelFocus::Treemap` arm: get `last_render_params` (or default 80×24), call `state.visualization_mut().handle_key(code, &params)`, match on `VisualizationAction`.
- After the action match, perform sync: if `HIGHLIGHT_SYNC` → call `viz.set_highlight(tree_state.selected())`. If `CELL_SELECT` → use `viz.selected_path()` to drive `tree_state.select()`.
- Add `c` keybinding to `handle_tree_keys` for color cycling from tree panel.

- [ ] **Step 5: Update all existing tests in `app.rs`, `mod.rs`, and `explorer.rs`**

Replace all `treemap_state()` / `treemap_state_mut()` calls with the new API. Tests that inject fake `CellLayout` / `TreemapLayout` into `treemap_state_mut()` now need to work through `visualization_mut()` — downcast to `TreemapVisualization` in tests, or use the trait's `selected_item()` / `selected_path()` methods for assertions.

For `explorer.rs` tests that reference `TreemapWidget`: remove the import, the rendering tests still work because `render_explorer` → `render_visualization_section` → `viz.render()` internally.

- [ ] **Step 6: Write new shell dispatch tests**

In `src/ui/mod.rs` tests:

```rust
#[test]
fn viz_drill_into_triggers_zoom() {
    let mut state = make_explorer_state();
    state.set_focus(PanelFocus::Treemap);
    // Inject a layout with a directory cell
    // ... setup with TreemapVisualization internals ...
    // Press Enter → VisualizationAction::DrillInto
    // Assert treemap_root changed
}

#[test]
fn viz_ignored_falls_through_to_global() {
    let mut state = make_explorer_state();
    state.set_focus(PanelFocus::Treemap);
    // Tab returns Ignored from viz, shell handles it
    handle_explorer_event(&key_event(KeyCode::Tab), &mut state);
    assert_eq!(state.focus(), PanelFocus::Legend);
}

#[test]
fn cycle_color_scheme_roundtrips() {
    let mut state = make_explorer_state();
    assert_eq!(state.color_scheme(), ColorScheme::FileType);
    state.cycle_color_scheme();
    assert_eq!(state.color_scheme(), ColorScheme::Mtime);
    state.cycle_color_scheme();
    assert_eq!(state.color_scheme(), ColorScheme::Depth);
    state.cycle_color_scheme();
    assert_eq!(state.color_scheme(), ColorScheme::FileType);
}
```

- [ ] **Step 7: Run `just check`**

Expected: all green. Full compilation, all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/ui/app.rs src/ui/mod.rs src/ui/views/explorer.rs
git commit -m "feat(ui): rewire ExplorerState and shell to use Visualization trait (#83)"
```

---

### Task 6: Legend Adaptation for Color Modes

**Files:**
- Modify: `src/ui/widgets/extension_legend.rs`
- Modify: `src/ui/views/explorer.rs` (pass `LegendContent` to legend widget)

**Interfaces:**
- Consumes: `ColorScheme` (Task 2), `ExtensionStat`, `ExplorerState::color_scheme()` (Task 5)
- Produces:
  - `LegendContent` enum with `Extensions` and `Gradient` variants
  - Updated `ExtensionLegendWidget` that accepts `LegendContent`

- [ ] **Step 1: Write failing test for gradient legend rendering**

In `src/ui/widgets/extension_legend.rs` tests:

```rust
#[test]
fn gradient_legend_renders_min_max_labels() {
    let content = LegendContent::Gradient {
        scheme: ColorScheme::Mtime,
        label_min: "2020-01-01".to_owned(),
        label_max: "2026-10-08".to_owned(),
    };
    let widget = ExtensionLegendWidget { content };
    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    widget.render(area, &mut buf);
    let text: String = buf.content().iter().map(|c| c.symbol().to_string()).collect();
    assert!(text.contains("2020"), "expected min label");
    assert!(text.contains("2026"), "expected max label");
}
```

- [ ] **Step 2: Run test to verify it fails**

- [ ] **Step 3: Define `LegendContent` enum and update `ExtensionLegendWidget`**

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

Change `ExtensionLegendWidget` to hold `pub content: LegendContent<'a>`. In `Widget::render`, match on the variant: `Extensions` → existing logic, `Gradient` → render a vertical gradient bar with min/max labels.

- [ ] **Step 4: Update `render_top_panels` in `explorer.rs` to pass `LegendContent`**

When `state.color_scheme() == ColorScheme::FileType` → `LegendContent::Extensions { ... }`. When `Mtime` → `LegendContent::Gradient { scheme: Mtime, label_min: formatted min mtime, label_max: formatted max mtime }`. When `Depth` → `LegendContent::Gradient { scheme: Depth, label_min: "0", label_max: formatted max_depth }`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib`
Expected: all pass.

- [ ] **Step 6: Run `just check`**

- [ ] **Step 7: Commit**

```bash
git add src/ui/widgets/extension_legend.rs src/ui/views/explorer.rs
git commit -m "feat(ui): adapt extension legend for color scheme gradients (#83)"
```

---

### Task 7: Overview+Detail Split

**Files:**
- Modify: `src/ui/views/explorer.rs` (split logic in `render_visualization_section`)
- Modify: `src/ui/app.rs` (overview field management)

**Interfaces:**
- Consumes: `RenderParams::use_overview_detail()` (Task 1), `TreemapVisualization::new()` (Task 4), `Visualization` trait
- Produces: overview+detail split rendering at ≥200×50 terminals

- [ ] **Step 1: Write failing test for overview+detail activation**

In `src/ui/views/explorer.rs` tests:

```rust
#[test]
fn explorer_renders_overview_at_large_terminal() {
    let mut state = make_test_state();
    let content = render_to_string(&mut state, 300, 80);
    assert!(
        content.contains("Overview"),
        "expected 'Overview' panel title at large terminal size"
    );
}

#[test]
fn explorer_no_overview_at_normal_terminal() {
    let mut state = make_test_state();
    let content = render_to_string(&mut state, 120, 40);
    assert!(
        !content.contains("Overview"),
        "expected no 'Overview' panel at normal terminal size"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

- [ ] **Step 3: Implement overview+detail split in `render_visualization_section`**

After computing `RenderParams`, check `params.use_overview_detail()`:
- **Yes**: split `content_area` horizontally 30/70 using `Layout`. Left panel: bordered block titled "Overview", render a fresh `TreemapVisualization` instance (stored in `state.overview`) with the full `treemap_root` node. Right panel: render the main `visualization` with the detail subtree node. Update `state.overview` highlight via `set_highlight()` to show which subtree the detail view is showing.
- **No**: if `state.overview` is `Some`, drop it (`state.overview = None`). Render single viz as before.

- [ ] **Step 4: Add overview lifecycle management to `ExplorerState`**

Add `ensure_overview(&mut self)` — creates the overview `TreemapVisualization` if `None`. Add `drop_overview(&mut self)` — sets to `None`. Called from `render_visualization_section` based on the threshold.

- [ ] **Step 5: Write test for threshold oscillation stability**

```rust
#[test]
fn overview_threshold_exact_boundary_is_stable() {
    let mut state = make_test_state();
    // Render at exactly 200×50 three times — overview should appear consistently
    for _ in 0..3 {
        let content = render_to_string(&mut state, 200, 50);
        assert!(content.contains("Overview"));
    }
}
```

- [ ] **Step 6: Write test for stale params with overview — Review Focus item 1**

```rust
#[test]
fn key_event_with_stale_params_no_panic() {
    let mut state = make_test_state();
    // Render at large size to populate layout
    let _ = render_to_string(&mut state, 300, 80);
    // Now handle a key event — params are from the 300×80 render
    // even though the "terminal" might have shrunk. Should not panic.
    state.set_focus(PanelFocus::Treemap);
    let event = key_event(KeyCode::Right);
    handle_explorer_event(&event, &mut state);
    // No panic = pass
}
```

- [ ] **Step 7: Run `just check`**

Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add src/ui/views/explorer.rs src/ui/app.rs
git commit -m "feat(ui): overview+detail split for large terminals (#83)"
```

---

### Task 8: VHS Visual Tests and Final Verification

**Files:**
- Create: `tests/vhs/resolution-adaptive.tape`
- Create: `tests/vhs/color-modes.tape`
- Create: `tests/vhs/overview-detail.tape`

**Interfaces:**
- Consumes: the full working system from Tasks 1–7

- [ ] **Step 1: Write `tests/vhs/resolution-adaptive.tape`**

Tape runs `nixdirstat scan` on a test directory at four terminal sizes: 80×24, 120×40, 200×60, 300×80. At each size: wait for explorer to load, screenshot. Compare screenshots visually: labels should thin out at higher resolutions, vignettes should be more pronounced, directory indents wider.

- [ ] **Step 2: Write `tests/vhs/color-modes.tape`**

Tape runs `nixdirstat scan` on a test directory at 120×40. Wait for explorer, press `c` to cycle to Mtime, screenshot. Press `c` to cycle to Depth, screenshot. Press `c` to return to FileType, screenshot. Verify legend changes with each mode.

- [ ] **Step 3: Write `tests/vhs/overview-detail.tape`**

Tape runs `nixdirstat scan` on a test directory at 300×80. Wait for explorer, screenshot showing overview+detail split. Navigate tree to zoom in, screenshot showing overview highlighting the zoomed region.

- [ ] **Step 4: Run `just vhs` and inspect screenshots**

Verify all visual tests produce correct output.

- [ ] **Step 5: Run `just check` (full CI)**

Expected: all green — fmt, clippy, test, doc, deny.

- [ ] **Step 6: Write Review Focus tests for empty tree and zero-range edge cases**

These may already be covered in Task 2 and Task 4 tests, but verify:

```rust
#[test]
fn render_empty_children_node_no_panic() {
    let root = make_dir("root", 0, vec![]);
    let mut viz = TreemapVisualization::new();
    let params = RenderParams::from_area(Rect::new(0, 0, 80, 24));
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
    viz.render(&root, Rect::new(0, 0, 80, 24), &mut buf, &params, &ColorScheme::FileType);
    // No panic = pass, buffer should be empty
}
```

- [ ] **Step 7: Commit**

```bash
git add tests/vhs/
git commit -m "test(ui): add VHS visual tests for resolution-adaptive rendering (#83)"
```
