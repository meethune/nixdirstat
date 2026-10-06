//! Explorer view: the main interactive view shown after a scan completes.
//!
//! Layout (three panels):
//!
//! ```text
//! ┌─ /path/to/root ────────────────────────────────────────┐
//! │┌─ Directory Tree ──────┐┌─ Extensions ────────────────┐│
//! ││ ▸ src       450 KiB   ││ .rs  ██  300 KiB   30.0%    ││
//! ││ ▸ docs      200 KiB   ││ .py  ██  150 KiB   15.0%    ││
//! │└───────────────────────┘└─────────────────────────────┘│
//! │┌─ Disk Usage ─────────────────────────────────────────┐│
//! ││ [recursive squarified treemap]                        ││
//! │└──────────────────────────────────────────────────────┘│
//! └────────────────────────────────────────────────────────┘
//! ```

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::ui::{
    app::ExplorerState,
    tree::find_node,
    widgets::{
        dir_tree::render_dir_tree, extension_legend::ExtensionLegendWidget, treemap::TreemapWidget,
    },
};

/// Render the full explorer view into `area`.
pub fn render_explorer(frame: &mut Frame<'_>, state: &mut ExplorerState, area: Rect) {
    if area.width < 40 || area.height < 12 {
        frame.render_widget(Paragraph::new("Terminal too small (need 40×12)"), area);
        return;
    }

    // Outer border with breadcrumb title.
    let title = format!(" {} ", state.breadcrumb_path());
    let outer = Block::default().borders(Borders::ALL).title(title);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Vertical split: top panels (35%) and treemap (65%).
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(inner);

    let top_area = vertical[0];
    let treemap_area = vertical[1];

    // Top horizontal split: tree (60%) | legend (40%).
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(top_area);

    let tree_area = horizontal[0];
    let legend_area = horizontal[1];

    // --- Directory tree (focused panel → cyan border) ---
    let tree_block = Block::default()
        .borders(Borders::ALL)
        .title(" Directory Tree ")
        .border_style(Style::default().fg(Color::Cyan)); // focused
    let tree_inner = tree_block.inner(tree_area);
    frame.render_widget(tree_block, tree_area);

    // Resolve the treemap root node. We use find_node directly on the tree
    // field to avoid borrowing all of ExplorerState, which would conflict
    // with the mutable borrows needed for tree_state and treemap_state.
    let tm_node = find_node(&state.tree, &state.treemap_root).unwrap_or(&state.tree);
    let total_size = tm_node.size;

    render_dir_tree(
        frame,
        tm_node,
        &mut state.tree_state,
        tree_inner,
        true, // tree is always focused
        state.sort_field,
        state.sort_ascending,
    );

    // --- Extension legend ---
    let legend_block = Block::default()
        .borders(Borders::ALL)
        .title(" Extensions ")
        .border_style(Style::default().fg(Color::Indexed(240)));
    let legend_inner = legend_block.inner(legend_area);
    frame.render_widget(legend_block, legend_area);

    let legend = ExtensionLegendWidget {
        stats: &state.extension_stats,
        total_size,
        scroll_offset: state.legend_scroll,
    };
    frame.render_widget(legend, legend_inner);

    // --- Treemap ---
    let treemap_block = Block::default()
        .borders(Borders::ALL)
        .title(" Disk Usage ")
        .border_style(Style::default().fg(Color::Indexed(240)));
    let treemap_inner = treemap_block.inner(treemap_area);
    frame.render_widget(treemap_block, treemap_area);

    // Re-resolve to avoid holding the borrow across the mutable treemap_state access.
    let tm_node = find_node(&state.tree, &state.treemap_root).unwrap_or(&state.tree);
    let treemap = TreemapWidget { root: tm_node };
    frame.render_stateful_widget(treemap, treemap_inner, &mut state.treemap_state);

    render_selection_info(frame, state, treemap_inner);

    // --- Help overlay ---
    if state.show_help {
        render_help_overlay(frame, inner);
    }

    // --- Error status line ---
    if let Some(ref msg) = state.error_message {
        let error_area = Rect {
            x: inner.x,
            y: inner.bottom().saturating_sub(1),
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(msg.as_str()).style(Style::default().fg(Color::White).bg(Color::Red)),
            error_area,
        );
    }
}

/// Render the selection info bar at the bottom of the treemap.
fn render_selection_info(frame: &mut Frame<'_>, state: &ExplorerState, treemap_area: Rect) {
    let selected = state.tree_state.selected();
    if selected.is_empty() {
        return;
    }
    let mut lookup_path = state.treemap_root.clone();
    lookup_path.extend(selected.iter().cloned());
    if let Some(node) = find_node(&state.tree, &lookup_path) {
        let info_line = Line::from(vec![
            Span::styled(
                format!(" \u{25b8} {}", node.name),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(
                format!("  {} ", crate::types::format_size(node.size)),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        let info_area = Rect {
            x: treemap_area.x,
            y: treemap_area.bottom().saturating_sub(1),
            width: treemap_area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(info_line).style(Style::default().bg(Color::Indexed(236))),
            info_area,
        );
    }
}

/// Render the help overlay with all keybindings.
fn render_help_overlay(frame: &mut Frame<'_>, area: Rect) {
    let help_width = 50.min(area.width.saturating_sub(4));
    let help_height = 20.min(area.height.saturating_sub(4));
    let help_area = Rect {
        x: area.x + (area.width.saturating_sub(help_width)) / 2,
        y: area.y + (area.height.saturating_sub(help_height)) / 2,
        width: help_width,
        height: help_height,
    };

    frame.render_widget(Clear, help_area);

    let help_text = vec![
        Line::from(vec![Span::styled(
            " NixDirStat — Keybindings ",
            Style::default().fg(Color::Yellow),
        )]),
        Line::from(""),
        Line::from(" ↑/↓ j/k        Navigate tree"),
        Line::from(" →/l/Enter      Go into directory"),
        Line::from(" ←/h/Bksp/u    Go up / collapse"),
        Line::from(" Home/g         Jump to first"),
        Line::from(" End/G          Jump to last"),
        Line::from(" n              Sort by name"),
        Line::from(" s              Sort by size"),
        Line::from(" m              Sort by modified"),
        Line::from(" r              Reverse sort"),
        Line::from(" Z              Zoom to root"),
        Line::from(" ?              Toggle this help"),
        Line::from(" q/Esc          Quit"),
    ];

    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title(" Help "))
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White).bg(Color::Black));

    frame.render_widget(help, help_area);
}
