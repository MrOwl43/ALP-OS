// SPDX-License-Identifier: AGPL-3.0-or-later
//! `alp-screen`: the native local screen (ADR 0028 #5). First screens: sign
//! in and home. Every action is a call to alpd; nothing here decides.
//!
//! Threads: alpd calls run on worker threads and hand results back to the
//! GTK main loop through a channel polled with a timeout, so the screen never
//! waits on alpd.

mod gallery;
mod install;
mod pairing;
mod setup;

use alp_screen::api::{ApiError, Client};
use alp_screen::i18n::{self, Texts};
use alp_screen::model::{self, Screen, Settings};
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

const APP_ID: &str = "org.alparchive.Screen";
/// How often the state is read (the web console uses the same pace).
const POLL: Duration = Duration::from_secs(2);

enum Msg {
    State(Result<Value, ApiError>),
    SignedIn(Result<Value, ApiError>),
    /// The answer to a settings action (sync clock, update, power).
    Done(Result<Value, ApiError>),
    Install(install::InstallMsg),
    Pair(pairing::PairMsg),
    Setup(setup::SetupMsg),
}

struct Ui {
    texts: Rc<Texts>,
    stack: gtk4::Stack,
    status: gtk4::Label,
    password: gtk4::PasswordEntry,
    sign_in: gtk4::Button,
    error: gtk4::Label,
    home_title: gtk4::Label,
    home_line: gtk4::Label,
    install_btn: gtk4::Button,
    photos_btn: gtk4::Button,
    gallery_url: RefCell<Option<String>>,
    gallery: Rc<gallery::GalleryUi>,
    tiles: HomeTiles,
    inst: Rc<install::InstallUi>,
    pair: Rc<pairing::PairUi>,
    setup: Rc<setup::SetupUi>,
    set: SettingsUi,
}

/// The settings page's changing parts.
struct SettingsUi {
    version: adw::ActionRow,
    clock: adw::ActionRow,
    update: adw::ActionRow,
    update_btn: gtk4::Button,
    phones: adw::PreferencesGroup,
    connect_phone: gtk4::Button,
    phone_rows: RefCell<Vec<adw::ActionRow>>,
    last: RefCell<Settings>,
    note: gtk4::Label,
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(build);
    app.run_with_args::<&str>(&[])
}

