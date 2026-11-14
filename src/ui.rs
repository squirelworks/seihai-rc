//! The window: `seihai-rc gui`, built only with `--features gui`.
//!
//! A login form, then the RC chat and log in the middle, the players and
//! RCs on the left, and actions for the selected player on the right. The
//! session runs on its own thread ([`crate::live`]); this module only
//! draws [`RcView`] and turns clicks into [`Command`]s.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui;

use crate::accounts::Account;
use crate::live::{warp_command, Command, LiveSession, LoginForm, RcView, Status};
use crate::live_nc::{LiveNc, NcCommand, NcView, ScriptTarget};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Players,
    Accounts,
    Files,
    Nc,
}

struct AccountsTab {
    pattern: String,
    selected: Option<String>,
    /// The account the form was last filled from.
    loaded: Option<Account>,
    edit: Account,
    level: String,
    reason: String,
    delete_armed: bool,
}

impl Default for AccountsTab {
    fn default() -> Self {
        Self {
            pattern: "%".into(),
            selected: None,
            loaded: None,
            edit: Account::default(),
            level: String::new(),
            reason: String::new(),
            delete_armed: false,
        }
    }
}

#[derive(Default)]
struct FilesTab {
    dir: String,
    selected: Option<String>,
    local: String,
    new_name: String,
    /// A fetch waiting to be written to this local path.
    save_to: Option<(String, String)>,
    delete_armed: bool,
}

#[derive(Default)]
struct NcTab {
    addr: String,
    live: Option<LiveNc>,
    view: NcView,
    /// What the editor is for, and its text.
    target: Option<ScriptTarget>,
    text: String,
    image: String,
    /// The script last loaded into the editor, to spot a new one.
    loaded: Option<(ScriptTarget, String)>,
    new_class: String,
    new_weapon: String,
    delete_armed: bool,
}

