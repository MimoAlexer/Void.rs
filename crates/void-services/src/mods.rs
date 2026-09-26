use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};
use void_sdk::{
    ABI_VERSION, FrameMetrics, HostApiV1, HudText, MAX_HUD_COMMANDS, MAX_HUD_TEXT_BYTES, ModApiV1,
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Wasm,
    Native,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModManifest {
    pub id: String,
    pub version: String,
    pub sdk: u32,
    pub runtime: Runtime,
    pub entry: PathBuf,
    #[serde(default)]
    pub capabilities: BTreeSet<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub deferrable: bool,
    #[serde(default)]
    pub settings: toml::Table,
}

impl ModManifest {
    pub fn parse(source: &str) -> Result<Self> {
        let value: Self = toml::from_str(source).context("invalid mod manifest")?;
        value.validate()?;
        Ok(value)
    }
    pub fn read(path: &Path) -> Result<Self> {
        Self::parse(&std::fs::read_to_string(path)?)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_empty()
                && self.id.len() <= 64
                && self
                    .id
                    .bytes()
                    .all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || b"_-".contains(&x)),
            "mod id must use 1-64 lowercase ASCII letters, digits, '-' or '_'"
        );
        semver::Version::parse(&self.version).context("invalid mod version")?;
        ensure!(
            self.sdk == ABI_VERSION,
            "unsupported mod SDK {} (host {})",
            self.sdk,
            ABI_VERSION
        );
        ensure!(
            !self.entry.as_os_str().is_empty()
                && self
                    .entry
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "mod entry must be a relative path inside the mod directory"
        );
        for capability in &self.capabilities {
            ensure!(
                matches!(capability.as_str(), "hud" | "metrics"),
                "unsupported capability: {capability}"
            );
        }
        ensure!(
            self.dependencies.is_empty(),
            "dependency resolution is not implemented; install standalone mods only"
        );
        Ok(())
    }
    pub fn resolve_entry(&self, directory: &Path) -> Result<PathBuf> {
        self.validate()?;
        let root = directory.canonicalize()?;
        let file = root.join(&self.entry).canonicalize()?;
        ensure!(
            file.starts_with(&root),
            "mod entry escapes directory through a link"
        );
        Ok(file)
    }
}

#[derive(Clone, Debug)]
pub struct ModLimits {
    pub memory_bytes: usize,
    pub fuel_per_frame: u64,
    pub max_module_bytes: usize,
}
impl Default for ModLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 32 * 1024 * 1024,
            fuel_per_frame: 500_000,
            max_module_bytes: 32 * 1024 * 1024,
        }
    }
}

#[cfg(feature = "wasm")]
mod sandbox {
    use super::*;
    use wasmtime::error::{Context, bail, ensure};
    use wasmtime::{
        Caller, Config, Engine, Linker, Module, Result, Store, StoreLimits, StoreLimitsBuilder,
        TypedFunc,
    };

