use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use void_services::{
    assets::AssetManager,
    auth::{self, AccountMetadata, AuthConfig},
    updates,
};

#[derive(Parser)]
#[command(version, about = "Void.rs launcher, verified assets and account setup")]
struct Args {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand)]
enum Commands {
    /// Start the installed client or the sibling development executable, then exit.
    Launch {
        #[arg(last = true)]
        args: Vec<OsString>,
    },
    /// Write a documented default launcher configuration without replacing a file.
    Init,
    /// Verify and prepare Mojang 26.2 assets, importing a local installation if given.
    Assets {
        #[arg(long)]
        minecraft_dir: Option<PathBuf>,
        #[arg(long)]
        metadata_only: bool,
    },
    /// Open Microsoft sign-in using the registered application in launcher.toml.
    Login,
    /// Refresh a saved account, checking ownership and the Java profile again.
    Refresh { account: uuid::Uuid },
    /// List stored public account metadata. Tokens are never printed.
    Accounts,
    /// Forget a saved account and remove its operating-system credential.
    Forget { account: uuid::Uuid },
    /// Verify a signed HTTPS release manifest, download, and activate between sessions.
    InstallUpdate { url: String },
    /// Restore the retained previous executable, preserving settings and mods.
    Rollback,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    schema: u32,
    channel: String,
    install_root: PathBuf,
    asset_cache: PathBuf,
    client_path: Option<PathBuf>,
    release_public_key: Option<String>,
    microsoft: Option<AuthConfig>,
}
impl Default for Config {
    fn default() -> Self {
        let data = ProjectDirs::from("rs", "Void", "Void.rs")
            .map(|dirs| dirs.data_local_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".void"));
        Self {
            schema: 1,
            channel: "stable".into(),
            install_root: data.join("install"),
            asset_cache: data.join("cache"),
            client_path: None,
            release_public_key: None,
            microsoft: None,
        }
    }
}
impl Config {
    fn load(path: &Path) -> Result<Self> {
        let mut value = if path.is_file() {
            toml::from_str(&std::fs::read_to_string(path)?).context("invalid launcher TOML")?
        } else {
            Self::default()
        };
        ensure!(
            value.schema == 1,
            "unsupported launcher schema {}",
            value.schema
        );
        ensure!(
            matches!(value.channel.as_str(), "stable" | "preview"),
            "channel must be stable or preview"
        );
        let base = path.parent().context("configuration path has no parent")?;
        value.install_root = resolve_path(base, &value.install_root);
        value.asset_cache = resolve_path(base, &value.asset_cache);
        value.client_path = value.client_path.map(|path| resolve_path(base, &path));
        Ok(value)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config_path = absolute(&args.config.unwrap_or_else(default_config_path))?;
    let config = Config::load(&config_path)?;
    match args
        .command
        .unwrap_or(Commands::Launch { args: Vec::new() })
    {
        Commands::Init => {
            ensure!(
                !config_path.exists(),
                "configuration already exists: {}",
                config_path.display()
            );
            let mut text = toml::to_string_pretty(&config)?;
            text.push_str("\n# Pin the actual publisher's Ed25519 public key (64 lowercase hex characters).\n# release_public_key = \"\"\n\n# Register your own Microsoft public/native application before enabling login.\n# [microsoft]\n# client_id = \"YOUR-REGISTERED-APPLICATION-ID\"\n# redirect_uri = \"http://localhost:43189/callback\"\n");
            write_atomic(&config_path, text.as_bytes())?;
            println!("Created {}", config_path.display());
        }
        Commands::Launch { args } => launch(&config, &args)?,
        Commands::Assets {
            minecraft_dir,
            metadata_only,
        } => {
            let manager = AssetManager::new(&config.asset_cache)?;
            let metadata = manager.fetch_version().await?;
            let index = manager.fetch_index(&metadata).await?;
            println!(
                "Verified Minecraft {} metadata and {} asset objects",
                metadata.id,
                index.objects.len()
            );
            if !metadata_only {
                let mut last = 0;
                let progress = manager
                    .prepare_assets(&metadata, minecraft_dir.as_deref(), |progress| {
                        let completed = progress.cached + progress.imported + progress.downloaded;
                        if completed == progress.total || completed >= last + 250 {
                            println!("Assets {completed}/{}", progress.total);
                            last = completed;
                        }
                    })
                    .await?;
                let archive = manager
                    .prepare_client_archive(&metadata, minecraft_dir.as_deref())
                    .await?;
                println!(
                    "Ready: {} cached, {} imported, {} downloaded. Client resource archive: {}",
                    progress.cached,
                    progress.imported,
                    progress.downloaded,
                    archive.display()
                );
            }
        }
        Commands::Login => {
            let auth_config = config.microsoft.context("configure your registered Microsoft application in launcher.toml; see docs/ACCOUNTS.md")?;
            let flow = auth::begin_sign_in(auth_config).await?;
            flow.open_browser()?;
            println!(
                "Complete Microsoft sign-in in your browser. Waiting for the local callback..."
            );
            let (session, refresh) = flow.finish().await?;
            auth::save_refresh_token(session.account.uuid, &refresh)?;
            let mut accounts = read_accounts(&config_path)?;
            accounts.retain(|value| value.uuid != session.account.uuid);
            accounts.push(session.account.clone());
            write_accounts(&config_path, &accounts)?;
            println!(
                "Signed in as {} ({})",
                session.account.username, session.account.uuid
            );
        }
        Commands::Refresh { account } => {
            let auth_config = config
                .microsoft
                .context("Microsoft application configuration is missing")?;
            let refresh = auth::load_refresh_token(account)?;
            let (session, rotated) = auth::refresh_account(&auth_config, &refresh).await?;
            ensure!(
                session.account.uuid == account,
                "refreshed account identity mismatch"
            );
            auth::save_refresh_token(account, &rotated)?;
            let mut accounts = read_accounts(&config_path)?;
            accounts.retain(|value| value.uuid != account);
            accounts.push(session.account.clone());
            write_accounts(&config_path, &accounts)?;
            println!(
                "Verified {} ({})",
                session.account.username, session.account.uuid
            );
        }
        Commands::Accounts => {
            for account in read_accounts(&config_path)? {
                println!("{} {}", account.uuid, account.username);
            }
        }
        Commands::Forget { account } => {
            auth::forget_account(account)?;
            let mut accounts = read_accounts(&config_path)?;
            accounts.retain(|value| value.uuid != account);
            write_accounts(&config_path, &accounts)?;
            println!("Removed account {account}");
        }
        Commands::InstallUpdate { url } => {
            let public_key = parse_public_key(config.release_public_key.as_deref().context(
                "no release verification key is pinned in launcher.toml; updates are disabled",
            )?)?;
            let installed_version = installed_version(&config.install_root)?;
            let envelope = download_manifest(&url).await?;
            let release = updates::verify_manifest(
                &envelope,
                &public_key,
                &config.channel,
                platform(),
                &installed_version,
            )?;
            ensure!(
                release.manifest().artifact == client_filename(),
                "release artifact must be {}",
                client_filename()
            );
            let _guard = updates::lock_for_update(&config.install_root)?;
            let staged = updates::stage_update(&release, &config.install_root).await?;
            // Keep the signature for verification on future launches and updates.
            write_atomic(&staged.join("signed-release.json"), &envelope)?;
            updates::activate_staged(&config.install_root, &release)?;
            println!(
                "Installed Void.rs {} ({})",
                release.manifest().version,
                release.manifest().channel
            );
        }
        Commands::Rollback => {
            let _guard = updates::lock_for_update(&config.install_root)?;
            let restored = updates::rollback(&config.install_root)?;
            println!("Restored {}", restored.display());
        }
    }
    Ok(())
}

fn default_config_path() -> PathBuf {
    ProjectDirs::from("rs", "Void", "Void.rs")
        .map(|dirs| dirs.config_dir().join("launcher.toml"))
        .unwrap_or_else(|| PathBuf::from("launcher.toml"))
}
fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}
fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(resolve_path(&std::env::current_dir()?, path))
}
fn client_filename() -> &'static str {
    if cfg!(windows) {
        "void-client.exe"
    } else {
        "void-client"
    }
}
fn platform() -> &'static str {
    if cfg!(windows) {
        "windows-x86_64"
    } else {
        "linux-x86_64"
    }
}
fn choose_client(config: &Config, launcher_exe: &Path) -> Result<PathBuf> {
    if let Some(path) = &config.client_path {
        ensure!(
            path.is_file(),
            "configured client does not exist: {}",
            path.display()
        );
        return Ok(path.clone());
    }
    let installed = config.install_root.join("current").join(client_filename());
    if installed.is_file() {
        return Ok(installed);
    }
    let sibling = launcher_exe
        .parent()
        .context("launcher has no parent directory")?
        .join(client_filename());
    ensure!(
        sibling.is_file(),
        "no installed client or sibling {} found; build with cargo build --release -p void-client or install a signed release",
        client_filename()
    );
    Ok(sibling)
}
fn launch_command(client: &Path, client_args: &[OsString]) -> Command {
    let mut command = Command::new(client);
    command.args(client_args);
    command
}
fn launch(config: &Config, args: &[OsString]) -> Result<()> {
    let executable = choose_client(config, &std::env::current_exe()?)?.canonicalize()?;
    ensure!(
        executable != std::env::current_exe()?.canonicalize()?,
        "client_path points back to the launcher"
    );
    let _guard = updates::lock_for_launch(&config.install_root)?;
    let marker = tempfile::Builder::new()
        .prefix(".void-startup-")
        .tempfile_in(&config.install_root)?;
    let mut command = launch_command(&executable, args);
    command.env(
        "VOID_INSTALL_LOCK",
        config.install_root.join(".client.lock"),
    );
    command.env("VOID_LAUNCH_READY", marker.path());
    let mut child = command
        .spawn()
        .with_context(|| format!("start {}", executable.display()))?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        if std::fs::read(marker.path())? == b"ready" {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "client exited with {status}");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // Do not let an uncooperative child continue without a lifetime update guard.
    child
        .kill()
        .context("client did not acknowledge its install lock; stop it manually before updating")?;
    let _ = child.wait();
    bail!(
        "client did not acknowledge its install lock within 15 seconds; use a compatible client build"
    )
}
fn installed_version(root: &Path) -> Result<String> {
    let path = root.join("current/release.json");
    if !path.is_file() {
        return Ok(env!("CARGO_PKG_VERSION").into());
    }
    let manifest: updates::ReleaseManifest = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(manifest.version)
}
fn parse_public_key(value: &str) -> Result<[u8; 32]> {
    hex::decode(value)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("release public key must be 32 bytes (64 hex characters)"))
}
async fn download_manifest(url: &str) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(30))
        .user_agent("Void.rs/0.1")
        .build()?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= 64 * 1024,
            "release manifest exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
