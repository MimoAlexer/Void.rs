use void_sdk::{HostApiV1, HudSink, HudText, ModApiV1};

struct NativeSink<'a>(&'a HostApiV1);
impl HudSink for NativeSink<'_> {
    fn text(&mut self, text: HudText) {
        // SAFETY: Called synchronously with live text and the host's frame context.
        unsafe {
            (self.0.hud_text)(
                self.0.context,
                text.x,
                text.y,
                text.rgba,
                text.text.as_ptr(),
                text.text.len(),
            );
        }
    }
}
unsafe extern "C" fn on_frame(host: *const HostApiV1) -> i32 {
    if host.is_null() {
        return -1;
    }
    // SAFETY: Host guarantees a live table for the duration of this callback.
    let host = unsafe { &*host };
    if host.abi_version != void_sdk::ABI_VERSION
        || (host.struct_size as usize) < std::mem::size_of::<HostApiV1>()
    {
        return -2;
    }
    void_sdk::draw_metrics(host.metrics, &mut NativeSink(host));
    0
}
static API: ModApiV1 = ModApiV1 {
    abi_version: void_sdk::ABI_VERSION,
    struct_size: std::mem::size_of::<ModApiV1>() as u32,
    on_frame: Some(on_frame),
    on_unload: None,
};
#[unsafe(no_mangle)]
pub extern "C" fn void_mod_v1() -> *const ModApiV1 {
    &API
}
