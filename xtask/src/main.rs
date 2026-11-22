//! Build helper, so ARC Restore builds and packages on Windows, macOS or
//! Linux with Rust alone (no Node.js / ares-package needed).
//!
//!   cargo xtask build    unit tests + static watcher binaries for 32- and 64-bit ARM
//!   cargo xtask ipk      build, then write build/<id>_<version>_all.ipk
//!   cargo xtask release-manifest   Homebrew Channel manifest + repo.json (release.rs)
//!
//! Cross-compiling needs cargo-zigbuild and zig: `pip install ziglang cargo-zigbuild`.
//!
//! The IPK writer follows the one in ytnative's xtask (itself after plx-native's
//! mkipk.py): explicit file modes (Windows has no execute bit), packageinfo.json,
//! Installed-Size in bytes, and an `ar` container with bare member names and
//! debian-binary first, which is what the webOS app installer accepts.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

mod release;

type Result<T> = std::result::Result<T, String>;

const TARGETS: [(&str, &str); 2] = [
    ("armv7", "armv7-unknown-linux-musleabihf"),
    ("aarch64", "aarch64-unknown-linux-musl"),
];

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    std::env::set_current_dir(&root).expect("repository root");
    let result = match std::env::args().nth(1).as_deref() {
        Some("build") => build(),
        Some("ipk") => build().and_then(|_| ipk()).map(|_| ()),
        Some("release-manifest") => release::manifest(),
        _ => Err("usage: cargo xtask build | ipk | release-manifest".into()),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let status = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed ({status})"))
    }
}

/// PATH plus where `pip install ziglang cargo-zigbuild` puts its tools: the
/// zig binary inside the `ziglang` package and the (user) scripts folders
/// holding `cargo-zigbuild`, which are often not on PATH on Windows.
fn path_with_zig() -> std::ffi::OsString {
    let path = std::env::var_os("PATH").unwrap_or_default();
    const FIND: &str = "import os, sysconfig\n\
        try:\n    import ziglang; print(os.path.dirname(ziglang.__file__))\n\
        except ImportError: pass\n\
        print(sysconfig.get_path('scripts'))\n\
        print(sysconfig.get_path('scripts', os.name + '_user'))";
    for python in ["python3", "python"] {
        let Ok(out) = Command::new(python).args(["-c", FIND]).output() else { continue };
        if !out.status.success() {
            continue;
        }
        let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let dir = PathBuf::from(line.trim());
            if !line.trim().is_empty() && dir.is_dir() && !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        if let Ok(joined) = std::env::join_paths(dirs) {
            return joined;
        }
    }
    path
}

fn build() -> Result<()> {
    let manifest = "watcher-rs/Cargo.toml";
    run(Command::new("cargo").args(["test", "-q", "--manifest-path", manifest]), "watcher unit tests")?;
    let path = path_with_zig();
    std::fs::create_dir_all("build/bin").map_err(|e| e.to_string())?;
    for (name, target) in TARGETS {
        let _ = Command::new("rustup").args(["target", "add", target]).output();
        run(
            Command::new("cargo")
                .env("PATH", &path)
                .args(["zigbuild", "--release", "--manifest-path", manifest, "--target", target]),
            &format!("cargo zigbuild for {target} (needs `pip install ziglang cargo-zigbuild`)"),
        )?;
        let from = format!("watcher-rs/target/{target}/release/arc-restore-watcher");
        let to = format!("build/bin/arc-restore-watcher-{name}");
        std::fs::copy(&from, &to).map_err(|e| format!("copying {from}: {e}"))?;
        println!("built {to} ({} KiB)", std::fs::metadata(&to).map_or(0, |m| m.len() / 1024));
    }
    Ok(())
}

/// A string field of app/appinfo.json (flat enough for a plain search).
pub fn appinfo_field(json: &str, key: &str) -> Result<String> {
    let needle = format!("\"{key}\"");
    let at = json.find(&needle).ok_or(format!("appinfo.json has no {key}"))? + needle.len();
    let rest = json[at..].trim_start().strip_prefix(':').ok_or("bad appinfo.json")?.trim_start();
    let rest = rest.strip_prefix('"').ok_or(format!("appinfo.json {key} is not a string"))?;
    Ok(rest[..rest.find('"').ok_or("bad appinfo.json")?].to_string())
}

pub fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One tar entry: a directory (None) or a file with its mode.
type Entry = (String, Option<(Vec<u8>, u32)>);

/// Adds `dir` (relative to the app root `base`) and everything under it.
fn collect(base: &Path, dir: &Path, prefix: &str, entries: &mut Vec<Entry>) -> Result<()> {
    let mut names: Vec<_> = std::fs::read_dir(base.join(dir))
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .collect();
    names.sort_by_key(|e| e.file_name());
    for entry in names {
        let rel = dir.join(entry.file_name());
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let path = format!("{prefix}/{rel_str}");
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            entries.push((path, None));
            collect(base, &rel, prefix, entries)?;
        } else {
            let mut data = std::fs::read(entry.path()).map_err(|e| e.to_string())?;
            let executable = rel_str.ends_with(".sh") || rel_str.starts_with("bin/");
            if rel_str.ends_with(".sh") {
                // A CRLF checkout must not reach the TV's /bin/sh.
                data = String::from_utf8_lossy(&data).replace("\r\n", "\n").into_bytes();
            }
            entries.push((path, Some((data, if executable { 0o755 } else { 0o644 }))));
        }
    }
    Ok(())
}

