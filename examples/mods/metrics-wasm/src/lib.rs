#[unsafe(no_mangle)]
pub extern "C" fn void_abi_version() -> i32 {
    void_sdk::ABI_VERSION as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn void_on_frame() {
    #[cfg(target_arch = "wasm32")]
    void_sdk::draw_metrics(void_sdk::wasm::metrics(), &mut void_sdk::wasm::Host);
}
