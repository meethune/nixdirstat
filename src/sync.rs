//! Synchronization primitives for cooperative task control.

use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Thread-safe pause/resume control for the scan pipeline.
///
/// The scanner calls [`wait_if_paused`](Self::wait_if_paused) at each batch
/// boundary; the UI calls [`pause`](Self::pause) / [`resume`](Self::resume).
#[derive(Debug)]
pub struct PauseToken {
    paused: AtomicBool,
    condvar: Condvar,
    mutex: Mutex<()>,
}

impl PauseToken {
    /// Create a new token in the unpaused state.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            paused: AtomicBool::new(false),
            condvar: Condvar::new(),
            mutex: Mutex::new(()),
        })
    }

    /// Signal the scanner to pause at the next check point.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    /// Resume a paused scanner.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        self.condvar.notify_all();
    }

    /// Whether the token is currently in the paused state.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Block the calling thread while paused. Returns immediately if not paused.
    pub fn wait_if_paused(&self) {
        if !self.paused.load(Ordering::SeqCst) {
            return;
        }
        let mut guard = self
            .mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while self.paused.load(Ordering::SeqCst) {
            guard = self
                .condvar
                .wait(guard)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        drop(guard);
    }
}

impl Default for PauseToken {
    fn default() -> Self {
        Self {
            paused: AtomicBool::new(false),
            condvar: Condvar::new(),
            mutex: Mutex::new(()),
        }
    }
}
