//! Socket-level integration using a deliberately small independently scripted server.
//! This does not stand in for a live vanilla/Paper compatibility test.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};
use void_protocol::{
    ClientCommand, ConnectOptions, ServerAddress, ServerEvent, State,
    codec::{FrameDecoder, Reader, Writer, encode_frame},
    connect_offline, status,
};

struct Peer {
    stream: TcpStream,
    decoder: FrameDecoder,
}
impl Peer {
    fn new(stream: TcpStream) -> Self {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Self {
            stream,
            decoder: FrameDecoder::default(),
        }
    }
    fn read(&mut self) -> Vec<u8> {
        loop {
            if let Some(packet) = self.decoder.next_packet().unwrap() {
                return packet;
            }
            let mut b = [0; 16384];
            let n = self.stream.read(&mut b).unwrap();
            assert!(n > 0, "client unexpectedly closed");
            self.decoder.push(&b[..n]).unwrap();
        }
    }
    fn expect(&mut self, id: i32) -> Vec<u8> {
        let packet = self.read();
        assert_eq!(Reader::new(&packet).varint().unwrap(), id);
        packet
    }
    fn send(&mut self, w: Writer) {
        let bytes = encode_frame(&w.0, self.decoder.threshold).unwrap();
        self.stream.write_all(&bytes).unwrap();
    }
    fn until(&mut self, wanted: i32) -> Vec<u8> {
        for _ in 0..128 {
            let p = self.read();
            if Reader::new(&p).varint().unwrap() == wanted {
                return p;
            }
        }
        panic!("no requested client packet")
    }
}
fn bind() -> (TcpListener, ServerAddress) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = ServerAddress::parse(&listener.local_addr().unwrap().to_string()).unwrap();
    (listener, addr)
}