struct App {
    tab: Tab,
    accounts: AccountsTab,
    files: FilesTab,
    nc: NcTab,
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
            tab: Tab::default(),
            accounts: AccountsTab::default(),
            files: FilesTab::default(),
            nc: NcTab::default(),
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
        if let Some(live) = &self.nc.live {
            for u in live.drain() {
                let before = self.nc.view.log.len();
                self.nc.view.apply(u);
                // One log for every tab.
                let new: Vec<String> = self.nc.view.log[before..].to_vec();
                self.view.log.extend(new);
            }
        }
        if self.nc.view.online.is_none() && self.nc.view.ended.is_some() {
            self.nc.live = None;
        }
        self.after_updates();
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
                if online {
                    ui.separator();
                    ui.selectable_value(&mut self.tab, Tab::Players, "Players");
                    ui.selectable_value(&mut self.tab, Tab::Accounts, "Accounts");
                    ui.selectable_value(&mut self.tab, Tab::Files, "Files");
                    ui.selectable_value(&mut self.tab, Tab::Nc, "NPC Control");
                    ui.separator();
                }
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
        } else if self.tab != Tab::Players {
            egui::Panel::bottom("tablog")
                .default_size(140.0)
                .show(ui, |ui| self.log_pane(ui));
            egui::CentralPanel::default().show(ui, |ui| match self.tab {
                Tab::Accounts => self.accounts_tab(ui),
                Tab::Files => self.files_tab(ui),
                _ => self.nc_tab(ui),
            });
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

/// A button that needs two clicks: the first arms it.
fn confirm_button(ui: &mut egui::Ui, armed: &mut bool, label: &str) -> bool {
    let text = if *armed {
        format!("Click again to {}", label.to_lowercase())
    } else {
        label.to_string()
    };
    if ui.button(text).clicked() {
        if *armed {
            *armed = false;
            return true;
        }
        *armed = true;
    }
    false
}

impl App {
    /// Fill forms from what arrived: an account read, a fetched file to
    /// save, a script to edit, an NPC-server address.
    fn after_updates(&mut self) {
        if self.view.account.is_some() && self.view.account != self.accounts.loaded {
            let a = self.view.account.clone().unwrap_or_default();
            self.accounts.level = a.admin_level.to_string();
            self.accounts.reason = a.ban_reason.join("\n");
            self.accounts.edit = a.clone();
            self.accounts.loaded = Some(a);
        }
        if let (Some((path, data)), Some((want, local))) = (&self.view.file, &self.files.save_to) {
            if path == want {
                let line = match std::fs::write(local, data) {
                    Ok(()) => format!("[saved] {path} to {local} ({} bytes)", data.len()),
                    Err(e) => format!("[error] could not write {local}: {e}"),
                };
                self.view.log.push(line);
                self.files.save_to = None;
                self.view.file = None;
            }
        }
        if self.nc.view.script.is_some() && self.nc.view.script != self.nc.loaded {
            let (target, text) = self
                .nc
                .view
                .script
                .clone()
                .unwrap_or((ScriptTarget::Npc(0), String::new()));
            if let ScriptTarget::Weapon { image, .. } = &target {
                self.nc.image = image.clone();
            }
            self.nc.target = Some(target.clone());
            self.nc.text = text.clone();
            self.nc.loaded = Some((target, text));
        }
        if self.nc.addr.is_empty() {
            if let Some(a) = self.view.npc_servers.first() {
                self.nc.addr = a.clone();
            }
        }
    }

    fn log_pane(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in self.view.log.iter().rev().take(200).rev() {
                    ui.monospace(line);
                }
            });
    }

    fn accounts_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Name pattern (% matches any)");
            ui.text_edit_singleline(&mut self.accounts.pattern);
            if ui.button("List").clicked() {
                let p = self.accounts.pattern.clone();
                self.send(Command::AccountList(p));
            }
        });
        ui.separator();
        let names = self.view.accounts.clone();
        ui.columns(2, |cols| {
            egui::ScrollArea::vertical()
                .id_salt("accounts")
                .show(&mut cols[0], |ui| {
                    for n in &names {
                        let sel = self.accounts.selected.as_deref() == Some(n.as_str());
                        if ui.selectable_label(sel, n).clicked() {
                            self.accounts.selected = Some(n.clone());
                            self.accounts.delete_armed = false;
                            if let Some(live) = &self.live {
                                live.send(Command::AccountGet(n.clone()));
                            }
                        }
                    }
                    if names.is_empty() {
                        ui.label("Press List.");
                    }
                });
            let ui = &mut cols[1];
            let Some(sel) = self.accounts.selected.clone() else {
                ui.label("Pick an account.");
                return;
            };
            if self.accounts.loaded.as_ref().map(|a| a.account.as_str()) != Some(sel.as_str()) {
                ui.label(format!("Reading {sel}…"));
                return;
            }
            let a = &mut self.accounts.edit;
            ui.heading(&a.account);
            egui::Grid::new("account").num_columns(2).show(ui, |ui| {
                ui.label("E-mail");
                ui.text_edit_singleline(&mut a.email);
                ui.end_row();
                ui.label("Banned");
                ui.checkbox(&mut a.banned, "");
                ui.end_row();
                ui.label("Ban length");
                ui.label(if a.ban_length.is_empty() {
                    "none"
                } else {
                    a.ban_length.as_str()
                });
                ui.end_row();
                ui.label("Load only");
                ui.checkbox(&mut a.load_only, "");
                ui.end_row();
                ui.label("Admin level");
                ui.text_edit_singleline(&mut self.accounts.level);
                ui.end_row();
                ui.label("Admin worlds");
                ui.text_edit_singleline(&mut a.admin_worlds);
                ui.end_row();
                ui.label("Ban reason");
                ui.text_edit_multiline(&mut self.accounts.reason);
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    match self.accounts.level.trim().parse::<u8>() {
                        Ok(level) => {
                            let mut out = self.accounts.edit.clone();
                            out.admin_level = level;
                            out.ban_reason = self
                                .accounts
                                .reason
                                .lines()
                                .map(str::to_string)
                                .filter(|l| !l.is_empty())
                                .collect();
                            if let Some(live) = &self.live {
                                live.send(Command::AccountSet(out));
                            }
                        }
                        Err(_) => self
                            .view
                            .log
                            .push("[error] the admin level is a number".into()),
                    }
                }
                if confirm_button(ui, &mut self.accounts.delete_armed, "Delete") {
                    if let Some(live) = &self.live {
                        live.send(Command::AccountDelete(sel.clone()));
                    }
                    self.accounts.selected = None;
                }
            });
        });
    }

    fn files_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Folder");
            ui.text_edit_singleline(&mut self.files.dir);
            if ui.button("List").clicked() {
                let d = self.files.dir.clone();
                self.send(Command::FilesList(d));
            }
        });
        ui.separator();
        let listing = self.view.dir.clone();
        ui.columns(2, |cols| {
            egui::ScrollArea::vertical()
                .id_salt("files")
                .show(&mut cols[0], |ui| match &listing {
                    Some((dir, names)) => {
                        ui.label(format!("{dir} ({} files)", names.len()));
                        for n in names {
                            let path = format!("{dir}{n}");
                            let sel = self.files.selected.as_deref() == Some(path.as_str());
                            if ui.selectable_label(sel, n).clicked() {
                                self.files.selected = Some(path);
                                self.files.new_name = n.clone();
                                self.files.delete_armed = false;
                            }
                        }
                    }
                    None => {
                        ui.label("Press List.");
                    }
                });
            let ui = &mut cols[1];
            ui.label("Local file");
            ui.text_edit_singleline(&mut self.files.local);
            let local = self.files.local.trim().to_string();
            if ui.button("Upload into this folder").clicked() {
                let dir = listing.as_ref().map(|l| l.0.clone()).unwrap_or_default();
                let name = std::path::Path::new(&local)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string());
                match (std::fs::read(&local), name) {
                    (Ok(data), Some(name)) => self.send(Command::FilePut {
                        path: format!("{dir}{name}"),
                        data,
                    }),
                    (Err(e), _) => self.view.log.push(format!("[error] {local}: {e}")),
                    (_, None) => self.view.log.push("[error] give a local file".into()),
                }
            }
            ui.separator();
            let Some(path) = self.files.selected.clone() else {
                ui.label("Pick a file.");
                return;
            };
            ui.heading(&path);
            if ui.button("Download to the local file").clicked() {
                if local.is_empty() {
                    self.view.log.push("[error] give a local file".into());
                } else {
                    self.files.save_to = Some((path.clone(), local.clone()));
                    self.send(Command::FileGet(path.clone()));
                }
            }
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.files.new_name);
                if ui.button("Rename").clicked() {
                    let new_name = self.files.new_name.trim().to_string();
                    if let Some(live) = &self.live {
                        live.send(Command::FileRename {
                            path: path.clone(),
                            new_name,
                        });
                    }
                    self.files.selected = None;
                }
            });
            if confirm_button(ui, &mut self.files.delete_armed, "Delete") {
                if let Some(live) = &self.live {
                    live.send(Command::FileDelete(path));
                }
                self.files.selected = None;
            }
        });
    }

    fn nc_send(&mut self, c: NcCommand) {
        if let Some(live) = &self.nc.live {
            live.send(c);
        }
    }

    fn nc_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("NPC-server (HOST:PORT)");
            ui.text_edit_singleline(&mut self.nc.addr);
            if ui.button("Find").clicked() {
                self.send(Command::FindNpcServer);
            }
            match &self.nc.view.online {
                Some(_) => {
                    if ui.button("Disconnect").clicked() {
                        self.nc_send(NcCommand::Quit);
                    }
                }
                None => {
                    if ui.button("Connect").clicked() && !self.nc.addr.trim().is_empty() {
                        self.nc.view = NcView::default();
                        self.nc.live = Some(LiveNc::start(
                            self.nc.addr.trim().to_string(),
                            self.form.account.trim().to_string(),
                            self.form.password.clone(),
                            Duration::from_secs(10),
                        ));
                    }
                }
            }
        });
        if self.nc.view.online.is_none() {
            ui.label(match &self.nc.view.ended {
                Some(why) => format!("Not connected: {why}"),
                None => "Find the NPC-server (or type its address), then Connect.".into(),
            });
            return;
        }
        ui.separator();
        let lists = self.nc.view.lists.clone();
        egui::Panel::left("nclists")
            .default_size(260.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("nclist")
                    .show(ui, |ui| {
                        egui::CollapsingHeader::new(format!("NPCs ({})", lists.npcs.len()))
                            .default_open(true)
                            .show(ui, |ui| {
                                for row in lists.npcs.values() {
                                    let label = format!(
                                        "{} {} ({})",
                                        row.id,
                                        row.name.as_deref().unwrap_or(""),
                                        row.level.as_deref().unwrap_or("")
                                    );
                                    let sel = self.nc.target == Some(ScriptTarget::Npc(row.id));
                                    if ui.selectable_label(sel, label).clicked() {
                                        self.nc.target = Some(ScriptTarget::Npc(row.id));
                                        self.nc.text.clear();
                                        self.nc.view.attributes = None;
                                        self.nc_send(NcCommand::NpcAttributes(row.id));
                                        self.nc_send(NcCommand::NpcScript(row.id));
                                    }
                                }
                            });
                        egui::CollapsingHeader::new(format!("Classes ({})", lists.classes.len()))
                            .show(ui, |ui| {
                                for c in &lists.classes {
                                    let sel =
                                        self.nc.target == Some(ScriptTarget::Class(c.clone()));
                                    if ui.selectable_label(sel, c).clicked() {
                                        self.nc.target = Some(ScriptTarget::Class(c.clone()));
                                        self.nc.text.clear();
                                        self.nc_send(NcCommand::ClassGet(c.clone()));
                                    }
                                }
                                ui.horizontal(|ui| {
                                    ui.text_edit_singleline(&mut self.nc.new_class);
                                    if ui.button("New class").clicked() {
                                        let n = self.nc.new_class.trim().to_string();
                                        if !n.is_empty() {
                                            self.nc.target = Some(ScriptTarget::Class(n));
                                            self.nc.text.clear();
                                        }
                                    }
                                });
                            });
                        egui::CollapsingHeader::new(format!("Weapons ({})", lists.weapons.len()))
                            .show(ui, |ui| {
                                for w in &lists.weapons {
                                    let sel = matches!(&self.nc.target,
                                    Some(ScriptTarget::Weapon { name, .. }) if name == w);
                                    if ui.selectable_label(sel, w).clicked() {
                                        self.nc.target = Some(ScriptTarget::Weapon {
                                            name: w.clone(),
                                            image: String::new(),
                                        });
                                        self.nc.text.clear();
                                        self.nc_send(NcCommand::WeaponGet(w.clone()));
                                    }
                                }
                                ui.horizontal(|ui| {
                                    ui.text_edit_singleline(&mut self.nc.new_weapon);
                                    if ui.button("New weapon").clicked() {
                                        let n = self.nc.new_weapon.trim().to_string();
                                        if !n.is_empty() {
                                            self.nc.target = Some(ScriptTarget::Weapon {
                                                name: n,
                                                image: String::new(),
                                            });
                                            self.nc.image.clear();
                                            self.nc.text.clear();
                                        }
                                    }
                                });
                                if ui.button("Refresh weapons").clicked() {
                                    self.nc_send(NcCommand::Weapons);
                                }
                            });
                    });
            });
        let Some(target) = self.nc.target.clone() else {
            ui.label("Pick an NPC, class or weapon.");
            return;
        };
        ui.horizontal(|ui| match &target {
            ScriptTarget::Npc(id) => {
                ui.heading(format!("NPC {id}"));
                if ui.button("Save script").clicked() {
                    let script = self.nc.text.clone();
                    self.nc_send(NcCommand::NpcScriptSet { id: *id, script });
                }
                if ui.button("Reset").clicked() {
                    self.nc_send(NcCommand::NpcReset(*id));
                }
                if confirm_button(ui, &mut self.nc.delete_armed, "Delete") {
                    self.nc_send(NcCommand::NpcDelete(*id));
                    self.nc.target = None;
                }
            }
            ScriptTarget::Class(name) => {
                ui.heading(format!("Class {name}"));
                if ui.button("Save").clicked() {
                    let script = self.nc.text.clone();
                    self.nc_send(NcCommand::ClassSet {
                        name: name.clone(),
                        script,
                    });
                }
                if confirm_button(ui, &mut self.nc.delete_armed, "Delete") {
                    self.nc_send(NcCommand::ClassDelete(name.clone()));
                    self.nc.target = None;
                }
            }
            ScriptTarget::Weapon { name, .. } => {
                ui.heading(format!("Weapon {name}"));
                ui.label("Image");
                ui.add(egui::TextEdit::singleline(&mut self.nc.image).desired_width(120.0));
                if ui.button("Save").clicked() {
                    let script = self.nc.text.clone();
                    let image = self.nc.image.clone();
                    self.nc_send(NcCommand::WeaponSet {
                        name: name.clone(),
                        image,
                        script,
                    });
                }
                if confirm_button(ui, &mut self.nc.delete_armed, "Delete") {
                    self.nc_send(NcCommand::WeaponDelete(name.clone()));
                    self.nc.target = None;
                }
            }
        });
        if let (ScriptTarget::Npc(_), Some(attrs)) = (&target, &self.nc.view.attributes) {
            egui::CollapsingHeader::new("Attributes").show(ui, |ui| {
                ui.monospace(attrs);
            });
        }
        egui::ScrollArea::vertical()
            .id_salt("script")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.nc.text)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(20),
                );
            });
    }
}