fn build(app: &adw::Application) {
    // Without a desktop (cage has no settings service) GTK may report no
    // text DPI (-1); libadwaita then turns every size given in text units
    // into zero and squeezes its columns to their narrowest. Use the
    // standard 96 DPI, which GTK stores times 1024.
    if let Some(st) = gtk4::Settings::default()
        && st.gtk_xft_dpi() <= 0
    {
        st.set_gtk_xft_dpi(96 * 1024);
    }
    let texts = Rc::new(Texts::load(Path::new(i18n::DIR), "en"));
    let client = Arc::new(Client::default());
    let (tx, rx) = mpsc::channel::<Msg>();

    // Sign-in page.
    let status = gtk4::Label::builder()
        .css_classes(["title-1"])
        .wrap(true)
        .build();
    let password = gtk4::PasswordEntry::builder()
        .show_peek_icon(true)
        .width_chars(28)
        .build();
    let sign_in = gtk4::Button::builder()
        .label(texts.t("lock.signin"))
        .css_classes(["suggested-action", "pill"])
        .build();
    let error = gtk4::Label::builder()
        .css_classes(["error"])
        .wrap(true)
        .build();
    let lock_page = column(&[
        gtk4::Label::builder()
            .label(texts.t("lock.user"))
            .css_classes(["title-2"])
            .build()
            .upcast(),
        status.clone().upcast(),
        password.clone().upcast(),
        sign_in.clone().upcast(),
        error.clone().upcast(),
    ]);

    // Home page.
    let home_title = gtk4::Label::builder()
        .css_classes(["title-1"])
        .wrap(true)
        .build();
    let home_line = gtk4::Label::builder().wrap(true).build();
    let lock_btn = gtk4::Button::builder()
        .label(texts.t("home.lock"))
        .css_classes(["pill"])
        .build();
    let settings_btn = gtk4::Button::builder()
        .label(texts.t("home.settings"))
        .css_classes(["pill"])
        .build();
    let install_btn = gtk4::Button::builder()
        .label(texts.t("install.open"))
        .css_classes(["suggested-action", "pill"])
        .visible(false)
        .build();
    let photos_btn = gtk4::Button::builder()
        .label(texts.t("home.gallery"))
        .css_classes(["suggested-action", "pill"])
        .visible(false)
        .build();
    let tiles = HomeTiles::new(&texts);
    let home_page = home_layout(
        &home_title,
        &home_line,
        &[&photos_btn, &install_btn, &settings_btn, &lock_btn],
        &tiles,
    );

    let stack = gtk4::Stack::new();
    let (settings_page, set, back) = settings_page(&texts, &client, &tx);
    let inst = install::InstallUi::new(texts.clone(), client.clone(), tx.clone(), &stack);
    let pair = pairing::PairUi::new(texts.clone(), client.clone(), tx.clone(), &stack);
    {
        let p = pair.clone();
        set.connect_phone.connect_clicked(move |_| p.open());
    }
    let setup_ui = setup::SetupUi::new(texts.clone(), client.clone(), tx.clone(), &stack);
    {
        let p = pair.clone();
        setup_ui.phone_connect.connect_clicked(move |_| p.open());
    }

    stack.add_named(&lock_page, Some("lock"));
    stack.add_named(&home_page, Some("home"));
    stack.add_named(&settings_page, Some("settings"));
    let gallery_back = gtk4::Button::builder()
        .label(texts.t("common.back"))
        .css_classes(["pill"])
        .build();
    let gallery = gallery::GalleryUi::new(&gallery_back);
    stack.add_named(&gallery.page, Some("gallery"));
    // Start on the sign-in page: the install, setup and pairing pages were
    // added first, and a stack shows its first page.
    stack.set_visible_child_name("lock");
    {
        let (st, g) = (stack.clone(), gallery.clone());
        gallery_back.connect_clicked(move |_| {
            g.close();
            st.set_visible_child_name("home");
        });
    }
    {
        let st = stack.clone();
        settings_btn.connect_clicked(move |_| st.set_visible_child_name("settings"));
        let st = stack.clone();
        back.connect_clicked(move |_| st.set_visible_child_name("home"));
    }

    let ui = Rc::new(Ui {
        texts,
        stack: stack.clone(),
        status,
        password,
        sign_in,
        error,
        home_title,
        home_line,
        install_btn: install_btn.clone(),
        photos_btn: photos_btn.clone(),
        gallery_url: RefCell::new(None),
        gallery: gallery.clone(),
        tiles,
        inst: inst.clone(),
        pair,
        setup: setup_ui,
        set,
    });
    install_btn.connect_clicked(move |_| inst.open());
    {
        let (u, client) = (ui.clone(), client.clone());
        photos_btn.connect_clicked(move |_| {
            if let Some(url) = u.gallery_url.borrow().clone() {
                u.gallery.open(&url, &client);
                u.stack.set_visible_child_name("gallery");
            }
        });
    }

    {
        // Look for an update, or install the one found (only on a press:
        // owner decision, ADR 0026).
        let (u, client, tx) = (ui.clone(), client.clone(), tx.clone());
        ui.set.update_btn.connect_clicked(move |_| {
            let path = if u.set.last.borrow().update_newer.is_some() {
                "live/update/apply"
            } else {
                "live/update/check"
            };
            let (client, tx) = (client.clone(), tx.clone());
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Done(client.post(path, &json!({}))));
            });
        });
    }
    {
        let (u, client, tx) = (ui.clone(), client.clone(), tx.clone());
        let send = move || {
            let ui = &u;
            let pw = zeroize::Zeroizing::new(ui.password.text().to_string());
            ui.password.set_text("");
            ui.sign_in.set_sensitive(false);
            let (client, tx) = (client.clone(), tx.clone());
            std::thread::spawn(move || {
                let r = client.post("live/console/signin", &json!({"password": pw.as_str()}));
                let _ = tx.send(Msg::SignedIn(r));
            });
        };
        let send = Rc::new(send);
        let s2 = send.clone();
        ui.sign_in.connect_clicked(move |_| send());
        ui.password.connect_activate(move |_| s2());
    }
    {
        let (client, tx) = (client.clone(), tx.clone());
        lock_btn.connect_clicked(move |_| {
            let (client, tx) = (client.clone(), tx.clone());
            std::thread::spawn(move || {
                let _ = client.post("live/console/lock", &json!({}));
                client.forget();
                let _ = tx.send(Msg::State(client.get("live/state")));
            });
        });
    }

    // State polling on a worker thread.
    {
        let (client, tx) = (client.clone(), tx.clone());
        std::thread::spawn(move || {
            loop {
                if tx.send(Msg::State(client.get("live/state"))).is_err() {
                    break;
                }
                std::thread::sleep(POLL);
            }
        });
    }

    // Results back on the main loop.
    {
        let ui = ui.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            while let Ok(m) = rx.try_recv() {
                handle(&ui, m);
            }
            glib::ControlFlow::Continue
        });
    }

    let win = adw::ApplicationWindow::builder()
        .application(app)
        .content(&stack)
        .default_width(1280)
        .default_height(800)
        .build();
    win.fullscreen();
    win.present();
}

