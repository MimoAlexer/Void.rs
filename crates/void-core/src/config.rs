use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("cannot serialize configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub graphics: GraphicsConfig,
    pub performance: PerformanceConfig,
    pub ui: UiConfig,
    pub controls: ControlsConfig,
    pub audio: AudioConfig,
    pub account: AccountConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_VERSION,
            graphics: GraphicsConfig::default(),
            performance: PerformanceConfig::default(),
            ui: UiConfig::default(),
            controls: ControlsConfig::default(),
            audio: AudioConfig::default(),
            account: AccountConfig::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphicsConfig {
    pub width: u32,
    pub height: u32,
    pub render_distance: u8,
    pub vsync: bool,
    pub fullscreen: bool,
    pub preferred_gpu: Option<String>,
}

impl Default for GraphicsConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            render_distance: 12,
            vsync: false,
            fullscreen: false,
            preferred_gpu: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PerformanceConfig {
    /// Zero means uncapped presentation. Scheduler estimates use 240 Hz then.
    pub target_fps: u32,
    pub event_horizon: EventHorizonConfig,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            target_fps: 240,
            event_horizon: EventHorizonConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchedulingMode {
    #[default]
    Baseline,
    FrameCoordination,
    EventHorizon,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EventHorizonConfig {
    pub mode: SchedulingMode,
    pub memory_budget_mib: u32,
    pub max_jobs: usize,
    pub nearby_deadline_frames: u32,
    pub aging_ms: u32,
    pub fairness_reserve_percent: u8,
    pub safety_margin_us: u32,
    pub max_frames_in_flight: u8,
}

impl Default for EventHorizonConfig {
    fn default() -> Self {
        Self {
            mode: SchedulingMode::Baseline,
            memory_budget_mib: 256,
            max_jobs: 4096,
            nearby_deadline_frames: 2,
            aging_ms: 250,
            fairness_reserve_percent: 10,
            safety_margin_us: 150,
            max_frames_in_flight: 2,
        }
    }
}

impl EventHorizonConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(1..=4096).contains(&self.memory_budget_mib) {
            return invalid("event_horizon.memory_budget_mib must be 1..=4096");
        }
        if !(1..=65536).contains(&self.max_jobs) {
            return invalid("event_horizon.max_jobs must be 1..=65536");
        }
        if !(1..=120).contains(&self.nearby_deadline_frames) {
            return invalid("event_horizon.nearby_deadline_frames must be 1..=120");
        }
        if !(1..=60_000).contains(&self.aging_ms) {
            return invalid("event_horizon.aging_ms must be 1..=60000");
        }
        if !(1..=50).contains(&self.fairness_reserve_percent) {
            return invalid("event_horizon.fairness_reserve_percent must be 1..=50");
        }
        if self.safety_margin_us > 100_000 {
            return invalid("event_horizon.safety_margin_us exceeds 100000");
        }
        if !(1..=2).contains(&self.max_frames_in_flight) {
            return invalid("event_horizon.max_frames_in_flight must be 1 or 2");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub scale: f32,
    pub accent: [f32; 3],
    pub background: [f32; 3],
    pub video_enabled: bool,
    pub video_path: Option<PathBuf>,
    pub show_diagnostics: bool,
    pub hud: BTreeMap<String, HudWidgetConfig>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            scale: 1.0,
            accent: [0.94, 0.64, 0.35],
            background: [0.015, 0.015, 0.018],
            video_enabled: true,
            video_path: None,
            show_diagnostics: true,
            hud: [
                ("performance".to_owned(), HudWidgetConfig::default()),
                (
                    "keystrokes".to_owned(),
                    HudWidgetConfig {
                        y: 100.0,
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ControlsConfig {
    pub mouse_sensitivity: f32,
    pub invert_y: bool,
    pub toggle_sprint: bool,
    pub fov_degrees: f32,
    pub bindings: KeyBindings,
}

impl Default for ControlsConfig {
    fn default() -> Self {
        Self {
            mouse_sensitivity: 0.5,
            invert_y: false,
            toggle_sprint: false,
            fov_degrees: 70.0,
            bindings: KeyBindings::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HudWidgetConfig {
    /// Logical UI points relative to the top-left corner.
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub enabled: bool,
}
impl Default for HudWidgetConfig {
    fn default() -> Self {
        Self {
            x: 16.0,
            y: 16.0,
            scale: 1.0,
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyBindings {
    /// Physical winit KeyCode names, or MouseLeft/MouseRight/MouseMiddle.
    pub forward: String,
    pub backward: String,
    pub left: String,
    pub right: String,
    pub jump: String,
    pub sprint: String,
    pub attack: String,
    pub use_item: String,
    pub zoom: String,
    pub menu: String,
}
impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            forward: "KeyW".into(),
            backward: "KeyS".into(),
            left: "KeyA".into(),
            right: "KeyD".into(),
            jump: "Space".into(),
            sprint: "ControlLeft".into(),
            attack: "MouseLeft".into(),
            use_item: "MouseRight".into(),
            zoom: "KeyC".into(),
            menu: "Escape".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccountConfig {
    /// Public application registration ID. Access/refresh tokens never belong here.
    pub client_id: String,
    pub redirect_uri: String,
}
impl Default for AccountConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            redirect_uri: "http://127.0.0.1:43189/callback".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    pub master_volume: f32,
}
impl Default for AudioConfig {
    fn default() -> Self {
        Self { master_volume: 1.0 }
    }
}

fn invalid<T>(message: &str) -> Result<T, ConfigError> {
    Err(ConfigError::Invalid(message.to_owned()))
}

impl Config {
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let mut value: toml::Value = toml::from_str(source)?;
        let table = value
            .as_table_mut()
            .ok_or_else(|| ConfigError::Invalid("expected a TOML table".into()))?;
        let version = table
            .get("schema_version")
            .map(|v| {
                v.as_integer()
                    .ok_or_else(|| ConfigError::Invalid("schema_version must be an integer".into()))
            })
            .transpose()?
            .unwrap_or(i64::from(CONFIG_VERSION));
        // Version 0 used graphics.frame_limit. A migration is in-memory until
        // explicitly saved so merely inspecting a config never rewrites it.
        if version == 0 {
            let fps = table
                .get_mut("graphics")
                .and_then(toml::Value::as_table_mut)
                .and_then(|g| g.remove("frame_limit"));
            if let Some(fps) = fps {
                let performance = table
                    .entry("performance".to_owned())
                    .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                let performance = performance
                    .as_table_mut()
                    .ok_or_else(|| ConfigError::Invalid("performance must be a table".into()))?;
                performance.entry("target_fps".to_owned()).or_insert(fps);
            }
            table.insert(
                "schema_version".into(),
                toml::Value::Integer(i64::from(CONFIG_VERSION)),
            );
        } else if version != i64::from(CONFIG_VERSION) {
            return invalid(&format!(
                "unsupported schema_version {version}; expected {CONFIG_VERSION}"
            ));
        }
        let config: Self = value.try_into()?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != CONFIG_VERSION {
            return invalid("unsupported schema_version");
        }
        if !(320..=16384).contains(&self.graphics.width)
            || !(240..=16384).contains(&self.graphics.height)
        {
            return invalid("window size must be 320x240 through 16384x16384");
        }
        // Vanilla 26.2 Options bytecode, official client SHA1
        // 2dc72797acbc1b63fc16a11c4ac393605f453754: min 2, max 32, default 12.
        if !(2..=32).contains(&self.graphics.render_distance) {
            return invalid("render_distance must be 2..=32 chunks");
        }
        if self.performance.target_fps > 2000 {
            return invalid("target_fps must be 0 (uncapped) or at most 2000");
        }
        self.performance.event_horizon.validate()?;
        if !self.ui.scale.is_finite() || !(0.5..=4.0).contains(&self.ui.scale) {
            return invalid("ui.scale must be finite and 0.5..=4.0");
        }
        if self
            .ui
            .accent
            .iter()
            .chain(self.ui.background.iter())
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return invalid("UI colors must contain finite values in 0..=1");
        }
        if self.ui.hud.len() > 256
            || self.ui.hud.iter().any(|(name, widget)| {
                name.is_empty()
                    || name.len() > 128
                    || !widget.x.is_finite()
                    || !widget.y.is_finite()
                    || widget.x.abs() > 32768.0
                    || widget.y.abs() > 32768.0
                    || !widget.scale.is_finite()
                    || !(0.5..=4.0).contains(&widget.scale)
            })
        {
            return invalid(
                "HUD requires at most 256 named widgets with finite positions and scale 0.5..=4.0",
            );
        }
        let bindings = &self.controls.bindings;
        if [
            &bindings.forward,
            &bindings.backward,
            &bindings.left,
            &bindings.right,
            &bindings.jump,
            &bindings.sprint,
            &bindings.attack,
            &bindings.use_item,
            &bindings.zoom,
            &bindings.menu,
        ]
        .into_iter()
        .any(|binding| {
            binding.is_empty()
                || binding.len() > 64
                || !binding.chars().all(|c| c.is_ascii_alphanumeric())
        }) {
            return invalid(
                "keybindings must be physical KeyCode or Mouse names, 1..=64 ASCII alphanumeric characters",
            );
        }
        if self.account.client_id.len() > 128 || self.account.redirect_uri.len() > 2048 {
            return invalid("account client ID or redirect URI exceeds supported length");
        }
        if !self.controls.mouse_sensitivity.is_finite()
            || !(0.0..=2.0).contains(&self.controls.mouse_sensitivity)
        {
            return invalid("mouse_sensitivity must be finite and 0..=2");
        }
        if !self.controls.fov_degrees.is_finite()
            || !(30.0..=110.0).contains(&self.controls.fov_degrees)
        {
            return invalid("fov_degrees must be finite and 30..=110");
        }
        if !self.audio.master_volume.is_finite() || !(0.0..=1.0).contains(&self.audio.master_volume)
        {
            return invalid("master_volume must be finite and 0..=1");
        }
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Self::from_toml(&fs::read_to_string(path)?)
    }

    /// Write, flush, and atomically replace in the destination directory.
    /// A failed validation/write never destroys the previous configuration.
    pub fn save_atomic(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        self.validate()?;
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(path).map_err(|e| ConfigError::Io(e.error))?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}

/// Consumers apply live-safe fields themselves; restart_required covers renderer
/// selection and presentation changes that need coordinated reinitialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reload {
    pub changed: bool,
    pub restart_required: bool,
}

pub struct ConfigStore {
    path: PathBuf,
    current: Config,
}
impl ConfigStore {
    /// Missing files use defaults, but corrupt existing files are reported.
    pub fn load_or_default(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let path = path.into();
        let current = match Config::load(&path) {
            Ok(config) => config,
            Err(ConfigError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                Config::default()
            }
            Err(error) => return Err(error),
        };
        Ok(Self { path, current })
    }
    pub fn current(&self) -> &Config {
        &self.current
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn reload(&mut self) -> Result<Reload, ConfigError> {
        let next = Config::load(&self.path)?;
        let result = Reload {
            changed: next != self.current,
            restart_required: next.graphics != self.current.graphics
                || next.performance != self.current.performance,
        };
        self.current = next;
        Ok(result)
    }
    pub fn replace_and_save(&mut self, next: Config) -> Result<(), ConfigError> {
        next.save_atomic(&self.path)?;
        self.current = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_reload_keeps_last_good_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("client.toml");
        Config::default().save_atomic(&path).unwrap();
        let mut store = ConfigStore::load_or_default(&path).unwrap();
        fs::write(&path, "[performance]\ntarget_fps = -1").unwrap();
        assert!(store.reload().is_err());
        assert_eq!(store.current(), &Config::default());
        let mut changed = Config::default();
        changed.ui.scale = 2.0;
        changed.save_atomic(&path).unwrap();
        assert_eq!(
            store.reload().unwrap(),
            Reload {
                changed: true,
                restart_required: false
            }
        );
    }
    #[test]
    fn invalid_save_cannot_replace_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("client.toml");
        Config::default().save_atomic(&path).unwrap();
        let before = fs::read(&path).unwrap();
        let mut broken = Config::default();
        broken.ui.scale = f32::NAN;
        assert!(broken.save_atomic(&path).is_err());
        assert_eq!(before, fs::read(&path).unwrap());
        assert_eq!(Config::load(&path).unwrap(), Config::default());
    }
    #[test]
    fn migration_and_strict_validation() {
        let config =
            Config::from_toml("schema_version = 0\n[graphics]\nframe_limit = 144").unwrap();
        assert_eq!(config.performance.target_fps, 144);
        assert_eq!(config.schema_version, 1);
        assert!(Config::from_toml("schema_version=999").is_err());
        assert!(Config::from_toml("[controls]\nmouse_sensitivty=1").is_err());
        assert!(Config::from_toml("[performance.event_horizon]\nmemory_budget_mib=0").is_err());
    }

    #[test]
    fn published_defaults_match_schema_and_vanilla_render_distance_range() {
        let default_toml = include_str!("../../../config/default.toml");
        assert_eq!(Config::from_toml(default_toml).unwrap(), Config::default());
        assert!(Config::from_toml("[graphics]\nrender_distance=2").is_ok());
        assert!(Config::from_toml("[graphics]\nrender_distance=32").is_ok());
        assert!(Config::from_toml("[graphics]\nrender_distance=33").is_err());
        assert!(Config::from_toml("[graphics]\nrender_distance=1").is_err());
    }
}
