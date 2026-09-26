//! Shared harness logic: deterministic seed checking requires no nightly/libFuzzer.
use void_protocol::{
    codec::{FrameDecoder, Reader, encode_frame},
    nbt,
};

pub const MAX_INPUT: usize = 65_536;
const MAX_PACKETS: usize = 4096;

/// First byte selects None/0/256 compression; second sets fragment size 1..256.
/// Remaining bytes are arbitrary framed wire data. Errors are expected; panic is not.
pub fn frame_codec(input: &[u8]) {
    if input.len() > MAX_INPUT || input.len() < 2 {
        return;
    }
    let threshold = match input[0] % 3 {
        0 => None,
        1 => Some(0),
        _ => Some(256),
    };
    let mut decoder = FrameDecoder::default();
    decoder.threshold = threshold;
    let mut packets = 0;
    for fragment in input[2..].chunks(usize::from(input[1]) + 1) {
        if decoder.push(fragment).is_err() {
            return;
        }
        loop {
            let packet = match decoder.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(_) => return,
            };
            packets += 1;
            if packets > MAX_PACKETS {
                return;
            }
            // Recompression may legitimately exceed the wire-frame limit when the
            // original sender used a different compression level. Assert the round
            // trip only if the canonical encoder accepts the encoded size.
            match encode_frame(&packet, threshold) {
                Ok(encoded) => {
                    let mut canonical = FrameDecoder::default();
                    canonical.threshold = threshold;
                    canonical
                        .push(&encoded)
                        .expect("one encoded frame fits the receive bound");
                    assert_eq!(
                        canonical.next_packet().unwrap().as_deref(),
                        Some(packet.as_slice())
                    );
                    assert_eq!(canonical.next_packet().unwrap(), None);
                }
                Err(void_protocol::Error::Limit("encoded frame size")) => {}
                Err(error) => panic!("unexpected canonical encoder error: {error}"),
            }
            let mut reader = Reader::new(&packet);
            let _ = reader.varint();
            let _ = reader.string(32767);
            let _ = reader.varlong();
            let _ = reader.lp_vec3();
        }
    }
}

/// Arbitrary unnamed network NBT root. Trailing packet bytes are permitted here.
pub fn network_nbt(input: &[u8]) {
    if input.len() > MAX_INPUT {
        return;
    }
    let mut reader = Reader::new(input);
    if let Ok(value) = nbt::read(&mut reader) {
        assert!(reader.remaining() <= input.len());
        let _ = value.text();
        let _ = value.get("text");
        let _ = value.integer();
    }
}
