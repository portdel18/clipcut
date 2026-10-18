//! Updates: the app asks GitHub Releases for a newer version (`latest.json`, written by the
//! release workflow) and installs it on request. Updates are signed; the public key is baked in at
//! build time (`CLIPCUT_UPDATER_PUBKEY`). A build without one never checks or installs anything
//! and points people to the Releases page instead.

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::UpdaterExt;

use crate::editing;

/// Where releases are published.
pub const RELEASES: &str = "https://github.com/portdel18/clipcut/releases";

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

/// Asks for a newer version. Without a key, or offline, there is nothing to report but the
/// version itself: the check never gets in the way of editing.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let mut info = UpdateInfo { current, available: None, notes: None, can_update: key().is_some(), releases: RELEASES };
    if !info.can_update {
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
    use super::verify_with;

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
