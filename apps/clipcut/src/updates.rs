//! Updates: the app asks GitHub Releases for a newer version and installs it on request.
//!
//! - With an update key baked in at build time (`CLIPCUT_UPDATER_PUBKEY`), through Tauri's updater:
//!   `latest.json` (written by the release workflow) and a signature checked against the key.
//! - Without one (Windows), straight from the latest release: its `-setup.exe` is downloaded over
//!   HTTPS from GitHub, checked against the SHA-256 GitHub lists for it, and run in passive mode
//!   (`/P /UPDATE /R`: a progress bar, then ClipCut starts again). This trusts the GitHub account
//!   that publishes releases; the signed path also guards against that account being misused.
//! - Elsewhere it points people to the Releases page.

use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::UpdaterExt;

use crate::editing;

/// Where releases are published.
pub const RELEASES: &str = "https://github.com/portdel18/clipcut/releases";
/// The latest published release (drafts and pre-releases are never "latest").
const LATEST: &str = "https://api.github.com/repos/portdel18/clipcut/releases/latest";
/// No installer is anywhere near this; a bigger download is refused.
const MAX_INSTALLER: u64 = 1 << 30;

/// The public key updates are checked against, when this build has one.
pub fn key() -> Option<&'static str> {
    option_env!("CLIPCUT_UPDATER_PUBKEY").map(str::trim).filter(|k| !k.is_empty())
}

/// The updater plugin, registered only when there is a key to check updates against.
pub fn plugin<R: Runtime>() -> Option<tauri::plugin::TauriPlugin<R, tauri_plugin_updater::Config>> {
    key().map(|k| tauri_plugin_updater::Builder::new().pubkey(k).build())
}

#[derive(Serialize)]
pub struct UpdateInfo {
    /// This version.
    current: String,
    /// A newer version, when there is one.
    available: Option<String>,
    /// What changed in it.
    notes: Option<String>,
    /// Whether this build installs updates itself (else people download them from `releases`).
    can_update: bool,
    releases: &'static str,
}

/// A release on GitHub, as its API describes it (only what updating needs).
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    /// `sha256:<hex>`, which GitHub works out itself when the file is uploaded.
    #[serde(default)]
    digest: Option<String>,
}

impl Release {
    fn version(&self) -> Option<semver::Version> {
        semver::Version::parse(self.tag_name.trim().trim_start_matches('v')).ok()
    }
    /// The Windows installer the release workflow publishes (`ClipCut_<version>_x64-setup.exe`).
    fn installer(&self) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name.to_ascii_lowercase().ends_with("-setup.exe"))
    }
    /// The release's version when it is newer than `current` and has an installer.
    fn newer_than(&self, current: &str) -> Option<semver::Version> {
        let current = semver::Version::parse(current).ok()?;
        self.version().filter(|v| *v > current && self.installer().is_some())
    }
}

/// Whether this copy installs releases itself without an update key: the installer is for Windows.
fn plain() -> bool {
    key().is_none() && cfg!(windows)
}

fn client() -> Result<reqwest::Client, String> {
    // the updater plugin does the same: reqwest's rustls needs a crypto provider
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::builder()
        .user_agent(concat!("ClipCut/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Couldn't check for updates ({e})."))
}

/// The latest release, from GitHub's API.
async fn latest_release() -> Result<Release, String> {
    let offline =
        |e: reqwest::Error| format!("Couldn't check for updates ({e}). Check the internet connection, or download the latest version from {RELEASES}.");
    let r = client()?.get(LATEST).header("Accept", "application/vnd.github+json").timeout(Duration::from_secs(30)).send().await.map_err(offline)?;
    match r.status().as_u16() {
        200 => {
            r.json::<Release>().await.map_err(|e| format!("GitHub's answer about the latest version couldn't be read ({e}). New versions are at {RELEASES}."))
        }
        404 => Err(format!("No version of ClipCut is published yet (or the releases aren't public). New versions will be at {RELEASES}.")),
        403 | 429 => Err(format!("GitHub asked ClipCut to wait before checking again. Try again in an hour, or download the latest version from {RELEASES}.")),
        code => Err(format!("GitHub didn't answer ({code}). Try again later, or download the latest version from {RELEASES}.")),
    }
}

