//! Signed update verification and directory-based activation/rollback.
//! The launcher must stop the client before changing the installed directory.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    time::Duration,
};

/// Shared guard held by an installed client for its entire process lifetime.
/// The launcher waits for acknowledgement before releasing its own shared guard,
/// closing the spawn-to-lock race against concurrent update commands.
pub fn client_install_guard() -> Result<Option<File>> {
    let lock_path = if let Some(path) = std::env::var_os("VOID_INSTALL_LOCK") {
        Some(PathBuf::from(path))
    } else {
        let executable = std::env::current_exe()?;
        executable
            .parent()
            .filter(|parent| parent.file_name().is_some_and(|name| name == "current"))
            .and_then(Path::parent)
            .map(|root| root.join(".client.lock"))
    };
    let Some(lock_path) = lock_path else {
        return Ok(None);
    };
    let file = open_install_lock(&lock_path)?;
    file.lock_shared()
        .context("wait for in-progress client update")?;
    if let Some(ready_path) = std::env::var_os("VOID_LAUNCH_READY") {
        let ready_path = PathBuf::from(ready_path);
        ensure!(
            ready_path
                .parent()
                .context("startup marker parent missing")?
                .canonicalize()?
                == lock_path
                    .parent()
                    .context("lock parent missing")?
                    .canonicalize()?,
            "startup marker outside installation directory"
        );
        ensure!(
            ready_path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".void-startup-")),
            "invalid startup marker name"
        );
        let mut ready = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(ready_path)?;
        ready.write_all(b"ready")?;
        ready.sync_all()?;
    }
    Ok(Some(file))
}

/// Launcher-side guard, held until the child acknowledges its shared lock.
pub fn lock_for_launch(install_root: &Path) -> Result<File> {
    let file = open_install_lock(&install_root.join(".client.lock"))?;
    file.lock_shared()
        .context("wait for in-progress client update")?;
    Ok(file)
}

/// Fails immediately while an installed client/launcher holds a shared guard.
/// Hold this guard through staging, activation, or rollback.
pub fn lock_for_update(install_root: &Path) -> Result<File> {
    let file = open_install_lock(&install_root.join(".client.lock"))?;
    file.try_lock()
        .context("client is running or another update is in progress; close it before updating")?;
    Ok(file)
}

fn open_install_lock(path: &Path) -> Result<File> {
    std::fs::create_dir_all(path.parent().context("lock has no parent")?)?;
    Ok(OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)?)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema: u32,
    pub version: String,
    pub channel: String,
    pub platform: String,
    pub artifact: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignedEnvelope {
    pub payload: String,
    pub signature: String,
}

/// Only this type can be staged; construct it by verifying a trusted release key.
pub struct VerifiedRelease {
    manifest: ReleaseManifest,
}
impl VerifiedRelease {
    pub fn manifest(&self) -> &ReleaseManifest {
        &self.manifest
    }
}

pub fn verify_manifest(
    envelope: &[u8],
    public_key: &[u8; 32],
    expected_channel: &str,
    expected_platform: &str,
    installed_version: &str,
) -> Result<VerifiedRelease> {
    ensure!(
        envelope.len() <= 64 * 1024,
        "update manifest exceeds size limit"
    );
    let envelope: SignedEnvelope = serde_json::from_slice(envelope)?;
    let payload = STANDARD.decode(envelope.payload)?;
    let signature_bytes = STANDARD.decode(envelope.signature)?;
    let key = VerifyingKey::from_bytes(public_key).context("invalid release verification key")?;
    key.verify_strict(&payload, &Signature::from_slice(&signature_bytes)?)
        .context("release signature is invalid")?;
    let manifest: ReleaseManifest = serde_json::from_slice(&payload)?;
    ensure!(manifest.schema == 1, "unsupported update schema");
    ensure!(
        matches!(manifest.channel.as_str(), "stable" | "preview")
            && manifest.channel == expected_channel,
        "release channel mismatch"
    );
    ensure!(
        manifest.platform == expected_platform,
        "release platform mismatch"
    );
    let version = semver::Version::parse(&manifest.version)?;
    ensure!(
        version > semver::Version::parse(installed_version)?,
        "update is not newer than the installed version"
    );
    ensure!(
        manifest.channel != "stable" || version.pre.is_empty(),
        "stable manifest contains a prerelease"
    );
    let artifact = Path::new(&manifest.artifact);
    ensure!(
        artifact.components().count() == 1
            && artifact
                .components()
                .all(|x| matches!(x, Component::Normal(_))),
        "artifact must be a single filename"
    );
    ensure!(
        !manifest.artifact.contains(['/', '\\', ':']) && !manifest.artifact.ends_with(['.', ' ']),
        "invalid artifact filename"
    );
    let url = url::Url::parse(&manifest.url)?;
    ensure!(
        url.scheme() == "https" && url.username().is_empty() && url.password().is_none(),
        "artifact URL must use HTTPS without credentials"
    );
    ensure!(
        manifest.sha256.len() == 64
            && manifest
                .sha256
                .bytes()
                .all(|x| x.is_ascii_hexdigit() && !x.is_ascii_uppercase()),
        "invalid SHA-256 digest"
    );
    ensure!(
        manifest.size > 0 && manifest.size <= 512 * 1024 * 1024,
        "artifact size exceeds 512 MiB limit"
    );
    Ok(VerifiedRelease { manifest })
}

