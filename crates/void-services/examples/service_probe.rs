//! Explicit live verification; ordinary unit tests do not download proprietary assets.
use anyhow::{Context, Result, bail};
use std::path::Path;
use void_sdk::FrameMetrics;
#[cfg(feature = "wasm")]
use void_services::mods::ModLimits;
use void_services::{assets::AssetManager, mods::ModManifest};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next().context(
        "usage: service_probe assets CACHE | wasm MOD_DIRECTORY | trusted-native MOD_DIRECTORY",
    )?;
    let path = args.next().context("missing path")?;
    match command.as_str() {
        "assets" => {
            let manager = AssetManager::new(path)?;
            let metadata = manager.fetch_version().await?;
            let index = manager.fetch_index(&metadata).await?;
            println!(
                "Verified Mojang {} metadata, asset index {} with {} objects",
                metadata.id,
                metadata.asset_index.id,
                index.objects.len()
            );
        }
        #[cfg(feature = "wasm")]
        "wasm" => {
            let manifest = ModManifest::read(&Path::new(&path).join("mod.toml"))?;
            let mut module = void_services::mods::WasmMod::load(
                Path::new(&path),
                manifest,
                ModLimits::default(),
            )?;
            let output = module.on_frame(FrameMetrics {
                fps: 240.0,
                ping_ms: 28,
                ..Default::default()
            })?;
            println!(
                "{} emitted {} HUD commands: {:?}",
                module.manifest.id,
                output.len(),
                output
            );
        }
        "trusted-native" => {
            let manifest = ModManifest::read(&Path::new(&path).join("mod.toml"))?;
            // SAFETY: This developer-only command explicitly requests execution of
            // a trusted library. Use only with the in-repository example you built.
            let mut module = unsafe {
                void_services::mods::NativeMod::load_trusted(Path::new(&path), manifest)?
            };
            let output = module.on_frame(FrameMetrics {
                fps: 240.0,
                ping_ms: 28,
                ..Default::default()
            })?;
            println!(
                "{} emitted {} HUD commands: {:?}",
                module.manifest.id,
                output.len(),
                output
            );
        }
        _ => bail!("unknown command: {command}"),
    }
    Ok(())
}