fn handle(ui: &Ui, m: Msg) {
    match m {
        Msg::State(Ok(w)) => {
            let s = Screen::from_state(&w);
            // First start of the installed system: its pages come first.
            if ui.setup.show(model::setup::step(&w).as_ref()) {
                if s.open {
                    ui.pair.show_pending(&model::pairing::pending(&w));
                }
                return;
            }
            if s.open {
                let (title, line) = s.home_keys();
                ui.home_title.set_label(&ui.texts.t(title));
                ui.home_line.set_label(&match (&s.message, line) {
                    (Some(m), _) if s.level.needs_user() => m.clone(),
                    (_, Some(k)) => ui.texts.t(k),
                    _ => String::new(),
                });
                ui.tiles
                    .show(&ui.texts, &model::home::Tiles::from_state(&w));
                let url = model::gallery::url(&w);
                ui.photos_btn.set_visible(url.is_some());
                if url.is_none() && ui.stack.visible_child_name().as_deref() == Some("gallery") {
                    ui.gallery.close();
                    ui.stack.set_visible_child_name("home");
                }
                *ui.gallery_url.borrow_mut() = url;
                ui.pair.show_pending(&model::pairing::pending(&w));
                show_settings(&ui.set, &ui.texts, &Settings::from_state(&w));
                let (can_install, job) = model::install::state(&w);
                ui.install_btn.set_visible(can_install);
                ui.inst.show_job(&job);
                if ui.stack.visible_child_name().as_deref() == Some("lock") {
                    ui.stack.set_visible_child_name("home");
                }
            } else {
                let key = if s.level.needs_user() {
                    "lock.msg.bad"
                } else {
                    "lock.msg.ok"
                };
                ui.status.set_label(&ui.texts.t(key));
                ui.gallery.close();
                ui.pair.locked();
                ui.stack.set_visible_child_name("lock");
            }
        }
        Msg::State(Err(e)) => {
            ui.status.set_label(&ui.texts.t(&e.key));
        }
        Msg::Install(m) => ui.inst.handle(m),
        Msg::Pair(m) => ui.pair.handle(m),
        Msg::Setup(m) => ui.setup.handle(m),
        Msg::Done(r) => {
            ui.set.note.set_label(&match r {
                Ok(_) => String::new(),
                Err(e) if e.message.is_empty() => ui.texts.t(&e.key),
                Err(e) => e.message,
            });
        }
        Msg::SignedIn(r) => {
            ui.sign_in.set_sensitive(true);
            match r {
                Ok(_) => {
                    ui.error.set_label("");
                    ui.stack.set_visible_child_name("home");
                }
                Err(e) => {
                    let text = if e.message.is_empty() {
                        ui.texts.t(&e.key)
                    } else {
                        e.message
                    };
                    ui.error.set_label(&text);
                }
            }
        }
    }
}

