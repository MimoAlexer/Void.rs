use crate::{
    ChunkData, Error, Result, ServerAddress,
    codec::{FrameDecoder, Reader, Writer, encode_frame},
    nbt::{self, Nbt},
    status::handshake,
};
use crossbeam_channel::{Receiver, Sender, bounded};
use md5::{Digest, Md5};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpStream,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Connecting,
    Login,
    Configuration,
    Play,
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub address: ServerAddress,
    pub username: String,
    pub view_distance: u8,
    pub timeout: Duration,
}
impl ConnectOptions {
    pub fn offline(address: ServerAddress, username: impl Into<String>) -> Self {
        Self {
            address,
            username: username.into(),
            view_distance: 12,
            timeout: Duration::from_secs(15),
        }
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct PlayerPosition {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
}
#[derive(Debug, Clone)]
pub enum ClientCommand {
    Move(PlayerPosition),
    Chat(String),
    Swing,
    Attack(i32),
    Input(u8),
    Sprint(bool),
    Respawn,
    /// Required explicit user action: never automatically accept a server's conduct text.
    AcceptCodeOfConduct,
    Disconnect,
}
#[derive(Debug, Clone)]
pub enum ServerEvent {
    State(State),
    Login {
        uuid: uuid::Uuid,
        username: String,
    },
    Registry {
        name: String,
        entries: usize,
    },
    World {
        entity_id: i32,
        dimension: String,
        min_y: i32,
        height: u32,
        gamemode: u8,
    },
    Chunk(ChunkData),
    UnloadChunk {
        x: i32,
        z: i32,
    },
    Position(PlayerPosition),
    BlockUpdate {
        x: i32,
        y: i32,
        z: i32,
        state: u32,
    },
    EntitySpawn {
        id: i32,
        uuid: uuid::Uuid,
        kind: i32,
        position: PlayerPosition,
        velocity: [f64; 3],
    },
    EntityPosition {
        id: i32,
        position: PlayerPosition,
    },
    EntityVelocity {
        id: i32,
        velocity: [f64; 3],
    },
    EntityRemove(Vec<i32>),
    Abilities {
        flags: u8,
        flying_speed: f32,
        walking_speed: f32,
    },
    Chat(String),
    Health {
        health: f32,
        food: i32,
        saturation: f32,
    },
    CodeOfConduct(String),
    /// An explicit capability limitation; never equates skipped state with full support.
    Notice(String),
    Disconnected(String),
}
pub struct Connection {
    pub events: Receiver<ServerEvent>,
    pub commands: Sender<ClientCommand>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.commands.try_send(ClientCommand::Disconnect);
    }
}

pub fn connect_offline(options: ConnectOptions) -> Result<Connection> {
    connect(options, None)
}
pub fn connect_online(
    mut options: ConnectOptions,
    identity: crate::OnlineIdentity,
) -> Result<Connection> {
    options.username = identity.username.clone();
    connect(options, Some(identity))
}
fn connect(options: ConnectOptions, identity: Option<crate::OnlineIdentity>) -> Result<Connection> {
    if options.username.is_empty()
        || options.username.len() > 16
        || !options
            .username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(Error::Malformed(
            "username must be 1-16 ASCII letters, digits or underscores",
        ));
    }
    if !(2..=32).contains(&options.view_distance) {
        return Err(Error::Limit("view distance must be 2..32"));
    }
    let (events_tx, events) = bounded(256);
    let (commands, commands_rx) = bounded(128);
    thread::Builder::new()
        .name("void-network".into())
        .spawn(move || {
            if let Err(error) = run(options, identity, &events_tx, &commands_rx) {
                let _ = events_tx.send_timeout(
                    ServerEvent::Disconnected(error.to_string()),
                    Duration::from_millis(100),
                );
            }
        })?;
    Ok(Connection { events, commands })
}

