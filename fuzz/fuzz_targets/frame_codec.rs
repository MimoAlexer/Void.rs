#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|input: &[u8]| {
    void_protocol_fuzz::frame_codec(input);
});
