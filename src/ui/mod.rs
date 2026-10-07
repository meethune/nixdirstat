//! TUI application shell.
//!
//! Provides terminal setup/teardown, panic recovery, and the two top-level
//! entry points for the TUI:
//! - [`run_scan_ui`] — start a scan, display the progress view, then
//!   transition to the explorer when the scan completes.
//! - [`run_explore_ui`] — open an existing scan database and display the
//!   file-explorer view directly.

pub mod app;
pub mod colors;
pub mod pixel_grid;
pub mod tree;
pub mod views;
pub mod widgets;

use std::{io::Stdout, path::Path, time::Duration};

use crossterm::{
    ExecutableCommand as _,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    error::{PipelineError, UiError},
    pipeline::{PipelineConfig, run_pipeline},
    storage::{ReadStorage as _, sqlite::SqliteStorage},
    types::EntryQuery,
};
use app::{AppState, ExplorerState, PanelFocus, ScanProgressState, TreeSortField};
use tree::build_tree;
use views::{explorer::render_explorer, progress::render_progress};

/// Initialise the terminal for TUI rendering.
///
/// Enables raw mode, switches to the alternate screen buffer, installs a
/// panic hook that restores the terminal before printing the panic message,
/// and returns a [`Terminal`] backed by stdout.
///
/// # Errors
///
/// Returns [`UiError::Terminal`] if `enable_raw_mode` or the alternate-screen
/// command fails.
pub fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, UiError> {
    enable_raw_mode()?;
    std::io::stdout().execute(EnterAlternateScreen)?;

    // Install a panic hook that restores the terminal before printing the
    // panic message.  Errors are ignored because we are already unwinding.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = std::io::stdout().execute(LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let backend = CrosstermBackend::new(std::io::stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to its state before [`setup_terminal`] was called.
///
/// Disables raw mode and leaves the alternate screen buffer.
///
/// # Errors
///
/// Returns [`UiError::Terminal`] if `disable_raw_mode` or the leave-screen
/// command fails.
pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), UiError> {
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// Run the scan UI: start the pipeline, display the progress view, then
/// transition to the explorer view when scanning completes.
///
/// Terminal setup and teardown are handled automatically. If the event loop
/// fails, terminal restoration is still attempted; a restoration failure is
/// logged but the original error is preserved.
///
/// # Errors
///
/// Returns [`UiError`] if terminal setup/teardown fails or the event loop
/// encounters an unrecoverable error.
pub async fn run_scan_ui(config: PipelineConfig) -> Result<(), UiError> {
    let mut terminal = setup_terminal()?;
    let result = run_scan_ui_inner(&mut terminal, config).await;
    finish_with_restore(&mut terminal, result)
}

/// Run the explorer UI directly from a previously recorded scan database.
///
/// Opens the `SQLite` database at `storage_path`, loads the scan root and its
/// children, then presents the interactive file-explorer view.
///
/// Terminal restoration follows the same policy as [`run_scan_ui`].
///
/// # Errors
///
/// Returns [`UiError`] if terminal setup/teardown fails or the event loop
/// encounters an unrecoverable error.
pub async fn run_explore_ui(storage_path: &Path) -> Result<(), UiError> {
    let mut terminal = setup_terminal()?;
    let result = run_explore_ui_inner(&mut terminal, storage_path).await;
    finish_with_restore(&mut terminal, result)
}

/// Restore the terminal and return the inner result, preferring the original
/// error when both the inner operation and restoration fail.
fn finish_with_restore(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    result: Result<(), UiError>,
) -> Result<(), UiError> {
    if let Err(restore_err) = restore_terminal(terminal) {
        return match result {
            Ok(()) => Err(restore_err),
            Err(original) => Err(UiError::WithRestoreFailure {
                original: Box::new(original),
                restore: Box::new(restore_err),
            }),
        };
    }
    result
}

// ---------------------------------------------------------------------------
// Private inner loops
// ---------------------------------------------------------------------------

/// Inner event loop for the scan UI (runs after terminal setup, before teardown).
async fn run_scan_ui_inner(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    config: PipelineConfig,
) -> Result<(), UiError> {
    let cancel = CancellationToken::new();
    let is_root = nix::unistd::geteuid().is_root();

    let (mut progress_rx, completion_rx) = run_pipeline(config, cancel.clone())
        .await
        .map_err(UiError::from)?;

    let mut state = AppState::Scanning(ScanProgressState::new(is_root));

    // Spawn a blocking poller that forwards crossterm events over a channel.
    // Polling with a short timeout lets the task notice cancellation between
    // user interactions without holding up other select! arms.
    let (event_tx, mut event_rx) =
        mpsc::channel::<Result<crossterm::event::Event, std::io::Error>>(32);
    let event_cancel = cancel.clone();
    let _event_task =
        tokio::task::spawn_blocking(move || poll_crossterm_events(&event_tx, &event_cancel));

    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut completion_done = false;
    tokio::pin!(completion_rx);

    loop {
        terminal.draw(|f| {
            let area = f.area();
            match &mut state {
                AppState::Scanning(scan_state) => render_progress(f, scan_state, area),
                AppState::Exploring(explorer_state) => render_explorer(f, explorer_state, area),
            }
        })?;

        tokio::select! {
            biased;

            event = event_rx.recv() => {
                match event {
                    Some(Ok(e)) => {
                        if let AppState::Exploring(ref mut explorer_state) = state {
                            if explorer_state.has_modal_popup() {
                                handle_explorer_event(&e, explorer_state);
                                continue;
                            }
                            if should_quit_event(&e) {
                                cancel.cancel();
                                break;
                            }
                            if handle_explorer_event(&e, explorer_state) {
                                cancel.cancel();
                                break;
                            }
                        } else if should_quit_event(&e) {
                            cancel.cancel();
                            break;
                        }
                    }
                    Some(Err(io_err)) => return Err(UiError::EventStreamIo(io_err)),
                    None if cancel.is_cancelled() => break,
                    None => return Err(UiError::EventStreamEnded),
                }
            }

            Some(progress) = progress_rx.recv() => {
                if let AppState::Scanning(ref mut scan_state) = state {
                    scan_state.update(progress);
                }
            }

            result = &mut completion_rx, if !completion_done => {
                completion_done = true;
                match result {
                    Ok(Ok(pipeline_result)) => {
                        match load_explorer_state(&pipeline_result.storage_path, &pipeline_result.metadata.root, pipeline_result.metadata.warnings) {
                            Ok(explorer_state) => {
                                state = AppState::Exploring(Box::new(explorer_state));
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    Ok(Err(e)) => return Err(e.into()),
                    Err(_) => return Err(PipelineError::ChannelClosed.into()),
                }
            }

            _ = tick.tick() => {} // force redraw for elapsed-time counter

            () = cancel.cancelled() => break,
        }
    }

    Ok(())
}

/// Inner event loop for the explore UI (runs after terminal setup, before teardown).
async fn run_explore_ui_inner(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    storage_path: &Path,
) -> Result<(), UiError> {
    let cancel = CancellationToken::new();

    let (event_tx, mut event_rx) =
        mpsc::channel::<Result<crossterm::event::Event, std::io::Error>>(32);
    let event_cancel = cancel.clone();
    let _event_task =
        tokio::task::spawn_blocking(move || poll_crossterm_events(&event_tx, &event_cancel));

    let mut explorer_state = load_explorer_state(storage_path, storage_path, Vec::new())?;

    loop {
        terminal.draw(|f| {
            render_explorer(f, &mut explorer_state, f.area());
        })?;

        tokio::select! {
            biased;

            event = event_rx.recv() => {
                match event {
                    Some(Ok(e)) => {
                        if explorer_state.has_modal_popup() {
                            handle_explorer_event(&e, &mut explorer_state);
                            continue;
                        }
                        if should_quit_event(&e)
                            || handle_explorer_event(&e, &mut explorer_state)
                        {
                            cancel.cancel();
                            break;
                        }
                    }
                    Some(Err(io_err)) => return Err(UiError::EventStreamIo(io_err)),
                    None if cancel.is_cancelled() => break,
                    None => return Err(UiError::EventStreamEnded),
                }
            }

            () = cancel.cancelled() => break,
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Poll for crossterm events and forward them to `tx` until cancelled or the
/// sender is closed.
///
/// I/O errors from `poll()` or `read()` are sent through the channel so that
/// the async event loop can surface them as [`UiError::EventStreamIo`] instead
/// of the opaque [`UiError::EventStreamEnded`].
///
/// Intended for use with [`tokio::task::spawn_blocking`]: the short poll
/// timeout lets the task notice a cancellation signal between key presses
/// without busy-spinning.
fn poll_crossterm_events(
    tx: &mpsc::Sender<Result<crossterm::event::Event, std::io::Error>>,
    cancel: &CancellationToken,
) {
    while !cancel.is_cancelled() {
        match crossterm::event::poll(Duration::from_millis(50)) {
            Ok(true) => match crossterm::event::read() {
                Ok(event) => {
                    if tx.blocking_send(Ok(event)).is_err() {
                        break;
                    }
                },
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    break;
                },
            },
            Ok(false) => {},
            Err(e) => {
                let _ = tx.blocking_send(Err(e));
                break;
            },
        }
    }
}

/// Return `true` if the crossterm event signals that the user wants to quit.
///
/// Recognised keys: `q`, `Ctrl-C`.
///
/// `Esc` is handled context-sensitively in [`handle_explorer_event`] (drill-up
/// or unfocus when treemap is active) and does not trigger a global quit.
const fn should_quit_event(event: &crossterm::event::Event) -> bool {
    use crossterm::event::{Event as CEvent, KeyCode, KeyModifiers};
    matches!(
        event,
        CEvent::Key(key)
            if matches!(
                (key.code, key.modifiers),
                (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL)
            )
    )
}

/// Handle a crossterm event for the explorer view.
///
/// Dispatches to panel-specific handlers based on [`PanelFocus`]:
/// - [`PanelFocus::Treemap`]: spatial navigation, drill-down, drill-up.
/// - [`PanelFocus::Tree`] / [`PanelFocus::Legend`]: tree navigation + global actions.
///
/// Returns `true` when the caller should quit (e.g. `Esc` in Tree or Legend focus).
/// `Tab` cycles focus forward (Tree → Treemap → Legend → Tree) in all panels.
/// `Esc` in Treemap: drills up when zoomed in, or sets focus back to Tree at root.
/// `Esc` in Tree/Legend: signals quit.
fn handle_explorer_event(event: &crossterm::event::Event, state: &mut ExplorerState) -> bool {
    use crossterm::event::{Event as CEvent, KeyCode, KeyEventKind};

    let CEvent::Key(key) = event else {
        return false;
    };
    if key.kind != KeyEventKind::Press {
        return false;
    }

    state.clear_error();

    // When the warnings popup is open, handle its own navigation first.
    if state.show_warnings() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('w') | KeyCode::Backspace => {
                state.toggle_show_warnings();
            },
            KeyCode::Down | KeyCode::Char('j') => state.scroll_warnings(1),
            KeyCode::Up | KeyCode::Char('k') => state.scroll_warnings(-1),
            KeyCode::PageDown => state.scroll_warnings(10),
            KeyCode::PageUp => state.scroll_warnings(-10),
            _ => {},
        }
        return false;
    }

    let should_quit = match state.focus() {
        PanelFocus::Treemap => handle_treemap_keys(key.code, state),
        PanelFocus::Tree | PanelFocus::Legend => handle_tree_keys(key.code, state),
    };

    if !should_quit {
        state.sync_tree_to_treemap_selection();
        state.sync_treemap_highlight();
    }
    should_quit
}

/// Handle keyboard input when the treemap panel has focus.
///
/// `h`/`←`, `j`/`↓`, `k`/`↑`, `l`/`→` move the spatial selection.
/// `Enter` drills into the selected cell's directory (if it is a directory).
/// `Backspace` zooms out one level.
/// `Tab` cycles focus to the next panel.
/// `Esc` drills up when zoomed in, or sets focus to Tree at the scan root.
///
/// Returns `false` — treemap-panel Esc never quits.
fn handle_treemap_keys(code: crossterm::event::KeyCode, state: &mut ExplorerState) -> bool {
    use crate::ui::widgets::treemap::Direction;
    use crossterm::event::KeyCode;

    // Snapshot the cell layout before mutably borrowing treemap_state.
    let cells = state.treemap_state_mut().layout.cells.clone();

    // Auto-select the first cell on the first keypress after entering treemap focus.
    if state.treemap_state_mut().selected_index.is_none() && !cells.is_empty() {
        state.treemap_state_mut().selected_index = Some(0);
    }

    match code {
        KeyCode::Left | KeyCode::Char('h') => {
            state
                .treemap_state_mut()
                .move_selection(Direction::Left, &cells);
        },
        KeyCode::Right | KeyCode::Char('l') => {
            state
                .treemap_state_mut()
                .move_selection(Direction::Right, &cells);
        },
        KeyCode::Up | KeyCode::Char('k') => {
            state
                .treemap_state_mut()
                .move_selection(Direction::Up, &cells);
        },
        KeyCode::Down | KeyCode::Char('j') => {
            state
                .treemap_state_mut()
                .move_selection(Direction::Down, &cells);
        },
        KeyCode::Enter => {
            let selected_path = state
                .treemap_state_mut()
                .selected_index
                .and_then(|i| cells.get(i))
                .map(|c| c.path.clone());
            if let Some(path) = selected_path {
                state.zoom_into_path(path);
            }
        },
        KeyCode::Backspace => state.zoom_out(),
        KeyCode::Tab => state.cycle_focus(),
        KeyCode::Char('?') => state.toggle_show_help(),
        KeyCode::Char('w') => {
            if !state.warnings().is_empty() {
                state.toggle_show_warnings();
            }
        },
        KeyCode::Esc => {
            if state.treemap_root().is_empty() {
                // At scan root: return focus directly to the Tree panel.
                state.set_focus(PanelFocus::Tree);
            } else {
                // Drilled in: Esc zooms out one level.
                state.zoom_out();
            }
        },
        _ => {},
    }
    false
}

/// Handle keyboard input when the tree or legend panel has focus.
///
/// Preserves all existing tree-navigation keybindings.
/// `Tab` cycles focus forward.
/// `Esc` signals quit (returns `true`).
fn handle_tree_keys(code: crossterm::event::KeyCode, state: &mut ExplorerState) -> bool {
    use crossterm::event::KeyCode;

    match code {
        // Esc quits from tree/legend panels.
        KeyCode::Esc => return true,
        // Navigate tree.
        KeyCode::Up | KeyCode::Char('k') => {
            state.tree_state_mut().key_up();
        },
        KeyCode::Down | KeyCode::Char('j') => {
            state.tree_state_mut().key_down();
        },
        // Go into directory: expand tree node AND zoom treemap.
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => {
            state.tree_state_mut().key_right();
            state.zoom_into_selected();
        },
        // Go up: collapse tree node AND zoom out.
        KeyCode::Left | KeyCode::Char('h' | 'u') | KeyCode::Backspace => {
            let collapsed = !state.tree_state_mut().key_left();
            if collapsed {
                state.zoom_out();
            }
        },
        // Page up/down.
        KeyCode::PageUp => {
            state
                .tree_state_mut()
                .select_relative(|current| current.unwrap_or(0).saturating_sub(10));
        },
        KeyCode::PageDown => {
            state
                .tree_state_mut()
                .select_relative(|current| current.unwrap_or(0).saturating_add(10));
        },
        // Jump to first/last.
        KeyCode::Home | KeyCode::Char('g') => {
            state.tree_state_mut().select_first();
        },
        KeyCode::End | KeyCode::Char('G') => {
            state.tree_state_mut().select_last();
        },
        // Sorting.
        KeyCode::Char('n') => state.set_sort(TreeSortField::Name),
        KeyCode::Char('s') => state.set_sort(TreeSortField::Size),
        KeyCode::Char('m') => state.set_sort(TreeSortField::Modified),
        KeyCode::Char('r') => state.toggle_sort_direction(),
        // Zoom to root (power-user shortcut).
        KeyCode::Char('Z') => state.zoom_to_root(),
        // File info popup.
        KeyCode::Char('i') => state.toggle_show_info(),
        // Help.
        KeyCode::Char('?') => state.toggle_show_help(),
        // Warnings popup.
        KeyCode::Char('w') => {
            if !state.warnings().is_empty() {
                state.toggle_show_warnings();
            }
        },
        // Cycle focus forward.
        KeyCode::Tab => state.cycle_focus(),
        _ => {},
    }
    false
}

/// Build an [`ExplorerState`] by loading all entries and constructing a [`DirNode`] tree.
fn load_explorer_state(
    storage_path: &Path,
    _root_hint: &Path,
    warnings: Vec<crate::types::ScanWarning>,
) -> Result<ExplorerState, UiError> {
    let storage = SqliteStorage::open_readonly(storage_path).map_err(UiError::StorageLoad)?;
    let metadata = storage.load_scan_metadata().map_err(UiError::StorageLoad)?;
    let entries = storage
        .query_entries(&EntryQuery {
            limit: None,
            ..EntryQuery::default()
        })
        .map_err(UiError::StorageLoad)?;
    let tree = build_tree(&entries, &metadata.root);
    let free_space = crate::analyzer::compute_free_space(&metadata.root).ok();
    let mut state = ExplorerState::new(tree, metadata.root);
    state.set_free_space(free_space);
    state.set_warnings(warnings);
    Ok(state)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tree::DirNode;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use std::time::SystemTime;

    fn key_event(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn key_event_with_modifiers(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn make_test_tree() -> DirNode {
        DirNode {
            name: "root".to_owned(),
            size: 350,
            allocated: 350,
            file_count: 3,
            children: vec![
                DirNode {
                    name: "subdir".to_owned(),
                    size: 300,
                    allocated: 300,
                    file_count: 2,
                    children: vec![
                        DirNode {
                            name: "file1.rs".to_owned(),
                            size: 100,
                            allocated: 100,
                            file_count: 1,
                            children: vec![],
                            is_dir: false,
                            extension: Some("rs".to_owned()),
                            mtime: SystemTime::UNIX_EPOCH,
                        },
                        DirNode {
                            name: "file2.py".to_owned(),
                            size: 200,
                            allocated: 200,
                            file_count: 1,
                            children: vec![],
                            is_dir: false,
                            extension: Some("py".to_owned()),
                            mtime: SystemTime::UNIX_EPOCH,
                        },
                    ],
                    is_dir: true,
                    extension: None,
                    mtime: SystemTime::UNIX_EPOCH,
                },
                DirNode {
                    name: "file3.txt".to_owned(),
                    size: 50,
                    allocated: 50,
                    file_count: 1,
                    children: vec![],
                    is_dir: false,
                    extension: Some("txt".to_owned()),
                    mtime: SystemTime::UNIX_EPOCH,
                },
            ],
            is_dir: true,
            extension: None,
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    fn make_explorer_state() -> ExplorerState {
        ExplorerState::new(make_test_tree(), std::path::PathBuf::from("/test"))
    }

    // --- should_quit_event ---

    #[test]
    fn quit_on_q() {
        assert!(should_quit_event(&key_event(KeyCode::Char('q'))));
    }

    #[test]
    fn no_quit_on_escape() {
        // Esc is handled context-sensitively in handle_explorer_event (drill-up or
        // unfocus treemap) and no longer triggers a global quit.
        assert!(!should_quit_event(&key_event(KeyCode::Esc)));
    }

    #[test]
    fn quit_on_ctrl_c() {
        assert!(should_quit_event(&key_event_with_modifiers(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn no_quit_on_regular_key() {
        assert!(!should_quit_event(&key_event(KeyCode::Char('a'))));
    }

    #[test]
    fn no_quit_on_resize_event() {
        assert!(!should_quit_event(&Event::Resize(80, 24)));
    }

    // --- handle_explorer_event ---

    #[test]
    fn handle_event_sort_by_name() {
        let mut state = make_explorer_state();
        handle_explorer_event(&key_event(KeyCode::Char('n')), &mut state);
        assert_eq!(state.sort_field(), TreeSortField::Name);
    }

    #[test]
    fn handle_event_sort_by_size() {
        let mut state = make_explorer_state();
        state.set_sort(TreeSortField::Name);
        handle_explorer_event(&key_event(KeyCode::Char('s')), &mut state);
        assert_eq!(state.sort_field(), TreeSortField::Size);
    }

    #[test]
    fn handle_event_toggle_help() {
        let mut state = make_explorer_state();
        assert!(!state.show_help());
        handle_explorer_event(&key_event(KeyCode::Char('?')), &mut state);
        assert!(state.show_help());
    }

    #[test]
    fn handle_event_toggle_info() {
        let mut state = make_explorer_state();
        assert!(!state.show_info());
        handle_explorer_event(&key_event(KeyCode::Char('i')), &mut state);
        assert!(state.show_info());
    }

    #[test]
    fn tab_cycles_focus_forward() {
        use crate::ui::app::PanelFocus;
        let mut state = make_explorer_state();
        assert_eq!(state.focus(), PanelFocus::Tree);
        handle_explorer_event(&key_event(KeyCode::Tab), &mut state);
        assert_eq!(state.focus(), PanelFocus::Treemap);
        handle_explorer_event(&key_event(KeyCode::Tab), &mut state);
        assert_eq!(state.focus(), PanelFocus::Legend);
        handle_explorer_event(&key_event(KeyCode::Tab), &mut state);
        assert_eq!(state.focus(), PanelFocus::Tree);
    }

    #[test]
    fn treemap_esc_at_root_returns_to_tree() {
        use crate::ui::app::PanelFocus;
        let mut state = make_explorer_state();
        // Move focus to Treemap.
        handle_explorer_event(&key_event(KeyCode::Tab), &mut state);
        assert_eq!(state.focus(), PanelFocus::Treemap);
        // Esc at root → set focus directly back to Tree (not Legend).
        let quit = handle_explorer_event(&key_event(KeyCode::Esc), &mut state);
        assert!(!quit, "Esc in treemap at root should not quit");
        assert_eq!(state.focus(), PanelFocus::Tree);
    }

    #[test]
    fn treemap_backspace_zooms_out() {
        use crate::ui::app::PanelFocus;
        let mut state = make_explorer_state();
        // Zoom in first (via tree).
        state.tree_state_mut().select(vec!["subdir".to_owned()]);
        state.zoom_into_selected();
        assert_eq!(state.treemap_root(), &["subdir"]);
        // Switch to treemap focus.
        handle_explorer_event(&key_event(KeyCode::Tab), &mut state);
        assert_eq!(state.focus(), PanelFocus::Treemap);
        // Backspace should zoom out.
        handle_explorer_event(&key_event(KeyCode::Backspace), &mut state);
        assert_eq!(state.treemap_root(), &[] as &[String]);
    }

    #[test]
    fn treemap_esc_drilled_in_zooms_out() {
        use crate::ui::app::PanelFocus;
        let mut state = make_explorer_state();
        // Zoom in via tree.
        state.tree_state_mut().select(vec!["subdir".to_owned()]);
        state.zoom_into_selected();
        assert_eq!(state.treemap_root(), &["subdir"]);
        // Switch to treemap focus.
        handle_explorer_event(&key_event(KeyCode::Tab), &mut state);
        assert_eq!(state.focus(), PanelFocus::Treemap);
        // Esc while drilled in → zoom out (not cycle focus, not quit).
        let quit = handle_explorer_event(&key_event(KeyCode::Esc), &mut state);
        assert!(!quit, "Esc in treemap drilled-in should not quit");
        assert_eq!(state.treemap_root(), &[] as &[String]);
        assert_eq!(state.focus(), PanelFocus::Treemap); // still focused on treemap
    }

    #[test]
    fn esc_in_tree_panel_signals_quit() {
        let mut state = make_explorer_state();
        let quit = handle_explorer_event(&key_event(KeyCode::Esc), &mut state);
        assert!(quit, "Esc in tree panel should signal quit");
    }

    #[test]
    fn esc_in_legend_panel_signals_quit() {
        use crate::ui::app::PanelFocus;
        let mut state = make_explorer_state();
        // Tab twice to reach Legend.
        handle_explorer_event(&key_event(KeyCode::Tab), &mut state);
        handle_explorer_event(&key_event(KeyCode::Tab), &mut state);
        assert_eq!(state.focus(), PanelFocus::Legend);
        let quit = handle_explorer_event(&key_event(KeyCode::Esc), &mut state);
        assert!(quit, "Esc in legend panel should signal quit");
    }
}
