//! Small, versioned extension boundary shared by built-in, native and Wasm mods.
//! No Rust allocation or Rust-layout-dependent type crosses the native boundary.
use serde::{Deserialize, Serialize};
use std::ffi::c_void;

pub const ABI_VERSION: u32 = 1;
pub const MAX_HUD_TEXT_BYTES: usize = 1024;
pub const MAX_HUD_COMMANDS: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HudText {
    pub x: f32,
    pub y: f32,
    pub rgba: u32,
    pub text: String,
}

pub trait HudSink {
    fn text(&mut self, text: HudText);
}

impl HudSink for Vec<HudText> {
    fn text(&mut self, text: HudText) {
        self.push(text);
    }
}

/// A frame snapshot. Values come from the client, never guest memory.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FrameMetrics {
    pub fps: f32,
    pub ping_ms: u32,
    pub left_cps: f32,
    pub right_cps: f32,
    pub pressed_keys: u32,
}

/// The built-in metrics module uses exactly the same HUD abstraction as mods.
pub fn draw_metrics(metrics: FrameMetrics, sink: &mut impl HudSink) {
    let rows = [
        format!("FPS {:.0}", metrics.fps),
        format!("PING {} ms", metrics.ping_ms),
        format!("CPS {:.0} / {:.0}", metrics.left_cps, metrics.right_cps),
        format!("KEYS {:05b}", metrics.pressed_keys & 31),
    ];
    for (row, text) in rows.into_iter().enumerate() {
        sink.text(HudText {
            x: 12.0,
            y: 12.0 + row as f32 * 20.0,
            rgba: 0xE6D9FFFF,
            text,
        });
    }
}

/// Native mods must call callbacks synchronously and not retain these pointers.
#[repr(C)]
pub struct HostApiV1 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub context: *mut c_void,
    pub metrics: FrameMetrics,
    /// Returns zero on success. The host copies bounded UTF-8 before returning.
    pub hud_text: unsafe extern "C" fn(*mut c_void, f32, f32, u32, *const u8, usize) -> i32,
}

/// Export `void_mod_v1() -> *const ModApiV1`. Keep the table alive until unload.
#[repr(C)]
pub struct ModApiV1 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub on_frame: Option<unsafe extern "C" fn(*const HostApiV1) -> i32>,
    pub on_unload: Option<unsafe extern "C" fn()>,
}

/// Versioned Wasm host bindings. The runtime links no WASI/filesystem/network APIs.
#[cfg(target_arch = "wasm32")]
pub mod wasm {
    use super::{HudSink, HudText};

    #[link(wasm_import_module = "void")]
    unsafe extern "C" {
        fn hud_text(x: f32, y: f32, rgba: i32, ptr: *const u8, len: i32) -> i32;
        fn metric(index: i32) -> f32;
    }

    pub struct Host;
    impl HudSink for Host {
        fn text(&mut self, text: HudText) {
            if text.text.len() <= super::MAX_HUD_TEXT_BYTES {
                // SAFETY: Wasm memory is bounds-checked and copied by the host.
                unsafe {
                    hud_text(
                        text.x,
                        text.y,
                        text.rgba as i32,
                        text.text.as_ptr(),
                        text.text.len() as i32,
                    );
                }
            }
        }
    }

    pub fn metrics() -> super::FrameMetrics {
        // SAFETY: Scalar-only, versioned imports are supplied by the runtime.
        unsafe {
            super::FrameMetrics {
                fps: metric(0),
                ping_ms: metric(1) as u32,
                left_cps: metric(2),
                right_cps: metric(3),
                pressed_keys: metric(4) as u32,
            }
        }
    }
}
