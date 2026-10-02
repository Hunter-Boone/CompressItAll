//! `cargo xtask keygen` / `cargo run -p cia-license --features keygen --bin cia-keygen`
//!
//! Prints a fresh Ed25519 pair for license signing: the seed for the server's
//! `LICENSE_SIGNING_KEY` and the public key entry for `crates/cia-license/src/keys.rs`.
//! The seed is printed once and never written anywhere.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

fn main() {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).expect("OS randomness");
    let public = cia_license::token::public_key(&seed);

    let hex: String = public.iter().map(|b| format!("{b:02x}")).collect();
    let literal = public
        .iter()
        .map(|b| format!("0x{b:02x}"))
        .collect::<Vec<_>>()
        .join(", ");

    println!("# New Ed25519 license signing key. Keep the seed secret; it is not stored anywhere.");
    println!("LICENSE_SIGNING_KEY={}", STANDARD.encode(seed));
    println!("LICENSE_SIGNING_KID=<kid, e.g. 2026-10>");
    println!();
    println!("public key (hex): {hex}");
    println!();
    println!("# crates/cia-license/src/keys.rs entry:");
    println!("    (\"<kid>\", [{literal}]),");
}
