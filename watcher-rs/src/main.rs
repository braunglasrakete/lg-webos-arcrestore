//! ARC Restore watcher for rooted LG webOS TVs.
//!
//! Returns the TV to its previous Sound Out when disconnecting a Bluetooth
//! speaker drops audio back to the TV speakers. It replaces the polling loop
//! of `app/scripts/watch.sh`: two Luna subscriptions wake it when the Sound
//! Out setting or the live audio route changes, and it then takes the same
//! fresh pair of readings watch.sh took every 2 s and applies the same rules.
//! If the subscriptions cannot run it falls back to watch.sh's 2 s polling.
//!
//! Usage: arc-restore-watcher <state dir> <boot hook>
//! It exits when the boot hook is removed (Disable in the app).

mod engine;
mod json;
#[cfg(unix)]
mod luna;
#[cfg(unix)]
mod watcher;

#[cfg(unix)]
pub use watcher::{debug, log, Msg};

fn main() {
    #[cfg(unix)]
    watcher::run();
    #[cfg(not(unix))]
    {
        eprintln!("arc-restore-watcher runs on webOS (Linux) only; use `cargo test` here.");
        std::process::exit(1);
    }
}
