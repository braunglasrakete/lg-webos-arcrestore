//! Talks to webOS services through `luna-send`.
//!
//! One-shot calls run `luna-send -n 1` directly, as watch.sh did. The
//! long-lived subscriptions run under `script` so `luna-send` gets a terminal
//! and flushes each reply line; this follows the approach of the MIT-licensed
//! Webos-EARC-Volume-Overlay watcher (Copyright (c) 2026 eARC Volume Overlay
//! contributors). The JSON helpers in json.rs come from the same project.

use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::json::{bool_field, sound_output};
use crate::{debug, log, Msg};

const LUNA_SEND: &str = "/usr/bin/luna-send";
const SCRIPT: &str = "/usr/bin/script";

pub const SETTINGS: &str = "luna://com.webos.settingsservice";
pub const AUDIO: &str = "luna://com.webos.service.audio";

/// Children die with the thread that started them instead of being orphaned.
fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    cmd
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// One call, one reply. Returns the reply text, or None on timeout/failure.
pub fn call(uri: &str, payload: &str) -> Option<String> {
    let mut child = command(LUNA_SEND)
        .args(["-w", "2500", "-n", "1", uri, payload])
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    wait_with_timeout(&mut child, Duration::from_millis(4000));
    let text = reader.join().ok()?;
    debug(&format!("{} -> {}", uri, text.trim()));
    (!text.trim().is_empty()).then_some(text)
}

pub fn configured_output() -> Option<String> {
    call(
        &format!("{}/getSystemSettings", SETTINGS),
        r#"{"category":"sound","keys":["soundOutput"]}"#,
    )
    .and_then(|t| sound_output(&t))
}

pub fn live_output() -> Option<String> {
    call(&format!("{}/getSoundOutput", AUDIO), "{}").and_then(|t| sound_output(&t))
}

/// Asks the settings service for `output`. True if it answered returnValue:true.
pub fn set_output(output: &str) -> bool {
    let payload = format!(r#"{{"category":"sound","settings":{{"soundOutput":"{}"}}}}"#, output);
    call(&format!("{}/setSystemSettings", SETTINGS), &payload)
        .and_then(|t| bool_field(&t, "returnValue"))
        .unwrap_or(false)
}

/// A subscription that wakes the watcher when its `soundOutput` changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    /// The Sound Out setting (settings service).
    Settings,
    /// The live route, carried by master volume updates (audio service).
    Volume,
}

impl Feed {
    pub fn name(self) -> &'static str {
        match self {
            Feed::Settings => "settings",
            Feed::Volume => "volume",
        }
    }

    fn args(self) -> String {
        match self {
            Feed::Settings => format!(
                "-i '{}/getSystemSettings' '{{\"category\":\"sound\",\"keys\":[\"soundOutput\"],\"subscribe\":true}}'",
                SETTINGS
            ),
            Feed::Volume => format!("-i '{}/master/getVolume' '{{\"subscribe\":true}}'", AUDIO),
        }
    }
}

/// Keeps one subscription running, restarting it if it ends. Sends
/// `Msg::Feed(feed, true)` once a reply arrives, `Msg::Feed(feed, false)` when
/// it ends, and `Msg::Changed` whenever the reported output differs from the
/// last one.
pub fn follow(feed: Feed, tx: Sender<Msg>) {
    let mut backoff = Duration::from_secs(2);
    loop {
        let started = Instant::now();
        let mut cmd = command(SCRIPT);
        cmd.args(["-q", "-f", "-c", &format!("{} {}", LUNA_SEND, feed.args()), "/dev/null"]);
        match cmd.stdout(Stdio::piped()).spawn() {
            Ok(mut child) => {
                let stdout = child.stdout.take().expect("piped stdout");
                let mut last: Option<String> = None;
                let mut alive = false;
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let line = line.trim();
                    if !line.starts_with('{') {
                        continue;
                    }
                    debug(&format!("{} feed: {}", feed.name(), line));
                    if bool_field(line, "returnValue") == Some(false) {
                        log(&format!("{} subscription rejected: {}", feed.name(), line));
                        continue;
                    }
                    if !alive {
                        alive = true;
                        if tx.send(Msg::Feed(feed, true)).is_err() {
                            let _ = child.kill();
                            return;
                        }
                    }
                    let Some(output) = sound_output(line) else { continue };
                    if last.as_deref() != Some(output.as_str()) {
                        debug(&format!("{} feed reports {}", feed.name(), output));
                        last = Some(output);
                        if tx.send(Msg::Changed(feed)).is_err() {
                            let _ = child.kill();
                            return;
                        }
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                if tx.send(Msg::Feed(feed, false)).is_err() {
                    return;
                }
                log(&format!("{} subscription ended; restarting", feed.name()));
            }
            Err(e) => log(&format!("cannot start {} subscription: {}", feed.name(), e)),
        }
        // A subscription that dies at once gets retried less and less often.
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(2);
        } else {
            backoff = (backoff * 2).min(Duration::from_secs(120));
        }
        std::thread::sleep(backoff);
    }
}
