# Explorer View Redesign — Recursive Treemap, Directory Tree, Extension Legend

## Overview

Redesign the NixDirStat explorer view from a flat directory browser to a WinDirStat-style disk usage visualizer. The treemap shows all files across the entire scan simultaneously (each file is a colored rectangle proportional to its size). The directory tree shows the hierarchy with expandable nodes. The extension legend maps colors to file extensions.

This replaces the current single-level treemap, flat file table, and category bar chart.

## Data Model: `DirNode` Tree

Build an in-memory tree from the flat `entries` table at load time. This is the single source of truth for both the treemap and the directory tree.

```rust
pub struct DirNode {
    pub name: String,
    pub size: u64,
    pub allocated: u64,
    pub file_count: u64,
    pub children: Vec<DirNode>,
    pub is_dir: bool,
    pub extension: Option<String>,
}
```

### Construction

Query all entries from storage sorted by `path_text ASC` (lexicographic = depth-first ordering). Walk the sorted list, splitting each path into components and inserting nodes into the tree. Directory sizes come from the database (already aggregated by the analyzer). Tree build is O(n) — one pass, one allocation per entry.

### Location

New module `src/ui/tree.rs`. Pure data structure with no rendering logic.

### Scale

13K entries: tree build <10ms, ~5MB memory. 2M entries: ~200ms build, ~100MB memory. Within the 2GB budget.

## Recursive Treemap

The treemap widget renders the `DirNode` tree recursively:

1. Start with the treemap root node's children and the full treemap area.
2. Call `squarify` on the children (weighted by `size`).
3. For each child rectangle:
   - **File:** render as a colored cell. Color from `extension_color()` (FNV-1a hash, existing palette). Label if cell width ≥ 3.
   - **Directory:** recurse into its children, squarifying within the allocated rectangle.
4. **Pruning:** stop recursing when a cell's area < 2 terminal cells. Fill the cell with the extension color of the largest file in that subtree.

### Visual Boundaries

Use 1-cell padding between sibling groups when the directory rectangle is ≥ 6 cells in both dimensions. No padding for smaller directories.

### Highlight

When a node is selected in the directory tree, draw a bright border around the corresponding treemap region.

### Zoom

Pressing `z` on a directory makes it the treemap root — the treemap re-renders showing only that subtree at full resolution. `Z` resets to the scan root. `Right`/`Enter` on a directory also zooms. `Left`/`Backspace` zooms out. The breadcrumb path in the outer border title reflects the current treemap root.

### Widget Changes

- `TreemapWidget` takes `&DirNode` (the current treemap root) instead of `Vec<TreemapItem>`
- `TreemapItem` struct deleted
- `TreemapState` gains `highlighted_path: Option<Vec<String>>` for cross-panel highlighting (path as name components from treemap root)
- `build_treemap_items()` in `explorer.rs` deleted
- Render method becomes recursive
- `extension_color()` and `EXTENSION_PALETTE` unchanged

## Directory Tree Panel (replaces file table)

Replaces `FileTableWidget` with `DirTreeWidget` using `tui-tree-widget` (v0.24).

### Display per Row

```
▸ Users           528.7 GiB  38.94%  ████████░░
▾ Games           519.7 GiB  38.28%  ████████░░
    ▸ Steam       412.3 GiB  30.37%  ██████░░░░
    ▸ Epic         87.1 GiB   6.42%  █░░░░░░░░░
  pagefile.sys     13.5 GiB   0.99%
```

Each row: expand/collapse indicator (directories only), name, formatted size, percentage of parent, proportional bar. Files and directories intermixed within each level, sorted by size descending (default).

### Expand/Collapse

