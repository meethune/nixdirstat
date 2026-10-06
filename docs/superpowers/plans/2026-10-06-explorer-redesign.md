# Explorer View Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the flat directory browser with a WinDirStat-style recursive treemap, directory tree, and extension legend.

**Architecture:** Build an in-memory `DirNode` tree from the flat entries table. The treemap renders recursively (squarify each directory's children within its allocated area). The directory tree uses `tui-tree-widget`. The extension legend aggregates bytes per extension with color swatches. All three panels read from the same `DirNode` tree — no SQL queries during navigation.

**Tech Stack:** Rust, ratatui, `tui-tree-widget` (v0.24), `streemap`, existing `extension_color()` FNV-1a palette.

**Spec:** `docs/superpowers/specs/2026-10-06-explorer-redesign.md`

## Global Constraints

- `unsafe` code forbidden (`unsafe_code = "forbid"`).
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` must pass.
- `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` denied in non-test code.
- All public items require `///` doc comments.
- VHS visual tests required for TUI changes — run `just vhs` and inspect screenshots before delivery.
- Conventional commits. Work on `feat/explorer-redesign` branch.
- Closes #28.

## Review Focus

1. **Terminal < 40×12** — explorer should render a fallback message, not panic. Add test in Task 5: `explorer_renders_fallback_at_minimum_size`.
2. **Scan with zero files** (empty directory) — `DirNode` tree has only the root with no children. Treemap, tree, and legend should render empty states. Add test in Task 1: `build_tree_empty_directory`.
3. **Deeply nested paths (100+ levels)** — recursive treemap must not stack-overflow. The pruning threshold (area < 2 cells) guarantees bounded recursion by terminal area, not tree depth. Add test in Task 4: `treemap_deep_nesting_no_overflow`.
4. **Single huge file dominates** — one file is 99% of the scan. Treemap should still show the small files in their remaining sliver. Add test in Task 4: `treemap_dominant_file_shows_siblings`.
5. **Non-UTF-8 filenames** — `DirNode.name` uses `to_string_lossy()`. The lossy character `�` should render without panic. Add test in Task 1: `build_tree_non_utf8_filename`.

---

## File Structure

| Path | Responsibility | Task |
|------|---------------|------|
| `src/ui/tree.rs` | `DirNode` struct, `build_tree()`, `collect_extension_stats()`, `find_node()` | 1 |
| `src/ui/widgets/extension_legend.rs` | `ExtensionLegendWidget` — scrollable extension list with color swatches | 2 |
| `src/ui/widgets/dir_tree.rs` | `DirTreeWidget` — directory tree via `tui-tree-widget` | 3 |
| `src/ui/widgets/treemap.rs` | Rewritten: recursive rendering from `&DirNode` | 4 |
| `src/ui/views/explorer.rs` | Rewritten: new 3-panel layout | 5 |
| `src/ui/app.rs` | Rewritten: `ExplorerState` restructured around `DirNode` | 5 |
| `src/ui/mod.rs` | Modified: data loading, keybinding handlers | 5 |
| `src/ui/widgets/mod.rs` | Modified: module declarations | 2, 3 |
| `Cargo.toml` | Add `tui-tree-widget = "0.24"` | 3 |
| `src/ui/widgets/file_table.rs` | Deleted | 5 |
| `src/ui/widgets/type_chart.rs` | Deleted | 5 |
| `tests/vhs/explore-redesign.tape` | VHS tape for redesigned explorer | 5 |

---

### Task 1: DirNode Tree and Extension Stats

**Files:**
- Create: `src/ui/tree.rs`
- Modify: `src/ui/mod.rs` (add `pub mod tree;`)

**Interfaces:**
- Consumes: `FileEntry` (from `crate::types`) — fields `path`, `size`, `allocated_size`, `file_type`, `extension` (derived from path), `mtime`
- Produces:
  - `DirNode { name: String, size: u64, allocated: u64, file_count: u64, children: Vec<DirNode>, is_dir: bool, extension: Option<String>, mtime: SystemTime }`
  - `ExtensionStat { extension: Option<String>, count: u64, total_size: u64 }`
  - `pub fn build_tree(entries: &[FileEntry], root_path: &Path) -> DirNode` — constructs tree from sorted entries
  - `pub fn collect_extension_stats(node: &DirNode) -> Vec<ExtensionStat>` — walks tree, returns sorted by total_size desc
  - `pub fn find_node<'a>(root: &'a DirNode, path: &[String]) -> Option<&'a DirNode>` — navigate to a subtree by path components

- [ ] **Step 1: Write failing tests in `src/ui/tree.rs`**

| Test | Assertion |
|------|-----------|
| `build_tree_flat_directory` | root with 3 files → `root.children.len() == 3`, each child `is_dir == false`, `root.is_dir == true` |
| `build_tree_nested` | `/root/sub/file.txt` → root has child "sub" (is_dir), "sub" has child "file.txt" |
| `build_tree_sizes_from_entries` | file with `size == 100` → corresponding `DirNode.size == 100` |
| `build_tree_directory_sizes_aggregated` | dir with 2 files (50+30) → dir node `size == 80` (from entry's pre-aggregated size) |
| `build_tree_empty_directory` | root with no files → `root.children.is_empty()`, `root.size == 0` |
| `build_tree_non_utf8_filename` | entry with lossy path containing `�` → builds without panic, name contains replacement char |
| `build_tree_extensions` | "file.rs" → `extension == Some("rs")`, "Makefile" → `extension == None` |
| `collect_stats_groups_by_extension` | 2 `.rs` (10+20) + 1 `.py` (50) → stats has rs: total_size=30, py: total_size=50 |
| `collect_stats_sorted_by_size` | largest extension first in result |
| `find_node_root` | empty path → returns root |
| `find_node_nested` | path `["sub", "deep"]` → returns the "deep" node |
| `find_node_missing` | path `["nonexistent"]` → returns None |

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test tree::tests`

- [ ] **Step 3: Implement `DirNode`, `ExtensionStat`, `build_tree`, `collect_extension_stats`, `find_node`**

`build_tree`: iterate entries, for each entry strip the `root_path` prefix to get relative components. Walk/create `DirNode` tree by component. Set `is_dir` from `FileType::Directory`. Set `extension` from `path.extension()`. Set `size`/`allocated`/`mtime` from entry fields.

`collect_extension_stats`: recursive walk collecting `HashMap<Option<String>, ExtensionStat>` from leaf nodes (files only). Collect, sort by `total_size` desc.

`find_node`: walk `children` by matching `name` for each path component.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test tree::tests`

- [ ] **Step 5: Add `pub mod tree;` to `src/ui/mod.rs`**

- [ ] **Step 6: Lint and format**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 7: Commit**

```bash
git add src/ui/tree.rs src/ui/mod.rs
git commit -m "feat(ui): add DirNode tree builder and extension stats"
```

---

### Task 2: Extension Legend Widget

**Files:**
- Create: `src/ui/widgets/extension_legend.rs`
- Modify: `src/ui/widgets/mod.rs` (add `pub mod extension_legend;`)
- Modify: `src/ui/widgets/treemap.rs` (make `extension_color` `pub(crate)`)

**Interfaces:**
- Consumes: `ExtensionStat` from Task 1, `extension_color(Option<&str>) -> Color` from treemap module
- Produces:
  - `ExtensionLegendWidget<'a> { stats: &'a [ExtensionStat], total_size: u64, scroll_offset: usize }`
  - Implements `Widget`
  - `pub(crate) fn extension_color(ext: Option<&str>) -> Color` (visibility change in treemap.rs)

- [ ] **Step 1: Make `extension_color` pub(crate) in `treemap.rs`**

Change `fn extension_color` to `pub(crate) fn extension_color`.

- [ ] **Step 2: Write failing tests**

| Test | Assertion |
|------|-----------|
| `legend_renders_extension_names` | stats with ".py" and ".rs" → buffer contains "py" and "rs" |
| `legend_renders_sizes` | stat with total_size 1048576 → buffer contains "MiB" |
| `legend_renders_percentages` | stat with 50% of total → buffer contains "50" |
| `legend_renders_color_swatches` | stat with ".py" → at least one cell has `bg == extension_color(Some("py"))` |
| `legend_empty_stats` | empty stats vec → no panic, renders empty area |
| `legend_sorted_order` | largest stat first in rendered output |

- [ ] **Step 3: Implement `ExtensionLegendWidget`**

Render a row per `ExtensionStat`: extension name (or "(none)") left-aligned, 2-cell color swatch using `extension_color()`, formatted size via `format_size()`, percentage `(stat.total_size * 100 / total_size)`. Respect `scroll_offset` to support scrolling.

- [ ] **Step 4: Run tests**

Run: `cargo test extension_legend`

- [ ] **Step 5: Lint and format**

- [ ] **Step 6: Commit**

```bash
git add src/ui/widgets/extension_legend.rs src/ui/widgets/mod.rs src/ui/widgets/treemap.rs
git commit -m "feat(ui): add extension legend widget with color swatches"
```

---

### Task 3: Directory Tree Widget

**Files:**
- Create: `src/ui/widgets/dir_tree.rs`
- Modify: `src/ui/widgets/mod.rs` (add `pub mod dir_tree;`)
- Modify: `Cargo.toml` (add `tui-tree-widget = "0.24"`)

**Interfaces:**
- Consumes: `DirNode` from Task 1, `format_size` from `crate::types`
- Produces:
  - `pub fn render_dir_tree(frame: &mut Frame, node: &DirNode, state: &mut TreeState, area: Rect, parent_size: u64, focused: bool)` — renders the tree into `area`
  - `pub fn dir_node_to_tree_items(node: &DirNode, parent_size: u64) -> Vec<TreeItem<'static, String>>` — converts `DirNode` to `tui-tree-widget` items

- [ ] **Step 1: Add `tui-tree-widget = "0.24"` to `Cargo.toml`**

- [ ] **Step 2: Write failing tests**

| Test | Assertion |
|------|-----------|
| `tree_renders_directory_names` | node with children "src" and "docs" → buffer contains "src" and "docs" |
| `tree_renders_sizes` | child with size 1048576 → buffer contains "MiB" |
| `tree_renders_percentages` | child 50% of parent → buffer contains "50" |
| `tree_renders_expand_indicator` | directory node → buffer contains "▸" or "▾" |
| `tree_files_have_no_indicator` | file node → no "▸" or "▾" before name |
| `tree_empty_node` | node with no children → renders without panic |

- [ ] **Step 3: Implement `dir_node_to_tree_items` and `render_dir_tree`**

`dir_node_to_tree_items`: recursively convert `DirNode` children to `TreeItem` values. Each item's text: `format!("{:<name_width$} {:>9} {:>5.1}%", name, format_size(size), pct)`. Directories get child items; files don't. Sort children by the current sort field before conversion.

`render_dir_tree`: create `Tree::new(items)` with `tui-tree-widget`, set highlight style (blue bg for focused, dim for unfocused), render as `StatefulWidget` with `TreeState`.

- [ ] **Step 4: Run tests**

Run: `cargo test dir_tree`

- [ ] **Step 5: Lint and format**

- [ ] **Step 6: Commit**

```bash
git add src/ui/widgets/dir_tree.rs src/ui/widgets/mod.rs Cargo.toml Cargo.lock
git commit -m "feat(ui): add directory tree widget using tui-tree-widget"
```

---

### Task 4: Recursive Treemap

**Files:**
- Rewrite: `src/ui/widgets/treemap.rs`

**Interfaces:**
- Consumes: `DirNode` from Task 1
- Produces:
  - `TreemapWidget<'a> { root: &'a DirNode }` — replaces old `TreemapWidget { items: Vec<TreemapItem> }`
  - `TreemapState { highlighted_path: Option<Vec<String>> }` — replaces old `TreemapState { selected: Option<usize> }`
  - Implements `StatefulWidget`
  - `TreemapItem` struct deleted
  - `extension_color()`, `category_color()`, `EXTENSION_PALETTE` preserved

- [ ] **Step 1: Write failing tests**

| Test | Assertion |
|------|-----------|
| `treemap_recursive_renders_files` | root with subdir containing 2 files → both file names appear in buffer (or their extension colors present) |
| `treemap_proportional_areas` | root with 75/25 split files → larger file's color occupies ≥60% of cells |
| `treemap_directory_recursion` | root/dir/file.rs → file.rs gets colored cells even though it's nested |
| `treemap_pruning_small_cells` | 100 tiny files → no panic, cells filled with color (pruned, not individually rendered) |
| `treemap_deep_nesting_no_overflow` | 150-level deep path with one file → renders without stack overflow |
| `treemap_dominant_file_shows_siblings` | one file 99% + one file 1% → both have at least 1 colored cell (in a 100-cell buffer) |
| `treemap_highlight_path` | highlighted_path set → cells in that region have highlighted style |
| `treemap_empty_root` | root with no children → no panic, empty area |

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test treemap::tests`

- [ ] **Step 3: Rewrite `TreemapWidget` render method**

Delete `TreemapItem`. Change `TreemapWidget` to hold `root: &'a DirNode`. The `render` method:

```
fn render_recursive(node: &DirNode, area: Rect, buf: &mut Buffer, highlight: &Option<Vec<String>>, depth: usize)
```

1. Filter node's children to `size > 0`, collect into squarify input.
2. Call `streemap::squarify` to assign rectangles.
3. For each child rect:
   - File: fill with `extension_color(child.extension.as_deref())`, render label if width ≥ 3.
   - Directory: if area ≥ 2 cells, recurse into `render_recursive(child, child_rect, ...)`. If area < 2, fill with color of largest file in subtree.
4. Padding: if directory rect is ≥ 6×6, shrink inner area by 1 cell on each edge before recursing.
5. Highlight: if current path matches `highlighted_path` prefix, draw bright border.

- [ ] **Step 4: Run tests**

Run: `cargo test treemap::tests`

- [ ] **Step 5: Lint and format**

- [ ] **Step 6: Commit**

```bash
git add src/ui/widgets/treemap.rs
git commit -m "feat(ui): rewrite treemap for recursive DirNode rendering"
```

---

### Task 5: Explorer Layout, State, and Wiring

**Files:**
- Rewrite: `src/ui/views/explorer.rs`
- Rewrite: `src/ui/app.rs` (`ExplorerState` and related types)
- Modify: `src/ui/mod.rs` (data loading, keybinding handlers)
- Delete: `src/ui/widgets/file_table.rs`
- Delete: `src/ui/widgets/type_chart.rs`
- Modify: `src/ui/widgets/mod.rs` (remove old, add new module declarations)
- Create: `tests/vhs/explore-redesign.tape`

**Interfaces:**
- Consumes: `DirNode`, `ExtensionStat`, `build_tree`, `collect_extension_stats`, `find_node` from Task 1; `ExtensionLegendWidget` from Task 2; `render_dir_tree` from Task 3; `TreemapWidget` from Task 4; `Storage` trait, `SqliteStorage`, `format_size`, `ScanMetadata`
- Produces: working redesigned explorer view with all keybindings from the spec

- [ ] **Step 1: Restructure `ExplorerState` in `app.rs`**

Replace current `ExplorerState` with:
```rust
pub struct ExplorerState {
    pub tree: DirNode,
    pub tree_state: tui_tree_widget::TreeState<String>,
    pub extension_stats: Vec<ExtensionStat>,
    pub treemap_root: Vec<String>,
    pub treemap_state: TreemapState,
    pub focus: PanelFocus,
    pub sort_field: TreeSortField,
    pub sort_ascending: bool,
    pub show_help: bool,
    pub error_message: Option<String>,
    pub legend_scroll: usize,
    pub scan_root: PathBuf,
}
```

Add `PanelFocus` enum (`Tree`, `Legend`), `TreeSortField` enum (`Size`, `Name`, `Modified`).

Add methods: `zoom_in(&mut self, dir_name: String)`, `zoom_out(&mut self)`, `zoom_to_root(&mut self)`, `current_treemap_node(&self) -> Option<&DirNode>`, `toggle_focus(&mut self)`, `set_sort(&mut self, field: TreeSortField)`, `toggle_sort_direction(&mut self)`, `breadcrumb_path(&self) -> String`.

Remove: `navigate_into`, `navigate_up`, `cycle_sort`, `reverse_sort`, `select_prev`, `select_next`, `selected_entry`, all `SortField`/`SortDirection` usage, `entries`, `type_stats`, `selected_index`.

Delete `ScanProgressState.update`'s dependency on old types — keep `ScanProgressState` unchanged.

- [ ] **Step 2: Rewrite `explorer.rs` layout**

New layout:
```
Vertical split: [30-40% top, 60-70% bottom]
  Top: Horizontal split [60% tree, 40% legend]
  Bottom: treemap (full width)
Outer block with breadcrumb title
```

`render_explorer(frame, state, area)`:
1. Outer `Block` with title `format!(" {} ", state.breadcrumb_path())`.
2. Vertical split: top panels (35%), treemap (65%).
3. Top horizontal split: tree (60%), legend (40%).
4. Render `render_dir_tree(frame, current_node, &mut state.tree_state, tree_area, ...)`.
5. Render `ExtensionLegendWidget { stats: &state.extension_stats, ... }` in legend area.
6. Render `TreemapWidget { root: current_node }` in treemap area.
7. If `state.show_help`, render help overlay.
8. If `state.error_message.is_some()`, render error status line.

- [ ] **Step 3: Rewrite keybinding handler in `mod.rs`**

`handle_explorer_event(event, state)` — no storage parameter needed (navigation is in-memory):

| Key | Action |
|-----|--------|
| `Up`/`k` | if Tree focus: `state.tree_state.key_up()`; if Legend: decrement scroll |
| `Down`/`j` | if Tree: `state.tree_state.key_down()`; if Legend: increment scroll |
| `Right`/`l`/`Enter` | if Tree: `state.tree_state.toggle_selected()` (expand) + zoom treemap |
| `Left`/`h`/`Backspace` | if Tree: `state.tree_state.key_left()` (collapse) or zoom out |
| `Tab` | `state.toggle_focus()` |
| `Home`/`g` | `state.tree_state.select_first()` |
| `End`/`G` | `state.tree_state.select_last()` |
| `n` | `state.set_sort(TreeSortField::Name)` |
| `s` | `state.set_sort(TreeSortField::Size)` |
| `m` | `state.set_sort(TreeSortField::Modified)` |
| `r` | `state.toggle_sort_direction()` |
| `z` | zoom treemap to selected directory |
| `Z` | `state.zoom_to_root()` |
| `?` | `state.show_help = !state.show_help` |
| `i` | show file info popup for selected node |

- [ ] **Step 4: Update data loading in `mod.rs`**

Replace `load_explorer_state` to:
1. Open `SqliteStorage::open_readonly(path)`.
2. Load metadata: `storage.load_scan_metadata()`.
3. Query ALL entries: `storage.query_entries(&EntryQuery { limit: None, ..Default })`.
4. Build tree: `build_tree(&entries, &metadata.root)`.
5. Compute stats: `collect_extension_stats(&tree)`.
6. Construct `ExplorerState` with tree, stats, default zoom (root).

Remove `query_directory_children` and `query_type_stats` calls from UI code.

- [ ] **Step 5: Delete old widgets, update module declarations**

Delete `src/ui/widgets/file_table.rs` and `src/ui/widgets/type_chart.rs`. Update `src/ui/widgets/mod.rs` to remove their module declarations (already added `extension_legend` and `dir_tree` in Tasks 2-3).

- [ ] **Step 6: Write tests**

| Test | Assertion |
|------|-----------|
| `explorer_renders_three_panels` | 80×24 buffer: tree content in top-left, legend in top-right, treemap colors in bottom |
| `explorer_renders_breadcrumb` | buffer contains the root path in the border title |
| `explorer_renders_fallback_at_minimum_size` | 30×8 buffer → renders "Terminal too small" without panic |
| `zoom_in_changes_treemap_root` | after `zoom_in("sub")`, `treemap_root == ["sub"]`, extension stats recomputed |
| `zoom_out_restores_parent` | after zoom_in then zoom_out, `treemap_root` is empty |
| `toggle_focus_switches_panels` | starts at Tree, toggle → Legend, toggle → Tree |
| `help_overlay_toggles` | `show_help = true` → buffer contains "?" somewhere (help text visible) |

- [ ] **Step 7: Run full test suite**

Run: `cargo test`

- [ ] **Step 8: Lint, format, doc check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`

- [ ] **Step 9: VHS visual verification**

Create `tests/vhs/explore-redesign.tape` testing the redesigned explorer against the Reporting directory. Run `just vhs` and inspect screenshots.

- [ ] **Step 10: Commit**

```bash
git add src/ui/ tests/vhs/ Cargo.toml Cargo.lock
git commit -m "feat(ui): redesign explorer with recursive treemap, dir tree, and extension legend

Closes #28"
```
