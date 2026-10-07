# High-Fidelity Treemap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the 1-cell-per-pixel treemap with HalfBlock sub-cell rendering, edge darkening, cell labels, spatial navigation with drill-down, a colorblind-safe palette, and sub-block directory tree bars.

**Architecture:** Two new modules (`pixel_grid.rs`, `colors.rs`) provide rendering primitives and palette mapping. The existing `TreemapWidget` is rewritten to paint into a `PixelGrid` at 2× vertical resolution, then flush to the ratatui `Buffer`. `PanelFocus` gains a `Treemap` variant with spatial navigation and drill-down. The `ExtensionLegendWidget` and `make_bar()` switch to the new palette and sub-block characters.

**Tech Stack:** Rust, ratatui 0.30, streemap 0.1, proptest (dev), crossterm

**Spec:** `docs/superpowers/specs/2026-10-06-hires-treemap-design.md`

## Global Constraints

- Rust stable, edition 2024, MSRV 1.95
- No `unsafe` code
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` must pass
- No `#[allow(...)]` except for verified false positives with a comment
- Conventional Commits for all commit messages
- `FileCategory` has 9 variants: `Code`, `Image`, `Document`, `Archive`, `Audio`, `Video`, `Binary`, `NoExtension`, `Other`
- `DirNode` does not carry `FileCategory`; derive it at render time via `FileCategory::from_extension()`

## Review Focus

1. **Indexed/named Color input to `darken()`**: The spec assumes RGB input. If `darken()` receives `Color::Indexed(n)` or a named variant like `Color::Gray`, it must convert to RGB first or return the color unchanged — silently dropping channels would produce black.
2. **Zero-dimension treemap cells**: `streemap::squarify` can produce float rects that round to 0×0 in pixel space. The pixel grid must not panic on `fill_rect(x, y, 0, h, _)` or `fill_rect(x, y, w, 0, _)` — it should no-op.
3. **Treemap area too small for breadcrumb + status + content**: If `treemap_area.height < 3`, subtracting 2 lines for breadcrumb/status leaves 0 or negative content height. The layout must clamp gracefully.
4. **`drill_path` and `treemap_root` divergence**: `ExplorerState` already has `treemap_root: Vec<String>` used by `zoom_into_selected()`/`zoom_out()`. The spec introduces `drill_path` on `TreemapState`. These must be unified — use the existing `treemap_root`, not a parallel path.
5. **`Esc` key collision**: `Esc` currently quits the app (`should_quit_event`). The spec wants `Esc` to drill up when the treemap is focused. The quit handler must check panel focus before quitting.

---

### Task 1: Color Palette Module (`colors.rs`)

**Files:**
- Create: `src/ui/colors.rs`
- Modify: `src/ui/mod.rs` — add `pub mod colors;`
- Test: inline `#[cfg(test)] mod tests` in `src/ui/colors.rs`

**Interfaces:**
- Consumes: `FileCategory` from `crate::types`
- Produces:
  - `category_color(cat: FileCategory) -> Color` — Okabe-Ito RGB color
  - `darken(color: Color, amount: f32) -> Color` — proportional RGB darkening
  - `contrast_text_color(bg: Color) -> Color` — black or white for readability
  - `no_color_fallback(cat: FileCategory) -> Color` — evenly-spaced grayscale
  - `is_color_enabled() -> bool` — checks `NO_COLOR` env var

- [ ] **Step 1: Write failing tests for `category_color`**

```rust
#[test]
fn category_color_code_is_okabe_ito_blue() {
    assert_eq!(category_color(FileCategory::Code), Color::Rgb(0, 114, 178));
}

#[test]
fn category_color_all_variants_are_distinct() {
    let colors: Vec<Color> = [
        FileCategory::Code, FileCategory::Image, FileCategory::Document,
        FileCategory::Archive, FileCategory::Audio, FileCategory::Video,
        FileCategory::Binary, FileCategory::NoExtension, FileCategory::Other,
    ].iter().map(|c| category_color(*c)).collect();
    let unique: std::collections::HashSet<_> = colors.iter().collect();
    assert_eq!(unique.len(), colors.len());
}
```

- [ ] **Step 2: Write failing tests for `darken`**

