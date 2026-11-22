![ARC Restore on webOS, with the watcher enabled and HDMI ARC/eARC selected](docs/screenshot.png)

# ARC Restore

ARC Restore returns a rooted LG webOS TV to its previous **Sound Out** when disconnecting a Bluetooth speaker sends audio back to the built-in TV speakers. Tested on an LG C3 running webOS 24 with an HDMI ARC/eARC soundbar.

[Download the latest IPK](https://github.com/braunglasrakete/lg-webos-arcrestore/releases/latest)

## Install and enable

Requires a rooted LG webOS TV with [Homebrew Channel](https://github.com/webosbrew/webos-homebrew-channel) and its root service working.

1. Sideload the IPK. If you use the webOS CLI, replace `webos` with your device name.

   ```sh
   ares-install -d webos org.arc-restore.app_1.1.0_all.ipk
   ```

2. Select your usual **Sound Out**, such as HDMI ARC/eARC.
3. Open **ARC Restore** and select **Install & Enable**. Use the remote's up/down arrows and OK button, or the Magic Remote pointer.
4. Connect a Bluetooth speaker, then turn it off. Confirm audio returns to the previous output.

The watcher starts through Homebrew Channel's boot hook. It can take a few seconds to become active after reboot. **Refresh Status** shows the watcher state and the outputs it sees. Select **Disable** to stop the watcher and remove its boot hook before uninstalling the app.

The watcher detects the transition from Bluetooth audio to TV Speaker. Selecting TV Speaker manually while Bluetooth audio is active can trigger the same restore.

## How the watcher works

Since 1.1.0 the watcher is a small native program (`watcher-rs/`, Rust, static binaries for 32- and 64-bit ARM). Instead of asking the TV for its Sound Out every 2 seconds, it subscribes to two webOS services and wakes only when something changes:

- the **Sound Out setting** (`com.webos.settingsservice/getSystemSettings`), and
- the **live audio route**, reported with master volume updates (`com.webos.service.audio/master/getVolume`).

On each change it takes the same two readings the old shell watcher took and applies the same rules: remember the last non-Bluetooth output, restore it once Bluetooth falls back to TV Speaker, verify, and retry at most three times, 2 seconds apart. While the subscriptions run it also re-checks every 10 seconds when Bluetooth is in use and every minute otherwise, as a safety net. If neither subscription works, it polls every 2 seconds like before. **Reacts to changes** in the app shows which mode is active.

If the native watcher cannot run on a TV, `control.sh` starts the original `app/scripts/watch.sh` instead.

### Troubleshooting

The watcher logs to `/tmp/arc-restore/watcher.log`. For a detailed log of every service reply, run this as root on the TV, then select **Disable** and **Install & Enable** to restart the watcher:

```sh
mkdir -p /tmp/arc-restore && touch /tmp/arc-restore/debug
```

The flag is cleared when the TV reboots. To force the old shell watcher, create `/var/lib/webosbrew/arc-restore/shell` instead.

## Build from source

You need Rust (rustup) and [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) (`pip install ziglang cargo-zigbuild`). From the repository root:

```sh
cargo xtask build   # unit tests, then build/bin/arc-restore-watcher-{armv7,aarch64}
cargo xtask ipk     # build, then build/org.arc-restore.app_<version>_all.ipk
```

This needs only Rust and cargo-zigbuild, on Windows too. `scripts/build-watcher.sh` does the build step from a POSIX shell. `cargo test --manifest-path watcher-rs/Cargo.toml` runs the unit tests on any OS.

`cargo xtask release-manifest` writes the Homebrew Channel manifest and a `repo.json` index next to the IPK. The **Publish ARC Restore** workflow attaches all three to a GitHub release, so `https://github.com/<owner>/<repo>/releases/latest/download/repo.json` works as a Homebrew Channel repository.

The JSON helpers and the subscription approach in `watcher-rs` are adapted from the MIT-licensed [Webos-EARC-Volume-Overlay](https://github.com/danzig666/Webos-EARC-Volume-Overlay) watcher (Copyright (c) 2026 eARC Volume Overlay contributors).