/// Checks a download against its asset: its size and, always, the SHA-256 GitHub lists for it.
fn check_download(asset: &Asset, data: &[u8]) -> Result<(), String> {
    if data.len() as u64 != asset.size {
        return Err(format!("The download was cut short ({} of {} bytes). Try again.", data.len(), asset.size));
    }
    let want = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .ok_or_else(|| format!("GitHub doesn't list a checksum for this installer, so ClipCut won't run it. Download it from {RELEASES} instead."))?;
    let got: String = Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect();
    if !got.eq_ignore_ascii_case(want.trim()) {
        return Err(format!("The download doesn't match the release's checksum, so ClipCut won't run it. Try again, or download it from {RELEASES}."));
    }
    Ok(())
}

/// Downloads the installer, at most [`MAX_INSTALLER`] bytes.
async fn download(asset: &Asset) -> Result<Vec<u8>, String> {
    let fail = |e: reqwest::Error| format!("Couldn't download the update ({e}). Check the internet connection and try again.");
    if asset.size > MAX_INSTALLER {
        return Err(format!("The update is unexpectedly large. Download it from {RELEASES} instead."));
    }
    let mut r = client()?.get(&asset.browser_download_url).send().await.map_err(fail)?;
    if !r.status().is_success() {
        return Err(format!("Couldn't download the update (GitHub answered {}). Try again later.", r.status().as_u16()));
    }
    let mut data = Vec::with_capacity(usize::try_from(asset.size).unwrap_or(0));
    while let Some(chunk) = r.chunk().await.map_err(fail)? {
        if (data.len() + chunk.len()) as u64 > asset.size {
            return Err("The download is bigger than the release says. Try again.".into());
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

/// Without a key: downloads the latest release's installer, checks it, saves the project and
/// runs the installer, which closes ClipCut, installs and starts it again.
async fn install_plain(app: &AppHandle) -> Result<(), String> {
    let release = latest_release().await?;
    let current = app.package_info().version.to_string();
    let (Some(version), Some(asset)) = (release.newer_than(&current), release.installer()) else {
        return Err("ClipCut is already up to date.".into());
    };
    let data = download(asset).await?;
    check_download(asset, &data)?;
    // named here, not after the asset, so a release can't choose where it lands
    let dir = std::env::temp_dir().join("ClipCut update");
    let path = dir.join(format!("ClipCut_{version}_x64-setup.exe"));
    let saved = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&saved, data)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Couldn't save the update ({e}). Check there's space on the drive."))?;
    editing::save_on_close(app);
    // Tauri's installer: /P a progress bar and no questions, /UPDATE keep settings, /R start ClipCut after
    std::process::Command::new(&path)
        .args(["/P", "/UPDATE", "/R"])
        .spawn()
        .map_err(|e| format!("Couldn't start the update ({e}). It was saved as {}; run it yourself.", path.display()))?;
    app.exit(0);
    Ok(())
}

/// Asks for a newer version. Offline, or on a system this copy can't update, there is nothing
/// to report but the version itself: the check never gets in the way of editing.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let mut info = UpdateInfo { current, available: None, notes: None, can_update: key().is_some() || plain(), releases: RELEASES };
    if !info.can_update {
        return Ok(info);
    }
    if plain() {
        let release = latest_release().await?;
        info.available = release.newer_than(&info.current).map(|v| v.to_string());
        info.notes = release.body.filter(|b| !b.trim().is_empty() && info.available.is_some());
        return Ok(info);
    }
    let update = app
        .updater()
        .map_err(|e| format!("Couldn't check for updates: {e}"))?
        .check()
        .await
        .map_err(|e| format!("Couldn't check for updates ({e}). Check the internet connection, or download the latest version from {RELEASES}."))?;
    if let Some(u) = update {
        info.available = Some(u.version.clone());
        info.notes = u.body.clone().filter(|b| !b.trim().is_empty());
    }
    Ok(info)
}

/// Downloads and installs the newer version, then starts it. On Windows the installer takes over
/// and the app closes, so the interface saves the project first.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    if plain() {
        return install_plain(&app).await;
    }
    if key().is_none() {
        return Err(format!("This copy of ClipCut can't update itself. Download the latest version from {RELEASES}."));
    }
    let fail = |e: tauri_plugin_updater::Error| format!("Couldn't update ClipCut ({e}). Try again, or download the latest version from {RELEASES}.");
    let Some(update) = app.updater().map_err(fail)?.check().await.map_err(fail)? else {
        return Err("ClipCut is already up to date.".into());
    };
    editing::save_on_close(&app);
    update.download_and_install(|_, _| {}, || {}).await.map_err(fail)?;
    app.restart()
}