```rust
#[test]
fn darken_zero_is_identity() {
    let c = Color::Rgb(200, 100, 50);
    assert_eq!(darken(c, 0.0), c);
}

#[test]
fn darken_one_is_black() {
    assert_eq!(darken(Color::Rgb(200, 100, 50), 1.0), Color::Rgb(0, 0, 0));
}

#[test]
fn darken_thirty_percent() {
    assert_eq!(darken(Color::Rgb(200, 100, 50), 0.3), Color::Rgb(140, 70, 35));
}

#[test]
fn darken_non_rgb_returns_unchanged() {
    assert_eq!(darken(Color::Gray, 0.5), Color::Gray);
}
```

- [ ] **Step 3: Write failing tests for `contrast_text_color`**

```rust
#[test]
fn contrast_white_on_dark_blue() {
    assert_eq!(contrast_text_color(Color::Rgb(0, 114, 178)), Color::White);
}

#[test]
fn contrast_black_on_yellow() {
    assert_eq!(contrast_text_color(Color::Rgb(240, 228, 66)), Color::Black);
}
```

- [ ] **Step 4: Write failing tests for `no_color_fallback` and `is_color_enabled`**

```rust
#[test]
fn no_color_fallback_all_distinct() {
    let grays: Vec<Color> = [
        FileCategory::Code, FileCategory::Image, FileCategory::Document,
        FileCategory::Archive, FileCategory::Audio, FileCategory::Video,
        FileCategory::Binary, FileCategory::NoExtension, FileCategory::Other,
    ].iter().map(|c| no_color_fallback(*c)).collect();
    let unique: std::collections::HashSet<_> = grays.iter().collect();
    assert_eq!(unique.len(), grays.len());
}
```

- [ ] **Step 5: Run tests to verify they fail**

