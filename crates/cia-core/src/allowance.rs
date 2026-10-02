//! The free allowance: 3 `Fitted` files per rolling 24 hours (DESIGN.md 5.2).
//! Pure state transitions; hosts persist `AllowanceState` as JSON.

use serde::{Deserialize, Serialize};

pub const FREE_FILES_PER_DAY: usize = 3;
pub const WINDOW_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowanceState {
    /// Unix ms of each use still inside the window.
    pub uses: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowanceView {
    pub remaining: usize,
    /// When the next slot frees, if none are free now.
    pub next_free_at_ms: Option<u64>,
}

impl AllowanceState {
    fn prune(&mut self, now_ms: u64) {
        self.uses
            .retain(|&t| t + WINDOW_MS > now_ms && t <= now_ms + 60_000);
        self.uses.sort_unstable();
    }

    pub fn view(&mut self, now_ms: u64) -> AllowanceView {
        self.prune(now_ms);
        let remaining = FREE_FILES_PER_DAY.saturating_sub(self.uses.len());
        let next_free_at_ms = if remaining == 0 {
            self.uses.first().map(|t| t + WINDOW_MS)
        } else {
            None
        };
        AllowanceView {
            remaining,
            next_free_at_ms,
        }
    }

    /// Record one `Fitted` outcome. Returns false if no slot was free (caller should have checked).
    pub fn consume(&mut self, now_ms: u64) -> bool {
        self.prune(now_ms);
        if self.uses.len() >= FREE_FILES_PER_DAY {
            return false;
        }
        self.uses.push(now_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rolling_window() {
        let mut s = AllowanceState::default();
        let t0 = 1_000_000_000_000u64;
        assert_eq!(s.view(t0).remaining, 3);
        assert!(s.consume(t0));
        assert!(s.consume(t0 + 1000));
        assert!(s.consume(t0 + 2000));
        assert!(!s.consume(t0 + 3000));
        let v = s.view(t0 + 3000);
        assert_eq!(v.remaining, 0);
        assert_eq!(v.next_free_at_ms, Some(t0 + WINDOW_MS));
        // first slot frees exactly 24 h after it was used
        assert_eq!(s.view(t0 + WINDOW_MS - 1).remaining, 0);
        assert_eq!(s.view(t0 + WINDOW_MS).remaining, 1);
        assert_eq!(s.view(t0 + WINDOW_MS + 2000).remaining, 3);
    }
    #[test]
    fn clock_rollback_drops_future_uses() {
        let mut s = AllowanceState {
            uses: vec![5_000_000_000_000],
        };
        assert_eq!(s.view(1_000_000_000_000).remaining, 3);
    }
}
