//! A clonable cancel token shared between the UI thread and the runner.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Set once from any thread; the runner polls it and kills the process tree.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
