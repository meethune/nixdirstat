//! Filesystem change watcher using the `notify` crate.
//!
//! Watches the scan root for filesystem changes and sends a debounced signal
//! when any modification is detected. The signal is a unit value — the watcher
//! collapses multiple rapid changes into a single notification.

use std::{path::Path, time::Duration};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

const DEBOUNCE_DURATION: Duration = Duration::from_secs(2);

/// Start watching a directory tree for filesystem changes.
///
/// Returns a channel receiver that emits `()` when changes are detected
/// (debounced to avoid flooding on rapid modifications) and the watcher
/// handle that must be kept alive.
///
/// # Errors
///
/// Returns `notify::Error` if the watcher cannot be created or the path
/// cannot be watched.
pub fn start_watcher(
    root: &Path,
) -> Result<(mpsc::Receiver<()>, RecommendedWatcher), notify::Error> {
    let (signal_tx, signal_rx) = mpsc::channel(1);

    let debounce_tx = signal_tx.clone();
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let mut watcher = RecommendedWatcher::new(event_tx, notify::Config::default())?;
    watcher.watch(root, RecursiveMode::Recursive)?;

    std::thread::spawn(move || {
        debounce_loop(event_rx, debounce_tx);
    });

    drop(signal_tx);
    Ok((signal_rx, watcher))
}

// Ownership required: called from std::thread::spawn(move || ...).
#[allow(clippy::needless_pass_by_value)]
fn debounce_loop(
    rx: std::sync::mpsc::Receiver<Result<notify::Event, notify::Error>>,
    tx: mpsc::Sender<()>,
) {
    loop {
        match rx.recv() {
            Ok(Ok(_event)) => {
                drain_pending(&rx);
                std::thread::sleep(DEBOUNCE_DURATION);
                drain_pending(&rx);
                if tx.blocking_send(()).is_err() {
                    return;
                }
            },
            Ok(Err(_)) | Err(_) => return,
        }
    }
}

fn drain_pending(rx: &std::sync::mpsc::Receiver<Result<notify::Event, notify::Error>>) {
    while rx.try_recv().is_ok() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn watcher_detects_file_creation() {
        let dir = TempDir::new().unwrap();
        let (mut rx, _watcher) = start_watcher(dir.path()).unwrap();

        std::fs::write(dir.path().join("new.txt"), "hello").unwrap();

        let result = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        assert!(
            result.is_ok(),
            "expected change notification within timeout"
        );
    }
}
