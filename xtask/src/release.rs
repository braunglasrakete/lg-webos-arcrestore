//! `cargo xtask release-manifest`: the webOS Homebrew Channel package manifest
//! (webosbrew `PackageManifest` schema) and a one-package `repo.json` index for
//! the IPK in build/, written next to it. Attach all three to the GitHub release
//! tagged `v<version>`; then
//!   releases/latest/download/repo.json
//! is a custom-repository URL for Homebrew Channel (Settings > Add repository),
//! which offers each new release as an update.
//!
//! The repository is `ARC_RESTORE_REPO` (owner/name), else the `origin` remote.

use std::process::Command;

use sha2::{Digest, Sha256};

use crate::{appinfo_field, json_string, Result};

const DESCRIPTION: &str = "Returns a rooted LG TV to its previous Sound Out, such as HDMI ARC/eARC, \
when disconnecting a Bluetooth speaker sends audio back to the TV speakers.";
const SHORT: &str = "Restore ARC/eARC Sound Out after Bluetooth disconnects";

fn repository() -> Result<String> {
    if let Ok(repo) = std::env::var("ARC_RESTORE_REPO") {
        return Ok(repo.trim().to_string());
    }
    let out = Command::new("git").args(["remote", "get-url", "origin"]).output().map_err(|e| e.to_string())?;
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("git@github.com:"))
        .ok_or(format!("origin is not a GitHub repository ({url}); set ARC_RESTORE_REPO=owner/name"))?;
    Ok(path.trim_end_matches(".git").to_string())
}

pub fn manifest() -> Result<()> {
    let info = std::fs::read_to_string("app/appinfo.json").map_err(|e| e.to_string())?;
    let id = appinfo_field(&info, "id")?;
    let version = appinfo_field(&info, "version")?;
    let title = appinfo_field(&info, "title")?;
    let kind = appinfo_field(&info, "type")?;
    let repo = repository()?;
    let base = format!("https://github.com/{repo}");

    let file = format!("{id}_{version}_all.ipk");
    let ipk = std::path::Path::new("build").join(&file);
    let data = std::fs::read(&ipk).map_err(|e| format!("reading {} (run `cargo xtask ipk` first): {e}", ipk.display()))?;
    let sha: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    // Pinned to the tag, so an old manifest keeps pointing at its own icon and IPK.
    let icon = format!("https://raw.githubusercontent.com/{repo}/v{version}/app/icon.png");
    let ipk_url = format!("{base}/releases/download/v{version}/{file}");
    let manifest_name = format!("{id}.manifest.json");

    let manifest = format!(
        "{{\n  \"id\": {id},\n  \"version\": {version},\n  \"type\": {kind},\n  \"title\": {title},\n  \
         \"appDescription\": {desc},\n  \"iconUri\": {icon},\n  \"sourceUrl\": {source},\n  \
         \"rootRequired\": true,\n  \"ipkUrl\": {ipk_url},\n  \"ipkHash\": {{ \"sha256\": \"{sha}\" }},\n  \
         \"ipkSize\": {size}\n}}",
        id = json_string(&id),
        version = json_string(&version),
        kind = json_string(&kind),
        title = json_string(&title),
        desc = json_string(DESCRIPTION),
        icon = json_string(&icon),
        source = json_string(&base),
        ipk_url = json_string(&ipk_url),
        size = data.len(),
    );
    let out = std::path::Path::new("build").join(&manifest_name);
    std::fs::write(&out, format!("{manifest}\n")).map_err(|e| e.to_string())?;
    println!("wrote {} (ipk sha256 {sha})", out.display());

    // Same shape as repo.webosbrew.org/api/apps.json: paging + packages, each
    // with its manifest embedded (Homebrew Channel reads the version from it).
    let embedded = manifest.replace('\n', "\n      ");
    let repo_json = format!(
        "{{\n  \"paging\": {{ \"page\": 1, \"count\": 1, \"maxPage\": 1, \"itemsTotal\": 1, \"prevUrl\": null, \"nextUrl\": null }},\n  \
         \"packages\": [\n    {{\n      \"id\": {id},\n      \"title\": {title},\n      \"iconUri\": {icon},\n      \
         \"manifestUrl\": {manifest_url},\n      \"manifest\": {embedded},\n      \"pool\": \"main\",\n      \
         \"shortDescription\": {short}\n    }}\n  ]\n}}\n",
        id = json_string(&id),
        title = json_string(&title),
        icon = json_string(&icon),
        manifest_url = json_string(&format!("{base}/releases/download/v{version}/{manifest_name}")),
        short = json_string(SHORT),
    );
    std::fs::write("build/repo.json", repo_json).map_err(|e| e.to_string())?;
    println!("wrote build/repo.json");
    Ok(())
}