/// Opens the Releases page in the browser.
#[tauri::command]
pub fn open_releases() -> Result<(), String> {
    let r = if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(RELEASES).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(RELEASES).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(RELEASES).spawn()
    };
    r.map(|_| ()).map_err(|e| format!("Couldn't open the browser ({e}). New versions are at {RELEASES}."))
}

/// `--verify-update <installer> <installer.sig> <version>`: checks a release's update signature
/// with the key built into this copy, as the updater will, so a release whose signing key doesn't
/// match the app's key never ships. Returns the exit code.
pub fn verify_update(file: &str, sig: &str, version: &str) -> i32 {
    let checked = (|| {
        let key = key().ok_or("this build has no update key (CLIPCUT_UPDATER_PUBKEY)")?;
        let data = std::fs::read(file).map_err(|e| format!("couldn't read {file}: {e}"))?;
        let sig = std::fs::read_to_string(sig).map_err(|e| format!("couldn't read {sig}: {e}"))?;
        verify_with(key, &data, &sig, version)
    })();
    match checked {
        Ok(()) => {
            println!("{file}: signed for version {version} with this app's update key");
            0
        }
        Err(e) => {
            eprintln!("{file}: {e}");
            1
        }
    }
}

/// Checks `data` against a Tauri update signature (base64 of a minisign signature) and the
/// version it was signed for.
fn verify_with(key: &str, data: &[u8], sig: &str, version: &str) -> Result<(), String> {
    let text = |b64: &str, what: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .ok_or_else(|| format!("the {what} isn't a Tauri signer {what}"))
    };
    let key = minisign_verify::PublicKey::decode(&text(key, "key")?).map_err(|e| format!("the update key is unreadable: {e}"))?;
    let sig = minisign_verify::Signature::decode(&text(sig, "signature")?).map_err(|e| format!("the signature is unreadable: {e}"))?;
    key.verify(data, &sig, true).map_err(|e| format!("the signature doesn't match this app's update key: {e}"))?;
    let signed = sig.trusted_comment().split('\t').find_map(|f| f.strip_prefix("version:"));
    match signed {
        Some(v) if v.trim_start_matches('v') == version.trim_start_matches('v') => Ok(()),
        Some(v) => Err(format!("signed for version {v}, not {version}")),
        None => Err("the signature doesn't say which version it is for (sign with --app-version)".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Release, check_download, verify_with};

    /// What GitHub's API answers for a release (cut down), with the installer's real SHA-256.
    fn release(tag: &str) -> Release {
        let data = b"ClipCut test installer\n";
        let sum: String = <sha2::Sha256 as sha2::Digest>::digest(data).iter().map(|b| format!("{b:02x}")).collect();
        serde_json::from_value(serde_json::json!({
            "tag_name": tag, "body": "## What's new\n- things", "draft": false,
            "assets": [
                {"name": "ClipCut_0.3.0_x64-setup.exe.sig", "browser_download_url": "https://x/sig", "size": 10, "digest": "sha256:00"},
                {"name": "ClipCut_0.3.0_x64-setup.exe", "browser_download_url": "https://x/exe", "size": data.len(), "digest": format!("sha256:{sum}")},
                {"name": "latest.json", "browser_download_url": "https://x/json", "size": 5}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn the_latest_release_is_installed_only_when_newer_and_checked() {
        let r = release("v0.3.0");
        assert_eq!(r.installer().unwrap().browser_download_url, "https://x/exe");
        assert_eq!(r.newer_than("0.2.0").unwrap().to_string(), "0.3.0");
        assert!(r.newer_than("0.3.0").is_none() && r.newer_than("0.4.0").is_none());
        assert!(release("0.3.0").newer_than("0.2.9").is_some());
        assert!(release("nonsense").newer_than("0.2.0").is_none());
        assert!(r.newer_than("not a version").is_none());
        let mut bare = release("v0.3.0");
        bare.assets.retain(|a| !a.name.ends_with(".exe"));
        assert!(bare.newer_than("0.2.0").is_none(), "a release without an installer isn't offered");

        let exe = r.installer().unwrap();
        assert_eq!(check_download(exe, b"ClipCut test installer\n"), Ok(()));
        assert!(check_download(exe, b"ClipCut test installer").unwrap_err().contains("cut short"));
        assert!(check_download(exe, b"ClipCut TEST installer\n").unwrap_err().contains("checksum"));
        let mut unlisted = release("v0.3.0");
        unlisted.assets.iter_mut().for_each(|a| a.digest = None);
        assert!(check_download(unlisted.installer().unwrap(), b"ClipCut test installer\n").unwrap_err().contains("doesn't list a checksum"));
    }

    // made with `tauri signer generate` and `tauri signer sign --app-version 1.2.3` (a test key,
    // used nowhere else)
    const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDg2MDdCRDU1MjNCNkJCQ0EKUldUS3U3WWpWYjBIaGg2anpYU1YwV2E4cUR1NzhJaWhhWS9qOE9Ock52amc4RHllSTZtQStaSEkK";
    const OTHER: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYzRUJDMUZFOUU4RjhEODAKUldTQWpZK2Uvc0hyWStHdGE1SVpsdDBob3phOFp5c3BzdW9QZ2VOeVBXdG1nRFdjRWc1WVlUM3cK";
    const SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUS3U3WWpWYjBIaGwrVG1lOHpueVVqU2svTXZQb0dvQ2REeGV4VW12c0FrR1FERVErUEdOcmNrT1k4Wk9sMHZteS93YkQ5NytFL0VGV1VZeGVLblRNVXcyWE9qZWNjeVFBPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMzU2OTEyCWZpbGU6Zml4dHVyZS5iaW4JdmVyc2lvbjoxLjIuMwo1bExvV01JZjNkVDZLK2wyQkFjbUhqU0tIYzZVMW83K0JrV2pha3lXOWFpd3l0Zk5CUjVLem1PUHdScGVqQWozelhHWUVpSlZlUE5LZzBxRFFPUlhEdz09Cg==";
    const DATA: &[u8] = b"ClipCut test installer\n";

    #[test]
    fn release_signatures_are_checked_like_the_updater_does() {
        assert_eq!(verify_with(KEY, DATA, SIG, "1.2.3"), Ok(()));
        assert_eq!(verify_with(KEY, DATA, SIG, "v1.2.3"), Ok(()));
        assert!(verify_with(KEY, DATA, SIG, "1.2.4").unwrap_err().contains("signed for version 1.2.3"));
        assert!(verify_with(OTHER, DATA, SIG, "1.2.3").unwrap_err().contains("doesn't match"));
        assert!(verify_with(KEY, b"something else", SIG, "1.2.3").is_err());
        assert!(verify_with(KEY, DATA, "not base64!", "1.2.3").is_err());
        assert!(verify_with("", DATA, SIG, "1.2.3").is_err());
    }
}
