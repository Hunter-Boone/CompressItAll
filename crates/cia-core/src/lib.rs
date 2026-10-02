#![forbid(unsafe_code)]
//! Smidge engine core. Everything here is shared by the native engine and the
//! WASM engine: the job model, preset data, byte budgets, per-message
//! allocation, output naming, the free allowance and the user-facing copy.
//! No codecs live here (see DESIGN.md section 3).

pub mod allocation;
pub mod allowance;
pub mod copy;
pub mod events;
pub mod format;
pub mod limits;
pub mod model;
pub mod naming;
pub mod presets;
pub mod zip_overhead;

pub use model::*;

/// Generate a new ULID string for job/item/artifact ids.
pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("os randomness");
    ulid::Ulid::from_bytes(bytes).to_string()
}
