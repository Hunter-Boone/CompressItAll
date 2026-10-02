#![forbid(unsafe_code)]
//! Smidge licensing, shared by the desktop app, the WASM engine and the tests
//! that keep the TypeScript copies (`packages/api-types`) in step.
//!
//! - [`key`]: product key format, generation and checksum (DESIGN.md 5.4)
//! - [`token`]: `SMG1.<payload>.<sig>` Ed25519 license tokens (DESIGN.md 5.5)
//! - [`keys`]: the public keys the app trusts, by kid
//! - [`validate`]: offline validation, identical on desktop and web
//! - [`device`] (feature `native`): device hash and name
//!
//! The `sign` feature adds [`token::sign`]; the `keygen` feature adds the
//! `cia-keygen` binary. Neither is compiled into the shipped apps.

pub mod key;
pub mod keys;
pub mod token;
pub mod validate;

#[cfg(feature = "native")]
pub mod device;

pub use token::{Kind, Payload, Plan, TokenError};
pub use validate::{validate, validate_with_keys, LicenseState, LicenseStatus};
