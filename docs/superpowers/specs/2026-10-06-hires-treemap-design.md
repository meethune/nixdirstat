# High-Fidelity Treemap Rendering & Interaction

**Date:** 2026-10-06
**Status:** Draft
**Scope:** Visual fidelity upgrade, treemap interaction, colorblind-safe palette, sub-block bars

## Overview

Replace the current 1-cell-per-pixel solid-color treemap with HalfBlock sub-cell rendering (2× vertical resolution), add edge darkening for depth perception, enable treemap keyboard navigation with drill-down, and adopt a colorblind-safe categorical color palette.

The spec deferred cushion treemap rendering to post-MVP. This is that upgrade.

## Goals

1. **Visual fidelity:** Double the treemap's effective vertical resolution using HalfBlock (`▀`/`▄`) characters with per-pixel RGB color, producing roughly square pixels.
2. **Depth perception:** Edge darkening on treemap cells creates visual separation and hierarchy without explicit border characters.
3. **Labels:** Large treemap cells display filenames, making the treemap self-describing without requiring cross-reference to the legend.
4. **Treemap navigation:** Make the treemap a focusable panel with spatial keyboard navigation, drill-down into directories, and a breadcrumb bar.
5. **Colorblind-safe palette:** Replace the 22-color FNV-hashed indexed palette with the 8-color Okabe-Ito palette mapped to `FileCategory` variants.
6. **Sub-block bars:** Replace full-block bars (`████░░░░░░`) in the directory tree with sub-block characters (`▏▎▍▌▋▊▉█`) for smooth proportional rendering.
7. **`NO_COLOR` support:** Degrade gracefully to distinguishable gray shades when color is unavailable.

## Non-Goals

- Sunburst / radial treemap visualization (remains deferred).
- Mouse interaction (keyboard-first; mouse support is a future enhancement).
- Adaptive marker auto-detection (HalfBlock is universally supported; higher markers are a future opt-in).
- Custom color themes or user-configurable palettes.

---

## 1. HalfBlock Rendering Pipeline

### Current Flow

```
streemap::squarify → Rect<f32> → Rect<u16> → buf.set_style(rect, bg_color)
```

Each terminal cell = 1 pixel. Solid background color. No symbols written.

### New Flow

```
streemap::squarify → Rect<f32> → PixelGrid (2× vertical res) → edge darkening → flush to Buffer
                                                                                → label overlay
```

### Pixel Grid Abstraction

A new `PixelGrid` struct represents a virtual canvas at 2× vertical resolution:

- **Dimensions:** `width = area.width`, `height = area.height * 2` (in pixels).
- **Storage:** `Vec<Color>` of length `width * height`, initialized to the terminal background color.
- **Paint:** `fill_rect(x, y, w, h, color)` fills a rectangular region in pixel coordinates.
- **Edge darken:** `darken_edges(x, y, w, h, amount)` darkens the outermost 1–2 pixel rows/cols of a rectangle by reducing RGB channels proportionally (default ~25–30%).

### Flush to Buffer

When the pixel grid is flushed to a ratatui `Buffer`:

- For each terminal cell at `(col, row)`, read the top pixel `(col, row*2)` and bottom pixel `(col, row*2 + 1)`.
- If both colors are identical: write `█` (full block) with `fg = color`.
- If top differs from bottom: write `▀` with `fg = top_color`, `bg = bottom_color`.
- If both are the terminal background: write a space.

This produces 2× vertical resolution with per-pixel RGB color and universal terminal support.

### Edge Darkening

For each treemap rectangle, after painting its base color into the pixel grid, darken the outermost pixel border:

- Outer 1 pixel row/col: darken by 30%.
- If the rectangle is large enough (≥6 pixels in both dimensions): also darken the next inner row/col by 15%.

This creates a subtle "raised center" effect — a lightweight approximation of cushion shading that works within HalfBlock's per-pixel color model.

The darkening function: `darken(color, amount) → Color` reduces each RGB channel by `amount` (0.0–1.0). E.g., `darken(Color::Rgb(200, 100, 50), 0.3)` → `Color::Rgb(140, 70, 35)`.

### Label Overlay

After flushing the pixel grid to the buffer, overlay text labels on large cells:

- **Eligibility:** Cell must be ≥8 terminal columns wide AND ≥2 terminal rows tall.
- **Content:** Filename (not full path), truncated with `…` if it exceeds the cell width minus 2 (1-char padding each side).
- **Positioning:** Centered horizontally, vertically centered in the cell.
- **Contrast:** Compute luminance of the cell's base color: `L = 0.299*R + 0.587*G + 0.114*B`. If `L > 128`, use black text; otherwise white text. This ensures readability on any category color.
- **Rendering:** Write the label string directly into the buffer with `buf.set_string()`, overwriting the half-block characters in that row. The label row loses sub-cell resolution but gains readability.

---

## 2. Color Palette & Category Mapping

### Okabe-Ito Palette

Replace the 22-color FNV hash palette with the Okabe-Ito categorical palette, which is safe for all common forms of color vision deficiency (protanopia, deuteranopia, tritanopia):

| FileCategory | Color Name | RGB | Hex |
|---|---|---|---|
| `Code` | Blue | (0, 114, 178) | `#0072B2` |
| `Document` | Sky Blue | (86, 180, 233) | `#56B4E9` |
| `Image` | Orange | (230, 159, 0) | `#E69F00` |
| `Audio` | Vermillion | (213, 94, 0) | `#D55E00` |
| `Video` | Reddish Purple | (204, 121, 167) | `#CC79A7` |
| `Archive` | Bluish Green | (0, 158, 115) | `#009E73` |
| `Binary` | Yellow | (240, 228, 66) | `#F0E442` |
| `NoExtension` | Dark Gray | (120, 120, 120) | `#787878` |
| `Other` | Light Gray | (170, 170, 170) | `#AAAAAA` |

The enum has 9 variants (`Code`, `Image`, `Document`, `Archive`, `Audio`, `Video`, `Binary`, `NoExtension`, `Other`). The 7 Okabe-Ito hues cover the semantic categories; `NoExtension` and `Other` get distinct grays since they lack semantic meaning. The mapping is `FileCategory → Color`, not extension → color. All `.rs`, `.py`, `.js` files share the Code/Blue color. All `.jpg`, `.png`, `.svg` files share the Image/Orange color. The extension legend already groups by category, so this is consistent.

### NO_COLOR Fallback

When the `NO_COLOR` environment variable is set (any value), or the terminal is detected as not supporting RGB color:

- Map each `FileCategory` to an evenly-spaced grayscale value (from `Color::Rgb(40,40,40)` to `Color::Rgb(220,220,220)`).
- The extension legend remains the primary identification mechanism.
- Edge darkening still applies (darker grays on edges of lighter gray cells).
- Labels become more important since color no longer distinguishes categories.

### Directory Tree Sub-Block Bars

Replace the current `make_bar()` function in `dir_tree.rs`:

**Current:** `████░░░░░░` — full blocks only, 1 gradation per character.

**New:** Use sub-block characters for 8× granularity per character:

```
▏ = 1/8    ▎ = 2/8    ▍ = 3/8    ▌ = 4/8
▋ = 5/8    ▊ = 6/8    ▉ = 7/8    █ = 8/8
```

For a bar of width `w` characters representing proportion `p`:
- Full blocks: `floor(p * w)`
- Fractional character: select from the 8 sub-blocks based on `fract(p * w) * 8`
- Remaining: spaces

This produces visually smooth proportional bars.

---

## 3. Treemap Navigation & Interaction

### Panel Focus Extension

Add `Treemap` as a third variant of `PanelFocus`:

```rust
enum PanelFocus {
    Tree,
    Treemap,  // NEW
    Legend,
}
```

`Tab` cycles: Tree → Treemap → Legend → Tree.

### TreemapState Extension

Extend `TreemapState` with navigation state:

```rust
struct TreemapState {
    highlighted_path: Option<PathBuf>,   // existing — driven by tree cursor
    selected_index: Option<usize>,       // NEW — index into current layout's leaf cells
    drill_path: Vec<PathBuf>,            // NEW — breadcrumb stack for drill-down
}
```

- `selected_index` tracks which leaf cell is selected when the treemap is focused. `None` when treemap is not focused or no cells exist.
- `drill_path` is the stack of directories the user has drilled into. The last element is the current treemap root. Empty means the scan root is the treemap root.

### Keybindings (Treemap Focused)

| Key | Action |
|-----|--------|
| `h` / `←` | Move selection to nearest cell to the left |
| `j` / `↓` | Move selection to nearest cell below |
| `k` / `↑` | Move selection to nearest cell above |
| `l` / `→` | Move selection to nearest cell to the right |
| `Enter` | Drill down: if selected cell is a directory, push to `drill_path` and re-root treemap |
| `Esc` / `Backspace` | Go up: pop from `drill_path`, re-root treemap at parent |
| `Tab` | Cycle focus to Legend panel |

