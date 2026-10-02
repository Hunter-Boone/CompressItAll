// Dev builds keep the console on Windows; release builds hide it.
#![cfg_attr(
    all(not(debug_assertions), not(feature = "dev-build")),
    windows_subsystem = "windows"
)]

fn main() {
    cia_desktop::run()
}
