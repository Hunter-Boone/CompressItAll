//! Job log (DESIGN.md 3.12): one JSON document per job with the job, plan,
//! every attempt, every outcome, capabilities and timings.

use cia_core::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct JobLog {
    pub job_id: String,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub capabilities: Capabilities,
    pub job: Job,
    pub plan: Option<Plan>,
    pub attempts: Vec<Attempt>,
    pub summary: Option<JobSummary>,
    pub warnings: Vec<String>,
}

impl JobLog {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }
}
