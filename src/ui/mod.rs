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
use app::{AppState, ExplorerState, ScanProgressState, TreeSortField};
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
                        if should_quit_event(&e) {
                            cancel.cancel();
                            break;
                        }
                        if let AppState::Exploring(ref mut explorer_state) = state {
                            handle_explorer_event(&e, explorer_state);
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
                        match load_explorer_state(&pipeline_result.storage_path, &pipeline_result.metadata.root) {
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

    let mut explorer_state = load_explorer_state(storage_path, storage_path)?;

    loop {
        terminal.draw(|f| {
            render_explorer(f, &mut explorer_state, f.area());
        })?;

        tokio::select! {
            biased;

            event = event_rx.recv() => {
                match event {
                    Some(Ok(e)) => {
                        if should_quit_event(&e) {
                            cancel.cancel();
                            break;
                        }
                        handle_explorer_event(&e, &mut explorer_state);
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
/// Recognised keys: `q`, `Escape`, `Ctrl-C`.
const fn should_quit_event(event: &crossterm::event::Event) -> bool {
    use crossterm::event::{Event as CEvent, KeyCode, KeyModifiers};
    matches!(
        event,
        CEvent::Key(key)
            if matches!(
                (key.code, key.modifiers),
                (KeyCode::Char('q') | KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL)
            )
    )
}

/// Handle a crossterm event for the explorer view.
///
/// Navigation model: Enter/Right = expand tree AND zoom treemap (unified
/// "go into directory"). Left/Backspace/u = collapse AND zoom out.
/// The legend panel is always visible but non-interactive.
fn handle_explorer_event(event: &crossterm::event::Event, state: &mut ExplorerState) {
    use crossterm::event::{Event as CEvent, KeyCode, KeyEventKind};

    let CEvent::Key(key) = event else { return };
    if key.kind != KeyEventKind::Press {
        return;
    }

    state.clear_error();

    match key.code {
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
        _ => {},
    }

    state.sync_treemap_highlight();
}

/// Build an [`ExplorerState`] by loading all entries and constructing a [`DirNode`] tree.
fn load_explorer_state(storage_path: &Path, _root_hint: &Path) -> Result<ExplorerState, UiError> {
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
    fn quit_on_escape() {
        assert!(should_quit_event(&key_event(KeyCode::Esc)));
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
}