### Spatial Navigation Algorithm

Given the current selection and a direction, find the next cell:

1. Compute the center point `(cx, cy)` of the currently selected cell.
2. Filter candidate cells to those whose center is strictly in the target half-plane:
   - Left: `candidate.cx < current.cx`
   - Right: `candidate.cx > current.cx`
   - Up: `candidate.cy < current.cy`
   - Down: `candidate.cy > current.cy`
3. Among candidates, select the one with the shortest Euclidean distance from the current center.
4. If no candidates exist (edge of treemap), selection doesn't move.

This handles irregular squarified layouts naturally — the user always moves toward the nearest cell in the requested direction regardless of how cells are arranged.

### Selection Highlight

The selected cell gets a visual highlight:

- **Border:** The outermost pixel ring of the selected cell is rendered in white (or black on light cells, using the same luminance test as labels). This replaces the edge darkening on the selected cell.
- **Status bar:** A single line below the treemap (or at the bottom of the treemap area) showing: `filename  |  1.2 GB  |  2026-09-15  |  /full/path/to/file`

### Breadcrumb Bar

A single line above the treemap area showing the drill-down path:

```
/ > home > user > Projects > nixdirstat > src
```

- Each segment is a directory in `drill_path`.
- The current (deepest) segment is highlighted (bold or brighter color).
- Parent segments are dim.
- When `drill_path` is empty, shows just the scan root.

### Cross-Panel Sync

Bidirectional synchronization between panels:

- **Tree → Treemap:** When the directory tree cursor moves (and treemap is not focused), the treemap highlights the corresponding region. This already works.
- **Treemap → Tree:** When the treemap selection moves (treemap is focused), the directory tree scrolls to show the selected entry's parent directory and the extension legend scrolls to show the selected entry's category. New behavior.
- **Drill-down sync:** When the user drills down in the treemap, the directory tree expands and selects the corresponding directory node.

---

## 4. Module Architecture

### New Files

#### `src/ui/pixel_grid.rs`

The HalfBlock pixel grid abstraction. Responsibilities:

- Allocate a 2D grid of `Color` at 2× vertical resolution for a given `Rect`.
- `fill_rect()` — paint a rectangle in pixel coordinates.
- `darken_edges()` — apply edge darkening to a rectangle.
- `flush_to_buffer()` — convert the pixel grid to half-block characters and write to a ratatui `Buffer`.

This module knows about pixels and colors. It does not know about treemaps, files, or categories.

#### `src/ui/colors.rs`

Color palette and mapping. Responsibilities:

- Okabe-Ito palette constants.
- `category_color(cat: FileCategory) -> Color` — map category to color.
- `darken(color: Color, amount: f32) -> Color` — darken a color by a proportion.
- `contrast_text_color(bg: Color) -> Color` — return black or white for maximum contrast.
- `no_color_fallback(cat: FileCategory) -> Color` — grayscale mapping.
- `is_color_enabled() -> bool` — check `NO_COLOR` env var.

This module knows about `FileCategory` and color math. It does not know about rendering.

### Modified Files

#### `src/ui/widgets/treemap.rs`

Rewrite render internals:

1. Call `streemap::squarify` as before to get layout rectangles.
2. Convert float rects to pixel coordinates (2× vertical scale).
3. For each rect: look up category color via `colors.rs`, paint into `PixelGrid`, apply edge darkening.
4. Flush pixel grid to buffer.
5. Overlay labels on eligible cells.
6. If a cell is selected, render selection highlight border.

Add spatial navigation methods:

- `move_selection(direction: Direction) -> Option<usize>`
- `drill_down() -> Option<PathBuf>`
- `drill_up() -> Option<PathBuf>`

#### `src/ui/widgets/dir_tree.rs`

Replace `make_bar()` with sub-block proportional bars using `▏▎▍▌▋▊▉█`.

#### `src/ui/widgets/extension_legend.rs`

Update color swatches to use `colors::category_color()` instead of the FNV hash palette.

#### `src/ui/app.rs`

- Add `Treemap` to `PanelFocus`.
- Extend key event handling: when treemap is focused, dispatch arrow/vim keys to spatial navigation, Enter/Esc to drill-down/up.
- Add `drill_path: Vec<PathBuf>` to `ExplorerState` (or keep it in `TreemapState`).