pub(crate) struct Wire {
    stream: TcpStream,
    decoder: FrameDecoder,
    encrypt: Option<crate::crypto::Cfb8>,
    decrypt: Option<crate::crypto::Cfb8>,
}
impl Wire {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            decoder: FrameDecoder::default(),
            encrypt: None,
            decrypt: None,
        }
    }
    pub fn send(&mut self, packet: Writer) -> Result<()> {
        let mut frame = encode_frame(&packet.0, self.decoder.threshold)?;
        if let Some(encrypt) = &mut self.encrypt {
            encrypt.apply(&mut frame, true);
        }
        self.stream.write_all(&frame)?;
        Ok(())
    }
    fn enable_encryption(&mut self, key: &[u8; 16]) -> Result<()> {
        // No encrypted server response can precede the client's encryption response.
        // Bytes already buffered at this transition would violate the handshake.
        if self.decoder.buffered_len() != 0 {
            return Err(Error::Malformed(
                "bytes buffered across encryption transition",
            ));
        }
        self.encrypt = Some(crate::crypto::Cfb8::new(key));
        self.decrypt = Some(crate::crypto::Cfb8::new(key));
        Ok(())
    }
    pub fn poll(&mut self) -> Result<Option<Vec<u8>>> {
        if let Some(p) = self.decoder.next_packet()? {
            return Ok(Some(p));
        }
        let mut buf = [0; 65536];
        match self.stream.read(&mut buf) {
            Ok(0) => Err(Error::Disconnected("connection closed".into())),
            Ok(n) => {
                if let Some(decrypt) = &mut self.decrypt {
                    decrypt.apply(&mut buf[..n], false);
                }
                self.decoder.push(&buf[..n])?;
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
    pub fn wait_packet(&mut self, deadline: Instant) -> Result<Vec<u8>> {
        loop {
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            if let Some(p) = self.poll()? {
                return Ok(p);
            }
        }
    }
}
fn emit(events: &Sender<ServerEvent>, event: ServerEvent) -> Result<()> {
    events
        .send_timeout(event, Duration::from_millis(500))
        .map_err(|_| Error::Backpressure)
}
fn offline_uuid(username: &str) -> uuid::Uuid {
    let mut bytes: [u8; 16] = Md5::digest(format!("OfflinePlayer:{username}").as_bytes()).into();
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}
fn send_client_info(wire: &mut Wire, view_distance: u8) -> Result<()> {
    let mut w = Writer::packet(0);
    w.string("en_us");
    w.u8(view_distance);
    w.varint(0);
    w.bool(true);
    w.u8(0x7f);
    w.varint(1);
    w.bool(false);
    w.bool(true);
    w.varint(0);
    wire.send(w)
}
#[derive(Clone, Copy)]
struct Dimension {
    min_y: i32,
    height: u32,
}
fn run(
    options: ConnectOptions,
    identity: Option<crate::OnlineIdentity>,
    events: &Sender<ServerEvent>,
    commands: &Receiver<ClientCommand>,
) -> Result<()> {
    emit(events, ServerEvent::State(State::Connecting))?;
    let mut wire = Wire::new(options.address.connect(options.timeout)?);
    wire.send(handshake(&options.address, 2))?;
    let mut hello = Writer::packet(0);
    hello.string(&options.username);
    hello.bytes(
        identity
            .as_ref()
            .map(|i| i.uuid)
            .unwrap_or_else(|| offline_uuid(&options.username))
            .as_bytes(),
    );
    wire.send(hello)?;
    let mut state = State::Login;
    emit(events, ServerEvent::State(state))?;
    let mut dimensions = HashMap::<i32, Dimension>::new();
    let mut dimension = Dimension {
        min_y: -64,
        height: 384,
    };
    let mut player = PlayerPosition::default();
    let mut player_entity_id = 0;
    let mut secure_chat_required = false;
    let mut positioned = false;
    let mut loaded = false;
    let mut chunk_received = false;
    let mut entities = HashMap::<i32, PlayerPosition>::new();
    let mut last_packet = Instant::now();
    let mut last_tick = Instant::now();
    let mut last_movement = Instant::now();
    let mut unknown_reported = std::collections::HashSet::new();
    loop {
        for command in commands.try_iter().take(128) {
            match command {
                ClientCommand::Disconnect => {
                    emit(
                        events,
                        ServerEvent::Disconnected("disconnected by client".into()),
                    )?;
                    return Ok(());
                }
                ClientCommand::AcceptCodeOfConduct if state == State::Configuration => {
                    wire.send(Writer::packet(9))?
                }
                ClientCommand::Move(position) if state == State::Play && positioned => {
                    if [position.x, position.y, position.z]
                        .iter()
                        .any(|v| !v.is_finite())
                        || !position.yaw.is_finite()
                        || !position.pitch.is_finite()
                    {
                        return Err(Error::Malformed("non-finite local position"));
                    }
                    player = position;
                    send_move(&mut wire, player)?;
                    last_movement = Instant::now();
                }
                ClientCommand::Swing if state == State::Play => {
                    let mut w = Writer::packet(0x3e);
                    w.varint(0);
                    wire.send(w)?;
                }
                ClientCommand::Attack(id) if state == State::Play => {
                    let mut w = Writer::packet(1);
                    w.varint(id);
                    wire.send(w)?;
                }
                ClientCommand::Input(bits) if state == State::Play => {
                    let mut w = Writer::packet(0x2b);
                    w.u8(bits & 0x7f);
                    wire.send(w)?;
                }
                ClientCommand::Sprint(sprinting) if state == State::Play => {
                    let mut w = Writer::packet(0x2a);
                    w.varint(player_entity_id);
                    // 26.2 removed the old sneak entries: START_SPRINTING is 1, STOP is 2.
                    w.varint(if sprinting { 1 } else { 2 });
                    w.varint(0);
                    wire.send(w)?;
                }
                ClientCommand::Respawn if state == State::Play => {
                    let mut w = Writer::packet(0x0c);
                    w.varint(0);
                    wire.send(w)?;
                }
                ClientCommand::Chat(text) if state == State::Play => {
                    if text.encode_utf16().count() > 256 {
                        return Err(Error::Limit("chat length"));
                    }
                    if !text.starts_with('/') && secure_chat_required {
                        emit(events,ServerEvent::Notice("Message not sent: this server requires signed chat, which is not implemented.".into()))?;
                        continue;
                    }
                    if let Some(command) = text.strip_prefix('/') {
                        let mut w = Writer::packet(7);
                        w.string(command);
                        wire.send(w)?;
                    } else {
                        let mut w = Writer::packet(9);
                        w.string(&text);
                        w.i64(
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as i64,
                        );
                        w.i64(0);
                        w.bool(false);
                        w.varint(0);
                        w.bytes(&[0, 0, 0]);
                        // LastSeenMessages.EMPTY.computeChecksum() = 1 in 26.2.
                        w.u8(1);
                        wire.send(w)?;
                    }
                }
                _ => {}
            }
        }
        if state == State::Play && last_tick.elapsed() >= Duration::from_millis(50) {
            if positioned && last_movement.elapsed() >= Duration::from_secs(1) {
                send_move(&mut wire, player)?;
                last_movement = Instant::now();
            }
            wire.send(Writer::packet(0x0d))?;
            last_tick = Instant::now();
        }
        if last_packet.elapsed() > Duration::from_secs(60) {
            return Err(Error::Timeout);
        }
        let Some(packet) = wire.poll()? else {
            continue;
        };
        last_packet = Instant::now();
        let mut r = Reader::new(&packet);
        let id = r.varint()?;
        match state {
            State::Login=>match id {
                0=>return Err(Error::Disconnected(r.string(32767)?)),
                1=>{let (response,key)=crate::crypto::encryption_response(&mut r,identity.as_ref())?;wire.send(response)?;wire.enable_encryption(&key)?;},
                2=>{
                    let uuid=r.uuid()?;let username=r.string(16)?;
                    for _ in 0..r.count(1024)? {r.string(32767)?;r.string(32767)?;if r.bool()? {r.string(32767)?;}}
                    r.uuid()?;r.finish()?;emit(events,ServerEvent::Login {uuid,username})?;
                    wire.send(Writer::packet(3))?;state=State::Configuration;emit(events,ServerEvent::State(state))?;send_client_info(&mut wire,options.view_distance)?;
                    let mut brand=Writer::packet(2);brand.string("minecraft:brand");brand.string("Void.rs");wire.send(brand)?;
                },
                3=>{let threshold=r.count(crate::codec::MAX_DECOMPRESSED)?;wire.decoder.threshold=Some(threshold);},
                4=>{let transaction=r.varint()?;let mut w=Writer::packet(2);w.varint(transaction);w.bool(false);wire.send(w)?;},
                5=>cookie_response(&mut wire,&mut r,4)?,
                _=>return Err(Error::Unsupported(format!("unknown login packet 0x{id:02x}"))),
            },
            State::Configuration=>match id {
                0=>cookie_response(&mut wire,&mut r,1)?,
                2=>return Err(Error::Disconnected(nbt::read(&mut r)?.text())),
                3=>{wire.send(Writer::packet(3))?;state=State::Play;positioned=false;loaded=false;chunk_received=false;emit(events,ServerEvent::State(state))?;},
                4=>{let mut w=Writer::packet(4);w.i64(r.i64()?);wire.send(w)?;},
                5=>{let mut w=Writer::packet(5);w.i32(r.i32()?);wire.send(w)?;},
                7=>{
                    let name=r.string(32767)?;let count=r.count(65536)?;
                    for entry in 0..count {
                        let _key=r.string(32767)?;let value=if r.bool()? {Some(nbt::read(&mut r)?)} else {None};
                        if name=="minecraft:dimension_type" {
                            let value=value.ok_or_else(||Error::Unsupported("dimension registry omitted data despite empty known packs".into()))?;
                            let min_y=value.get("min_y").and_then(Nbt::integer).ok_or(Error::Malformed("dimension missing min_y"))?;
                            let height=value.get("height").and_then(Nbt::integer).ok_or(Error::Malformed("dimension missing height"))?;
                            if !(16..=4096).contains(&height)||height%16!=0||min_y%16!=0 {return Err(Error::Malformed("invalid dimension bounds"));}
                            dimensions.insert(entry as i32,Dimension {min_y,height:height as u32});
                        }
                    }
                    r.finish()?;emit(events,ServerEvent::Registry {name,entries:count})?;
                },
                9=>decline_pack(&mut wire,&mut r,6,events)?,
                11=>return Err(Error::Unsupported("server transfer requires user-visible destination confirmation and new session".into())),
                14=>{let mut w=Writer::packet(7);w.varint(0);wire.send(w)?;},
                19=>emit(events,ServerEvent::CodeOfConduct(r.string(32767)?))?,
                1|6|8|10|12|13|15|16|17|18=>{},
                _=>return Err(Error::Unsupported(format!("unknown configuration packet 0x{id:02x}"))),
            },
            State::Play=>match id {
                0x01=>{
                    let id=r.varint()?;let uuid=r.uuid()?;let kind=r.varint()?;
                    let x=r.f64()?;let y=r.f64()?;let z=r.f64()?;let velocity=r.lp_vec3()?;
                    let pitch=angle(r.u8()?);let yaw=angle(r.u8()?);r.u8()?;r.varint()?;r.finish()?;
                    let position=PlayerPosition {x,y,z,yaw,pitch,on_ground:false};
                    if entities.len()>=65536&&!entities.contains_key(&id){return Err(Error::Limit("entities"));}
                    entities.insert(id,position);emit(events,ServerEvent::EntitySpawn {id,uuid,kind,position,velocity})?;
                },
                0x08=>{let packed=r.i64()?;let block=r.varint()?;if block<0{return Err(Error::Malformed("negative block state"));}emit(events,ServerEvent::BlockUpdate {x:(packed>>38) as i32,y:(packed<<52>>52) as i32,z:(packed<<26>>38) as i32,state:block as u32})?;},
                0x0b=>{let mut w=Writer::packet(0x0b);w.f32(16.0);wire.send(w)?;},
                0x15=>cookie_response(&mut wire,&mut r,0x15)?,
                0x20=>return Err(Error::Disconnected(nbt::read(&mut r)?.text())),
                0x23=>{
                    let id=r.varint()?;let x=r.f64()?;let y=r.f64()?;let z=r.f64()?;let velocity=[r.f64()?,r.f64()?,r.f64()?];let yaw=r.f32()?;let pitch=r.f32()?;let on_ground=r.bool()?;
                    let position=PlayerPosition {x,y,z,yaw,pitch,on_ground};if let Some(p)=entities.get_mut(&id){*p=position;}
                    emit(events,ServerEvent::EntityPosition {id,position})?;emit(events,ServerEvent::EntityVelocity {id,velocity})?;
                },
                0x25=>{let z=r.i32()?;let x=r.i32()?;emit(events,ServerEvent::UnloadChunk {x,z})?;},
                0x2c=>{let mut w=Writer::packet(0x1c);w.i64(r.i64()?);wire.send(w)?;},
                0x2d=>{
                    let chunk=crate::chunk::read_chunk(&mut r,dimension.min_y,dimension.height)?;
                    emit(events,ServerEvent::Chunk(chunk))?;
                    chunk_received=true;
                    if positioned&&!loaded {wire.send(Writer::packet(0x2c))?;loaded=true;}
                },
                0x31=>{
                    let entity_id=r.i32()?;player_entity_id=entity_id;r.bool()?;for _ in 0..r.count(1024)? {r.string(32767)?;}
                    r.varint()?;r.varint()?;r.varint()?;r.bool()?;r.bool()?;r.bool()?;
                    let (dim_id,name,gamemode)=spawn_info(&mut r)?;
                    dimension = *dimensions.get(&dim_id).ok_or(Error::Malformed("unknown dimension registry ID"))?;
                    r.bool()?;let secure_chat=r.bool()?;secure_chat_required=secure_chat;r.finish()?;
                    if secure_chat {emit(events,ServerEvent::Notice("Server enforces signed chat; unsigned chat is unavailable in this build.".into()))?;}
                    emit(events,ServerEvent::World {entity_id,dimension:name,min_y:dimension.min_y,height:dimension.height,gamemode})?;
                },
                0x3d=>{let mut w=Writer::packet(0x2d);w.i32(r.i32()?);wire.send(w)?;},
                0x35|0x36|0x38=>{
                    let entity=r.varint()?;let mut position=entities.get(&entity).copied().unwrap_or_default();
                    if id!=0x38 {position.x=relative_axis(position.x,r.i16()?);position.y=relative_axis(position.y,r.i16()?);position.z=relative_axis(position.z,r.i16()?);}
                    if id!=0x35 {position.yaw=angle(r.u8()?);position.pitch=angle(r.u8()?);}position.on_ground=r.bool()?;
                    if let Some(p)=entities.get_mut(&entity){*p=position;emit(events,ServerEvent::EntityPosition {id:entity,position})?;}
                },
                0x40=>emit(events,ServerEvent::Abilities {flags:r.u8()?,flying_speed:r.f32()?,walking_speed:r.f32()?})?,
                0x48=>{
                    let teleport=r.varint()?;let x=r.f64()?;let y=r.f64()?;let z=r.f64()?;
                    r.f64()?;r.f64()?;r.f64()?;let yaw=r.f32()?;let pitch=r.f32()?;let flags=r.i32()?;
                    player=PlayerPosition {x:x+if flags&1!=0{player.x}else{0.0},y:y+if flags&2!=0{player.y}else{0.0},z:z+if flags&4!=0{player.z}else{0.0},yaw:yaw+if flags&8!=0{player.yaw}else{0.0},pitch:pitch+if flags&16!=0{player.pitch}else{0.0},on_ground:false};
                    let mut w=Writer::packet(0);w.varint(teleport);wire.send(w)?;send_move(&mut wire,player)?;positioned=true;emit(events,ServerEvent::Position(player))?;
                    if chunk_received&&!loaded{wire.send(Writer::packet(0x2c))?;loaded=true;}
                },
                0x4d=>{let n=r.count(65536)?;let mut ids=Vec::with_capacity(n);for _ in 0..n{let id=r.varint()?;entities.remove(&id);ids.push(id);}emit(events,ServerEvent::EntityRemove(ids))?;},
                0x51=>decline_pack(&mut wire,&mut r,0x31,events)?,
                0x52=>{let (id,name,gamemode)=spawn_info(&mut r)?;dimension = *dimensions.get(&id).ok_or(Error::Malformed("unknown respawn dimension ID"))?;positioned=false;loaded=false;chunk_received=false;entities.clear();emit(events,ServerEvent::World {entity_id:player_entity_id,dimension:name,min_y:dimension.min_y,height:dimension.height,gamemode})?;},
                0x54=>{
                    let section=r.i64()?;let sx=(section>>42) as i32;let sy=(section<<44>>44) as i32;let sz=(section<<22>>42) as i32;
                    for _ in 0..r.count(4096)?{let change=r.varlong()?;let state=change>>12;if state<0||state>u32::MAX as i64{return Err(Error::Malformed("invalid section block state"));}emit(events,ServerEvent::BlockUpdate {x:sx*16+((change>>8)&15) as i32,y:sy*16+(change&15) as i32,z:sz*16+((change>>4)&15) as i32,state:state as u32})?;}
                },
                0x65=>emit(events,ServerEvent::EntityVelocity {id:r.varint()?,velocity:r.lp_vec3()?})?,
                0x68=>emit(events,ServerEvent::Health {health:r.f32()?,food:r.varint()?,saturation:r.f32()?})?,
                0x76=>{wire.send(Writer::packet(0x10))?;state=State::Configuration;dimensions.clear();emit(events,ServerEvent::State(state))?;send_client_info(&mut wire,options.view_distance)?;},
                0x79=>emit(events,ServerEvent::Chat(nbt::read(&mut r)?.text()))?,
                0x7d=>{
                    let id=r.varint()?;let x=r.f64()?;let y=r.f64()?;let z=r.f64()?;let velocity=[r.f64()?,r.f64()?,r.f64()?];let yaw=r.f32()?;let pitch=r.f32()?;let flags=r.i32()?;let on_ground=r.bool()?;
                    let previous=entities.get(&id).copied().unwrap_or_default();let position=PlayerPosition {x:x+if flags&1!=0{previous.x}else{0.0},y:y+if flags&2!=0{previous.y}else{0.0},z:z+if flags&4!=0{previous.z}else{0.0},yaw:yaw+if flags&8!=0{previous.yaw}else{0.0},pitch:pitch+if flags&16!=0{previous.pitch}else{0.0},on_ground};
                    if let Some(p)=entities.get_mut(&id){*p=position;}emit(events,ServerEvent::EntityPosition{id,position})?;emit(events,ServerEvent::EntityVelocity{id,velocity})?;
                },
                0x81=>return Err(Error::Unsupported("server transfer not implemented".into())),
                // These packets have no effect on this subset's state or acknowledgements.
                0x00|0x0c|0x18|0x26|0x5e|0x5f|0x6f|0x71|0x7f|0x80=>{},
                _=>{if unknown_reported.insert(id) {emit(events,ServerEvent::Notice(format!("Play packet 0x{id:02x} is not yet applied; vanilla compatibility remains incomplete.")))?;}},
            },
            _=>return Err(Error::Malformed("invalid connection state")),
        }
    }
}
fn send_move(wire: &mut Wire, p: PlayerPosition) -> Result<()> {
    let mut w = Writer::packet(0x1f);
    w.f64(p.x);
    w.f64(p.y);
    w.f64(p.z);
    w.f32(p.yaw);
    w.f32(p.pitch);
    w.u8(u8::from(p.on_ground));
    wire.send(w)
}
fn angle(byte: u8) -> f32 {
    (byte as i8 as f32) * 360.0 / 256.0
}
fn relative_axis(base: f64, delta: i16) -> f64 {
    if delta == 0 {
        base
    } else {
        ((base * 4096.0 + 0.5).floor() + delta as f64) / 4096.0
    }
}
fn cookie_response(wire: &mut Wire, r: &mut Reader<'_>, id: i32) -> Result<()> {
    let key = r.string(32767)?;
    let mut w = Writer::packet(id);
    w.string(&key);
    w.bool(false);
    wire.send(w)
}
fn decline_pack(
    wire: &mut Wire,
    r: &mut Reader<'_>,
    id: i32,
    events: &Sender<ServerEvent>,
) -> Result<()> {
    let uuid = r.uuid()?;
    r.string(32767)?;
    r.string(40)?;
    let forced = r.bool()?;
    let mut w = Writer::packet(id);
    w.bytes(uuid.as_bytes());
    w.varint(1);
    wire.send(w)?;
    emit(
        events,
        ServerEvent::Notice(
            if forced {
                "Required server resource pack cannot be loaded yet; server may disconnect."
            } else {
                "Optional server resource pack declined: pack loading is not implemented."
            }
            .into(),
        ),
    )
}
fn spawn_info(r: &mut Reader<'_>) -> Result<(i32, String, u8)> {
    let dimension = r.varint()?;
    let name = r.string(32767)?;
    r.i64()?;
    let gamemode = r.u8()?;
    r.u8()?;
    r.bool()?;
    r.bool()?;
    if r.bool()? {
        r.string(32767)?;
        r.i64()?;
    }
    r.varint()?;
    r.varint()?;
    Ok((dimension, name, gamemode))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn java_offline_uuid() {
        assert_eq!(
            offline_uuid("Notch").to_string(),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
    }
    #[test]
    fn invalid_user_input_rejected_before_thread() {
        let mut opts = ConnectOptions::offline(ServerAddress::parse("localhost").unwrap(), "x x");
        assert!(connect_offline(opts.clone()).is_err());
        opts.username = "Test".into();
        opts.view_distance = 0;
        assert!(connect_offline(opts).is_err());
    }
}