/// Download one signed executable artifact into a fresh staging directory.
/// The caller supplies the platform-specific client filename in the signed manifest.
pub async fn stage_update(release: &VerifiedRelease, install_root: &Path) -> Result<PathBuf> {
    let manifest = &release.manifest;
    let stage = install_root.join("staged");
    ensure!(
        !stage.exists(),
        "a staged update already exists; activate or explicitly discard it first"
    );
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(300))
        .user_agent("Void.rs/0.1")
        .build()?;
    let mut response = client.get(&manifest.url).send().await?.error_for_status()?;
    if let Some(size) = response.content_length() {
        ensure!(size == manifest.size, "update download length mismatch");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len().saturating_add(chunk.len()) as u64 <= manifest.size,
            "update exceeds signed size"
        );
        bytes.extend_from_slice(&chunk);
    }
    verify_artifact(&bytes, manifest)?;
    std::fs::create_dir_all(install_root)?;
    // A unique sibling remains invisible until the verified files are complete.
    let temp = tempfile::Builder::new()
        .prefix("update-")
        .tempdir_in(install_root)?;
    crate::assets::atomic_write(&temp.path().join(&manifest.artifact), &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            temp.path().join(&manifest.artifact),
            std::fs::Permissions::from_mode(0o755),
        )?;
    }
    crate::assets::atomic_write(
        &temp.path().join("release.json"),
        &serde_json::to_vec(manifest)?,
    )?;
    std::fs::rename(temp.path(), &stage).context("publish complete staged update")?;
    Ok(stage)
}

pub fn verify_artifact(bytes: &[u8], manifest: &ReleaseManifest) -> Result<()> {
    ensure!(
        bytes.len() as u64 == manifest.size,
        "update artifact length mismatch"
    );
    ensure!(
        hex::encode(Sha256::digest(bytes)) == manifest.sha256,
        "update artifact SHA-256 mismatch"
    );
    Ok(())
}

/// Activate a prepared directory. Returns the previous version's backup path.
/// The caller must ensure no installed executable is running. Settings/mods live
/// outside current/staged/previous and are never visited by this function.
pub fn activate_staged(install_root: &Path, release: &VerifiedRelease) -> Result<Option<PathBuf>> {
    let current = install_root.join("current");
    let stage = install_root.join("staged");
    let previous = install_root.join("previous");
    ensure!(stage.is_dir(), "no staged update exists");
    ensure!(
        !previous.exists(),
        "previous version is retained; move it to an archive before replacing it"
    );
    let manifest = release.manifest();
    // Recheck against the signature-verified release, never trust mutable stage metadata.
    verify_artifact(&std::fs::read(stage.join(&manifest.artifact))?, manifest)?;
    let had_current = current.exists();
    if had_current {
        std::fs::rename(&current, &previous).context("back up installed client")?;
    }
    if let Err(error) = std::fs::rename(&stage, &current) {
        if had_current {
            std::fs::rename(&previous, &current)
                .context("activation failed and restoring previous client also failed")?;
        }
        return Err(error).context("activate staged client; previous client restored");
    }
    Ok(had_current.then_some(previous))
}