Run: `cargo test --lib ui::colors -- --nocapture`
Expected: compilation errors (module/functions don't exist)

- [ ] **Step 6: Implement `colors.rs`**

Add `pub mod colors;` to `src/ui/mod.rs`. Implement all five functions. `category_color` is a `match` on `FileCategory` returning the Okabe-Ito RGB constants from the spec table. `darken` extracts RGB channels, multiplies by `(1.0 - amount)`, rounds. Non-RGB variants return unchanged. `contrast_text_color` computes `L = 0.299*R + 0.587*G + 0.114*B`; returns `Color::Black` if `L > 128`, else `Color::White`. `no_color_fallback` maps each variant to an evenly-spaced gray from 40 to 220. `is_color_enabled` checks `std::env::var("NO_COLOR")`.

- [ ] **Step 7: Add proptest for `darken`**

```rust
proptest! {
    #[test]
    fn darken_channels_never_increase(r in 0u8..=255, g in 0u8..=255, b in 0u8..=255, amt in 0.0f32..=1.0) {
        let result = darken(Color::Rgb(r, g, b), amt);
        if let Color::Rgb(dr, dg, db) = result {
            prop_assert!(dr <= r); prop_assert!(dg <= g); prop_assert!(db <= b);
        }
    }
}
```

- [ ] **Step 8: Run all tests to verify they pass**

Run: `cargo test --lib ui::colors`
Expected: all pass

- [ ] **Step 9: Run clippy and fmt**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: clean

- [ ] **Step 10: Commit**

```bash
git add src/ui/colors.rs src/ui/mod.rs
git commit -m "feat(ui): add Okabe-Ito color palette module with darken and contrast"
```

---

### Task 2: Pixel Grid Module (`pixel_grid.rs`)

**Files:**
- Create: `src/ui/pixel_grid.rs`
- Modify: `src/ui/mod.rs` — add `pub mod pixel_grid;`
- Test: inline `#[cfg(test)] mod tests` in `src/ui/pixel_grid.rs`

**Interfaces:**
- Consumes: `darken()` from `crate::ui::colors`, `ratatui::{layout::Rect, buffer::Buffer, style::Color}`
- Produces:
  - `PixelGrid::new(width: u16, height: u16, bg: Color) -> Self`
  - `PixelGrid::clear(&mut self, bg: Color)`
  - `PixelGrid::fill_rect(&mut self, x: u16, y: u16, w: u16, h: u16, color: Color)`
  - `PixelGrid::darken_edges(&mut self, x: u16, y: u16, w: u16, h: u16)`
  - `PixelGrid::flush_to_buffer(&self, buf: &mut Buffer, area: Rect)`
  - `PixelGrid::pixel_width(&self) -> u16`
  - `PixelGrid::pixel_height(&self) -> u16`

- [ ] **Step 1: Write failing tests for `fill_rect` and pixel access**

```rust
#[test]
fn fill_rect_sets_pixels() {
    let mut grid = PixelGrid::new(10, 4, Color::Reset);
    grid.fill_rect(2, 1, 3, 2, Color::Rgb(255, 0, 0));
    // Pixel (2,1) should be red, pixel (0,0) should be bg
}

#[test]
fn fill_rect_zero_dimension_is_noop() {
    let mut grid = PixelGrid::new(10, 4, Color::Reset);
    grid.fill_rect(0, 0, 0, 5, Color::Rgb(255, 0, 0)); // no panic
    grid.fill_rect(0, 0, 5, 0, Color::Rgb(255, 0, 0)); // no panic
}

#[test]
fn overlapping_rects_last_wins() {
    let mut grid = PixelGrid::new(10, 4, Color::Reset);
    grid.fill_rect(0, 0, 5, 4, Color::Rgb(255, 0, 0));
    grid.fill_rect(2, 1, 3, 2, Color::Rgb(0, 0, 255));
    // Pixel (3, 2) should be blue
}
```

- [ ] **Step 2: Write failing tests for `flush_to_buffer`**

```rust
#[test]
fn flush_same_colors_produces_full_block() {
    let mut grid = PixelGrid::new(1, 2, Color::Reset);
    let red = Color::Rgb(255, 0, 0);
    grid.fill_rect(0, 0, 1, 2, red);
    let area = Rect::new(0, 0, 1, 1);
    let mut buf = Buffer::empty(area);
    grid.flush_to_buffer(&mut buf, area);
    let cell = &buf[(0, 0)];
    assert_eq!(cell.symbol(), "█");
    assert_eq!(cell.fg, red);
}

#[test]
fn flush_different_colors_produces_upper_half() {
    let mut grid = PixelGrid::new(1, 2, Color::Reset);
    let red = Color::Rgb(255, 0, 0);
    let blue = Color::Rgb(0, 0, 255);
    grid.fill_rect(0, 0, 1, 1, red);
    grid.fill_rect(0, 1, 1, 1, blue);
    let area = Rect::new(0, 0, 1, 1);
    let mut buf = Buffer::empty(area);
    grid.flush_to_buffer(&mut buf, area);
    let cell = &buf[(0, 0)];
    assert_eq!(cell.symbol(), "▀");
    assert_eq!(cell.fg, red);
    assert_eq!(cell.bg, blue);
}

#[test]
fn flush_both_bg_produces_space() {
    let bg = Color::Reset;
    let grid = PixelGrid::new(1, 2, bg);
    let area = Rect::new(0, 0, 1, 1);
    let mut buf = Buffer::empty(area);
    grid.flush_to_buffer(&mut buf, area);
    assert_eq!(buf[(0, 0)].symbol(), " ");
}
```

- [ ] **Step 3: Write failing test for `darken_edges`**

```rust
#[test]
fn darken_edges_darkens_border_preserves_interior() {
    let red = Color::Rgb(200, 100, 50);
    let mut grid = PixelGrid::new(6, 8, Color::Reset);
    grid.fill_rect(0, 0, 6, 8, red);
    grid.darken_edges(0, 0, 6, 8);
    // Corner pixel (0,0) should be darker than center pixel (3,4)
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test --lib ui::pixel_grid`
Expected: compilation errors

- [ ] **Step 5: Implement `pixel_grid.rs`**

Add `pub mod pixel_grid;` to `src/ui/mod.rs`. `PixelGrid` stores `Vec<Color>` of size `width * (height * 2)`, where `width`/`height` are terminal cell dimensions. `fill_rect` clamps to grid bounds, no-ops on zero dimensions. `darken_edges` calls `colors::darken()` on the outermost pixel ring at 0.30, and the next inner ring at 0.15 when both dimensions ≥ 6 pixels. `flush_to_buffer` iterates terminal cells, reads top/bottom pixel pairs, emits `█`/`▀`/` ` with appropriate fg/bg. `clear` resets all pixels to the given bg color.

- [ ] **Step 6: Add proptest for `fill_rect` containment**

```rust
proptest! {
    #[test]
    fn fill_rect_only_affects_interior(
        gw in 1u16..=50, gh in 1u16..=25,
        rx in 0u16..=49, ry in 0u16..=49, rw in 0u16..=50, rh in 0u16..=50,
        r in 0u8..=255, g in 0u8..=255, b in 0u8..=255,
    ) {
        let bg = Color::Reset;
        let color = Color::Rgb(r, g, b);
        let mut grid = PixelGrid::new(gw, gh, bg);
        grid.fill_rect(rx, ry, rw, rh, color);
        // Every pixel outside the clamped rect should still be bg
    }
}
```

- [ ] **Step 7: Run all tests**

Run: `cargo test --lib ui::pixel_grid`
Expected: all pass

- [ ] **Step 8: Run clippy and fmt**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: clean

- [ ] **Step 9: Commit**

```bash
git add src/ui/pixel_grid.rs src/ui/mod.rs
git commit -m "feat(ui): add HalfBlock pixel grid with edge darkening and buffer flush"
```

---

### Task 3: Sub-Block Directory Tree Bars

**Files:**
- Modify: `src/ui/widgets/dir_tree.rs:111-124` — rewrite `make_bar()`
- Test: existing tests in same file + new tests

**Interfaces:**
- Consumes: nothing new
- Produces: `make_bar(value: u64, total: u64, width: usize) -> String` — same signature, new sub-block output

- [ ] **Step 1: Write failing tests for sub-block bar behavior**

```rust
#[test]
fn bar_full_is_all_full_blocks() {
    assert_eq!(make_bar(100, 100, 10), "██████████");
}

#[test]
fn bar_empty_is_all_spaces() {
    assert_eq!(make_bar(0, 100, 10), "          ");
}

#[test]
fn bar_half_is_five_full_blocks() {
    assert_eq!(make_bar(50, 100, 10), "█████     ");
}

#[test]
fn bar_one_eighth() {
    let bar = make_bar(1, 8, 1);
    assert_eq!(bar, "▏");
}

#[test]
fn bar_three_eighths() {
    let bar = make_bar(3, 8, 1);
    assert_eq!(bar, "▍");
}
```

- [ ] **Step 2: Run tests to verify the new assertions fail**

Run: `cargo test --lib ui::widgets::dir_tree`
Expected: assertion failures (current `make_bar` uses `░` not spaces, and has no fractional characters)

- [ ] **Step 3: Rewrite `make_bar`**

Replace the body. Compute `filled_f64 = (value as f64 / total as f64) * width as f64`. Full blocks: `filled_f64.floor() as usize`. Fractional index: `((filled_f64.fract()) * 8.0).round() as usize`, indexing into `['▏','▎','▍','▌','▋','▊','▉','█']` (index 0 maps to space, 1–7 to sub-blocks, 8 wraps to full block). Remaining: spaces. Total string width must equal `width` characters.

- [ ] **Step 4: Add proptest for bar width invariant**

```rust
proptest! {
    #[test]
    fn bar_char_count_equals_width(v in 0u64..=1000, t in 1u64..=1000, w in 1usize..=50) {
        let bar = make_bar(v.min(t), t, w);
        prop_assert_eq!(bar.chars().count(), w);
    }
}
```

- [ ] **Step 5: Run all tests**

Run: `cargo test --lib ui::widgets::dir_tree`
Expected: all pass

- [ ] **Step 6: Run clippy and fmt**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add src/ui/widgets/dir_tree.rs
git commit -m "feat(ui): replace full-block bars with sub-block proportional rendering"
```

---

### Task 4: Treemap Widget Rewrite (HalfBlock Rendering)

**Files:**
- Modify: `src/ui/widgets/treemap.rs` — rewrite render internals
- Test: rewrite `#[cfg(test)] mod tests` in same file

**Interfaces:**
- Consumes: `PixelGrid` from `crate::ui::pixel_grid`, `category_color`/`darken`/`contrast_text_color`/`is_color_enabled`/`no_color_fallback` from `crate::ui::colors`, `FileCategory::from_extension()` from `crate::types`
- Produces:
  - `TreemapWidget::render()` — paints into PixelGrid, flushes to buffer, overlays labels
  - `extension_color()` — REMOVED (replaced by `colors::category_color`)
  - `category_color()` — REMOVED from this file (moved to `colors.rs`)
  - `EXTENSION_PALETTE` — REMOVED
  - `TreemapState` — unchanged for now (navigation is Task 5)
  - `TreemapLayout` — new struct holding `Vec<CellLayout>` for spatial navigation
  - `CellLayout { rect: Rect, name: String, extension: Option<String>, is_dir: bool, size: u64, mtime: SystemTime, path: Vec<String> }`

- [ ] **Step 1: Write failing tests for HalfBlock rendering**

```rust
#[test]
fn treemap_renders_half_block_characters() {
    let root = make_dir("root", 100, vec![make_file("a.rs", 100, Some("rs"))]);
    let buf = render_treemap(&root, 40, 10);
    let has_half = buf.content().iter().any(|c| c.symbol() == "▀" || c.symbol() == "█");
    assert!(has_half, "expected half-block characters in output");
}

#[test]
fn treemap_uses_category_colors_not_extension_hash() {
    let root = make_dir("root", 100, vec![make_file("a.rs", 100, Some("rs"))]);
    let buf = render_treemap(&root, 40, 10);
    let code_color = crate::ui::colors::category_color(FileCategory::Code);
    let has_code_color = buf.content().iter().any(|c| c.fg == code_color || c.bg == code_color);
    assert!(has_code_color, "expected Okabe-Ito blue for .rs file");
}
```

- [ ] **Step 2: Write failing test for edge darkening**

```rust
#[test]
fn treemap_edge_pixels_are_darker_than_interior() {
    let root = make_dir("root", 100, vec![make_file("a.rs", 100, Some("rs"))]);
    let buf = render_treemap(&root, 40, 10);
    let code_color = crate::ui::colors::category_color(FileCategory::Code);
    // At least some cells should have colors that differ from the base category color
    // (the edge-darkened variants)
    let has_darkened = buf.content().iter().any(|c| {
        let is_fg_variant = c.fg != code_color && c.fg != Color::Reset;
        let is_bg_variant = c.bg != code_color && c.bg != Color::Reset;
        (is_fg_variant || is_bg_variant) && c.symbol() == "▀"
    });
    assert!(has_darkened, "expected edge-darkened colors");
}
```

- [ ] **Step 3: Write failing test for label overlay**

```rust
#[test]
fn treemap_labels_on_large_cells() {
    let root = make_dir("root", 100, vec![make_file("bigfile.rs", 100, Some("rs"))]);
    let buf = render_treemap(&root, 40, 10);
    let content: String = buf.content().iter().map(|c| c.symbol().to_string()).collect();
    assert!(content.contains("bigfile.rs"), "expected filename label on large cell");
}

#[test]
fn treemap_no_label_on_tiny_cells() {
    let files: Vec<DirNode> = (0..50).map(|i| make_file(&format!("f{i}.rs"), 2, Some("rs"))).collect();
    let root = make_dir("root", 100, files);
    let buf = render_treemap(&root, 40, 10);
    let content: String = buf.content().iter().map(|c| c.symbol().to_string()).collect();
    assert!(!content.contains("f0.rs"), "tiny cells should not have labels");
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test --lib ui::widgets::treemap`
Expected: failures (current renderer produces no half-block characters and no labels)

- [ ] **Step 5: Rewrite `treemap.rs` render internals**

Remove `EXTENSION_PALETTE`, `extension_color()`, and the old `category_color()`. Keep `TreemapState`, `TreemapWidget`, `f32_rect_to_ratatui()`. Rewrite `StatefulWidget::render()`:

1. Create `PixelGrid::new(area.width, area.height, bg_color)`.
2. Call `streemap::squarify` as before.
3. For each layout rect: convert to pixel coords (x stays, y doubles, h doubles), determine `FileCategory` via `FileCategory::from_extension()`, get color via `colors::category_color()` (or `no_color_fallback()` if `!is_color_enabled()`), call `grid.fill_rect()`, call `grid.darken_edges()`.
4. Call `grid.flush_to_buffer()`.
5. For each layout rect: if terminal cell rect ≥ 8 wide and ≥ 2 tall, overlay label with `buf.set_string()` using `contrast_text_color()` for fg, category color for bg. Truncate filename with `…`.
6. Build and store a `Vec<CellLayout>` in the method for use by Task 5.

Introduce `CellLayout` struct to capture per-cell geometry and metadata for spatial navigation.

- [ ] **Step 6: Update existing tests**

Update `render_treemap()` helper and tests that checked for `extension_color()` results. Tests now check for category colors and half-block symbols.

- [ ] **Step 7: Run all tests**

Run: `cargo test --lib ui::widgets::treemap`
Expected: all pass

- [ ] **Step 8: Run clippy and fmt**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: clean

- [ ] **Step 9: Commit**

```bash
git add src/ui/widgets/treemap.rs
git commit -m "feat(ui): rewrite treemap with HalfBlock rendering, edge darkening, and labels"
```

---

### Task 5: Extension Legend Palette Update

**Files:**
- Modify: `src/ui/widgets/extension_legend.rs`
- Test: update existing tests in same file

**Interfaces:**
- Consumes: `category_color` from `crate::ui::colors`, `FileCategory::from_extension()` from `crate::types`
- Produces: `ExtensionLegendWidget::render()` — now uses category colors

- [ ] **Step 1: Write failing test for category-based colors**

```rust
#[test]
fn legend_uses_category_colors() {
    let stats = [make_stat(Some("rs"), 1024)];
    let backend = TestBackend::new(40, 5);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| {
        let widget = ExtensionLegendWidget { stats: &stats, total_size: 1024, scroll_offset: 0 };
        f.render_widget(widget, f.area());
    }).expect("draw");
    let buf = terminal.backend().buffer().clone();
    let code_color = crate::ui::colors::category_color(FileCategory::Code);
    let has_color = buf.content().iter().any(|c| c.fg == code_color);
    assert!(has_color, "expected Okabe-Ito blue swatch for .rs");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib ui::widgets::extension_legend::tests::legend_uses_category_colors`
Expected: failure (currently uses FNV hash color)

- [ ] **Step 3: Replace `extension_color` import with `colors::category_color`**

Change `use super::treemap::extension_color;` to `use crate::ui::colors::{category_color, is_color_enabled, no_color_fallback};`. In the render loop, derive `FileCategory` via `FileCategory::from_extension(stat.extension.as_deref().map(std::ffi::OsStr::new))`, then call `category_color(cat)` (or `no_color_fallback(cat)` if `!is_color_enabled()`).

- [ ] **Step 4: Update the existing `legend_renders_color_swatches` test**

Replace the assertion checking for `extension_color(Some("py"))` with a check for the category color of `.py` (Code → blue).

- [ ] **Step 5: Run all tests**

Run: `cargo test --lib ui::widgets::extension_legend`
Expected: all pass

- [ ] **Step 6: Run clippy and fmt**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add src/ui/widgets/extension_legend.rs
git commit -m "feat(ui): switch extension legend to Okabe-Ito category palette"
```

---

### Task 6: Treemap Navigation (Panel Focus, Spatial Movement, Drill-Down)

**Files:**
- Modify: `src/ui/app.rs` — add `Treemap` to `PanelFocus`, extend `ExplorerState`
- Modify: `src/ui/widgets/treemap.rs` — add `selected_index` to `TreemapState`, spatial navigation methods
- Modify: `src/ui/mod.rs:handle_explorer_event()` — dispatch treemap-focused keybindings
- Test: inline tests in `app.rs` and `treemap.rs`

**Interfaces:**
- Consumes: `CellLayout` from Task 4
- Produces:
  - `PanelFocus::Treemap` variant
  - `TreemapState::selected_index: Option<usize>`
  - `Direction` enum: `Up`, `Down`, `Left`, `Right`
  - `TreemapState::move_selection(&mut self, direction: Direction, cells: &[CellLayout]) -> bool`
  - `ExplorerState::cycle_focus(&mut self)` — replaces `toggle_focus`

- [ ] **Step 1: Write failing tests for spatial navigation**

```rust
// In treemap.rs tests:
#[test]
fn move_selection_right_finds_nearest_neighbor() {
    let cells = vec![
        CellLayout { rect: Rect::new(0, 0, 10, 10), ..Default::default() },
        CellLayout { rect: Rect::new(10, 0, 10, 10), ..Default::default() },
    ];
    let mut state = TreemapState { selected_index: Some(0), ..Default::default() };
    assert!(state.move_selection(Direction::Right, &cells));
    assert_eq!(state.selected_index, Some(1));
}

#[test]
fn move_selection_at_edge_is_noop() {
    let cells = vec![
        CellLayout { rect: Rect::new(0, 0, 10, 10), ..Default::default() },
    ];
    let mut state = TreemapState { selected_index: Some(0), ..Default::default() };
    assert!(!state.move_selection(Direction::Right, &cells));
    assert_eq!(state.selected_index, Some(0));
}
```

- [ ] **Step 2: Write failing tests for `PanelFocus::Treemap` and `cycle_focus`**

```rust
// In app.rs tests:
#[test]
fn cycle_focus_three_panels() {
    let mut state = make_explorer_state();
    assert_eq!(state.focus(), PanelFocus::Tree);
    state.cycle_focus();
    assert_eq!(state.focus(), PanelFocus::Treemap);
    state.cycle_focus();
    assert_eq!(state.focus(), PanelFocus::Legend);
    state.cycle_focus();
    assert_eq!(state.focus(), PanelFocus::Tree);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib ui::app::tests::cycle_focus_three_panels`
Expected: compilation error (`PanelFocus::Treemap` doesn't exist)

- [ ] **Step 4: Add `Treemap` to `PanelFocus`, implement `cycle_focus`**

Add the variant. Replace `toggle_focus` with `cycle_focus`: Tree → Treemap → Legend → Tree. Add `pub fn focus(&self) -> PanelFocus` getter.

- [ ] **Step 5: Add `selected_index` to `TreemapState`, implement `move_selection`**

Add `selected_index: Option<usize>` field. Add `Direction` enum. Implement `move_selection` using the spec's spatial algorithm: compute current cell center, filter candidates by half-plane, select nearest by Euclidean distance.

- [ ] **Step 6: Wire keybindings in `handle_explorer_event`**

When `state.focus() == PanelFocus::Treemap`:
- `h`/`←`/`j`/`↓`/`k`/`↑`/`l`/`→` → call `move_selection()` on the treemap state
- `Enter` → `zoom_into_selected()` using the selected treemap cell's path
- `Backspace` → `zoom_out()`
- `Tab` → `cycle_focus()`

When `PanelFocus::Tree`: existing behavior, but `Tab` calls `cycle_focus()`.
When `PanelFocus::Legend`: existing behavior, but `Tab` calls `cycle_focus()`.

Fix `Esc` collision: only quit if focus is not `Treemap`, or if `treemap_root` is already at scan root. When treemap is focused and drilled in, `Esc` zooms out instead.

- [ ] **Step 7: Add `q` as the sole unconditional quit key, make `Esc` context-sensitive**

In `should_quit_event`, remove `Esc`. In `handle_explorer_event`, handle `Esc`: if treemap focused and drilled in, zoom out; else if treemap focused, cycle to Tree; else quit. `q` and `Ctrl-C` always quit.

- [ ] **Step 8: Run all tests**

Run: `cargo test --lib ui::app && cargo test --lib ui::widgets::treemap && cargo test --lib ui::mod`
Expected: all pass

- [ ] **Step 9: Run clippy and fmt**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: clean

- [ ] **Step 10: Commit**

```bash
git add src/ui/app.rs src/ui/widgets/treemap.rs src/ui/mod.rs
git commit -m "feat(ui): add treemap panel focus with spatial navigation and drill-down"
```

---

### Task 7: Explorer View Integration (Breadcrumb, Status Bar, Selection Highlight, Cross-Panel Sync)

**Files:**
- Modify: `src/ui/views/explorer.rs` — add breadcrumb bar, status bar, selection highlight, focus-aware borders, cross-panel sync
- Modify: `src/ui/widgets/treemap.rs` — render selection highlight border in pixel grid
- Modify: `src/ui/app.rs` — add `sync_tree_to_treemap_selection()` for reverse sync
- Test: existing tests in `explorer.rs` + new tests

**Interfaces:**
- Consumes: `PanelFocus::Treemap`, `TreemapState::selected_index`, `CellLayout`, `ExplorerState::breadcrumb_path()`, `contrast_text_color()` from `colors.rs`
- Produces:
  - Breadcrumb bar: 1-line widget above treemap
  - Status bar: 1-line widget below treemap showing selected cell info
  - Focus-aware border colors on all three panels
  - Selection highlight border in the treemap pixel grid

- [ ] **Step 1: Write failing test for breadcrumb bar**

```rust
#[test]
fn explorer_renders_breadcrumb_bar() {
    let mut state = make_test_state();
    let content = render_to_string(&mut state, 120, 40);
    assert!(content.contains("/test"), "expected breadcrumb in output");
}
```

- [ ] **Step 2: Write failing test for focus-aware borders**

```rust
#[test]
fn explorer_treemap_panel_title_changes_color_with_focus() {
    let mut state = make_test_state();
    state.cycle_focus(); // Tree → Treemap
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| render_explorer(f, &mut state, f.area())).expect("draw");
    let buf = terminal.backend().buffer().clone();
    // "Disk Usage" title should be cyan when treemap is focused
    // "Directory Tree" title should be gray when tree is not focused
    let disk_usage_cells: Vec<_> = buf.content().iter()
        .filter(|c| c.symbol() == "D" && c.fg == Color::Cyan)
        .collect();
    assert!(!disk_usage_cells.is_empty(), "expected cyan Disk Usage title when treemap focused");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib ui::views::explorer`
Expected: failures

- [ ] **Step 4: Restructure explorer layout**

Split the treemap vertical area into 3 sub-areas: breadcrumb (1 line), treemap content (remaining), status bar (1 line). Clamp: if `treemap_area.height < 3`, skip breadcrumb and status bar. Make border styles focus-aware: cyan for the focused panel, gray (`Color::Indexed(240)`) for unfocused.

- [ ] **Step 5: Render breadcrumb bar**

Above the treemap: a `Paragraph` showing `state.breadcrumb_path()` split by `/` into styled `Span`s. Final segment bold, parents dim.

- [ ] **Step 6: Render status bar**

Below the treemap: when `state.treemap_state().selected_index.is_some()`, show `filename | size | date | path`. Reuse the existing `render_selection_info` pattern but source data from the selected `CellLayout` instead of the tree selection.

- [ ] **Step 7: Add selection highlight to treemap render**

In `treemap.rs`, when `state.selected_index == Some(i)`, replace `darken_edges` on that cell with a highlight border: paint the outermost pixel ring using `contrast_text_color(cell_color)` (white on dark, black on light).

- [ ] **Step 8: Wire cross-panel sync (treemap → tree)**

In `ExplorerState`, add `sync_tree_to_treemap_selection(&mut self)`: when treemap is focused and `selected_index` points to a `CellLayout`, set the tree widget's selection to the corresponding path components and expand parent nodes.

- [ ] **Step 9: Update help overlay keybindings text**

Add treemap navigation keys to the help overlay: `Tab` to cycle panels, arrows/hjkl for treemap navigation, Enter/Backspace for drill.

- [ ] **Step 10: Run all tests**

Run: `cargo test --lib ui::views::explorer && cargo test --lib ui::widgets::treemap`
Expected: all pass

- [ ] **Step 11: Run full CI check**

Run: `just check`
Expected: fmt, clippy, test, doc, deny all pass

- [ ] **Step 12: Commit**

```bash
git add src/ui/views/explorer.rs src/ui/widgets/treemap.rs src/ui/app.rs
git commit -m "feat(ui): integrate breadcrumb bar, status bar, selection highlight, and cross-panel sync"
```

---

### Task 8: VHS Visual Tests

**Files:**
- Create: `tests/vhs/hires_treemap.tape`
- Create: `tests/vhs/treemap_navigation.tape`
- Create: `tests/vhs/sub_block_bars.tape`
- Test: `just vhs` to run all tapes

**Interfaces:**
- Consumes: the built binary `nixdirstat`
- Produces: screenshots in `tests/vhs/screenshots/`

- [ ] **Step 1: Create `hires_treemap.tape`**

Tape that scans a test directory (use `tests/fixtures/` or create a small temp dir in the tape), waits for the explorer view, and takes a screenshot. The screenshot should show half-block characters with edge darkening.

- [ ] **Step 2: Create `treemap_navigation.tape`**

Tape that scans, presses `Tab` to focus the treemap, presses arrow keys to move selection (verify highlight moves), presses `Enter` to drill down (verify breadcrumb updates), presses `Backspace` to drill back out.

- [ ] **Step 3: Create `sub_block_bars.tape`**

Tape that scans, shows the directory tree with sub-block proportional bars visible.

- [ ] **Step 4: Run VHS suite**

Run: `just vhs`
Expected: all tapes produce screenshots without errors

- [ ] **Step 5: Visually inspect screenshots**

Manually verify: HalfBlock characters visible, edge darkening creates depth, labels appear on large cells, selection highlight is visible, breadcrumb updates on drill-down, sub-block bars are smooth.

- [ ] **Step 6: Commit**

```bash
git add tests/vhs/
git commit -m "test(vhs): add visual tests for HalfBlock treemap, navigation, and sub-block bars"
```