fn ipk() -> Result<PathBuf> {
    let info = std::fs::read_to_string("app/appinfo.json").map_err(|e| e.to_string())?;
    let id = appinfo_field(&info, "id")?;
    let version = appinfo_field(&info, "version")?;
    let title = appinfo_field(&info, "title")?;
    let vendor = appinfo_field(&info, "vendor")?;

    // The app directory is app/ plus the watchers in bin/.
    let stage = PathBuf::from("build/stage");
    let _ = std::fs::remove_dir_all(&stage);
    copy_dir(Path::new("app"), &stage)?;
    std::fs::create_dir_all(stage.join("bin")).map_err(|e| e.to_string())?;
    for (name, _) in TARGETS {
        let file = format!("arc-restore-watcher-{name}");
        std::fs::copy(Path::new("build/bin").join(&file), stage.join("bin").join(&file))
            .map_err(|e| format!("{file}: {e} (run `cargo xtask build`)"))?;
    }

    let app_prefix = format!("usr/palm/applications/{id}");
    let pkg_prefix = format!("usr/palm/packages/{id}");
    let mut entries: Vec<Entry> = Vec::new();
    for d in ["usr", "usr/palm", "usr/palm/applications", "usr/palm/packages", &app_prefix, &pkg_prefix] {
        entries.push((d.to_string(), None));
    }
    collect(&stage, Path::new(""), &app_prefix, &mut entries)?;
    let packageinfo = format!(
        "{{\n  \"app\": {id},\n  \"id\": {id},\n  \"loc_name\": {title},\n  \"package_format_version\": 2,\n  \"vendor\": {vendor},\n  \"version\": {version}\n}}\n",
        id = json_string(&id),
        title = json_string(&title),
        vendor = json_string(&vendor),
        version = json_string(&version),
    );
    entries.push((format!("{pkg_prefix}/packageinfo.json"), Some((packageinfo.into_bytes(), 0o644))));
    let _ = std::fs::remove_dir_all(&stage);

    let installed_bytes: u64 = entries.iter().filter_map(|e| e.1.as_ref()).map(|(d, _)| d.len() as u64).sum();
    let mtime = source_epoch();
    let data_tgz = targz(&entries, mtime)?;
    let control = format!(
        "Package: {id}\nVersion: {version}\nSection: misc\nPriority: optional\nArchitecture: all\n\
         Installed-Size: {installed_bytes}\nMaintainer: {vendor}\nDescription: {title}\n\
         webOS-Package-Format-Version: 2\nwebOS-Packager-Version: x.y.x\n"
    );
    let control_tgz = targz(&[("control".into(), Some((control.into_bytes(), 0o644)))], mtime)?;

    let mut ar = b"!<arch>\n".to_vec();
    for (name, blob) in [("debian-binary", b"2.0\n".to_vec()), ("control.tar.gz", control_tgz), ("data.tar.gz", data_tgz)] {
        ar.extend(format!("{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", mtime, 0, 0, "100644", blob.len()).as_bytes());
        ar.extend(&blob);
        if blob.len() % 2 == 1 {
            ar.push(b'\n');
        }
    }
    let out = PathBuf::from(format!("build/{id}_{version}_all.ipk"));
    std::fs::write(&out, ar).map_err(|e| e.to_string())?;
    println!("wrote {} ({} KiB, Installed-Size {} KiB)", out.display(), std::fs::metadata(&out).map_or(0, |m| m.len() / 1024), installed_bytes / 1024);
    Ok(out)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// SOURCE_DATE_EPOCH, else the HEAD commit's time, else now.
fn source_epoch() -> u64 {
    if let Some(t) = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|v| v.trim().parse().ok()) {
        return t;
    }
    let git = Command::new("git").args(["log", "-1", "--format=%ct"]).output();
    if let Some(t) = git.ok().and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|s| s.trim().parse().ok()) {
        return t;
    }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs())
}

fn targz(entries: &[Entry], mtime: u64) -> Result<Vec<u8>> {
    let e = |e: std::io::Error| e.to_string();
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    let mut tb = tar::Builder::new(gz);
    tb.mode(tar::HeaderMode::Deterministic);
    for (path, content) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_uid(0);
        h.set_gid(0);
        h.set_username("root").map_err(e)?;
        h.set_groupname("root").map_err(e)?;
        h.set_mtime(mtime);
        match content {
            None => {
                h.set_entry_type(tar::EntryType::Directory);
                h.set_mode(0o755);
                h.set_size(0);
                h.set_cksum();
                tb.append_data(&mut h, format!("{path}/"), std::io::empty()).map_err(e)?;
            }
            Some((data, mode)) => {
                h.set_entry_type(tar::EntryType::Regular);
                h.set_mode(*mode);
                h.set_size(data.len() as u64);
                h.set_cksum();
                tb.append_data(&mut h, path, data.as_slice()).map_err(e)?;
            }
        }
    }
    let gz = tb.into_inner().map_err(e)?;
    let mut out = gz.finish().map_err(e)?;
    out.flush().map_err(e)?;
    Ok(out)
}