    struct State {
        limits: StoreLimits,
        metrics: FrameMetrics,
        commands: Vec<HudText>,
        capabilities: BTreeSet<String>,
        host_calls: usize,
    }
    pub struct WasmMod {
        pub manifest: ModManifest,
        store: Store<State>,
        frame: TypedFunc<(), ()>,
        fuel: u64,
        fault: Option<String>,
    }
    impl WasmMod {
        pub fn load(directory: &Path, manifest: ModManifest, limits: ModLimits) -> Result<Self> {
            let entry = manifest
                .resolve_entry(directory)
                .map_err(wasmtime::Error::from_anyhow)?;
            ensure!(
                entry.metadata()?.len() <= limits.max_module_bytes as u64,
                "Wasm module exceeds size limit"
            );
            Self::from_bytes(&std::fs::read(entry)?, manifest, limits)
        }
        pub fn from_bytes(bytes: &[u8], manifest: ModManifest, limits: ModLimits) -> Result<Self> {
            manifest.validate().map_err(wasmtime::Error::from_anyhow)?;
            ensure!(
                manifest.runtime == Runtime::Wasm,
                "manifest is not a Wasm mod"
            );
            ensure!(
                bytes.len() <= limits.max_module_bytes,
                "Wasm module exceeds size limit"
            );
            ensure!(
                limits.memory_bytes > 0 && limits.fuel_per_frame > 0,
                "mod limits must be positive"
            );
            let mut config = Config::new();
            config.consume_fuel(true);
            config.wasm_multi_memory(false);
            config.max_wasm_stack(512 * 1024);
            let engine = Engine::new(&config)?;
            let module = Module::from_binary(&engine, bytes).context("compile Wasm mod")?;
            let mut store = Store::new(
                &engine,
                State {
                    limits: StoreLimitsBuilder::new()
                        .memory_size(limits.memory_bytes)
                        .memories(1)
                        .instances(1)
                        .tables(1)
                        .table_elements(4096)
                        .trap_on_grow_failure(true)
                        .build(),
                    metrics: FrameMetrics::default(),
                    commands: Vec::new(),
                    capabilities: manifest.capabilities.clone(),
                    host_calls: 0,
                },
            );
            store.limiter(|s| &mut s.limits);
            store.set_fuel(limits.fuel_per_frame)?;
            let mut linker = Linker::new(&engine);
            linker.func_wrap(
                "void",
                "metric",
                |mut caller: Caller<'_, State>, index: i32| -> Result<f32> {
                    consume_host_call(&mut caller)?;
                    ensure!(
                        caller.data().capabilities.contains("metrics"),
                        "metrics permission denied"
                    );
                    let m = caller.data().metrics;
                    Ok(match index {
                        0 => m.fps,
                        1 => m.ping_ms as f32,
                        2 => m.left_cps,
                        3 => m.right_cps,
                        4 => m.pressed_keys as f32,
                        _ => 0.0,
                    })
                },
            )?;
            linker.func_wrap(
                "void",
                "hud_text",
                |mut caller: Caller<'_, State>,
                 x: f32,
                 y: f32,
                 rgba: i32,
                 ptr: i32,
                 len: i32|
                 -> Result<i32> {
                    consume_host_call(&mut caller)?;
                    ensure!(
                        caller.data().capabilities.contains("hud"),
                        "HUD permission denied"
                    );
                    ensure!(x.is_finite() && y.is_finite(), "non-finite HUD position");
                    ensure!(
                        ptr >= 0 && len >= 0 && len as usize <= MAX_HUD_TEXT_BYTES,
                        "invalid HUD text length or pointer"
                    );
                    ensure!(
                        caller.data().commands.len() < MAX_HUD_COMMANDS,
                        "HUD command budget exhausted"
                    );
                    let memory = caller
                        .get_export("memory")
                        .and_then(|e| e.into_memory())
                        .context("mod must export memory")?;
                    let mut bytes = vec![0; len as usize];
                    memory
                        .read(&caller, ptr as usize, &mut bytes)
                        .context("HUD text outside guest memory")?;
                    let text = String::from_utf8(bytes).context("HUD text must be UTF-8")?;
                    caller.data_mut().commands.push(HudText {
                        x,
                        y,
                        rgba: rgba as u32,
                        text,
                    });
                    Ok(0)
                },
            )?;
            let instance = linker
                .instantiate(&mut store, &module)
                .context("instantiate bounded Wasm mod")?;
            let version = instance
                .get_typed_func::<(), i32>(&mut store, "void_abi_version")?
                .call(&mut store, ())?;
            ensure!(version == ABI_VERSION as i32, "Wasm ABI version mismatch");
            let frame = instance.get_typed_func::<(), ()>(&mut store, "void_on_frame")?;
            Ok(Self {
                manifest,
                store,
                frame,
                fuel: limits.fuel_per_frame,
                fault: None,
            })
        }
        pub fn on_frame(&mut self, metrics: FrameMetrics) -> Result<Vec<HudText>> {
            if let Some(fault) = &self.fault {
                bail!("mod disabled after previous fault: {fault}");
            }
            self.store.data_mut().metrics = metrics;
            self.store.data_mut().commands.clear();
            self.store.data_mut().host_calls = 0;
            self.store.set_fuel(self.fuel)?;
            if let Err(error) = self.frame.call(&mut self.store, ()) {
                self.store.data_mut().commands.clear();
                self.fault = Some(format!("{error:#}"));
                return Err(error.context(format!("mod {} disabled", self.manifest.id)));
            }
            Ok(std::mem::take(&mut self.store.data_mut().commands))
        }
        pub fn fault(&self) -> Option<&str> {
            self.fault.as_deref()
        }
    }
    fn consume_host_call(caller: &mut Caller<'_, State>) -> Result<()> {
        caller.data_mut().host_calls += 1;
        ensure!(
            caller.data().host_calls <= 1024,
            "host call budget exhausted"
        );
        Ok(())
    }
}
#[cfg(feature = "wasm")]
pub use sandbox::WasmMod;

