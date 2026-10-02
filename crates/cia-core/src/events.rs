//! Engine events (DESIGN.md 3.12). Desktop emits them on the Tauri event
//! `engine://event`; web workers post them to the main thread.

use crate::model::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum EngineEvent {
    JobState {
        job_id: JobId,
        state: JobState,
    },
    ItemState {
        job_id: JobId,
        item_id: ItemId,
        state: ItemState,
    },
    Progress {
        job_id: JobId,
        item_id: ItemId,
        fraction: f32,
        eta_ms: Option<u64>,
        label: String,
    },
    Attempt {
        job_id: JobId,
        attempt: Attempt,
    },
    ItemDone {
        job_id: JobId,
        item_id: ItemId,
        outcome: ItemOutcome,
    },
    JobDone {
        job_id: JobId,
        summary: JobSummary,
    },
    Warning {
        job_id: JobId,
        item_id: Option<ItemId>,
        message: String,
    },
}

/// A sink for events. Hosts implement it; the engine never knows about Tauri or workers.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: EngineEvent);
}

/// Collects events in memory (tests and the CLI).
#[derive(Default)]
pub struct VecSink(pub std::sync::Mutex<Vec<EngineEvent>>);

impl EventSink for VecSink {
    fn emit(&self, event: EngineEvent) {
        self.0.lock().unwrap().push(event);
    }
}