#[test]
fn status_handshake_and_echo() {
    let (listener, address) = bind();
    let server = thread::spawn(move || {
        let (s, _) = listener.accept().unwrap();
        let mut peer = Peer::new(s);
        let hello = peer.expect(0);
        let mut r = Reader::new(&hello);
        r.varint().unwrap();
        assert_eq!(r.varint().unwrap(), 776);
        assert_eq!(r.string(255).unwrap(), "127.0.0.1");
        r.u16().unwrap();
        assert_eq!(r.varint().unwrap(), 1);
        r.finish().unwrap();
        peer.expect(0);
        let mut response = Writer::packet(0);
        response.string(r#"{"version":{"name":"26.2","protocol":776},"players":{"online":2,"max":20},"description":{"text":"Void test","extra":[{"text":" server"}]}}"#);
        peer.send(response);
        let ping = peer.expect(1);
        peer.send(Writer(ping));
    });
    let result = status(&address, Duration::from_secs(5)).unwrap();
    assert_eq!(result.protocol, 776);
    assert_eq!(result.description, "Void test server");
    assert_eq!(result.online, 2);
    server.join().unwrap();
}

fn dimension_registry() -> Writer {
    let mut w = Writer::packet(7);
    w.string("minecraft:dimension_type");
    w.varint(1);
    w.string("minecraft:overworld");
    w.bool(true);
    w.u8(10); // unnamed compound
    for (name, value) in [("min_y", 0), ("height", 16)] {
        w.u8(3);
        w.u16(name.len() as u16);
        w.bytes(name.as_bytes());
        w.i32(value);
    }
    w.u8(0);
    w
}
fn login_world() -> Writer {
    let mut w = Writer::packet(0x31);
    w.i32(42);
    w.bool(false);
    w.varint(1);
    w.string("minecraft:overworld");
    w.varint(20);
    w.varint(12);
    w.varint(12);
    w.bool(false);
    w.bool(true);
    w.bool(false);
    w.varint(0);
    w.string("minecraft:overworld");
    w.i64(0);
    w.u8(0);
    w.u8(255);
    w.bool(false);
    w.bool(false);
    w.bool(false);
    w.varint(0);
    w.varint(63);
    w.bool(false);
    w.bool(false);
    w
}
#[test]
fn compressed_offline_configuration_chunk_teleport_keepalive_and_movement() {
    let (listener, address) = bind();
    let server = thread::spawn(move || {
        let (s, _) = listener.accept().unwrap();
        let mut peer = Peer::new(s);
        peer.expect(0);
        let hello = peer.expect(0);
        let mut r = Reader::new(&hello);
        r.varint().unwrap();
        assert_eq!(r.string(16).unwrap(), "Fixture");
        r.uuid().unwrap();
        r.finish().unwrap();
        let mut compression = Writer::packet(3);
        compression.varint(32);
        peer.send(compression);
        peer.decoder.threshold = Some(32);
        let mut success = Writer::packet(2);
        success.bytes(uuid::Uuid::nil().as_bytes());
        success.string("Fixture");
        success.varint(0);
        success.bytes(uuid::Uuid::nil().as_bytes());
        peer.send(success);
        peer.expect(3);
        peer.expect(0);
        peer.expect(2);
        let mut packs = Writer::packet(14);
        packs.varint(0);
        peer.send(packs);
        let response = peer.expect(7);
        assert_eq!(response, vec![7, 0]);
        peer.send(dimension_registry());
        peer.send(Writer::packet(3));
        peer.expect(3);
        peer.send(login_world());
        // Chunk first deliberately exercises loaded-ack ordering across packet batches.
        let mut chunk = Writer::packet(0x2d);
        chunk.bytes(include_bytes!("fixtures/26.2-chunk.bin"));
        peer.send(chunk);
        let mut teleport = Writer::packet(0x48);
        teleport.varint(23);
        teleport.f64(1.25);
        teleport.f64(2.0);
        teleport.f64(-3.5);
        teleport.f64(0.0);
        teleport.f64(0.0);
        teleport.f64(0.0);
        teleport.f32(90.0);
        teleport.f32(10.0);
        teleport.i32(0);
        peer.send(teleport);
        let ack = peer.until(0);
        assert_eq!(ack, vec![0, 23]);
        peer.until(0x1f);
        peer.until(0x2c);
        let mut keepalive = Writer::packet(0x2c);
        keepalive.i64(0x123456789);
        peer.send(keepalive);
        let reply = peer.until(0x1c);
        let mut r = Reader::new(&reply);
        r.varint().unwrap();
        assert_eq!(r.i64().unwrap(), 0x123456789);
        let move_packet = peer.until(0x1f);
        let mut r = Reader::new(&move_packet);
        r.varint().unwrap();
        assert_eq!(r.f64().unwrap(), 2.0);
        let mut disconnect = Writer::packet(0x20);
        disconnect.u8(8);
        disconnect.u16(4);
        disconnect.bytes(b"done");
        peer.send(disconnect);
    });
    let connection = connect_offline(ConnectOptions::offline(address, "Fixture")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut saw_play = false;
    let mut saw_chunk = false;
    let mut saw_position = false;
    let mut done = false;
    while Instant::now() < deadline {
        match connection
            .events
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
        {
            ServerEvent::State(State::Play) => saw_play = true,
            ServerEvent::Chunk(chunk) => {
                assert_eq!((chunk.x, chunk.z), (-4, 7));
                assert_eq!(chunk.sections[0].block_states[0], 1);
                assert_eq!(chunk.sections[0].block_states[16 * 16 + 9 * 16 + 3], 10);
                saw_chunk = true;
            }
            ServerEvent::Position(mut position) => {
                assert_eq!(position.x, 1.25);
                assert_eq!(position.yaw, 90.0);
                saw_position = true;
                position.x = 2.0;
                connection
                    .commands
                    .send(ClientCommand::Move(position))
                    .unwrap();
            }
            ServerEvent::Disconnected(reason) => {
                assert!(reason.contains("done"), "{reason}");
                done = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_play && saw_chunk && saw_position && done);
    server.join().unwrap();
}

#[test]
fn official_lpvec3_golden_fixture() {
    let mut r = Reader::new(include_bytes!("fixtures/26.2-lpvec3.bin"));
    for expected in [
        [0.0; 3],
        [0.3, -0.5, 0.7],
        [8.0, -16.0, 32.0],
        [0.0, 1.0, -1.0],
    ] {
        let actual = r.lp_vec3().unwrap();
        for (a, b) in actual.into_iter().zip(expected) {
            assert!((a - b).abs() < 0.005, "{a} != {b}");
        }
    }
    r.finish().unwrap();
}

#[test]
fn generated_block_data_handles_air_and_non_cubes() {
    use void_protocol::block_states::*;
    assert_eq!(STATE_COUNT, 32366);
    assert!(info(0).unwrap().air);
    assert!(info(15292).unwrap().air);
    assert!(info(15293).unwrap().air);
    assert!(collision_boxes(0).is_empty());
    assert_eq!(collision_boxes(1), &[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]);
    assert_eq!(info(1).unwrap().name, "minecraft:stone");
    assert!(info(u32::MAX).is_none());
}
