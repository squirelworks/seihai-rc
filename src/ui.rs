//! The window: `seihai-rc gui`, built only with `--features gui`.
//!
//! A login form, then the RC chat and log in the middle, the players and
//! RCs on the left, and actions for the selected player on the right. The
//! session runs on its own thread ([`crate::live`]); this module only
//! draws [`RcView`] and turns clicks into [`Command`]s.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui;

use crate::live::{warp_command, Command, LiveSession, LoginForm, RcView, Status};

/// How often the window looks for updates when nothing else repaints it.
const REFRESH: Duration = Duration::from_millis(100);

/// Open the window and block until it is closed.
/// With `connect_now` the window logs in at once with `form`.
pub fn run(form: LoginForm, connect_now: bool) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("seihai-rc")
            .with_inner_size([1000.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "seihai-rc",
        options,
        Box::new(move |_cc| {
            let mut app = App::new(form);
            if connect_now {
                app.connect();
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())
}

#[derive(Default)]
struct Actions {
    pm: String,
    admin_pm: String,
    level: String,
    x: String,
    y: String,
    /// The id a first click on Disconnect armed.
    kick_armed: Option<u16>,
}

struct App {
    form: LoginForm,
    form_error: Option<String>,
    live: Option<LiveSession>,
    view: RcView,
    chat: String,
    selected: Option<u16>,
    actions: Actions,
}

impl App {
    fn new(form: LoginForm) -> Self {
        Self {
            form,
            form_error: None,
            live: None,
            view: RcView::default(),
            chat: String::new(),
            selected: None,
            actions: Actions {
                x: "30".into(),
                y: "30".into(),
                ..Default::default()
            },
        }
    }

    fn send(&mut self, c: Command) {
        if let Some(live) = &self.live {
            live.send(c);
        }
    }

    fn connect(&mut self) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        match self.form.params((nanos >> 10) as u8) {
            Ok((addr, params)) => {
                self.form_error = None;
                self.view = RcView {
                    verbose: self.view.verbose,
                    status: Status::Connecting,
                    ..Default::default()
                };
                self.selected = None;
                self.live = Some(LiveSession::start(addr, params, Duration::from_secs(10)));
            }
            Err(e) => self.form_error = Some(e),
        }
    }

    fn login_screen(&mut self, ui: &mut egui::Ui) {
        ui.heading("Log in");
        egui::Grid::new("login").num_columns(2).show(ui, |ui| {
            ui.label("Server (HOST:PORT)");
            ui.text_edit_singleline(&mut self.form.server);
            ui.end_row();
            ui.label("Account");
            ui.text_edit_singleline(&mut self.form.account);
            ui.end_row();
            ui.label("Password");
            ui.add(egui::TextEdit::singleline(&mut self.form.password).password(true));
            ui.end_row();
            ui.label("Nickname");
            ui.add(egui::TextEdit::singleline(&mut self.form.nickname).hint_text("*account"));
            ui.end_row();
            ui.label("Login");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.form.classic, false, "gen5 (type 6)");
                ui.radio_value(&mut self.form.classic, true, "classic (type 1)");
            });
            ui.end_row();
        });
        let busy = self.view.status == Status::Connecting;
        if ui.add_enabled(!busy, egui::Button::new("Log in")).clicked() {
            self.connect();
        }
        if let Some(e) = &self.form_error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        match &self.view.status {
            Status::Connecting => {
                ui.label("Logging in…");
            }
            Status::Failed(why) | Status::Closed(why) => {
                ui.colored_label(ui.visuals().warn_fg_color, why);
            }
            _ => {}
        }
    }

    fn players_panel(&mut self, ui: &mut egui::Ui) {
        let entries: Vec<_> = self.view.sorted_players().into_iter().cloned().collect();
        let n_players = entries.iter().filter(|e| !e.is_rc()).count();
        ui.heading(format!("Players ({n_players})"));
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut rc_header = false;
            for e in &entries {
                if e.is_rc() && !rc_header {
                    ui.separator();
                    ui.heading(format!("RCs ({})", entries.len() - n_players));
                    rc_header = true;
                }
                let text = if e.is_rc() {
                    format!("{}  {}  {}", e.id, e.account, e.nickname)
                } else {
                    format!("{}  {}  {}  ({})", e.id, e.account, e.nickname, e.level)
                };
                let selected = self.selected == Some(e.id);
                if ui.selectable_label(selected, text).clicked() {
                    self.selected = Some(e.id);
                    self.actions.kick_armed = None;
                }
            }
        });
    }

    fn actions_panel(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.label("Pick a player on the left.");
            return;
        };
        let Some(e) = self.view.players.players.get(&id).cloned() else {
            ui.label(format!("Player {id} has left."));
            return;
        };
        ui.heading(format!("{} {}", e.id, e.account));
        ui.label(format!("nickname: {}", e.nickname));
        if !e.is_rc() {
            ui.label(format!("level: {}", e.level));
        }
        ui.separator();

        ui.label("Private message");
        ui.text_edit_singleline(&mut self.actions.pm);
        if ui.button("Send PM").clicked() && !self.actions.pm.is_empty() {
            let text = std::mem::take(&mut self.actions.pm);
            self.send(Command::Pm {
                ids: vec![id],
                text,
            });
        }
        ui.label("Admin message");
        ui.text_edit_singleline(&mut self.actions.admin_pm);
        if ui.button("Send admin message").clicked() && !self.actions.admin_pm.is_empty() {
            let text = std::mem::take(&mut self.actions.admin_pm);
            self.send(Command::AdminPm { id, text });
        }
        ui.separator();

        ui.label("Warp to level, x, y");
        ui.text_edit_singleline(&mut self.actions.level);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.actions.x).desired_width(50.0));
            ui.add(egui::TextEdit::singleline(&mut self.actions.y).desired_width(50.0));
        });
        if ui.button("Warp").clicked() {
            match warp_command(id, &self.actions.level, &self.actions.x, &self.actions.y) {
                Ok(c) => self.send(c),
                Err(err) => self.view.log.push(format!("[error] {err}")),
            }
        }
        ui.separator();

        if ui.button("Read props").clicked() {
            self.send(Command::Props(id));
        }
        let armed = self.actions.kick_armed == Some(id);
        let label = if armed {
            "Click again to disconnect"
        } else {
            "Disconnect"
        };
        if ui.button(label).clicked() {
            if armed {
                self.actions.kick_armed = None;
                self.send(Command::Kick(id));
            } else {
                self.actions.kick_armed = Some(id);
            }
        }
        if let Some((pid, lines)) = &self.view.props {
            if *pid == id {
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("props")
                    .show(ui, |ui| {
                        ui.monospace(lines.join("\n"));
                    });
            }
        }
    }

    fn chat_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.chat)
                    .hint_text("RC chat")
                    .desired_width(ui.available_width() - 220.0),
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Send").clicked() || enter) && !self.chat.is_empty() {
                let line = std::mem::take(&mut self.chat);
                self.send(Command::Chat(line));
                edit.request_focus();
            }
            if ui.button("Message all").clicked() && !self.chat.is_empty() {
                let text = std::mem::take(&mut self.chat);
                self.send(Command::AdminAll(text));
            }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(live) = &self.live {
            for u in live.drain() {
                self.view.apply(u);
            }
        }
        if matches!(self.view.status, Status::Failed(_) | Status::Closed(_)) {
            self.live = None;
        }
        let online = matches!(self.view.status, Status::Online(_));

        egui::Panel::top("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let text = match &self.view.status {
                    Status::LoggedOut => "Logged out".to_string(),
                    Status::Connecting => "Logging in…".to_string(),
                    Status::Online(addr) => format!("Online at {addr} as {}", self.form.account),
                    Status::Failed(why) => format!("Login failed: {why}"),
                    Status::Closed(why) => format!("Closed: {why}"),
                };
                ui.label(text);
                ui.checkbox(&mut self.view.verbose, "Show other ids");
                if online && ui.button("Log out").clicked() {
                    self.send(Command::Quit);
                }
            });
        });

        if !online {
            egui::CentralPanel::default().show(ui, |ui| self.login_screen(ui));
            if !self.view.log.is_empty() {
                egui::Panel::bottom("lastlog").show(ui, |ui| {
                    for line in self.view.log.iter().rev().take(5).rev() {
                        ui.monospace(line);
                    }
                });
            }
        } else {
            egui::Panel::left("players")
                .default_size(280.0)
                .min_size(220.0)
                .show(ui, |ui| self.players_panel(ui));
            egui::Panel::right("actions")
                .default_size(280.0)
                .min_size(220.0)
                .show(ui, |ui| self.actions_panel(ui));
            egui::Panel::bottom("chat").show(ui, |ui| self.chat_bar(ui));
            egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in &self.view.log {
                            ui.monospace(line);
                        }
                    });
            });
        }
        ctx.request_repaint_after(REFRESH);
    }
}
