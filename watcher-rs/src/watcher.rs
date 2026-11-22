//! The watcher process: subscriptions, readings and the status file.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::engine::Engine;
use crate::luna::{self, Feed};

const TMP_DIR: &str = "/tmp/arc-restore";
const LOG_PATH: &str = "/tmp/arc-restore/watcher.log";
/// Creating this file (or one named `debug` in the state dir) turns on
/// detailed logging at the next start.
const DEBUG_FLAG: &str = "/tmp/arc-restore/debug";
const LOG_LIMIT: u64 = 256 * 1024;

/// Polling when no subscription reports the Sound Out (watch.sh's interval).
const POLL_FALLBACK: Duration = Duration::from_secs(2);
/// Safety-net re-check while subscriptions work, with and without Bluetooth.
const POLL_BLUETOOTH: Duration = Duration::from_secs(10);
const POLL_IDLE: Duration = Duration::from_secs(60);
/// Lets the settings and audio services settle after a change event.
const DEBOUNCE: Duration = Duration::from_millis(300);

pub enum Msg {
    /// A subscription reported a new `soundOutput`.
    Changed(Feed),
    /// A subscription started answering (true) or ended (false).
    Feed(Feed, bool),
}

static DEBUG: OnceLock<bool> = OnceLock::new();

pub fn log(message: &str) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    if std::fs::metadata(LOG_PATH).is_ok_and(|m| m.len() > LOG_LIMIT) {
        let _ = std::fs::rename(LOG_PATH, format!("{}.1", LOG_PATH));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(LOG_PATH) {
        let _ = writeln!(file, "{}.{:03} {}", now.as_secs(), now.subsec_millis(), message);
    }
}

pub fn debug(message: &str) {
    if *DEBUG.get().unwrap_or(&false) {
        log(message);
    }
}

struct Watcher {
    state: PathBuf,
    hook: PathBuf,
    exe: Option<PathBuf>,
    engine: Engine,
    /// Feeds that have reported a reply since they last (re)started.
    feeds: [bool; 2],
    last_status: String,
}

fn feed_index(feed: Feed) -> usize {
    match feed {
        Feed::Settings => 0,
        Feed::Volume => 1,
    }
}

impl Watcher {
    /// Same test as watch.sh's `is_enabled`: the boot hook is executable and
    /// the watcher itself is still installed.
    fn enabled(&self) -> bool {
        let hook_ok = std::fs::metadata(&self.hook).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        let exe_ok = self.exe.as_deref().map_or(true, |p| p.exists());
        hook_ok && exe_ok
    }

    fn events(&self) -> &'static str {
        match self.feeds {
            [true, true] => "settings+volume",
            [true, false] => "settings",
            [false, true] => "volume",
            [false, false] => "polling",
        }
    }

    fn poll_interval(&self) -> Duration {
        if self.feeds == [false, false] {
            POLL_FALLBACK
        } else if self.engine.bluetooth_active() {
            POLL_BLUETOOTH
        } else {
            POLL_IDLE
        }
    }

    /// Takes a fresh pair of readings and acts on them, like one pass of
    /// watch.sh's loop.
    fn check(&mut self) {
        let configured = luna::configured_output();
        let live = luna::live_output();
        debug(&format!("readings: configured={:?} live={:?}", configured, live));
        let before = self.engine.detail;
        if let Some(previous) = self.engine.observe(configured, live, Instant::now()) {
            if self.enabled() {
                let accepted = luna::set_output(&previous);
                log(&format!(
                    "Bluetooth gone, restoring {}: {}",
                    previous,
                    if accepted { "accepted" } else { "refused" }
                ));
                self.engine.restore_sent(accepted);
            }
        }
        if self.engine.detail != before {
            log(&format!("state {}", self.engine.detail.as_str()));
        }
        self.publish();
    }

    /// Writes the status file control.sh prints, only when it changed.
    fn publish(&mut self) {
        let e = &self.engine;
        let status = format!(
            "route={}\nconfigured={}\nprevious={}\ndetail={}\nwatcher=native\nevents={}",
            e.live.as_deref().unwrap_or(""),
            e.configured.as_deref().unwrap_or(""),
            e.previous.as_deref().unwrap_or(""),
            e.detail.as_str(),
            self.events(),
        );
        if status == self.last_status {
            return;
        }
        let tmp = self.state.join(format!("status.{}", std::process::id()));
        let ok = std::fs::write(&tmp, format!("{}\n", status)).is_ok()
            && std::fs::rename(&tmp, self.state.join("status")).is_ok();
        if ok {
            self.last_status = status;
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version") {
        println!("arc-restore-watcher {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.len() < 3 {
        eprintln!("usage: arc-restore-watcher <state dir> <boot hook>");
        std::process::exit(2);
    }
    let state = PathBuf::from(&args[1]);
    let hook = PathBuf::from(&args[2]);
    let _ = std::fs::create_dir_all(TMP_DIR);
    let _ = DEBUG.set(
        Path::new(DEBUG_FLAG).exists()
            || state.join("debug").exists()
            || std::env::var("ARC_RESTORE_DEBUG").as_deref() == Ok("1"),
    );

    let mut w = Watcher {
        exe: std::env::current_exe().ok(),
        state,
        hook,
        engine: Engine::default(),
        feeds: [false, false],
        last_status: String::new(),
    };
    log(&format!("arc-restore-watcher {} starting", env!("CARGO_PKG_VERSION")));

    let (tx, rx) = channel();
    for feed in [Feed::Settings, Feed::Volume] {
        let tx = tx.clone();
        std::thread::spawn(move || luna::follow(feed, tx));
    }
    drop(tx);

    let mut check_at = Some(Instant::now());
    let mut next_poll = Instant::now();
    while w.enabled() {
        let now = Instant::now();
        let due = earliest(earliest(check_at, w.engine.next_deadline()), Some(next_poll)).unwrap();
        if due <= now {
            check_at = None;
            w.check();
            next_poll = Instant::now() + w.poll_interval();
            continue;
        }
        match rx.recv_timeout(due - now) {
            Ok(Msg::Changed(feed)) => {
                debug(&format!("{} changed", feed.name()));
                check_at = earliest(check_at, Some(Instant::now() + DEBOUNCE));
            }
            Ok(Msg::Feed(feed, alive)) => {
                let slot = &mut w.feeds[feed_index(feed)];
                if *slot != alive {
                    *slot = alive;
                    log(&format!("{} subscription {}; events: {}", feed.name(), if alive { "active" } else { "lost" }, w.events()));
                    // Anything may have changed while a feed was down.
                    check_at = earliest(check_at, Some(Instant::now()));
                    next_poll = next_poll.min(Instant::now() + w.poll_interval());
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                // Both feed threads are gone; carry on by polling alone.
                w.feeds = [false, false];
                std::thread::sleep(POLL_FALLBACK);
                check_at = Some(Instant::now());
            }
        }
    }
    log("boot hook removed; stopping");
    let _ = std::fs::remove_file(w.state.join("status"));
}
