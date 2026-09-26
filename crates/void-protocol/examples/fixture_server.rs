//! Explicit deterministic protocol fixture, NOT a vanilla server/game simulation.
//! `cargo run -p void-protocol --example fixture_server -- 127.0.0.1:25566`
//! Provides actual socket bytes to exercise networking -> ECS -> renderer integration.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};
use void_protocol::{
    Error, Result,
    codec::{FrameDecoder, Reader, Writer, encode_frame},
};

struct Peer {
    stream: TcpStream,
    decoder: FrameDecoder,
}
impl Peer {
    fn new(stream: TcpStream) -> Result<Self> {
        stream.set_read_timeout(Some(Duration::from_millis(20)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            decoder: FrameDecoder::default(),
        })
    }
    fn poll(&mut self) -> Result<Option<Vec<u8>>> {
        if let Some(packet) = self.decoder.next_packet()? {
            return Ok(Some(packet));
        }
        let mut bytes = [0; 16384];
        match self.stream.read(&mut bytes) {
            Ok(0) => Err(Error::Disconnected("fixture client closed".into())),
            Ok(n) => {
                self.decoder.push(&bytes[..n])?;
                self.decoder.next_packet()
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }
    fn next(&mut self) -> Result<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(p) = self.poll()? {
                return Ok(p);
            }
            if Instant::now() > deadline {
                return Err(Error::Timeout);
            }
        }
    }
    fn expect(&mut self, id: i32) -> Result<Vec<u8>> {
        let packet = self.next()?;
        if Reader::new(&packet).varint()? != id {
            return Err(Error::Malformed("fixture received unexpected packet"));
        }
        Ok(packet)
    }
    fn send(&mut self, w: Writer) -> Result<()> {
        self.stream
            .write_all(&encode_frame(&w.0, self.decoder.threshold)?)?;
        Ok(())
    }
}
fn main() -> Result<()> {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:25566".into());
    let listener = TcpListener::bind(&address)?;
    println!(
        "Void.rs deterministic TEST FIXTURE listening on {}",
        listener.local_addr()?
    );
    println!(
        "This is not Minecraft, Paper, a vanilla-compatible server, or a performance benchmark."
    );
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    if let Err(e) = serve(stream) {
                        eprintln!("Fixture connection ended: {e}");
                    }
                });
            }
            Err(e) => eprintln!("Accept: {e}"),
        }
    }
    Ok(())
}
fn serve(stream: TcpStream) -> Result<()> {
    let mut peer = Peer::new(stream)?;
    let hello = peer.expect(0)?;
    let mut r = Reader::new(&hello);
    r.varint()?;
    let protocol = r.varint()?;
    r.string(255)?;
    r.u16()?;
    let next_state = r.varint()?;
    r.finish()?;
    if next_state == 1 {
        peer.expect(0)?;
        let mut response = Writer::packet(0);
        response.string(r#"{"version":{"name":"26.2 / Void fixture","protocol":776},"players":{"online":1,"max":1},"description":{"text":"Void.rs TEST FIXTURE — not a real Minecraft server"},"enforcesSecureChat":false}"#);
        peer.send(response)?;
        let ping = peer.expect(1)?;
        peer.send(Writer(ping))?;
        return Ok(());
    }
    if next_state != 2 || protocol != 776 {
        return Err(Error::Unsupported("fixture only accepts 26.2 login".into()));
    }
    let hello = peer.expect(0)?;
    let mut r = Reader::new(&hello);
    r.varint()?;
    let username = r.string(16)?;
    let uuid = r.uuid()?;
    println!("Fixture login: {username}");
    let mut compression = Writer::packet(3);
    compression.varint(256);
    peer.send(compression)?;
    peer.decoder.threshold = Some(256);
    let mut success = Writer::packet(2);
    success.bytes(uuid.as_bytes());
    success.string(&username);
    success.varint(0);
    success.bytes(uuid::Uuid::nil().as_bytes());
    peer.send(success)?;
    peer.expect(3)?;
    peer.expect(0)?;
    peer.expect(2)?;
    let mut packs = Writer::packet(14);
    packs.varint(0);
    peer.send(packs)?;
    peer.expect(7)?;
    let mut registry = Writer::packet(7);
    registry.string("minecraft:dimension_type");
    registry.varint(1);
    registry.string("minecraft:overworld");
    registry.bool(true);
    registry.u8(10);
    for (name, value) in [("min_y", 0), ("height", 16)] {
        registry.u8(3);
        registry.u16(name.len() as u16);
        registry.bytes(name.as_bytes());
        registry.i32(value);
    }
    registry.u8(0);
    peer.send(registry)?;
    peer.send(Writer::packet(3))?;
    peer.expect(3)?;
    let mut world = Writer::packet(0x31);
    world.i32(42);
    world.bool(false);
    world.varint(1);
    world.string("minecraft:overworld");
    world.varint(1);
    world.varint(12);
    world.varint(12);
    world.bool(false);
    world.bool(true);
    world.bool(false);
    world.varint(0);
    world.string("minecraft:overworld");
    world.i64(0);
    world.u8(0);
    world.u8(255);
    world.bool(false);
    world.bool(true);
    world.bool(false);
    world.varint(0);
    world.varint(1);
    world.bool(false);
    world.bool(false);
    peer.send(world)?;
    let mut view = Writer::packet(0x5e);
    view.varint(0);
    view.varint(0);
    peer.send(view)?;
    let mut abilities = Writer::packet(0x40);
    abilities.u8(0);
    abilities.f32(0.05);
    abilities.f32(0.1);
    peer.send(abilities)?;
    let mut health = Writer::packet(0x68);
    health.f32(20.0);
    health.varint(20);
    health.f32(5.0);
    peer.send(health)?;
    peer.send(Writer::packet(0x0c))?;
    for z in -1i32..=1 {
        for x in -1i32..=1 {
            let mut chunk = Writer::packet(0x2d);
            chunk.i32(x);
            chunk.i32(z);
            chunk.bytes(&include_bytes!("../tests/fixtures/26.2-chunk.bin")[8..]);
            peer.send(chunk)?;
        }
    }
    let mut batch = Writer::packet(0x0b);
    batch.varint(9);
    peer.send(batch)?;
    let mut position = Writer::packet(0x48);
    position.varint(1);
    position.f64(8.0);
    position.f64(1.0);
    position.f64(8.0);
    for _ in 0..3 {
        position.f64(0.0);
    }
    position.f32(180.0);
    position.f32(0.0);
    position.i32(0);
    peer.send(position)?;
    let mut entity = Writer::packet(1);
    entity.varint(43);
    entity.bytes(uuid::Uuid::from_u128(43).as_bytes());
    entity.varint(156);
    entity.f64(8.0);
    entity.f64(1.0);
    entity.f64(3.0);
    entity.u8(0);
    entity.u8(0);
    entity.u8(0);
    entity.u8(0);
    entity.varint(0);
    peer.send(entity)?;
    chat(
        &mut peer,
        "Connected to the deterministic Void.rs network fixture. Vanilla compatibility is not certified.",
    )?;
    let start = Instant::now();
    let mut keepalive = Instant::now();
    let mut motion = Instant::now();
    let mut step = 0;
    loop {
        if keepalive.elapsed() > Duration::from_secs(2) {
            let mut w = Writer::packet(0x2c);
            w.i64(start.elapsed().as_millis() as i64);
            peer.send(w)?;
            keepalive = Instant::now();
        }
        if motion.elapsed() > Duration::from_millis(50) {
            let mut w = Writer::packet(0x35);
            w.varint(43);
            w.bytes(&(if step % 80 < 40 { 128i16 } else { -128i16 }).to_be_bytes());
            w.bytes(&0i16.to_be_bytes());
            w.bytes(&0i16.to_be_bytes());
            w.bool(true);
            peer.send(w)?;
            step += 1;
            motion = Instant::now();
        }
        if let Some(packet) = peer.poll()? {
            let mut r = Reader::new(&packet);
            match r.varint()? {
                0 => println!("Fixture teleport acknowledged: {}", r.varint()?),
                1 => {
                    println!(
                        "Fixture attack received for entity {} (no combat simulation)",
                        r.varint()?
                    );
                    chat(
                        &mut peer,
                        "Attack packet received; this fixture does not simulate combat.",
                    )?;
                }
                7 => {
                    let command = r.string(256)?;
                    chat(&mut peer, &format!("Fixture command: /{command}"))?;
                }
                9 => {
                    let message = r.string(256)?;
                    chat(&mut peer, &format!("Fixture echo: {message}"))?;
                }
                0x2c => println!("Fixture client finished initial loading"),
                _ => {}
            }
        }
    }
}
fn chat(peer: &mut Peer, text: &str) -> Result<()> {
    let mut w = Writer::packet(0x79);
    w.u8(8);
    w.u16(text.len() as u16);
    w.bytes(text.as_bytes());
    w.bool(false);
    peer.send(w)
}
