mod app;
mod entities;
mod modules;
mod terrain;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use std::{path::PathBuf, time::Duration};
use void_core::{Config, ConfigStore};
use winit::event_loop::EventLoop;

#[derive(Parser, Debug)]
#[command(
    name = "Void.rs",
    version,
    about = "Minecraft Java 26.2 client — experimental development build"
)]
pub struct Args {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Query a real Minecraft server without opening a window.
    #[arg(long)]
    status: Option<String>,
    /// Exit after this many rendered frames (rendering smoke test).
    #[arg(long)]
    smoke_frames: Option<u64>,
    /// Capture an actual Vulkan framebuffer as PNG near the end of a smoke test.
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Save frame measurements as JSON on exit; not an input-to-photon measurement.
    #[arg(long)]
    metrics: Option<PathBuf>,
    #[arg(long)]
    connect: Option<String>,
    #[arg(long, default_value = "VoidPlayer")]
    username: String,
}

fn main() -> Result<()> {
    let _install_guard = void_services::updates::client_install_guard()?;
    let args = Args::parse();
    if let Some(ref address) = args.status {
        let address = void_protocol::ServerAddress::parse(address)?;
        let status = void_protocol::status(&address, Duration::from_secs(5))?;
        println!("{status:#?}");
        return Ok(());
    }
    let path = args.config.clone().unwrap_or_else(|| {
        directories::ProjectDirs::from("rs", "Void", "Void.rs")
            .map(|p| p.config_dir().join("config.toml"))
            .unwrap_or_else(|| PathBuf::from("config.toml"))
    });
    if !path.exists() {
        Config::default().save_atomic(&path)?;
    }
    let config = ConfigStore::load_or_default(&path)?;
    let event_loop = EventLoop::new()?;
    let mut app = app::App::new(args, config);
    event_loop.run_app(&mut app).context("window event loop")?;
    if let Some(err) = app.failure.take() {
        anyhow::bail!(err);
    }
    Ok(())
}
mod accounts;