Directories start collapsed to one level (root's children visible). `Right`/`Enter` expands. `Left` collapses. `Up`/`Down` navigates.

### Sorting

Tree children are sortable within each directory level:
- `s` — sort by size (default)
- `n` — sort by name
- `m` — sort by modified
- `r` — reverse sort direction

Sorting applies to all levels uniformly.

### Cross-Panel Interaction

Selecting a node in the tree highlights the corresponding region in the treemap. Zooming the treemap (`z`) syncs with the selected tree node.

### Files

- Delete: `src/ui/widgets/file_table.rs`
- Create: `src/ui/widgets/dir_tree.rs`

## Extension Legend (replaces bar chart)

Replaces `TypeChartWidget` with `ExtensionLegendWidget` — a scrollable list of extensions sorted by total size.

### Display

```
 .py    ██   42.3 MiB   5.3%
 .pyc   ██   31.8 MiB   4.0%
 .so    ██   28.1 MiB   3.5%
 .png   ██   22.4 MiB   2.8%
 (none) ██   18.2 MiB   2.3%
```

Each row: extension name, color swatch (matching treemap's `extension_color()`), formatted size, percentage of total. No description column — the extension is the identifier.

### Data

Walk the `DirNode` tree to collect `HashMap<Option<String>, ExtensionStat>` where `ExtensionStat = { count: u64, total_size: u64 }`. Recomputed on zoom (scoped to current treemap root). Sorted by `total_size` descending.

### Scrolling

`Up`/`Down` scrolls when the legend has focus. `Tab` switches focus between directory tree and extension legend.

### Files

- Delete: `src/ui/widgets/type_chart.rs`
- Create: `src/ui/widgets/extension_legend.rs`

## Layout

```
┌─ /path/to/scan/root ──────────────────────────────────────────────┐
│┌─ Directory Tree ───────────────┐┌─ Extensions ─────────────────┐│
││ ▸ Ghostwriter     362.0 MiB    ││ .py    ██  42.3 MiB   5.3%   ││
││ ▸ research        223.3 MiB    ││ .pyc   ██  31.8 MiB   4.0%   ││
││ ▸ cervantes       125.0 MiB    ││ .so    ██  28.1 MiB   3.5%   ││
││ ▸ presentation     49.5 MiB    ││ .png   ██  22.4 MiB   2.8%   ││
││   demo.cast         4.6 MiB    ││ (none) ██  18.2 MiB   2.3%   ││
│└────────────────────────────────┘│ ...                            ││
│                                  └───────────────────────────────┘│
│┌─ Disk Usage ───────────────────────────────────────────────────┐│
││ [recursive squarified treemap — all files visible]              ││
│└─────────────────────────────────────────────────────────────────┘│
└───────────────────────────────────────────────────────────────────┘
```

- Top row: directory tree (60%) | extension legend (40%), side by side
- Bottom: treemap (full width), 60-70% of vertical space
- Outer border title: breadcrumb path of current treemap root
- Minimum terminal size: 40×12 (degrade gracefully below)

## Keybindings

| Key | Action |
|-----|--------|
| `Up`/`Down`/`j`/`k` | Navigate in focused panel |
| `Right`/`l`/`Enter` | Expand directory / zoom treemap in |
| `Left`/`h`/`Backspace` | Collapse directory / zoom treemap out |
| `Tab` | Switch focus: tree ↔ legend |
| `Home`/`g` | Jump to first entry in focused panel |
| `End`/`G` | Jump to last entry in focused panel |
| `PgUp`/`PgDn` | Page scroll in focused panel |
| `n` | Sort tree children by name |
| `s` | Sort tree children by size |
| `m` | Sort tree children by modified |
| `r` | Reverse current sort direction |
| `z` | Zoom treemap to selected directory |
| `Z` | Zoom treemap back to scan root |
| `i` | File info popup (full path, size, permissions, timestamps) |
| `?` | Toggle help overlay showing all keybindings |
| `q`/`Esc` | Quit |

Closes #28.

## ExplorerState Changes

```rust
pub struct ExplorerState {
    pub tree: DirNode,
    pub tree_state: tui_tree_widget::TreeState,
    pub extension_stats: Vec<ExtensionStat>,
    pub treemap_root: Vec<String>,   // path components from scan root to current zoom
    pub treemap_state: TreemapState,
    pub focus: PanelFocus,           // Tree | Legend
    pub sort_field: TreeSortField,   // Size | Name | Modified
    pub sort_ascending: bool,
    pub show_help: bool,
    pub error_message: Option<String>,
}

pub enum PanelFocus { Tree, Legend }
pub enum TreeSortField { Size, Name, Modified }

pub struct ExtensionStat {
    pub extension: Option<String>,
    pub count: u64,
    pub total_size: u64,
}
```

## Data Loading

At explorer initialization (both scan-to-explore transition and `explore` subcommand):

1. Query all entries from storage: `query_entries(&EntryQuery { limit: None, ..Default })`
2. Build `DirNode` tree from the flat entry list
3. Compute extension stats from the tree
4. Create `ExplorerState` with the tree as the root

No `query_directory_children` calls during navigation. No `query_type_stats` calls. Everything operates on the in-memory `DirNode` tree.

## Files Changed

| Action | File | Description |
|--------|------|-------------|
| Create | `src/ui/tree.rs` | `DirNode` struct, tree builder from entries |
| Create | `src/ui/widgets/dir_tree.rs` | Directory tree widget using `tui-tree-widget` |
| Create | `src/ui/widgets/extension_legend.rs` | Extension legend widget |
| Rewrite | `src/ui/widgets/treemap.rs` | Recursive rendering from `&DirNode` |
| Rewrite | `src/ui/views/explorer.rs` | New layout (tree+legend top, treemap bottom) |
| Rewrite | `src/ui/app.rs` | `ExplorerState` restructured |
| Modify | `src/ui/mod.rs` | Data loading builds `DirNode`; keybinding handlers |
| Modify | `src/ui/widgets/mod.rs` | Module declarations |
| Delete | `src/ui/widgets/file_table.rs` | Replaced by dir_tree |
| Delete | `src/ui/widgets/type_chart.rs` | Replaced by extension_legend |
| Modify | `Cargo.toml` | Add `tui-tree-widget = "0.24"` |

## Unchanged

- `src/ui/views/progress.rs` — scan progress view
- All non-UI modules (scanner, storage, analyzer, pipeline, platform, types, error, cli)
- `extension_color()` and `EXTENSION_PALETTE` in treemap.rs — reused by legend
- `category_color()` — retained for any future category-level views

## Testing

- **Unit tests:** `DirNode` tree builder (flat entries → correct tree structure, sizes match, extensions populated)
- **Widget tests:** headless `TestBackend` for each widget (dir_tree renders expandable nodes, extension_legend renders sorted list with swatches, treemap renders recursively with correct proportions)
- **VHS visual tests:** new tape for the redesigned explorer, tested against Reporting directory and deps directory
- **Integration:** verify `explore` subcommand loads data and renders without panic
