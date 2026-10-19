# Releasing ClipCut

Releases go on the repository's **Releases** page, made by the **ClipCut release** workflow
(`.github/workflows/clipcut-release.yml`). It builds the Windows installer for the version you name,
runs the packaged app's self-test, signs what it can, and publishes the release with the update
manifest that copies of ClipCut check.

## Make a release

Either push a tag:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

or open **Actions → ClipCut release → Run workflow** and type the version (`0.2.0`); the tag is made
on the commit you run it from. A version with a suffix (`0.2.0-beta.1`) becomes a pre-release:
people can download it, but no copy of ClipCut updates to it. The version comes from the tag, so
`Cargo.toml` doesn't need to change.

The release is a draft until everything is uploaded, so ClipCut never sees a release without its
update manifest. Its notes are GitHub's generated list of merged pull requests; edit them on the
release page afterwards if you like (the update dialog shows the notes from when it was made).

## Updates (free, set up once)

ClipCut updates itself either way. A few seconds after it starts it asks GitHub for the latest
release, shows **Update to x.y.z** in the top bar, and **Update and restart** (also in **About
ClipCut**, the name at the top left) saves the project, installs the new version and opens it again.

- **Without an update key** (nothing to set up): it downloads the latest release's `-setup.exe`
  and runs it only when it matches the SHA-256 checksum GitHub lists for that file. That catches a
  damaged or swapped download, but it trusts whoever can publish releases on this repository.
- **With an update key** (below, recommended): each update must also be signed with your private
  key, which never leaves your computer and GitHub's secrets, so a release made by anyone else is
  refused even if the GitHub account were misused.

Pre-releases and drafts are never offered. Copies installed from a release built without a key
switch to the signed path from the first release they install that has one.

1. Make a key pair on your own computer, with a password:

   ```sh
   npx @tauri-apps/cli@2 signer generate -w clipcut-updates.key
   ```

   Keep `clipcut-updates.key` and its password safe and private. If either is lost, copies already
   installed can't update to a release signed with a new key, and people have to download that
   release by hand once.
2. In the repository's **Settings → Secrets and variables → Actions**:
   - secret `TAURI_SIGNING_PRIVATE_KEY`: the contents of `clipcut-updates.key`;
   - secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password;
   - variable (not secret) `CLIPCUT_UPDATER_PUBKEY`: the contents of `clipcut-updates.key.pub`.

From the next release on, each update is checked against the public key built into the app, and
against the version it was signed for; the workflow runs the same check with the built app before it
publishes, so a key that doesn't match its public half fails the release instead of reaching
anyone.

The first release with the key is still installed by hand: copies from before it don't have the key.

## Code signing (optional, paid)

Without a code-signing certificate, Windows SmartScreen shows "Windows protected your PC" when the
installer is run (**More info → Run anyway**). Signing removes that warning once the certificate
has built up reputation (straight away for EV certificates). Either:

- **A certificate file** (`.pfx`) from a certificate authority: add secrets `WINDOWS_CERTIFICATE`
  (the file in base64: `[Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))` in
  PowerShell, or `base64 -w0 cert.pfx`) and `WINDOWS_CERTIFICATE_PASSWORD`. The workflow imports it,
  signs the app and its installer with SHA-256 and a DigiCert timestamp, and checks the signatures.
- **A signing service** (Azure Trusted Signing, SignPath, a cloud HSM): add a variable
  `WINDOWS_SIGN_COMMAND` with the command that signs one file, `%1` standing for its path (Tauri's
  `bundle.windows.signCommand`), plus whatever secrets that tool needs as environment variables in
  the workflow's build step.

Code signing and update signing are separate: updates are checked with the update key whether or
not the installer is code-signed.