pub fn rollback(install_root: &Path) -> Result<PathBuf> {
    let current = install_root.join("current");
    let previous = install_root.join("previous");
    let failed = install_root.join("failed");
    ensure!(previous.is_dir(), "no previous version is available");
    ensure!(
        !failed.exists(),
        "failed version is already retained; archive it before another rollback"
    );
    let had_current = current.exists();
    if had_current {
        std::fs::rename(&current, &failed)?;
    }
    if let Err(error) = std::fs::rename(&previous, &current) {
        if had_current {
            std::fs::rename(&failed, &current)
                .context("rollback and restoring current both failed")?;
        }
        return Err(error).context("restore previous client");
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    fn manifest() -> ReleaseManifest {
        ReleaseManifest {
            schema: 1,
            version: "0.2.0".into(),
            channel: "stable".into(),
            platform: "windows-x86_64".into(),
            artifact: "void-client.exe".into(),
            url: "https://example.com/client.exe".into(),
            sha256: hex::encode(Sha256::digest(b"client")),
            size: 6,
        }
    }
    #[test]
    fn running_client_lock_excludes_update_and_rollback() {
        let temp = tempfile::tempdir().unwrap();
        let first = lock_for_launch(temp.path()).unwrap();
        let second = lock_for_launch(temp.path()).unwrap();
        assert!(lock_for_update(temp.path()).is_err());
        drop(first);
        assert!(lock_for_update(temp.path()).is_err());
        drop(second);
        let update = lock_for_update(temp.path()).unwrap();
        assert!(lock_for_update(temp.path()).is_err());
        drop(update);
        assert!(lock_for_update(temp.path()).is_ok());
    }
    #[test]
    fn tampered_stage_cannot_replace_current_even_with_modified_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("staged");
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("void-client.exe"), b"evil!!").unwrap();
        let mut tampered = manifest();
        tampered.sha256 = hex::encode(Sha256::digest(b"evil!!"));
        std::fs::write(
            stage.join("release.json"),
            serde_json::to_vec(&tampered).unwrap(),
        )
        .unwrap();
        assert!(
            activate_staged(
                temp.path(),
                &VerifiedRelease {
                    manifest: manifest()
                }
            )
            .is_err()
        );
        assert!(!temp.path().join("current").exists());
    }
    #[test]
    fn verifies_signature_channel_platform_and_rejects_downgrade() {
        let signing = SigningKey::from_bytes(&[42; 32]);
        let payload = serde_json::to_vec(&manifest()).unwrap();
        let envelope = SignedEnvelope {
            payload: STANDARD.encode(&payload),
            signature: STANDARD.encode(signing.sign(&payload).to_bytes()),
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let key = signing.verifying_key().to_bytes();
        assert!(verify_manifest(&bytes, &key, "stable", "windows-x86_64", "0.1.0").is_ok());
        assert!(verify_manifest(&bytes, &key, "preview", "windows-x86_64", "0.1.0").is_err());
        assert!(verify_manifest(&bytes, &key, "stable", "linux-x86_64", "0.1.0").is_err());
        assert!(verify_manifest(&bytes, &key, "stable", "windows-x86_64", "0.2.0").is_err());
        assert!(verify_manifest(&bytes, &[0; 32], "stable", "windows-x86_64", "0.1.0").is_err());
    }
    #[test]
    fn activation_and_rollback_preserve_settings() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir(root.join("current")).unwrap();
        std::fs::write(root.join("current/old"), "old").unwrap();
        std::fs::write(root.join("settings.toml"), "keep = true").unwrap();
        std::fs::create_dir(root.join("staged")).unwrap();
        std::fs::write(root.join("staged/void-client.exe"), b"client").unwrap();
        std::fs::write(
            root.join("staged/release.json"),
            serde_json::to_vec(&manifest()).unwrap(),
        )
        .unwrap();
        activate_staged(
            root,
            &VerifiedRelease {
                manifest: manifest(),
            },
        )
        .unwrap();
        assert!(root.join("current/void-client.exe").exists());
        rollback(root).unwrap();
        assert!(root.join("current/old").exists());
        assert!(root.join("failed/void-client.exe").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("settings.toml")).unwrap(),
            "keep = true"
        );
    }
}
