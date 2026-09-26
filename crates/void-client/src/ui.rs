use crate::app::{App, Page};
use egui::{Color32, Context, RichText, Vec2};
use void_core::SchedulingMode;

const TEXT: Color32 = Color32::from_rgb(234, 230, 224);
const MUTED: Color32 = Color32::from_rgb(132, 136, 147);
const ACCENT: Color32 = Color32::from_rgb(239, 171, 101);
pub fn style(ctx: &Context, config: &void_core::config::UiConfig) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.window_fill = Color32::from_rgba_unmultiplied(13, 16, 23, 248);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(24, 28, 36);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 40, 40);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(66, 48, 34);
    style.visuals.selection.bg_fill = Color32::from_rgb(108, 70, 36);
    let [r, g, b] = config.accent.map(|v| (v * 255.).round() as u8);
    style.visuals.hyperlink_color = Color32::from_rgb(r, g, b);
    let [r, g, b] = config.background.map(|v| (v * 255.).round() as u8);
    style.visuals.window_fill = Color32::from_rgba_unmultiplied(r, g, b, 248);
    style.spacing.item_spacing = Vec2::new(12., 12.);
    style.spacing.button_padding = Vec2::new(18., 12.);
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(16.));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(16.));
    ctx.set_style(style);
}
fn button(ui: &mut egui::Ui, label: &str) -> bool {
    ui.add_sized([272., 48.], egui::Button::new(label).corner_radius(8.))
        .clicked()
}
pub fn draw(ctx: &Context, app: &mut App) {
    let rect = ctx.content_rect();
    while app
        .left_clicks
        .front()
        .is_some_and(|t| t.elapsed().as_secs_f32() > 1.)
    {
        app.left_clicks.pop_front();
    }
    while app
        .right_clicks
        .front()
        .is_some_and(|t| t.elapsed().as_secs_f32() > 1.)
    {
        app.right_clicks.pop_front();
    }
    if app.page == Page::Game {
        game(ctx, app);
        return;
    }
    egui::Area::new("brand".into())
        .fixed_pos([48., 32.])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (response, painter) =
                    ui.allocate_painter(Vec2::splat(24.), egui::Sense::hover());
                painter.circle_stroke(
                    response.rect.center(),
                    8.,
                    egui::Stroke::new(2.0_f32, ACCENT),
                );
                ui.label(RichText::new("V O I D . R S").size(19.).strong());
                ui.add_space(16.);
                ui.label(RichText::new("JAVA 26.2").size(11.).color(MUTED));
            });
        });
    egui::Area::new("build".into())
        .anchor(egui::Align2::RIGHT_TOP, [-40., 40.])
        .show(ctx, |ui| {
            ui.label(
                RichText::new("DEVELOPMENT BUILD  /  0.1.0")
                    .size(11.)
                    .color(MUTED),
            );
        });
    if app.page == Page::Home {
        egui::Area::new("home".into())
            .fixed_pos([72., (rect.height() * 0.25).max(125.)])
            .show(ctx, |ui| {
                ui.set_width(360.);
                ui.label(
                    RichText::new("BEYOND THE HORIZON")
                        .color(ACCENT)
                        .size(11.)
                        .extra_letter_spacing(2.),
                );
                ui.add_space(8.);
                ui.label(RichText::new("Void.rs").size(72.).strong());
                ui.label(
                    RichText::new("A different kind of Minecraft client.")
                        .color(MUTED)
                        .size(17.),
                );
                ui.add_space(30.);
                if button(ui, "Multiplayer") {
                    app.page = Page::Multiplayer;
                }
                if button(ui, "Settings") {
                    app.page = Page::Settings;
                }
                if button(ui, "Mods & HUD") {
                    app.page = Page::Mods;
                }
                if button(ui, "Performance") {
                    app.page = Page::Diagnostics;
                }
                ui.add_space(15.);
                ui.label(
                    RichText::new("Built in Rust. Drawn in Vulkan.")
                        .color(MUTED)
                        .size(12.),
                );
            });
        egui::Area::new("orbit-label".into())
            .fixed_pos([rect.width() * 0.64, rect.height() * 0.77])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("E V E N T   H O R I Z O N")
                        .size(12.)
                        .color(ACCENT),
                );
                ui.label(
                    RichText::new("Responsiveness, under pressure.")
                        .size(13.)
                        .color(MUTED),
                );
            });
    } else {
        let title = match app.page {
            Page::Multiplayer => "Multiplayer",
            Page::Settings => "Settings",
            Page::Mods => "Mods & HUD",
            _ => "Performance",
        };
        egui::Area::new("content".into())
            .fixed_pos([72., 120.])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(10, 13, 20, 244))
                    .corner_radius(14.)
                    .inner_margin(28.)
                    .show(ui, |ui| {
                        ui.set_width((rect.width() - 200.).min(710.));
                        ui.horizontal(|ui| {
                            if ui.button("Back").clicked() {
                                app.page = Page::Home;
                            }
                            ui.add_space(10.);
                            ui.heading(title);
                        });
                        ui.add_space(12.);
                        egui::ScrollArea::vertical()
                            .max_height((rect.height() - 260.).max(200.))
                            .show(ui, |ui| match app.page {
                                Page::Multiplayer => multiplayer(ui, app),
                                Page::Settings => settings(ui, app),
                                Page::Mods => mods(ui, app),
                                _ => diagnostics(ui, app),
                            });
                    });
            });
    }
    egui::Area::new("footer".into())
        .anchor(egui::Align2::LEFT_BOTTOM, [48., -28.])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{:.0} FPS", app.fps))
                        .color(MUTED)
                        .size(11.),
                );
                ui.separator();
                ui.label(
                    RichText::new(
                        app.renderer
                            .as_ref()
                            .map(|r| format!("{}  ·  Vulkan {}", r.gpu_name, r.api_version))
                            .unwrap_or_default(),
                    )
                    .color(MUTED)
                    .size(11.),
                );
            });
        });
}
fn multiplayer(ui: &mut egui::Ui, app: &mut App) {
    ui.label("Server address");
    ui.add(
        egui::TextEdit::singleline(&mut app.address)
            .desired_width(f32::INFINITY)
            .hint_text("play.example.net"),
    );
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !app.busy,
                egui::Button::new(if app.busy {
                    "Checking…"
                } else {
                    "Check server"
                }),
            )
            .clicked()
        {
            app.ping();
        }
        if app.connection.is_some() && ui.button("Disconnect").clicked() {
            app.disconnect();
        }
    });
    if let Some(status) = &app.status {
        ui.add_space(8.);
        ui.label(RichText::new(&status.description).strong());
        ui.label(
            RichText::new(format!(
                "{}  ·  {} / {} players  ·  {:.0} ms",
                status.version_name, status.online, status.maximum, status.latency_ms
            ))
            .color(MUTED),
        );
    }
    ui.separator();
    ui.label(RichText::new("Account").strong());
    if let Some(session) = &app.online {
        ui.label(format!("Microsoft · {}", session.account.username));
        if ui
            .add_enabled(
                app.connection.is_none(),
                egui::Button::new("Use offline test identity"),
            )
            .clicked()
        {
            app.online = None;
            app.username = app.args.username.clone();
        }
    } else {
        ui.label(RichText::new("Sign in to join online servers. Without an account, connections use an offline test identity.").color(MUTED));
        if ui
            .add_enabled(!app.busy, egui::Button::new("Sign in with Microsoft"))
            .clicked()
        {
            app.login();
        }
    }
    if !app.accounts.is_empty() {
        let mut refresh = None;
        for (i, account) in app.accounts.iter().enumerate() {
            if ui
                .add_enabled(
                    !app.busy && app.connection.is_none(),
                    egui::Button::new(format!("Switch / refresh {}", account.username)),
                )
                .clicked()
            {
                refresh = Some(i);
            }
        }
        if let Some(index) = refresh {
            app.refresh_account(index);
        }
    }
    ui.horizontal(|ui| {
        ui.label("Player name");
        ui.text_edit_singleline(&mut app.username);
    });
    if ui
        .add_enabled(
            app.connection.is_none(),
            egui::Button::new(if app.online.is_some() {
                "Join server"
            } else {
                "Connect with offline identity"
            }),
        )
        .clicked()
    {
        app.connect();
    }
    if !app.notice.is_empty() {
        ui.add_space(8.);
        ui.label(RichText::new(&app.notice).color(ACCENT));
    }
    if let Some(text) = app.conduct.clone() {
        ui.separator();
        ui.label(text);
        ui.horizontal(|ui| {
            if ui.button("Accept server rules").clicked() {
                app.command(void_protocol::ClientCommand::AcceptCodeOfConduct);
                app.conduct = None;
            }
            if ui.button("Decline").clicked() {
                app.disconnect();
            }
        });
    }
}
fn settings(ui: &mut egui::Ui, app: &mut App) {
    let mut config = app.settings_draft.clone();
    ui.label(RichText::new("Graphics").strong());
    ui.add(
        egui::Slider::new(&mut config.graphics.render_distance, 2..=32)
            .text("Render distance · chunks"),
    );
    ui.add(
        egui::Slider::new(&mut config.performance.target_fps, 0..=1000)
            .text("Frame limit · 0 = uncapped"),
    );
    ui.checkbox(&mut config.graphics.vsync, "VSync");
    ui.separator();
    ui.label(RichText::new("Controls & interface").strong());
    ui.add(
        egui::Slider::new(&mut config.controls.mouse_sensitivity, 0.05..=2.)
            .text("Mouse sensitivity"),
    );
    ui.add(egui::Slider::new(&mut config.controls.fov_degrees, 30.0..=110.0).text("Field of view"));
    ui.add(egui::Slider::new(&mut config.ui.scale, 0.75..=2.).text("UI scale"));
    ui.checkbox(&mut config.controls.invert_y, "Invert mouse Y");
    ui.checkbox(&mut config.controls.toggle_sprint, "Toggle sprint");
    ui.collapsing("HUD layout", |ui| {
        for (name, widget) in &mut config.ui.hud {
            ui.label(name);
            ui.checkbox(&mut widget.enabled, "Visible");
            ui.add(egui::Slider::new(&mut widget.x, 0.0..=1920.0).text("X"));
            ui.add(egui::Slider::new(&mut widget.y, 0.0..=1080.0).text("Y"));
            ui.add(egui::Slider::new(&mut widget.scale, 0.5..=3.0).text("Scale"));
        }
    });
    ui.checkbox(
        &mut config.ui.video_enabled,
        "Play menu video when available",
    );
    ui.separator();
    ui.collapsing("Microsoft application registration",|ui|{
        ui.label("Application client ID");ui.text_edit_singleline(&mut config.account.client_id);
        ui.label("Loopback redirect URI");ui.text_edit_singleline(&mut config.account.redirect_uri);
        ui.label(RichText::new("A registered application is required for Microsoft login. No tokens are stored in TOML.").color(MUTED));
    });
    if ui
        .add_enabled(
            !app.busy,
            egui::Button::new("Prepare Minecraft 26.2 assets"),
        )
        .clicked()
    {
        app.prepare_assets();
    }
    ui.add_space(8.);
    if ui.button("Save settings").clicked() {
        match app.config.replace_and_save(config.clone()) {
            Ok(()) => {
                app.apply_ui_settings();
                app.notice =
                    "Settings saved to TOML. Presentation changes apply on resize/restart.".into();
            }
            Err(e) => app.notice = e.to_string(),
        }
    }
    app.settings_draft = config;
    ui.label(RichText::new(&app.notice).color(ACCENT));
    ui.label(
        RichText::new(app.config.path().display().to_string())
            .size(11.)
            .color(MUTED),
    );
}
fn mods(ui: &mut egui::Ui, app: &mut App) {
    ui.label(
        RichText::new("Your client, your modules.")
            .size(23.)
            .strong(),
    );
    ui.label(RichText::new("Rust SDK · portable Wasm · trusted native extensions").color(MUTED));
    ui.separator();
    for (name, description) in [
        (
            "Metrics HUD",
            "FPS, ping, click rate and keys — drawn through the public SDK.",
        ),
        (
            "Wasm runtime",
            "Bounded memory and execution. No implicit filesystem or network access.",
        ),
        (
            "Native runtime",
            "Versioned DLL/SO interface. Install only modules you trust.",
        ),
    ] {
        ui.label(RichText::new(name).strong());
        ui.label(description);
        ui.add_space(8.);
    }
    ui.separator();
    ui.label(
        RichText::new(app.mods.directory.display().to_string())
            .size(12.)
            .color(MUTED),
    );
    if ui.button("Scan mods folder").clicked() {
        app.mods.scan();
    }
    let mut enable = None;
    let mut disable = None;
    for i in 0..app.mods.available.len() {
        let id = app.mods.available[i].manifest.id.clone();
        let enabled = app.mods.enabled(&id);
        ui.horizontal(|ui| {
            let item = &mut app.mods.available[i];
            ui.label(format!(
                "{} {} · {:?}",
                id, item.manifest.version, item.manifest.runtime
            ));
            if item.manifest.runtime == void_services::mods::Runtime::Native {
                ui.checkbox(&mut item.trust_native, "I trust this native code");
            }
            if ui
                .add_enabled(
                    !enabled && !app.mods.loading,
                    egui::Button::new(if enabled { "Enabled" } else { "Enable" }),
                )
                .clicked()
            {
                enable = Some(i);
            }
            if enabled && ui.button("Disable").clicked() {
                disable = Some(id.clone());
            }
        });
        if let Some(error) = app.mods.error(&id) {
            ui.label(RichText::new(error).color(ACCENT));
        }
    }
    if let Some(index) = enable {
        app.mods.enable(index);
    }
    if let Some(id) = disable {
        app.mods.disable(&id);
    }
    ui.label(RichText::new(&app.mods.notice).color(ACCENT));
    ui.separator();
    ui.label("SDK preview");
    let mut commands = vec![];
    void_sdk::draw_metrics(
        void_sdk::FrameMetrics {
            fps: app.fps as f32,
            ping_ms: app
                .status
                .as_ref()
                .map(|s| s.latency_ms as u32)
                .unwrap_or(0),
            left_cps: app.left_clicks.len() as f32,
            right_cps: app.right_clicks.len() as f32,
            pressed_keys: app.pressed_keys(),
        },
        &mut commands,
    );
    for text in commands {
        ui.monospace(text.text);
    }
}
fn diagnostics(ui: &mut egui::Ui, app: &mut App) {
    ui.label(RichText::new("Event Horizon").size(25.).strong());
    ui.label(
        RichText::new("Experimental scheduling. Performance gains have not been established.")
            .color(ACCENT),
    );
    let mut config = app.settings_draft.clone();
    egui::ComboBox::from_label("Mode")
        .selected_text(format!("{:?}", config.performance.event_horizon.mode))
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut config.performance.event_horizon.mode,
                SchedulingMode::Baseline,
                "Baseline",
            );
            ui.selectable_value(
                &mut config.performance.event_horizon.mode,
                SchedulingMode::FrameCoordination,
                "Frame coordination",
            );
            ui.selectable_value(
                &mut config.performance.event_horizon.mode,
                SchedulingMode::EventHorizon,
                "Event Horizon",
            );
        });
    if ui.button("Save mode for next launch").clicked()
        && let Err(e) = app.config.replace_and_save(config.clone())
    {
        app.notice = e.to_string();
    }
    app.settings_draft = config;
    ui.label(format!(
        "Mod callbacks over budget: {}",
        app.mods.callback_overruns
    ));
    ui.add_space(10.);
    ui.horizontal(|ui| {
        ui.label(format!("{:.0} FPS", app.fps));
        ui.separator();
        ui.label(format!("CPU {:.2} ms", app.cpu_us / 1000.));
        ui.separator();
        ui.label(format!("GPU {:.2} ms", app.gpu_us.unwrap_or(0.) / 1000.));
    });
    let (response, painter) =
        ui.allocate_painter(Vec2::new(ui.available_width(), 110.), egui::Sense::hover());
    painter.rect_filled(response.rect, 6., Color32::from_rgb(18, 23, 32));
    let samples: Vec<_> = app.frame_history.iter().copied().collect();
    if samples.len() > 1 {
        let points = samples
            .iter()
            .enumerate()
            .map(|(i, us)| {
                egui::pos2(
                    response.rect.left()
                        + i as f32 / (samples.len() - 1) as f32 * response.rect.width(),
                    response.rect.bottom()
                        - (*us as f32 / 16000.).clamp(0., 1.) * response.rect.height(),
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(1.5_f32, ACCENT),
        ));
    }
    ui.label(
        RichText::new("Frame time · 0–16 ms · actual frames")
            .size(11.)
            .color(MUTED),
    );
    let stats = app.scheduler.stats();
    ui.label(format!(
        "Queued {} · admitted {} · stale {} · deadlines missed {}",
        app.scheduler.pending_count(),
        stats.admitted,
        stats.discarded_stale,
        stats.missed_deadlines
    ));
    ui.label(format!(
        "Scheduled memory {:.2} MiB",
        app.scheduler.resident_bytes() as f64 / 1048576.
    ));
    ui.separator();
    ui.collapsing("Connection messages", |ui| {
        for message in app.messages.iter().rev().take(20) {
            ui.label(message);
        }
    });
}
fn game(ctx: &Context, app: &mut App) {
    let rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        "crosshair".into(),
    ));
    let c = rect.center();
    painter.line_segment(
        [c - egui::vec2(6., 0.), c + egui::vec2(6., 0.)],
        egui::Stroke::new(1.5_f32, Color32::WHITE),
    );
    painter.line_segment(
        [c - egui::vec2(0., 6.), c + egui::vec2(0., 6.)],
        egui::Stroke::new(1.5_f32, Color32::WHITE),
    );
    for t in app.mods.hud() {
        let [r, g, b, a] = t.rgba.to_be_bytes();
        painter.text(
            egui::pos2(t.x, t.y),
            egui::Align2::LEFT_TOP,
            &t.text,
            egui::FontId::proportional(16.),
            Color32::from_rgba_unmultiplied(r, g, b, a),
        );
    }
    let mut commands = vec![];
    void_sdk::draw_metrics(
        void_sdk::FrameMetrics {
            fps: app.fps as f32,
            ping_ms: app
                .status
                .as_ref()
                .map(|s| s.latency_ms as u32)
                .unwrap_or(0),
            left_cps: app.left_clicks.len() as f32,
            right_cps: app.right_clicks.len() as f32,
            pressed_keys: app.pressed_keys(),
        },
        &mut commands,
    );
    for (i, t) in commands.iter().enumerate() {
        let key = if i == 3 { "keystrokes" } else { "performance" };
        if let Some(widget) = app.config.current().ui.hud.get(key)
            && widget.enabled
        {
            let row = if i == 3 { 0 } else { i };
            painter.text(
                egui::pos2(widget.x, widget.y + row as f32 * 20. * widget.scale),
                egui::Align2::LEFT_TOP,
                &t.text,
                egui::FontId::monospace(16. * widget.scale),
                Color32::WHITE,
            );
        }
    }
    egui::Area::new("world-diagnostics".into())
        .anchor(egui::Align2::LEFT_BOTTOM, [16., -20.])
        .show(ctx, |ui| {
            ui.label(format!(
                "XYZ {:.1} / {:.1} / {:.1}",
                app.position.x, app.position.y, app.position.z
            ));
            ui.label(format!(
                "{} chunks received · health {:.0} · food {}",
                app.terrain.chunks.len(),
                app.health,
                app.food
            ));
            ui.label(
                RichText::new("Terrain inspection build · vanilla models and physics incomplete")
                    .color(ACCENT),
            );
        });
    if !app.captured && app.args.smoke_frames.is_none() {
        egui::Window::new("Connected")
            .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("Mouse-look inspects the server terrain. Escape releases the cursor.");
                if ui.button("Look around").clicked() {
                    app.set_capture(true);
                }
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut app.chat_input);
                    if ui.button("Send chat").clicked() {
                        let text = std::mem::take(&mut app.chat_input);
                        app.command(void_protocol::ClientCommand::Chat(text));
                    }
                });
                if app.health <= 0. && ui.button("Respawn").clicked() {
                    app.command(void_protocol::ClientCommand::Respawn);
                }
                if ui.button("Disconnect").clicked() {
                    app.disconnect();
                }
                for m in app.messages.iter().rev().take(4) {
                    ui.label(m);
                }
            });
    }
}