#### `src/ui/views/explorer.rs`

- Add breadcrumb bar to the layout (1 line above treemap area).
- Add status bar to the layout (1 line below treemap area).
- Wire cross-panel sync for treemap → tree direction.

---

## 5. Testing Strategy

### Unit Tests

**`pixel_grid.rs`:**
- Paint a rectangle, verify grid contents at specific pixel coordinates.
- Paint overlapping rectangles, verify later paint wins.
- `darken_edges()`: verify outer pixels are darkened, inner pixels unchanged.
- `flush_to_buffer()`: verify correct `▀`/`▄`/`█`/space selection and fg/bg assignment for known pixel patterns.

**`colors.rs`:**
- Each `FileCategory` maps to the expected Okabe-Ito color.
- `darken()` reduces channels proportionally (test boundary: 0.0 = no change, 1.0 = black).
- `contrast_text_color()` returns white for dark colors, black for light colors.
- `NO_COLOR` fallback produces distinct grayscale values for all categories.

**`treemap.rs` (spatial navigation):**
- Given a known squarified layout, verify `move_selection(Right)` from cell 0 selects the expected neighbor.
- Edge cases: selection at boundary doesn't move; single-cell layout.
- `drill_down()` on a directory returns the path; on a file returns `None`.
- `drill_up()` with empty `drill_path` returns `None`.

**`dir_tree.rs` (sub-block bars):**
- Proportion 0.0 → empty, 1.0 → all full blocks.
- Proportion 0.5 with width 10 → `█████` (5 full blocks).
- Proportion 0.125 with width 8 → `▏` (1/8 of first cell).

### VHS Visual Tests

New tapes in `tests/vhs/`:

- **`hires_treemap.tape`**: Scan a test directory, verify HalfBlock rendering with edge darkening produces a visually distinct treemap with clear cell boundaries.
- **`treemap_navigation.tape`**: Focus treemap, navigate with arrow keys, verify selection highlight moves. Drill into a directory, verify breadcrumb updates. Drill back out.
- **`treemap_labels.tape`**: Scan a directory with large files, verify filenames appear on large treemap cells.
- **`no_color.tape`**: Run with `NO_COLOR=1`, verify grayscale treemap is still usable.
- **`sub_block_bars.tape`**: Verify directory tree shows smooth proportional bars.

### Property Tests (proptest)

- `pixel_grid`: For any rectangle within grid bounds, all pixels inside are the painted color, all pixels outside are unchanged.
- `darken()`: Output channels are always ≤ input channels. `darken(c, 0.0) == c`. `darken(c, 1.0) == black`.
- Sub-block bar: For any proportion in `[0.0, 1.0]` and any width, the bar string length equals width.

---

## 6. Compatibility & Degradation

| Terminal | HalfBlock | Sub-block bars | Okabe-Ito RGB | Edge darkening |
|----------|-----------|----------------|---------------|----------------|
| Modern (kitty, wezterm, alacritty, iTerm2) | Full support | Full support | Full support | Full support |
| tmux / screen | Full support | Full support | Full support (if underlying term supports) | Full support |
| Linux console (fbcon) | Full support | Full support | 256-color approximation | Works with approximated colors |
| `NO_COLOR` set | Full support (grayscale) | Full support | Grayscale fallback | Works with grayscale |
| SSH | Full support | Full support | Depends on client terminal | Full support |

HalfBlock (`▀`/`▄`/`█`) and sub-block characters (`▏▎▍▌▋▊▉█`) are in Unicode's basic blocks and are supported by virtually all modern terminal emulators and monospace fonts. No terminal capability detection is needed for the default configuration.

---

## 7. Performance Considerations

- **Pixel grid allocation:** `width × height × 2` colors per frame. For a 200×50 treemap area, that's 20,000 `Color` values (~80 KB). Allocate once and reuse across frames (clear instead of reallocate).
- **Edge darkening:** O(perimeter) per rectangle, negligible.
- **Label overlay:** Only computed for cells meeting the size threshold, typically <20 cells.
- **Spatial navigation:** O(n) scan of layout cells per keypress, where n is the number of visible cells (typically <500). No optimization needed.
- **Buffer flush:** One pass over the pixel grid, O(width × height). The ratatui diff engine then only writes changed cells to the terminal.

No performance concerns. The bottleneck remains the initial filesystem scan, not rendering.
