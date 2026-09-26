use crate::{
    Args,
    terrain::{self, Terrain},
    ui,
};
use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, bounded};
use egui::Context;
use serde::Serialize;
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use void_core::{
    ClientWorld, ConfigStore, FrameBudget, Job, JobKey, JobKind, Scheduler, WorldRevision,
};
use void_protocol::{
    ClientCommand, ConnectOptions, Connection, PlayerPosition, ServerAddress, ServerEvent,
    ServerStatus,
};
use void_render::{
    Renderer, Vertex,
    glam::{Mat4, Vec3},
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Multiplayer,
    Settings,
    Mods,
    Diagnostics,
    Game,
}
enum Response {
    Status(std::result::Result<ServerStatus, String>),
    SignedIn(std::result::Result<void_services::auth::OnlineSession, String>),
    Assets(String, bool),
}
struct MeshResult {
    epoch: u64,
    revision: u64,
    vertices: Vec<Vertex>,
}
#[derive(Serialize)]
struct Sample {
    frame_us: f64,
    cpu_us: f64,
    gpu_us: Option<f64>,
    fence_wait_us: f64,
    input_to_submit_us: Option<f64>,
    chunks: usize,
    vertices: usize,
}

pub struct App {
    pub args: Args,
    pub config: ConfigStore,
    pub window: Option<Window>,
    pub renderer: Option<Renderer>,
    pub failure: Option<String>,
    ctx: Context,
    egui_state: Option<egui_winit::State>,
    pub page: Page,
    pub address: String,
    pub username: String,
    pub status: Option<ServerStatus>,
    pub busy: bool,
    pub notice: String,
    pub messages: VecDeque<String>,
    pub online: Option<void_services::auth::OnlineSession>,
    responses: Receiver<Response>,
    send: Sender<Response>,
    pub connection: Option<Connection>,
    pub position: PlayerPosition,
    pub health: f32,
    pub food: i32,
    pub conduct: Option<String>,
    pub terrain: Terrain,
    pub world: ClientWorld,
    epoch: u64,
    mesh_rx: Receiver<MeshResult>,
    mesh_tx: Sender<MeshResult>,
    mesh_busy: bool,
    mesh_built_revision: u64,
    mesh: Vec<Vertex>,
    mesh_revision: u64,
    pending_mesh: Option<MeshResult>,
    pub scheduler: Scheduler,
    start: Instant,
    last_frame: Instant,
    next_frame: Instant,
    last_config_check: Instant,
    last_network_move: Instant,
    pub frames: u64,
    pub fps: f64,
    pub cpu_us: f64,
    pub gpu_us: Option<f64>,
    pub frame_history: VecDeque<f64>,
    samples: VecDeque<Sample>,
    last_input: Option<Instant>,
    keys: HashSet<KeyCode>,
    pub captured: bool,
    pub chat_input: String,
    pub left_clicks: VecDeque<Instant>,
    pub right_clicks: VecDeque<Instant>,
    pub mods: crate::modules::ModHost,
    pub accounts: Vec<void_services::auth::AccountMetadata>,
    pub settings_draft: void_core::Config,
    body: void_core::physics::PlayerBody,
    own_entity: i32,
    sprinting: bool,
}
impl App {
    pub fn new(args: Args, config: ConfigStore) -> Self {
        let (send, responses) = bounded(16);
        let (mesh_tx, mesh_rx) = bounded(1);
        let now = Instant::now();
        let scheduler = Scheduler::new(
            config.current().performance.event_horizon.clone(),
            config.current().performance.target_fps,
        )
        .expect("validated config");
        let username = args.username.clone();
        let address = args
            .connect
            .clone()
            .unwrap_or_else(|| "localhost:25565".into());
        let mods = crate::modules::ModHost::new(
            config
                .path()
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("mods"),
        );
        let (accounts, account_notice) = match crate::accounts::read(config.path()) {
            Ok(accounts) => (accounts, String::new()),
            Err(e) => (vec![], format!("Cannot read saved accounts: {e:#}")),
        };
        let settings_draft = config.current().clone();
        Self {
            args,
            config,
            window: None,
            renderer: None,
            failure: None,
            ctx: Context::default(),
            egui_state: None,
            page: Page::Home,
            address,
            username,
            status: None,
            busy: false,
            notice: account_notice,
            messages: VecDeque::new(),
            online: None,
            responses,
            send,
            connection: None,
            position: PlayerPosition::default(),
            health: 20.,
            food: 20,
            conduct: None,
            terrain: Terrain::new(),
            world: ClientWorld::new(),
            epoch: 0,
            mesh_rx,
            mesh_tx,
            mesh_busy: false,
            mesh_built_revision: 0,
            mesh: vec![],
            mesh_revision: 0,
            pending_mesh: None,
            scheduler,
            start: now,
            last_frame: now,
            next_frame: now,
            last_config_check: now,
            last_network_move: now,
            frames: 0,
            fps: 0.,
            cpu_us: 0.,
            gpu_us: None,
            frame_history: VecDeque::new(),
            samples: VecDeque::new(),
            last_input: None,
            keys: HashSet::new(),
            captured: false,
            chat_input: String::new(),
            left_clicks: VecDeque::new(),
            right_clicks: VecDeque::new(),
            mods,
            accounts,
            settings_draft,
            body: Default::default(),
            own_entity: 0,
            sprinting: false,
        }
    }
    pub fn ping(&mut self) {
        if self.busy {
            return;
        }
        let address = match ServerAddress::parse(&self.address) {
            Ok(a) => a,
            Err(e) => {
                self.notice = e.to_string();
                return;
            }
        };
        self.busy = true;
        self.notice = "Checking server…".into();
        let send = self.send.clone();
        std::thread::spawn(move || {
            let value =
                void_protocol::status(&address, Duration::from_secs(5)).map_err(|e| e.to_string());
            let _ = send.send(Response::Status(value));
        });
    }
    pub fn apply_ui_settings(&self) {
        self.ctx.set_zoom_factor(self.config.current().ui.scale);
        ui::style(&self.ctx, &self.config.current().ui);
    }
    pub fn pressed_keys(&self) -> u32 {
        let bindings = &self.config.current().controls.bindings;
        [
            &bindings.forward,
            &bindings.backward,
            &bindings.left,
            &bindings.right,
            &bindings.jump,
            &bindings.sprint,
        ]
        .iter()
        .enumerate()
        .fold(0, |mask, (i, key)| {
            mask | ((u32::from(self.keys.iter().any(|k| format!("{k:?}") == **key))) << i)
        })
    }
    pub fn connect(&mut self) {
        let address = match ServerAddress::parse(&self.address) {
            Ok(a) => a,
            Err(e) => {
                self.notice = e.to_string();
                return;
            }
        };
        let mut options = ConnectOptions::offline(address, self.username.clone());
        options.view_distance = self.config.current().graphics.render_distance.min(32);
        let result = if let Some(session) = &self.online {
            if session.expired() {
                self.notice = "Session expired. Sign in again before joining.".into();
                return;
            }
            void_protocol::connect_online(
                options,
                void_protocol::OnlineIdentity::new(
                    session.account.uuid,
                    session.account.username.clone(),
                    session.access_token().to_owned(),
                ),
            )
        } else {
            void_protocol::connect_offline(options)
        };
        match result {
            Ok(connection) => {
                self.reset_world();
                self.connection = Some(connection);
                self.notice = "Connecting…".into();
            }
            Err(e) => self.notice = e.to_string(),
        }
    }
    pub fn login(&mut self) {
        if self.busy {
            return;
        }
        let settings = &self.config.current().account;
        let config = void_services::auth::AuthConfig {
            client_id: settings.client_id.clone(),
            redirect_uri: settings.redirect_uri.clone(),
        };
        if let Err(e) = config.validate() {
            self.notice = e.to_string();
            return;
        }
        self.busy = true;
        self.notice = "Complete Microsoft sign-in in your browser.".into();
        let tx = self.send.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<void_services::auth::OnlineSession> {
                let runtime = tokio::runtime::Runtime::new()?;
                runtime.block_on(async {
                    let flow = void_services::auth::begin_sign_in(config).await?;
                    flow.open_browser()?;
                    let (session, refresh) = flow.finish().await?;
                    void_services::auth::save_refresh_token(session.account.uuid, &refresh)?;
                    Ok(session)
                })
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Response::SignedIn(result));
        });
    }
    pub fn prepare_assets(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = "Preparing verified Minecraft 26.2 assets…".into();
        let tx = self.send.clone();
        let root = directories::ProjectDirs::from("rs", "Void", "Void.rs")
            .map(|p| p.data_local_dir().join("cache"))
            .unwrap_or_else(|| std::path::PathBuf::from("cache"));
        let installed = std::env::var_os("APPDATA")
            .map(|p| std::path::PathBuf::from(p).join(".minecraft"))
            .or_else(|| {
                std::env::var_os("HOME").map(|p| std::path::PathBuf::from(p).join(".minecraft"))
            });
        std::thread::spawn(move || {
            let result = (|| -> Result<String> {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async {
                    let manager = void_services::assets::AssetManager::new(root)?;
                    let metadata = manager.fetch_version().await?;
                    manager
                        .prepare_client_archive(&metadata, installed.as_deref())
                        .await?;
                    let progress = manager
                        .prepare_assets(&metadata, installed.as_deref(), |p| {
                            let _ = tx.try_send(Response::Assets(
                                format!(
                                    "Assets: {} / {} verified",
                                    p.cached + p.imported + p.downloaded,
                                    p.total
                                ),
                                false,
                            ));
                        })
                        .await?;
                    Ok(format!("{} assets verified and ready", progress.total))
                })
            })();
            let message = match result {
                Ok(s) => s,
                Err(e) => format!("Asset setup failed: {e:#}"),
            };
            let _ = tx.send(Response::Assets(message, true));
        });
    }
    pub fn disconnect(&mut self) {
        self.connection.take();
        self.set_capture(false);
        self.page = Page::Multiplayer;
        self.reset_world();
    }
    pub fn refresh_account(&mut self, index: usize) {
        if self.busy {
            return;
        }
        let Some(account) = self.accounts.get(index).cloned() else {
            return;
        };
        let settings = &self.config.current().account;
        let config = void_services::auth::AuthConfig {
            client_id: settings.client_id.clone(),
            redirect_uri: settings.redirect_uri.clone(),
        };
        if let Err(e) = config.validate() {
            self.notice = e.to_string();
            return;
        }
        self.busy = true;
        self.notice = format!("Refreshing {}…", account.username);
        let tx = self.send.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<void_services::auth::OnlineSession> {
                let refresh = void_services::auth::load_refresh_token(account.uuid)?;
                let rt = tokio::runtime::Runtime::new()?;
                let (session, rotated) =
                    rt.block_on(void_services::auth::refresh_account(&config, &refresh))?;
                void_services::auth::save_refresh_token(session.account.uuid, &rotated)?;
                Ok(session)
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Response::SignedIn(result));
        });
    }
    fn reset_world(&mut self) {
        self.epoch += 1;
        self.terrain = Terrain::new();
        self.world = ClientWorld::new();
        self.mesh.clear();
        self.mesh_revision += 1;
        self.mesh_built_revision = 0;
        self.pending_mesh = None;
        self.scheduler.set_epoch(WorldRevision {
            world: self.epoch,
            resources: 0,
        });
        self.mods.reset_jobs();
        self.conduct = None;
        self.sprinting = false;
        self.body = Default::default();
        self.last_network_move = Instant::now();
    }
    pub fn command(&mut self, command: ClientCommand) -> bool {
        if let Some(c) = &self.connection {
            if c.commands.try_send(command).is_err() {
                self.notice = "Network command queue is full".into();
                return false;
            }
            return true;
        }
        false
    }
    fn message(&mut self, message: String) {
        if self.messages.len() >= 200 {
            self.messages.pop_front();
        }
        self.messages.push_back(message);
    }
    pub fn set_capture(&mut self, value: bool) {
        self.captured = value;
        if let Some(window) = &self.window {
            if value {
                let _ = window
                    .set_cursor_grab(CursorGrabMode::Locked)
                    .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            } else {
                let _ = window.set_cursor_grab(CursorGrabMode::None);
            }
            window.set_cursor_visible(!value);
        }
    }
    fn receive(&mut self) {
        while let Ok(response) = self.responses.try_recv() {
            match response {
                Response::Status(status) => {
                    self.busy = false;
                    match status {
                        Ok(status) => {
                            self.notice = if status.protocol == 776 {
                                "Server responds with Minecraft 26.2".into()
                            } else {
                                format!(
                                    "Version mismatch: {} (protocol {})",
                                    status.version_name, status.protocol
                                )
                            };
                            self.status = Some(status);
                        }
                        Err(e) => self.notice = e,
                    }
                }
                Response::SignedIn(result) => {
                    self.busy = false;
                    match result {
                        Ok(session) => {
                            self.username = session.account.username.clone();
                            self.notice = format!("Signed in as {}", session.account.username);
                            match crate::accounts::save(self.config.path(), session.account.clone())
                            {
                                Ok(accounts) => self.accounts = accounts,
                                Err(e) => {
                                    self.notice =
                                        format!("Signed in; could not save account metadata: {e:#}")
                                }
                            }
                            self.online = Some(session);
                        }
                        Err(e) => self.notice = e,
                    }
                }
                Response::Assets(message, done) => {
                    self.notice = message;
                    if done {
                        self.busy = false;
                    }
                }
            }
        }
        let events: Vec<_> = self
            .connection
            .as_ref()
            .map(|c| c.events.try_iter().take(256).collect())
            .unwrap_or_default();
        for event in events {
            match event {
                ServerEvent::State(state) => self.notice = format!("{state:?}"),
                ServerEvent::Login { username, .. } => {
                    self.message(format!("Logged in as {username}"))
                }
                ServerEvent::World {
                    entity_id,
                    dimension,
                    ..
                } => {
                    self.reset_world();
                    self.own_entity = entity_id;
                    self.message(format!("Joined {dimension}"));
                    let entity = void_core::world::EntitySnapshot {
                        id: entity_id,
                        kind: 0,
                        position: Default::default(),
                        velocity: Default::default(),
                        orientation: Default::default(),
                    };
                    let _ = self.world.apply(
                        self.world.next_sequence(),
                        void_core::world::ServerEvent::Spawn(entity),
                    );
                    self.page = Page::Game;
                }
                ServerEvent::Chunk(chunk) => self.terrain.insert(chunk),
                ServerEvent::UnloadChunk { x, z } => self.terrain.remove(x, z),
                ServerEvent::Position(position) => {
                    self.position = position;
                    self.body.position = [position.x, position.y, position.z];
                    self.body.velocity = [0.; 3];
                    self.body.on_ground = position.on_ground;
                }
                ServerEvent::EntitySpawn {
                    id,
                    kind,
                    position,
                    velocity,
                    ..
                } => {
                    let entity = void_core::world::EntitySnapshot {
                        id,
                        kind,
                        position: void_core::world::Position {
                            x: position.x,
                            y: position.y,
                            z: position.z,
                        },
                        velocity: void_core::world::Velocity {
                            x: velocity[0],
                            y: velocity[1],
                            z: velocity[2],
                        },
                        orientation: void_core::world::Orientation {
                            yaw: position.yaw,
                            pitch: position.pitch,
                        },
                    };
                    let _ = self.world.apply(
                        self.world.next_sequence(),
                        void_core::world::ServerEvent::Spawn(entity),
                    );
                }
                ServerEvent::EntityPosition { id, position } => {
                    let _ = self.world.apply(
                        self.world.next_sequence(),
                        void_core::world::ServerEvent::Move {
                            id,
                            position: void_core::world::Position {
                                x: position.x,
                                y: position.y,
                                z: position.z,
                            },
                            orientation: void_core::world::Orientation {
                                yaw: position.yaw,
                                pitch: position.pitch,
                            },
                        },
                    );
                }
                ServerEvent::EntityVelocity { id, velocity } => {
                    if id == self.own_entity {
                        self.body.velocity = velocity;
                    }
                    let _ = self.world.apply(
                        self.world.next_sequence(),
                        void_core::world::ServerEvent::Velocity {
                            id,
                            velocity: void_core::world::Velocity {
                                x: velocity[0],
                                y: velocity[1],
                                z: velocity[2],
                            },
                        },
                    );
                }
                ServerEvent::EntityRemove(ids) => {
                    for id in ids {
                        let _ = self.world.apply(
                            self.world.next_sequence(),
                            void_core::world::ServerEvent::Despawn { id },
                        );
                    }
                }
                ServerEvent::BlockUpdate { x, y, z, state } => {
                    if let Some(chunk) = self
                        .terrain
                        .chunks
                        .get_mut(&(x.div_euclid(16), z.div_euclid(16)))
                    {
                        let chunk = Arc::make_mut(chunk);
                        if let Some(section) =
                            chunk.sections.iter_mut().find(|s| s.y == y.div_euclid(16))
                        {
                            let i = ((y.rem_euclid(16) * 16 + z.rem_euclid(16)) * 16
                                + x.rem_euclid(16)) as usize;
                            let old = section.block_states[i];
                            section.block_states[i] = state;
                            let air =
                                |s| void_protocol::block_states::info(s).is_some_and(|b| b.air);
                            if air(old) && !air(state) {
                                section.non_air_count = section.non_air_count.saturating_add(1);
                            } else if !air(old) && air(state) {
                                section.non_air_count = section.non_air_count.saturating_sub(1);
                            }
                            self.terrain.revision += 1;
                        }
                    }
                }
                ServerEvent::Chat(message) => self.message(message),
                ServerEvent::Health { health, food, .. } => {
                    self.health = health;
                    self.food = food;
                }
                ServerEvent::CodeOfConduct(text) => {
                    self.conduct = Some(text);
                    self.set_capture(false);
                }
                ServerEvent::Notice(message) => self.message(message),
                ServerEvent::Disconnected(reason) => {
                    self.notice = reason.clone();
                    self.message(reason);
                    self.connection.take();
                    self.set_capture(false);
                    if self.page == Page::Game {
                        self.page = Page::Multiplayer;
                    }
                }
                _ => {}
            }
        }
        self.scheduler
            .invalidate_content(JobKey(1), self.terrain.revision);
        while let Ok(result) = self.mesh_rx.try_recv() {
            self.mesh_busy = false;
            if result.epoch != self.epoch || result.revision < self.terrain.revision {
                continue;
            }
            if result.vertices.len() >= 3_900_000 {
                self.message(
                    "Terrain exceeds the current mesh capacity; distant geometry is incomplete"
                        .into(),
                );
            }
            let now = self.start.elapsed().as_micros() as u64;
            let job = Job {
                key: JobKey(1),
                revision: result.revision,
                epoch: WorldRevision {
                    world: self.epoch,
                    resources: 0,
                },
                kind: JobKind::MeshUpload,
                enqueued_at_us: now,
                nearby_visible: true,
                distance_squared: 0,
                estimated_cpu_us: 500,
                estimated_gpu_us: 0,
                resident_bytes: (result.vertices.len() * std::mem::size_of::<Vertex>()) as u64,
            };
            match self.scheduler.enqueue(job) {
                Ok(_) => self.pending_mesh = Some(result),
                Err(e) => {
                    self.mesh_built_revision = 0;
                    self.message(format!("Mesh upload rejected: {e}"));
                }
            }
        }
        if !self.mesh_busy && self.terrain.revision != self.mesh_built_revision {
            self.mesh_built_revision = self.terrain.revision;
            self.mesh_busy = true;
            let snapshot = self.terrain.chunks.clone();
            let epoch = self.epoch;
            let revision = self.terrain.revision;
            let tx = self.mesh_tx.clone();
            let center = [self.position.x, self.position.y, self.position.z];
            let radius = self.config.current().graphics.render_distance;
            std::thread::spawn(move || {
                let vertices = terrain::mesh(&snapshot, center, radius);
                let _ = tx.send(MeshResult {
                    epoch,
                    revision,
                    vertices,
                });
            });
        }
    }
    fn simulate(&mut self) {
        let mut catchup = 0;
        while self.page == Page::Game
            && self.last_network_move.elapsed() >= Duration::from_millis(50)
            && catchup < 5
        {
            self.last_network_move += Duration::from_millis(50);
            catchup += 1;
            let b = &self.config.current().controls.bindings;
            let pressed =
                |name: &str| self.captured && self.keys.iter().any(|k| format!("{k:?}") == name);
            let forward = pressed(&b.forward);
            let backward = pressed(&b.backward);
            let left = pressed(&b.left);
            let right = pressed(&b.right);
            let jump = pressed(&b.jump);
            let sprint = forward
                && !backward
                && self.food > 6
                && (pressed(&b.sprint) || self.config.current().controls.toggle_sprint);
            let input = void_core::physics::MovementInput {
                forward: i8::from(forward) as f32 - i8::from(backward) as f32,
                strafe: i8::from(left) as f32 - i8::from(right) as f32,
                jump,
                sprint,
                yaw_degrees: self.position.yaw,
            };
            if void_core::physics::tick(&mut self.body, input, &self.terrain).is_ok() {
                self.position.x = self.body.position[0];
                self.position.y = self.body.position[1];
                self.position.z = self.body.position[2];
                self.position.on_ground = self.body.on_ground;
            }
            self.command(ClientCommand::Input(
                u8::from(forward)
                    | u8::from(backward) << 1
                    | u8::from(left) << 2
                    | u8::from(right) << 3
                    | u8::from(jump) << 4
                    | u8::from(sprint) << 6,
            ));
            if sprint != self.sprinting {
                self.command(ClientCommand::Sprint(sprint));
                self.sprinting = sprint;
            }
            self.command(ClientCommand::Move(self.position));
        }
        if catchup == 5 && self.last_network_move.elapsed() > Duration::from_millis(50) {
            self.last_network_move = Instant::now();
            self.notice = "Simulation could not keep up; waiting for server corrections".into();
        }
    }
    fn draw(&mut self) -> Result<()> {
        let started = Instant::now();
        let timing = self.renderer.as_mut().unwrap().prepare_frame()?;
        self.gpu_us = timing.gpu_us;
        self.receive();
        if self.last_config_check.elapsed() > Duration::from_secs(1) {
            self.last_config_check = Instant::now();
            match self.config.reload() {
                Ok(reload) if reload.changed => {
                    self.settings_draft = self.config.current().clone();
                    self.apply_ui_settings();
                    if reload.restart_required {
                        self.notice =
                            "Configuration loaded; some graphics changes require restart".into();
                    }
                }
                Err(e) => self.notice = e.to_string(),
                _ => {}
            }
        }
        let now_us = self.start.elapsed().as_micros() as u64;
        let mod_metrics = void_sdk::FrameMetrics {
            fps: self.fps as f32,
            ping_ms: self
                .status
                .as_ref()
                .map(|s| s.latency_ms as u32)
                .unwrap_or(0),
            left_cps: self.left_clicks.len() as f32,
            right_cps: self.right_clicks.len() as f32,
            pressed_keys: self.pressed_keys(),
        };
        self.mods
            .prepare_frame(mod_metrics, &mut self.scheduler, now_us);
        let target = self.config.current().performance.target_fps.max(1);
        let period = if target == 1 {
            4167
        } else {
            1_000_000 / u64::from(target)
        };
        let jobs = self.scheduler.admit(FrameBudget {
            now_us,
            cpu_us: period.saturating_sub(started.elapsed().as_micros() as u64 + 300),
            gpu_us: period.saturating_sub(self.gpu_us.unwrap_or(0.) as u64),
        });
        for job in jobs {
            if job.job.key.0 >= 100 {
                self.mods
                    .execute(job, mod_metrics, &mut self.scheduler, now_us);
                continue;
            }
            if let Some(mesh) = self.pending_mesh.take() {
                if mesh.epoch == self.epoch && mesh.revision == self.terrain.revision {
                    let upload = Instant::now();
                    let candidate_revision = self.mesh_revision + 1;
                    self.renderer
                        .as_mut()
                        .unwrap()
                        .update_mesh(candidate_revision, &mesh.vertices)?;
                    let verdict = self.scheduler.complete(
                        job.ticket,
                        self.start.elapsed().as_micros() as u64,
                        upload.elapsed().as_micros() as u64,
                        0,
                    );
                    if verdict == void_core::scheduler::Completion::Publish {
                        self.mesh = mesh.vertices;
                        self.mesh_revision = candidate_revision;
                    }
                } else {
                    self.scheduler.cancel_completed(job.ticket);
                }
            } else {
                self.scheduler.cancel_completed(job.ticket);
            }
        }
        self.renderer
            .as_mut()
            .unwrap()
            .update_mesh(self.mesh_revision, &self.mesh)?;
        self.renderer
            .as_mut()
            .unwrap()
            .update_entities(&crate::entities::mesh(&self.world, self.own_entity))?;
        self.simulate();
        let raw = self
            .egui_state
            .as_mut()
            .unwrap()
            .take_egui_input(self.window.as_ref().unwrap());
        let ctx = self.ctx.clone();
        let output = ctx.run(raw, |ctx| ui::draw(ctx, self));
        self.egui_state.as_mut().unwrap().handle_platform_output(
            self.window.as_ref().unwrap(),
            output.platform_output.clone(),
        );
        let camera = if self.page == Page::Game {
            let position = Vec3::new(
                self.position.x as f32,
                self.position.y as f32 + 1.62,
                self.position.z as f32,
            );
            let yaw = self.position.yaw.to_radians();
            let pitch = self.position.pitch.to_radians();
            let direction = Vec3::new(
                -yaw.sin() * pitch.cos(),
                -pitch.sin(),
                yaw.cos() * pitch.cos(),
            );
            let extent = self.renderer.as_ref().unwrap().extent;
            Some(
                Mat4::perspective_rh(
                    self.config.current().controls.fov_degrees.to_radians(),
                    extent.width as f32 / extent.height as f32,
                    0.05,
                    2048.,
                ) * Mat4::look_to_rh(position, direction, Vec3::Y),
            )
        } else {
            None
        };
        let capture = if self.frames == self.args.smoke_frames.unwrap_or(6).saturating_sub(1) {
            self.args.screenshot.clone()
        } else {
            None
        };
        if let Some(path) = &capture
            && let Some(parent) = path.parent()
        {
            std::fs::create_dir_all(parent)?;
        }
        let submitted = self.renderer.as_mut().unwrap().render(
            self.window.as_ref().unwrap(),
            &ctx,
            output,
            camera,
            capture.as_deref(),
        )?;
        if submitted {
            let now = Instant::now();
            let frame_us = now.duration_since(self.last_frame).as_secs_f64() * 1e6;
            self.last_frame = now;
            self.cpu_us = started.elapsed().as_secs_f64() * 1e6;
            self.fps = if self.frames == 0 {
                1e6 / frame_us
            } else {
                self.fps * 0.92 + (1e6 / frame_us) * 0.08
            };
            if self.frame_history.len() >= 180 {
                self.frame_history.pop_front();
            }
            self.frame_history.push_back(frame_us);
            if self.samples.len() >= 120_000 {
                self.samples.pop_front();
            }
            self.samples.push_back(Sample {
                frame_us,
                cpu_us: self.cpu_us,
                gpu_us: self.gpu_us,
                fence_wait_us: timing.fence_wait_us,
                input_to_submit_us: self
                    .last_input
                    .take()
                    .map(|t| now.duration_since(t).as_secs_f64() * 1e6),
                chunks: self.terrain.chunks.len(),
                vertices: self.mesh.len(),
            });
            self.frames += 1;
        }
        Ok(())
    }
    fn save_metrics(&self) {
        if let Some(path) = &self.args.metrics {
            let data = serde_json::json!({"schema_version":1,"kind":"actual_client_frames","scenario":if self.terrain.chunks.is_empty(){"menu"}else{"server_terrain_debug"},"mode":format!("{:?}",self.scheduler.mode()),"gpu":self.renderer.as_ref().map(|r|r.gpu_name.clone()),"input_metric":"event_received_to_submission_return; not input-to-photon","samples":self.samples});
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_vec_pretty(&data) {
                let _ = std::fs::write(path, json);
            }
        }
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.renderer.take();
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| -> Result<()> {
            let g = &self.config.current().graphics;
            let window = event_loop.create_window(
                Window::default_attributes()
                    .with_title("Void.rs — Event Horizon")
                    .with_inner_size(winit::dpi::PhysicalSize::new(g.width, g.height))
                    .with_min_inner_size(winit::dpi::LogicalSize::new(960., 600.)),
            )?;
            let renderer = Renderer::new(&window, g.vsync)?;
            self.apply_ui_settings();
            self.egui_state = Some(egui_winit::State::new(
                self.ctx.clone(),
                egui::ViewportId::ROOT,
                &window,
                Some(window.scale_factor() as f32),
                None,
                None,
            ));
            self.renderer = Some(renderer);
            self.window = Some(window);
            if self.args.connect.is_some() {
                self.connect();
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.failure = Some(format!("{e:#}"));
            event_loop.exit();
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let (Some(state), Some(window)) = (&mut self.egui_state, &self.window) {
            let _ = state.on_window_event(window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                self.save_metrics();
                event_loop.exit();
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                if let (Some(r), Some(w)) = (&mut self.renderer, &self.window)
                    && let Err(e) = r.resize(w, self.config.current().graphics.vsync)
                {
                    self.failure = Some(e.to_string());
                    event_loop.exit();
                }
            }
            WindowEvent::Focused(false) => {
                self.keys.clear();
                self.set_capture(false);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.last_input = Some(Instant::now());
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        self.keys.insert(code);
                    } else {
                        self.keys.remove(&code);
                    }
                    if code == KeyCode::Escape && event.state == ElementState::Pressed {
                        self.set_capture(!self.captured && self.page == Page::Game);
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                self.last_input = Some(Instant::now());
                if self.captured {
                    match button {
                        winit::event::MouseButton::Left => {
                            self.left_clicks.push_back(Instant::now());
                            if self.command(ClientCommand::Move(self.position))
                                && let Some(id) = crate::entities::attack_target(
                                    &self.world,
                                    self.own_entity,
                                    self.position,
                                    &self.terrain,
                                )
                            {
                                self.command(ClientCommand::Attack(id));
                            }
                            self.command(ClientCommand::Swing);
                        }
                        winit::event::MouseButton::Right => {
                            self.right_clicks.push_back(Instant::now())
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if self.scheduler.mode() != void_core::SchedulingMode::Baseline {
                    match self.renderer.as_ref().unwrap().is_frame_ready() {
                        Ok(true) => {}
                        Ok(false) => {
                            self.next_frame = Instant::now() + Duration::from_micros(250);
                            return;
                        }
                        Err(e) => {
                            self.failure = Some(e.to_string());
                            event_loop.exit();
                            return;
                        }
                    }
                }
                if let Err(e) = self.draw() {
                    self.failure = Some(format!("{e:#}"));
                    self.save_metrics();
                    event_loop.exit();
                    return;
                }
                if self.args.smoke_frames.is_some_and(|n| self.frames >= n) {
                    self.save_metrics();
                    event_loop.exit();
                }
                let target = self.config.current().performance.target_fps;
                self.next_frame = if target == 0 {
                    Instant::now()
                } else {
                    let next = self.next_frame + Duration::from_secs_f64(1. / target as f64);
                    if next < Instant::now() {
                        Instant::now()
                    } else {
                        next
                    }
                };
            }
            _ => {}
        }
    }
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.captured
        {
            self.last_input = Some(Instant::now());
            let sensitivity = self.config.current().controls.mouse_sensitivity * 0.3;
            self.position.yaw += delta.0 as f32 * sensitivity;
            self.position.pitch = (self.position.pitch
                + delta.1 as f32
                    * sensitivity
                    * if self.config.current().controls.invert_y {
                        -1.
                    } else {
                        1.
                    })
            .clamp(-90., 90.);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if Instant::now() >= self.next_frame
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
        self.receive();
        self.simulate();
        let wake = if self.page == Page::Game {
            self.next_frame
                .min(self.last_network_move + Duration::from_millis(50))
        } else {
            self.next_frame
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(wake));
    }
}