/// The settings page: About, date and time, updates, phones, power.
fn settings_page(
    texts: &Texts,
    client: &Arc<Client>,
    tx: &mpsc::Sender<Msg>,
) -> (gtk4::Box, SettingsUi, gtk4::Button) {
    // One alpd action on a worker thread; its answer comes back as Done.
    let act = |path: &'static str, body: Value| {
        let (client, tx) = (client.clone(), tx.clone());
        move || {
            let (client, tx, body) = (client.clone(), tx.clone(), body.clone());
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Done(client.post(path, &body)));
            });
        }
    };
    let row = |title: &str| adw::ActionRow::builder().title(title).build();

    // libadwaita's own settings page: a centred, width-limited column
    // that scrolls when it gets long.
    let page = adw::PreferencesPage::new();

    let about = adw::PreferencesGroup::builder()
        .title(texts.t("set.h.about"))
        .build();
    let version = row(&texts.t("about.version"));
    about.add(&version);
    page.add(&about);

    let computer = adw::PreferencesGroup::builder()
        .title(texts.t("set.h.computer"))
        .build();
    let clock = row(&texts.t("set.clock"));
    let sync = gtk4::Button::builder()
        .label(texts.t("set.sync"))
        .valign(gtk4::Align::Center)
        .build();
    let f = act("live/clock/sync", json!({}));
    sync.connect_clicked(move |_| f());
    clock.add_suffix(&sync);
    computer.add(&clock);
    let update = row(&texts.t("upd.title"));
    let update_btn = gtk4::Button::builder().valign(gtk4::Align::Center).build();
    update.add_suffix(&update_btn);
    computer.add(&update);
    let restart = gtk4::Button::builder()
        .label(texts.t("power.restart.yes"))
        .valign(gtk4::Align::Center)
        .build();
    let off = gtk4::Button::builder()
        .label(texts.t("power.yes"))
        .css_classes(["destructive-action"])
        .valign(gtk4::Align::Center)
        .build();
    let power = gtk4::Box::builder()
        .spacing(12)
        .halign(gtk4::Align::End)
        .margin_top(12)
        .build();
    power.append(&restart);
    power.append(&off);
    computer.add(&power);
    page.add(&computer);
    confirm(
        &restart,
        &texts.t("power.restart.title"),
        &texts.t("power.restart.sub"),
        &texts.t("power.restart.yes"),
        act("live/power/off", json!({"restart": true})),
    );
    confirm(
        &off,
        &texts.t("power.title"),
        &texts.t("power.sub"),
        &texts.t("power.yes"),
        act("live/power/off", json!({"restart": false})),
    );

    let connect_phone = gtk4::Button::builder()
        .label(texts.t("phone.title"))
        .css_classes(["suggested-action"])
        .valign(gtk4::Align::Center)
        .build();
    let phones = adw::PreferencesGroup::builder()
        .title(texts.t("set.h.phones"))
        .header_suffix(&connect_phone)
        .build();
    page.add(&phones);

    let back = gtk4::Button::builder()
        .label(texts.t("common.back"))
        .halign(gtk4::Align::Start)
        .build();
    let note = gtk4::Label::builder()
        .css_classes(["error"])
        .wrap(true)
        .build();
    let b = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(12)
        .build();
    b.append(&back);
    b.append(&note);
    page.set_vexpand(true);
    b.append(&page);
    let ui = SettingsUi {
        version,
        clock,
        update,
        update_btn,
        phones,
        connect_phone,
        phone_rows: RefCell::new(Vec::new()),
        last: RefCell::new(Settings::default()),
        note,
    };
    (b, ui, back)
}

/// Asks before switching off or restarting; [yes] runs only after "yes".
fn confirm(button: &gtk4::Button, title: &str, body: &str, yes: &str, f: impl Fn() + 'static) {
    let (title, body, yes) = (title.to_string(), body.to_string(), yes.to_string());
    let f = Rc::new(f);
    button.connect_clicked(move |b| {
        let d = adw::AlertDialog::new(Some(&title), Some(&body));
        d.add_responses(&[("cancel", "Cancel"), ("yes", &yes)]);
        d.set_response_appearance("yes", adw::ResponseAppearance::Destructive);
        d.set_default_response(Some("cancel"));
        d.set_close_response("cancel");
        let f = f.clone();
        d.connect_response(None, move |_, r| {
            if r == "yes" {
                f();
            }
        });
        d.present(Some(b));
    });
}