#[derive(Default, Deserialize, Serialize)]
struct Accounts {
    #[serde(default)]
    accounts: Vec<AccountMetadata>,
}
fn accounts_path(config: &Path) -> PathBuf {
    config.with_file_name("accounts.toml")
}
fn read_accounts(config: &Path) -> Result<Vec<AccountMetadata>> {
    let path = accounts_path(config);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    Ok(toml::from_str::<Accounts>(&std::fs::read_to_string(path)?)?.accounts)
}
fn write_accounts(config: &Path, accounts: &[AccountMetadata]) -> Result<()> {
    write_atomic(
        &accounts_path(config),
        toml::to_string_pretty(&Accounts {
            accounts: accounts.to_vec(),
        })?
        .as_bytes(),
    )
}
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("file has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_launch_and_exact_argument_forwarding() {
        assert!(
            Args::try_parse_from(["void-launcher"])
                .unwrap()
                .command
                .is_none()
        );
        let parsed = Args::try_parse_from([
            "void-launcher",
            "launch",
            "--",
            "--config",
            "folder with spaces/client.toml",
        ])
        .unwrap();
        let Some(Commands::Launch { args }) = parsed.command else {
            panic!("expected launch");
        };
        let command = launch_command(Path::new("client folder/void-client.exe"), &args);
        assert_eq!(command.get_program(), "client folder/void-client.exe");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec!["--config", "folder with spaces/client.toml"]
        );
    }
    #[test]
    fn chooses_installed_then_sibling_and_resolves_relative_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let config = Config {
            install_root: root.join("install"),
            ..Default::default()
        };
        let sibling = root.join(client_filename());
        std::fs::write(&sibling, b"client").unwrap();
        assert_eq!(
            choose_client(&config, &root.join("launcher")).unwrap(),
            sibling
        );
        let current = config.install_root.join("current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join(client_filename()), b"installed").unwrap();
        assert_eq!(
            choose_client(&config, &root.join("launcher")).unwrap(),
            current.join(client_filename())
        );
        assert_eq!(resolve_path(root, Path::new("cache")), root.join("cache"));
    }
    #[test]
    fn invalid_key_config_and_token_cli_flags_are_rejected() {
        assert!(parse_public_key("1234").is_err());
        assert!(
            Args::try_parse_from(["void-launcher", "login", "--access-token", "secret"]).is_err()
        );
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("launcher.toml");
        std::fs::write(&path, "schema=1\nchannel='unknown'").unwrap();
        assert!(Config::load(&path).is_err());
    }
}