struct NativeFrame {
    commands: Vec<HudText>,
    allow_hud: bool,
}
pub struct NativeMod {
    pub manifest: ModManifest,
    // Keep the library alive until all function pointers have been discarded.
    _library: libloading::Library,
    frame: unsafe extern "C" fn(*const HostApiV1) -> i32,
    unload: Option<unsafe extern "C" fn()>,
    faulted: bool,
}
impl NativeMod {
    /// Load a library that the user has explicitly trusted.
    ///
    /// # Safety
    /// The library must uphold the SDK pointer/lifetime/ABI contract and must not
    /// unwind across FFI. Its constructors execute during load, before validation.
    /// Native code has unrestricted process privileges and cannot be sandboxed.
    pub unsafe fn load_trusted(directory: &Path, manifest: ModManifest) -> Result<Self> {
        ensure!(
            manifest.runtime == Runtime::Native,
            "manifest is not a native mod"
        );
        let path = manifest.resolve_entry(directory)?;
        // SAFETY: Caller explicitly assumes the native library trust contract.
        let library = unsafe { libloading::Library::new(path)? };
        // SAFETY: Required SDK symbol has this signature under the trust contract.
        let entry =
            unsafe { library.get::<unsafe extern "C" fn() -> *const ModApiV1>(b"void_mod_v1\0")? };
        // SAFETY: The trusted entry returns a valid static SDK table or null.
        let table_ptr = unsafe { entry() };
        ensure!(!table_ptr.is_null(), "native mod returned a null API table");
        // SAFETY: The SDK requires the first two u32 fields in all API tables.
        let header = unsafe { std::slice::from_raw_parts(table_ptr.cast::<u32>(), 2) };
        ensure!(
            header[0] == ABI_VERSION && header[1] as usize >= std::mem::size_of::<ModApiV1>(),
            "native mod ABI version or table size mismatch"
        );
        // SAFETY: Table version/size were checked, and caller guarantees its validity.
        let table = unsafe { &*table_ptr };
        let frame = table.on_frame.context("native mod has no frame callback")?;
        Ok(Self {
            manifest,
            frame,
            unload: table.on_unload,
            _library: library,
            faulted: false,
        })
    }
    pub fn on_frame(&mut self, metrics: FrameMetrics) -> Result<Vec<HudText>> {
        ensure!(
            !self.faulted,
            "native mod disabled after previous callback failure"
        );
        let mut state = NativeFrame {
            commands: Vec::new(),
            allow_hud: self.manifest.capabilities.contains("hud"),
        };
        let api = HostApiV1 {
            abi_version: ABI_VERSION,
            struct_size: std::mem::size_of::<HostApiV1>() as u32,
            context: (&mut state as *mut NativeFrame).cast(),
            metrics: if self.manifest.capabilities.contains("metrics") {
                metrics
            } else {
                FrameMetrics::default()
            },
            hud_text: native_hud,
        };
        // SAFETY: Library accepted the trusted native SDK contract at construction.
        let status = unsafe { (self.frame)(&api) };
        if status != 0 {
            self.faulted = true;
            bail!(
                "native mod {} callback failed with {status}",
                self.manifest.id
            );
        }
        Ok(state.commands)
    }
}
impl Drop for NativeMod {
    fn drop(&mut self) {
        if let Some(unload) = self.unload {
            // SAFETY: The library remains loaded for the duration of this callback.
            unsafe { unload() }
        }
    }
}
unsafe extern "C" fn native_hud(
    context: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    rgba: u32,
    ptr: *const u8,
    len: usize,
) -> i32 {
    if context.is_null()
        || ptr.is_null()
        || len > MAX_HUD_TEXT_BYTES
        || !x.is_finite()
        || !y.is_finite()
    {
        return -1;
    }
    // SAFETY: Trusted native callback must return the context passed to this frame.
    let state = unsafe { &mut *context.cast::<NativeFrame>() };
    if !state.allow_hud || state.commands.len() >= MAX_HUD_COMMANDS {
        return -2;
    }
    // SAFETY: Trusted native callback promises a live buffer of len bytes.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return -3;
    };
    state.commands.push(HudText {
        x,
        y,
        rgba,
        text: text.to_owned(),
    });
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> ModManifest {
        ModManifest::parse("id='test'\nversion='1.0.0'\nsdk=1\nruntime='wasm'\nentry='test.wasm'\ncapabilities=['hud','metrics']").unwrap()
    }
    #[test]
    fn rejects_escape_and_wrong_sdk() {
        let mut m = manifest();
        m.entry = "../evil.wasm".into();
        assert!(m.validate().is_err());
        m.entry = "fine.wasm".into();
        m.sdk = 2;
        assert!(m.validate().is_err());
    }
    #[cfg(feature = "wasm")]
    #[test]
    fn wasm_emits_hud_and_cannot_access_ungranted_host_apis() {
        let bytes = wat::parse_str(r#"(module
            (import "void" "hud_text" (func $text (param f32 f32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1) (data (i32.const 0) "FPS")
            (func (export "void_abi_version") (result i32) i32.const 1)
            (func (export "void_on_frame") f32.const 12 f32.const 20 i32.const -1 i32.const 0 i32.const 3 call $text drop))"#).unwrap();
        let mut instance = WasmMod::from_bytes(&bytes, manifest(), ModLimits::default()).unwrap();
        assert_eq!(
            instance.on_frame(FrameMetrics::default()).unwrap()[0].text,
            "FPS"
        );
        let mut m = manifest();
        m.capabilities.clear();
        let mut denied = WasmMod::from_bytes(&bytes, m, ModLimits::default()).unwrap();
        assert!(denied.on_frame(FrameMetrics::default()).is_err());
        assert!(denied.fault().is_some());
    }
    #[cfg(feature = "wasm")]
    #[test]
    fn infinite_loop_and_memory_exhaustion_are_isolated() {
        let bytes = wat::parse_str("(module (func (export \"void_abi_version\") (result i32) i32.const 1) (func (export \"void_on_frame\") (loop $forever br $forever)))").unwrap();
        let mut instance = WasmMod::from_bytes(
            &bytes,
            manifest(),
            ModLimits {
                fuel_per_frame: 100,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(instance.on_frame(FrameMetrics::default()).is_err());
        assert!(instance.on_frame(FrameMetrics::default()).is_err());
        let bytes = wat::parse_str("(module (memory 1000) (func (export \"void_abi_version\") (result i32) i32.const 1) (func (export \"void_on_frame\")))").unwrap();
        assert!(WasmMod::from_bytes(&bytes, manifest(), ModLimits::default()).is_err());
    }
}