fn show_settings(ui: &SettingsUi, texts: &Texts, s: &Settings) {
    if *ui.last.borrow() == *s {
        return;
    }
    ui.version.set_subtitle(
        &[s.release.as_deref(), s.build.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
    );
    ui.clock.set_subtitle(&match s.clock {
        Some((_, true)) => texts.t("set.clock.synced"),
        Some((_, false)) => texts.t("set.clock.unsure"),
        None => String::new(),
    });
    ui.update.set_visible(s.update_supported);
    ui.update
        .set_subtitle(s.update_current.as_deref().unwrap_or_default());
    match &s.update_newer {
        Some(v) => ui
            .update_btn
            .set_label(&texts.fill("upd.install", &[("v", v)])),
        None => ui.update_btn.set_label(&texts.t("upd.check")),
    }
    if ui.last.borrow().devices != s.devices {
        for r in ui.phone_rows.borrow_mut().drain(..) {
            ui.phones.remove(&r);
        }
        for (_, name, model) in &s.devices {
            let r = adw::ActionRow::builder()
                .title(name.as_str())
                .subtitle(model.as_str())
                .build();
            ui.phones.add(&r);
            ui.phone_rows.borrow_mut().push(r);
        }
    }
    *ui.last.borrow_mut() = s.clone();
}

/// One home tile: a small label, a value, an optional note under it.
struct Tile {
    frame: gtk4::Box,
    value: gtk4::Label,
    note: gtk4::Label,
}

impl Tile {
    fn new(key: &str, big: bool) -> Self {
        let frame = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .css_classes(["card"])
            .build();
        let inner = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .margin_top(20)
            .margin_bottom(20)
            .margin_start(24)
            .margin_end(24)
            .valign(if big {
                gtk4::Align::Center
            } else {
                gtk4::Align::Start
            })
            .vexpand(true)
            .build();
        let label = |text: &str, classes: &[&str]| {
            gtk4::Label::builder()
                .label(text)
                .xalign(0.0)
                .wrap(true)
                .css_classes(classes.to_vec())
                .build()
        };
        let value = label("", &[if big { "title-1" } else { "title-3" }]);
        let note = label("", &["dim-label"]);
        inner.append(&label(key, &["dim-label", "heading"]));
        inner.append(&value);
        inner.append(&note);
        frame.append(&inner);
        Tile { frame, value, note }
    }

    fn set(&self, value: &str, note: &str, alarm: bool) {
        self.value.set_label(value);
        self.note.set_label(note);
        self.note.set_visible(!note.is_empty());
        if alarm {
            self.value.add_css_class("warning");
        } else {
            self.value.remove_css_class("warning");
        }
    }
}

/// The home tiles in an uneven grid (owner's 0.2.7 note 6): photos large on
/// the left, room and phones beside it, health and the disk below.
struct HomeTiles {
    grid: gtk4::Grid,
    photos: Tile,
    room: Tile,
    phones: Tile,
    health: Tile,
    kept: Tile,
}

impl HomeTiles {
    fn new(texts: &Texts) -> Self {
        let grid = gtk4::Grid::builder()
            .column_spacing(18)
            .row_spacing(18)
            .column_homogeneous(true)
            .row_homogeneous(true)
            .vexpand(true)
            .build();
        let photos = Tile::new(&texts.t("status.photos"), true);
        let room = Tile::new(&texts.t("status.room"), false);
        let phones = Tile::new(&texts.t("status.phones"), false);
        let health = Tile::new(&texts.t("lock.health"), false);
        let kept = Tile::new(&texts.t("status.kept"), false);
        grid.attach(&photos.frame, 0, 0, 2, 2);
        grid.attach(&room.frame, 2, 0, 1, 1);
        grid.attach(&phones.frame, 2, 1, 1, 1);
        grid.attach(&health.frame, 0, 2, 1, 1);
        grid.attach(&kept.frame, 1, 2, 2, 1);
        HomeTiles {
            grid,
            photos,
            room,
            phones,
            health,
            kept,
        }
    }

    fn show(&self, texts: &Texts, t: &model::home::Tiles) {
        use model::home::{number, room_photos, size};
        self.grid.set_visible(t.show);
        if !t.show {
            return;
        }
        self.photos.set(&number(t.photos), "", false);
        self.room.set(
            &size(t.free_bytes),
            &texts.fill("where.room", &[("photos", &room_photos(t.free_bytes))]),
            t.space != "ok",
        );
        let phones = if t.phones > 0 {
            texts.fill("status.phones.n", &[("n", &t.phones.to_string())])
        } else {
            texts.t("status.phones.none")
        };
        let last = t
            .last_upload
            .as_deref()
            .and_then(|s| glib::DateTime::from_iso8601(s, None).ok())
            .and_then(|d| d.to_local().ok())
            .and_then(|d| d.format("%e %b %Y, %H:%M").ok())
            .map(|w| texts.fill("status.last", &[("when", w.trim())]))
            .unwrap_or_default();
        self.phones.set(&phones, &last, false);
        self.health.set(
            &texts.t(&format!("health.{}", t.health)),
            "",
            t.health != "ok",
        );
        self.kept.set(
            t.place.as_deref().unwrap_or("–"),
            &texts.t("home.kept.note"),
            false,
        );
    }
}

/// The home page: the headline and the buttons on top, the tiles below,
/// using the whole screen instead of a narrow middle column.
fn home_layout(
    title: &gtk4::Label,
    line: &gtk4::Label,
    buttons: &[&gtk4::Button],
    tiles: &HomeTiles,
) -> adw::Clamp {
    title.set_xalign(0.0);
    line.set_xalign(0.0);
    let words = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    words.append(title);
    words.append(line);
    let actions = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    for b in buttons {
        actions.append(*b);
    }
    let top = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(24)
        .build();
    top.append(&words);
    top.append(&actions);
    let page = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(32)
        .margin_top(48)
        .margin_bottom(48)
        .margin_start(48)
        .margin_end(48)
        .build();
    page.append(&top);
    page.append(&tiles.grid);
    adw::Clamp::builder()
        .maximum_size(1200)
        .tightening_threshold(1000)
        .child(&page)
        .build()
}

fn column(children: &[gtk4::Widget]) -> gtk4::Box {
    let b = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(18)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::Center)
        .build();
    for c in children {
        b.append(c);
    }
    b
}
