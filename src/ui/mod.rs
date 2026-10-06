//! TUI application shell.
//!
//! Provides terminal setup/teardown, panic recovery, and the two top-level
//! entry points for the TUI:
//! - [`run_scan_ui`] — start a scan, display the progress view, then
//!   transition to the explorer when the scan completes.
//! - [`run_explore_ui`] — open an existing scan database and display the
//!   file-explorer view directly.

pub mod app;
pub mod views;
pub mod widgets;

use std::{
    io::Stdout,
    path::{Path, PathBuf},
    time::Duration,
};

use crossterm::{
    ExecutableCommand as _,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    error::UiError,
    pipeline::{PipelineConfig, run_pipeline},
    types::{SortDirection, SortField},
};
use app::{AppState, ExplorerState, ScanProgressState, TreemapState};
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
/// Terminal setup and teardown are handled automatically; the terminal is
/// restored even if an error occurs inside the event loop.
///
/// # Errors
///
/// Returns [`UiError`] if terminal setup/teardown fails or the event loop
/// encounters an unrecoverable error.
pub async fn run_scan_ui(config: PipelineConfig) -> Result<(), UiError> {
    let mut terminal = setup_terminal()?;
    let result = run_scan_ui_inner(&mut terminal, config).await;
    restore_terminal(&mut terminal)?;
    result
}

/// Run the explorer UI directly from a previously recorded scan database.
///
/// Opens the `SQLite` database at `storage_path`, then presents the interactive
/// file-explorer view.  The actual explorer rendering is implemented in Task 8;
/// for now a placeholder message is shown.
///
/// # Errors
///
/// Returns [`UiError`] if terminal setup/teardown fails or the event loop
/// encounters an unrecoverable error.
pub async fn run_explore_ui(_storage_path: &Path) -> Result<(), UiError> {
    let mut terminal = setup_terminal()?;
    let result = run_explore_ui_inner(&mut terminal).await;
    restore_terminal(&mut terminal)?;
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
        .map_err(|e| UiError::Crossterm(e.to_string()))?;

    let mut state = AppState::Scanning(ScanProgressState {
        file_count: 0,
        files_per_sec: 0.0,
        elapsed: Duration::ZERO,
        current_path: PathBuf::new(),
        is_root,
    });

    // Spawn a blocking poller that forwards crossterm events over a channel.
    // Polling with a short timeout lets the task notice cancellation between
    // user interactions without holding up other select! arms.
    let (event_tx, mut event_rx) = mpsc::channel::<crossterm::event::Event>(32);
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
                    Some(e) => {
                        if should_quit_event(&e) {
                            cancel.cancel();
                            break;
                        }
                        if let AppState::Exploring(ref mut explorer_state) = state {
                            handle_explorer_event(&e, explorer_state);
                        }
                    }
                    None => break, // event poller exited unexpectedly
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
                        state = AppState::Exploring(placeholder_explorer(
                            &pipeline_result.storage_path,
                        ));
                    }
                    Ok(Err(e)) => return Err(UiError::Crossterm(e.to_string())),
                    Err(_) => {} // sender dropped without sending — pipeline cancelled
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
) -> Result<(), UiError> {
    let cancel = CancellationToken::new();

    let (event_tx, mut event_rx) = mpsc::channel::<crossterm::event::Event>(32);
    let event_cancel = cancel.clone();
    let _event_task =
        tokio::task::spawn_blocking(move || poll_crossterm_events(&event_tx, &event_cancel));

    // Placeholder explorer state — in production this would be loaded from the
    // SQLite database passed to `run_explore_ui`.
    let mut explorer_state = ExplorerState {
        current_path: PathBuf::from("/"),
        breadcrumb: Vec::new(),
        entries: Vec::new(),
        type_stats: Vec::new(),
        selected_index: 0,
        sort_field: SortField::Size,
        sort_direction: SortDirection::Descending,
        treemap_state: TreemapState::default(),
    };

    loop {
        terminal.draw(|f| {
            render_explorer(f, &mut explorer_state, f.area());
        })?;

        tokio::select! {
            biased;

            event = event_rx.recv() => {
                match event {
                    Some(e) => {
                        if should_quit_event(&e) {
                            cancel.cancel();
                            break;
                        }
                        handle_explorer_event(&e, &mut explorer_state);
                    }
                    None => break, // event poller exited unexpectedly
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
/// Intended for use with [`tokio::task::spawn_blocking`]: the short poll
/// timeout lets the task notice a cancellation signal between key presses
/// without busy-spinning.
fn poll_crossterm_events(tx: &mpsc::Sender<crossterm::event::Event>, cancel: &CancellationToken) {
    while !cancel.is_cancelled() {
        match crossterm::event::poll(Duration::from_millis(50)) {
            Ok(true) => {
                let Ok(event) = crossterm::event::read() else {
                    break;
                };
                if tx.blocking_send(event).is_err() {
                    break;
                }
            },
            Ok(false) => {},
            Err(_) => break,
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

/// Handle a crossterm event for the explorer view, mutating `state` accordingly.
///
/// Key bindings:
/// - `↑` / `↓` — move the selection cursor
/// - `Enter` — navigate into the selected directory (no-op if not a directory)
/// - `Backspace` — navigate up to the parent directory
/// - `Tab` — cycle sort field forward
/// - `Shift+Tab` — cycle sort field (same as Tab; reverse is via `r`)
/// - `r` — reverse sort direction
fn handle_explorer_event(event: &crossterm::event::Event, state: &mut ExplorerState) {
    use crossterm::event::{Event as CEvent, KeyCode, KeyEventKind};

    let CEvent::Key(key) = event else { return };
    // Only handle key-press events (ignore key-repeat / key-release on platforms that emit them).
    if key.kind != KeyEventKind::Press {
        return;
    }

    match key.code {
        KeyCode::Up => state.select_prev(),
        KeyCode::Down => state.select_next(),
        KeyCode::Tab | KeyCode::BackTab => state.cycle_sort(),
        KeyCode::Char('r') => state.reverse_sort(),
        // Navigation into a directory requires storage access; the event loop
        // would need the storage handle. Callers with storage access should
        // call navigate_into / navigate_up directly on ExplorerState.
        _ => {},
    }
}

/// Build a minimal placeholder [`ExplorerState`] after pipeline completion.
///
/// Task 8 will replace this with a proper constructor that loads data from
/// the scan database and populates all fields.
fn placeholder_explorer(storage_path: &Path) -> ExplorerState {
    let current_path = storage_path
        .parent()
        .map_or_else(|| storage_path.to_path_buf(), Path::to_path_buf);
    ExplorerState {
        current_path,
        breadcrumb: Vec::new(),
        entries: Vec::new(),
        type_stats: Vec::new(),
        selected_index: 0,
        sort_field: SortField::Size,
        sort_direction: SortDirection::Descending,
        treemap_state: TreemapState::default(),
    }
}
