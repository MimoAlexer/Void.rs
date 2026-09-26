//! Mojang assets are read or downloaded into a content-addressed cache.
//! Invoke this service from an async worker; never from a render callback.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub const VERSION_MANIFEST: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const MINECRAFT_VERSION: &str = "26.2";
const MAX_METADATA_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Download {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub url: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VersionMetadata {
    pub id: String,
    #[serde(rename = "assetIndex")]
    pub asset_index: AssetIndexRef,
    pub downloads: BTreeMap<String, Download>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AssetIndex {
    pub objects: BTreeMap<String, AssetObject>,
}
#[derive(Clone, Debug, Default)]
pub struct AssetProgress {
    pub total: usize,
    pub cached: usize,
    pub imported: usize,
    pub downloaded: usize,
    pub bytes_downloaded: u64,
}

pub struct AssetManager {
    root: PathBuf,
    client: reqwest::Client,
}
impl AssetManager {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            root: root.into(),
            client: reqwest::Client::builder()
                .https_only(true)
                .timeout(Duration::from_secs(120))
                .user_agent("Void.rs/0.1")
                .build()?,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Metadata SHA-1 is anchored in Mojang's HTTPS manifest, then checked on disk.
    pub async fn fetch_version(&self) -> Result<VersionMetadata> {
        #[derive(Deserialize)]
        struct Manifest {
            versions: Vec<Version>,
        }
        #[derive(Deserialize)]
        struct Version {
            id: String,
            url: String,
            sha1: String,
        }
        let manifest: Manifest = serde_json::from_slice(
            &self
                .download_bounded(VERSION_MANIFEST, MAX_METADATA_BYTES)
                .await?,
        )?;
        let version = manifest
            .versions
            .into_iter()
            .find(|v| v.id == MINECRAFT_VERSION)
            .context("Mojang manifest does not contain Minecraft 26.2")?;
        let bytes = self
            .download_bounded(&version.url, MAX_METADATA_BYTES)
            .await?;
        verify_bytes(&bytes, &version.sha1, None)?;
        let metadata: VersionMetadata = serde_json::from_slice(&bytes)?;
        ensure!(
            metadata.id == MINECRAFT_VERSION,
            "version metadata identity mismatch"
        );
        atomic_write(
            &self
                .root
                .join("versions")
                .join(MINECRAFT_VERSION)
                .join("version.json"),
            &bytes,
        )?;
        Ok(metadata)
    }
    pub fn cached_version(&self) -> Result<VersionMetadata> {
        let bytes = std::fs::read(
            self.root
                .join("versions")
                .join(MINECRAFT_VERSION)
                .join("version.json"),
        )?;
        let metadata: VersionMetadata = serde_json::from_slice(&bytes)?;
        ensure!(
            metadata.id == MINECRAFT_VERSION,
            "cached version identity mismatch"
        );
        Ok(metadata)
    }
    pub async fn fetch_index(&self, metadata: &VersionMetadata) -> Result<AssetIndex> {
        let reference = &metadata.asset_index;
        safe_name(&reference.id)?;
        let path = self
            .root
            .join("assets/indexes")
            .join(format!("{}.json", reference.id));
        let bytes = if verify_file(&path, &reference.sha1, reference.size).unwrap_or(false) {
            std::fs::read(path)?
        } else {
            ensure!(
                reference.size <= MAX_METADATA_BYTES as u64,
                "asset index exceeds size limit"
            );
            let bytes = self
                .download_bounded(&reference.url, reference.size as usize)
                .await?;
            verify_bytes(&bytes, &reference.sha1, Some(reference.size))?;
            atomic_write(&path, &bytes)?;
            bytes
        };
        let index: AssetIndex = serde_json::from_slice(&bytes)?;
        for object in index.objects.values() {
            validate_hash(&object.hash)?;
        }
        Ok(index)
    }
    pub fn object_path(&self, hash: &str) -> Result<PathBuf> {
        validate_hash(hash)?;
        Ok(self.root.join("assets/objects").join(&hash[..2]).join(hash))
    }
    /// Reuse only byte-verified files from an optional existing .minecraft folder.
    pub async fn prepare_assets(
        &self,
        metadata: &VersionMetadata,
        installed_minecraft: Option<&Path>,
        mut progress: impl FnMut(&AssetProgress),
    ) -> Result<AssetProgress> {
        let index = self.fetch_index(metadata).await?;
        let mut state = AssetProgress {
            total: index.objects.len(),
            ..Default::default()
        };
        progress(&state);
        for object in index.objects.values() {
            let target = self.object_path(&object.hash)?;
            if verify_file(&target, &object.hash, object.size).unwrap_or(false) {
                state.cached += 1;
            } else {
                let installed = installed_minecraft.map(|root| {
                    root.join("assets/objects")
                        .join(&object.hash[..2])
                        .join(&object.hash)
                });
                if let Some(source) = installed
                    .filter(|path| verify_file(path, &object.hash, object.size).unwrap_or(false))
                {
                    let bytes = std::fs::read(source)?;
                    // Recheck after reading to handle concurrent installation changes.
                    verify_bytes(&bytes, &object.hash, Some(object.size))?;
                    atomic_write(&target, &bytes)?;
                    state.imported += 1;
                } else {
                    ensure!(
                        object.size <= 256 * 1024 * 1024,
                        "individual asset exceeds 256 MiB limit"
                    );
                    let url = format!(
                        "https://resources.download.minecraft.net/{}/{}",
                        &object.hash[..2],
                        object.hash
                    );
                    let bytes = self.download_bounded(&url, object.size as usize).await?;
                    verify_bytes(&bytes, &object.hash, Some(object.size))?;
                    atomic_write(&target, &bytes)?;
                    state.downloaded += 1;
                    state.bytes_downloaded += object.size;
                }
            }
            progress(&state);
        }
        Ok(state)
    }
    /// Fetch the original client JAR for textures/models/data; never execute it.
    pub async fn prepare_client_archive(
        &self,
        metadata: &VersionMetadata,
        installed_minecraft: Option<&Path>,
    ) -> Result<PathBuf> {
        let download = metadata
            .downloads
            .get("client")
            .context("version has no client asset archive")?;
        ensure!(
            download.size <= 256 * 1024 * 1024,
            "client archive exceeds size limit"
        );
        let target = self
            .root
            .join("versions")
            .join(MINECRAFT_VERSION)
            .join("client.jar");
        if verify_file(&target, &download.sha1, download.size).unwrap_or(false) {
            return Ok(target);
        }
        if let Some(root) = installed_minecraft {
            let source = root
                .join("versions")
                .join(MINECRAFT_VERSION)
                .join(format!("{MINECRAFT_VERSION}.jar"));
            if verify_file(&source, &download.sha1, download.size).unwrap_or(false) {
                let bytes = std::fs::read(source)?;
                verify_bytes(&bytes, &download.sha1, Some(download.size))?;
                atomic_write(&target, &bytes)?;
                return Ok(target);
            }
        }
        let bytes = self
            .download_bounded(&download.url, download.size as usize)
            .await?;
        verify_bytes(&bytes, &download.sha1, Some(download.size))?;
        atomic_write(&target, &bytes)?;
        Ok(target)
    }
    async fn download_bounded(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let mut response = self.client.get(url).send().await?.error_for_status()?;
        if let Some(length) = response.content_length() {
            ensure!(length <= max_bytes as u64, "download exceeds expected size");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= max_bytes,
                "download exceeds expected size"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}
pub fn validate_hash(hash: &str) -> Result<()> {
    ensure!(
        hash.len() == 40
            && hash
                .bytes()
                .all(|x| x.is_ascii_hexdigit() && !x.is_ascii_uppercase()),
        "invalid lowercase SHA-1 object hash"
    );
    Ok(())
}
pub fn verify_bytes(bytes: &[u8], expected_sha1: &str, expected_size: Option<u64>) -> Result<()> {
    validate_hash(expected_sha1)?;
    if let Some(size) = expected_size {
        ensure!(bytes.len() as u64 == size, "asset size mismatch");
    }
    ensure!(
        hex::encode(Sha1::digest(bytes)) == expected_sha1,
        "asset SHA-1 mismatch"
    );
    Ok(())
}
pub fn verify_file(path: &Path, expected_sha1: &str, expected_size: u64) -> Result<bool> {
    validate_hash(expected_sha1)?;
    if !path.is_file() {
        return Ok(false);
    }
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() != expected_size {
        return Ok(false);
    }
    let mut hasher = Sha1::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hasher.update(&buffer[..size]);
    }
    Ok(hex::encode(hasher.finalize()) == expected_sha1)
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("destination has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|error| error.error)
        .context("atomically publish verified file")?;
    Ok(())
}
fn safe_name(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|x| x.is_ascii_alphanumeric() || b"._-".contains(&x))
        || value == "."
        || value == ".."
    {
        bail!("unsafe asset index identifier");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_corrupt_bytes_and_path_traversal() {
        let digest = hex::encode(Sha1::digest(b"hello"));
        assert!(verify_bytes(b"hello", &digest, Some(5)).is_ok());
        assert!(verify_bytes(b"Hello", &digest, Some(5)).is_err());
        assert!(
            AssetManager::new("cache")
                .unwrap()
                .object_path("../../escape")
                .is_err()
        );
    }
    #[test]
    fn publication_replaces_old_asset_and_preserves_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects/test");
        atomic_write(&path, b"old").unwrap();
        atomic_write(&path, b"new content").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new content");
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }
}
