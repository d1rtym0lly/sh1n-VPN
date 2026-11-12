#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

#[cfg(windows)]
mod core;
mod profile;
#[cfg(windows)]
mod store;
#[cfg(windows)]
mod subscription;
#[cfg(windows)]
mod theme;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--check-core") => std::process::exit(match core::diagnostic() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("{e}");
                1
            }
        }),
        Some("--smoke-proxy") => std::process::exit(match core::smoke() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("Smoke test failed: {e}");
                1
            }
        }),
        Some("--import-clipboard") => {
            std::process::exit(ui::import_cli(std::env::args().nth(2).as_deref()))
        }
        Some("--showcase") => {
            ui::run(std::env::args().nth(2).as_deref().or(Some("home")));
            return;
        }
        _ => {}
    }
    ui::run(None);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("sh1n VPN: GUI is currently available on Windows only.");
}
