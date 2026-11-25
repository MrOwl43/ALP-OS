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
mod style;
mod topbar;

use alp_screen::api::{ApiError, Client};
use alp_screen::i18n::{self, Texts};
use alp_screen::model::{self, Screen, Settings};
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use style::Tone;

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
    bar: topbar::TopBar,
    /// The last state for the top bar, and whether the console is open and
    /// on a first-start page (the bar is drawn again on every page change).
    last_bar: RefCell<Option<model::bar::Bar>>,
    open: Cell<bool>,
    in_setup: Cell<bool>,
    lock: LockUi,
    home: HomeUi,
    gallery_url: RefCell<Option<String>>,
    gallery: Rc<gallery::GalleryUi>,
    tiles: HomeTiles,
    inst: Rc<install::InstallUi>,
    pair: Rc<pairing::PairUi>,
    setup: Rc<setup::SetupUi>,
    set: SettingsUi,
}

/// The sign-in page's changing parts.
struct LockUi {
    clock: gtk4::Label,
    date: gtk4::Label,
    status: gtk4::Label,
    badge: gtk4::Image,
    password: gtk4::PasswordEntry,
    sign_in: gtk4::Button,
    error: style::Note,
    tiles: LockTiles,
}

/// What the lock screen always shows (owner, 2026-11-25): temperature,
/// the disks, phones and the last backup. An item the owner hid in the
/// settings says so instead of showing a value.
struct LockTiles {
    temp: Tile,
    disks: Tile,
    phones: Tile,
    backup: Tile,
}

impl LockTiles {
    fn show(&self, texts: &Texts, t: &model::home::Tiles) {
        use model::home::size;
        let hidden = |k: &str| t.hidden.iter().any(|h| h == k);
        let hid = texts.t("lock.hidden");
        match t.temperature_c {
            Some(c) => {
                self.temp.set(&format!("{c} °C"), "", c >= 80);
                if c >= 90 {
                    Tone::Bad.apply(&self.temp.chip);
                }
            }
            None => self.temp.set("–", &texts.t("lock.temp.none"), false),
        }
        let free = if hidden("space") {
            hid.clone()
        } else {
            texts.fill("lock.free", &[("free", &size(t.free_bytes))])
        };
        self.disks.set(
            &texts.t(&format!("health.{}", t.health)),
            &free,
            t.health != "ok",
        );
        if hidden("phones") {
            self.phones.set(&hid, "", false);
        } else if t.phones > 0 {
            self.phones.set(
                &texts.fill("status.phones.n", &[("n", &t.phones.to_string())]),
                "",
                false,
            );
        } else {
            self.phones.set(&texts.t("status.phones.none"), "", false);
        }
        let last = t
            .last_upload
            .as_deref()
            .and_then(|s| glib::DateTime::from_iso8601(s, None).ok())
            .and_then(|d| d.to_local().ok())
            .and_then(|d| d.format("%e %b, %H:%M").ok())
            .map(|w| w.trim().to_string());
        match (hidden("backup"), last) {
            (true, _) => self.backup.set(&hid, "", false),
            (false, Some(w)) => self.backup.set(&w, "", false),
            (false, None) => self.backup.set("–", "", false),
        }
    }
}

/// The home page's changing parts.
struct HomeUi {
    badge: gtk4::Image,
    title: gtk4::Label,
    line: gtk4::Label,
    photos_btn: gtk4::Button,
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
    note: style::Note,
    /// Advanced: install on this computer (only while running from a stick).
    advanced: adw::PreferencesGroup,
    /// The Advanced category in the list (shown with [`SettingsUi::advanced`]).
    advanced_row: gtk4::ListBoxRow,
    install_btn: gtk4::Button,
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(build);
    app.run_with_args::<&str>(&[])
}

/// The lock screen, where the computer spends most of its time (owner,
/// 2026-11-25): the time on top, the status and the facts large in the
/// middle, the sign-in along the bottom, all on the wallpaper.
fn lock_page(texts: &Texts) -> (gtk4::ScrolledWindow, LockUi) {
    let clock = gtk4::Label::builder().css_classes(["clock-big"]).build();
    let date = gtk4::Label::builder().css_classes(["date-line"]).build();
    let top = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .halign(gtk4::Align::Center)
        .margin_top(18)
        .build();
    top.append(&clock);
    top.append(&date);

    // The middle: the status disc and words, then the facts.
    let badge = style::hero("emblem-ok-symbolic", Tone::Ok);
    badge.add_css_class("ring");
    let status = style::text("", &["title-1", "lock-status"], true);
    let tiles = LockTiles {
        temp: Tile::new(
            &texts.t("lock.temp"),
            "computer-symbolic",
            Tone::Warn,
            false,
        ),
        disks: Tile::new(
            &texts.t("lock.disks"),
            "drive-harddisk-symbolic",
            Tone::Ok,
            false,
        ),
        phones: Tile::new(
            &texts.t("status.phones"),
            "phone-symbolic",
            Tone::Violet,
            false,
        ),
        backup: Tile::new(
            &texts.t("lock.last"),
            "emblem-synchronizing-symbolic",
            Tone::Blue,
            false,
        ),
    };
    let facts = gtk4::Grid::builder()
        .column_spacing(16)
        .row_spacing(16)
        .column_homogeneous(true)
        .css_classes(["lock-facts"])
        .margin_top(18)
        .build();
    for (i, t) in [&tiles.temp, &tiles.disks, &tiles.phones, &tiles.backup]
        .into_iter()
        .enumerate()
    {
        facts.attach(&t.frame, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    let middle = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(10)
        .valign(gtk4::Align::Center)
        .vexpand(true)
        .css_classes(["on-wallpaper"])
        .build();
    middle.append(&badge);
    middle.append(&status);
    middle.append(&facts);
    let middle = adw::Clamp::builder()
        .maximum_size(820)
        .tightening_threshold(820)
        .child(&middle)
        .vexpand(true)
        .margin_start(32)
        .margin_end(32)
        .build();

    // The bottom: sign in.
    let avatar = style::icon("system-lock-screen-symbolic", 22);
    avatar.add_css_class("avatar");
    let user = gtk4::Label::builder()
        .label(texts.t("lock.user"))
        .css_classes(["login-name"])
        .build();
    let password = gtk4::PasswordEntry::builder()
        .show_peek_icon(true)
        .width_request(300)
        .placeholder_text(texts.t("pw.label"))
        .css_classes(["login-field"])
        .valign(gtk4::Align::Center)
        .build();
    password.update_property(&[gtk4::accessible::Property::Label(&texts.t("pw.label"))]);
    let sign_in = gtk4::Button::builder()
        .icon_name("go-next-symbolic")
        .css_classes(["suggested-action", "login-go"])
        .valign(gtk4::Align::Center)
        .build();
    style::name(&sign_in, &texts.t("lock.signin"));
    let bar = gtk4::Box::builder()
        .spacing(12)
        .halign(gtk4::Align::Center)
        .css_classes(["login-bar"])
        .build();
    bar.append(&avatar);
    bar.append(&user);
    bar.append(&password);
    bar.append(&sign_in);
    let error = style::Note::new("bad", "");
    error.root.set_halign(gtk4::Align::Center);
    let bottom = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(10)
        .margin_bottom(28)
        .margin_top(12)
        .css_classes(["login"])
        .build();
    bottom.append(&error.root);
    bottom.append(&bar);

    let whole = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    whole.append(&top);
    whole.append(&middle);
    whole.append(&bottom);
    let page = style::scroll(&whole);
    let ui = LockUi {
        clock,
        date,
        status,
        badge,
        password,
        sign_in,
        error,
        tiles,
    };
    (page, ui)
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
    style::install(&style::Theme::DEFAULT);
    let texts = Rc::new(Texts::load(Path::new(i18n::DIR), "en"));
    let client = Arc::new(Client::default());
    let (tx, rx) = mpsc::channel::<Msg>();

    let (lock_page, lock) = lock_page(&texts);

    // Home page.
    let home = HomeUi {
        badge: style::hero("emblem-ok-symbolic", Tone::Ok),
        title: style::text("", &["title-1"], false),
        line: style::text("", &["lead"], false),
        photos_btn: style::button(
            "image-x-generic-symbolic",
            &texts.t("home.gallery"),
            &["suggested-action", "pill", "light-btn"],
        ),
    };
    home.badge.add_css_class("ring");
    home.title.set_xalign(0.5);
    home.title.set_justify(gtk4::Justification::Center);
    home.line.set_xalign(0.5);
    home.line.set_justify(gtk4::Justification::Center);
    home.photos_btn.set_visible(false);
    let lock_btn = style::button(
        "system-lock-screen-symbolic",
        &texts.t("home.lock"),
        &["pill", "glass-btn"],
    );
    let settings_btn = style::button(
        "emblem-system-symbolic",
        &texts.t("home.settings"),
        &["pill", "glass-btn"],
    );
    let tiles = HomeTiles::new(&texts);
    let home_page = home_layout(&home, &[&home.photos_btn, &settings_btn, &lock_btn], &tiles);

    let stack = gtk4::Stack::builder()
        .transition_type(gtk4::StackTransitionType::Crossfade)
        .transition_duration(150)
        .vexpand(true)
        .build();
    let (settings_page, set) = settings_page(&texts, &client, &tx);
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
    let gallery = gallery::GalleryUi::new();
    stack.add_named(&gallery.page, Some("gallery"));
    // Start on the sign-in page: the install, setup and pairing pages were
    // added first, and a stack shows its first page.
    stack.set_visible_child_name("lock");
    {
        let st = stack.clone();
        settings_btn.connect_clicked(move |_| st.set_visible_child_name("settings"));
    }

    let bar = topbar::TopBar::new(&texts);
    let ui = Rc::new(Ui {
        texts,
        stack: stack.clone(),
        bar,
        last_bar: RefCell::new(None),
        open: Cell::new(false),
        in_setup: Cell::new(false),
        lock,
        home,
        gallery_url: RefCell::new(None),
        gallery: gallery.clone(),
        tiles,
        inst: inst.clone(),
        pair,
        setup: setup_ui,
        set,
    });
    // Install from the stick: chosen in the first setup; here only under
    // Settings > Advanced, for testing (owner, 2026-11-25).
    {
        let i = inst.clone();
        *ui.setup.open_install.borrow_mut() = Some(Box::new(move || i.open()));
    }
    ui.set.install_btn.connect_clicked(move |_| inst.open());
    {
        let u = ui.clone();
        ui.bar.home.connect_clicked(move |_| {
            u.gallery.close();
            u.stack.set_visible_child_name("home");
        });
        let u = ui.clone();
        ui.bar.settings.connect_clicked(move |_| {
            u.gallery.close();
            u.stack.set_visible_child_name("settings");
        });
        let u = ui.clone();
        stack.connect_visible_child_name_notify(move |_| redraw_bar(&u));
    }
    {
        let (client, tx) = (client.clone(), tx.clone());
        let power = move |restart: bool| {
            let (client, tx) = (client.clone(), tx.clone());
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Done(
                    client.post("live/power/off", &json!({"restart": restart})),
                ));
            });
        };
        let t = ui.texts.clone();
        ui.bar.power.connect_clicked(move |b| {
            let d = adw::AlertDialog::new(Some(&t.t("power.title")), Some(&t.t("power.sub")));
            d.add_responses(&[
                ("cancel", &t.t("common.cancel")),
                ("restart", &t.t("power.restart.yes")),
                ("off", &t.t("power.yes")),
            ]);
            d.set_response_appearance("off", adw::ResponseAppearance::Destructive);
            d.set_default_response(Some("cancel"));
            d.set_close_response("cancel");
            let power = power.clone();
            d.connect_response(None, move |_, r| match r {
                "restart" => power(true),
                "off" => power(false),
                _ => {}
            });
            d.present(Some(b));
        });
    }
    {
        let (u, client) = (ui.clone(), client.clone());
        ui.home.photos_btn.connect_clicked(move |_| {
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
            let ui = &u.lock;
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
        ui.lock.sign_in.connect_clicked(move |_| send());
        ui.lock.password.connect_activate(move |_| s2());
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

    // The top bar above every page.
    let shell = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    shell.append(&ui.bar.root);
    shell.append(&stack);
    let win = adw::ApplicationWindow::builder()
        .application(app)
        .content(&shell)
        .default_width(1280)
        .default_height(800)
        .css_classes(["alp"])
        .build();
    win.fullscreen();
    win.present();
}

/// Draws the top bar for the page now showing.
fn redraw_bar(ui: &Ui) {
    if let Some(b) = ui.last_bar.borrow().as_ref() {
        let page = ui.stack.visible_child_name().unwrap_or_default();
        ui.bar
            .show(&ui.texts, b, &page, ui.open.get(), ui.in_setup.get());
    }
}

/// The lock page's clock and date (local time).
fn show_time(ui: &LockUi) {
    if let Ok(now) = glib::DateTime::now_local() {
        if let Ok(s) = now.format("%H:%M") {
            ui.clock.set_label(&s);
        }
        if let Ok(s) = now.format("%A, %e %B") {
            ui.date.set_label(s.replace("  ", " ").trim());
        }
    }
}

fn handle(ui: &Ui, m: Msg) {
    match m {
        Msg::State(Ok(w)) => {
            let s = Screen::from_state(&w);
            *ui.last_bar.borrow_mut() = Some(model::bar::Bar::from_state(&w));
            ui.open.set(s.open);
            show_time(&ui.lock);
            // First start of the installed system: its pages come first.
            let in_setup = ui.setup.show(model::setup::step(&w).as_ref());
            ui.in_setup.set(in_setup);
            if in_setup {
                // The place step uses the install pages: keep their progress.
                ui.inst.show_job(&model::install::state(&w).1);
                if s.open {
                    ui.pair.show_pending(&model::pairing::pending(&w));
                }
                redraw_bar(ui);
                return;
            }
            if s.open {
                let (title, line) = s.home_keys();
                ui.home.title.set_label(&ui.texts.t(title));
                ui.home.line.set_label(&match (&s.message, line) {
                    (Some(m), _) if s.level.needs_user() => m.clone(),
                    (_, Some(k)) => ui.texts.t(k),
                    _ => String::new(),
                });
                let (icon, tone) = level_look(s.level);
                ui.home.badge.set_icon_name(Some(icon));
                tone.apply(&ui.home.badge);
                ui.tiles
                    .show(&ui.texts, &model::home::Tiles::from_state(&w));
                let url = model::gallery::url(&w);
                ui.home.photos_btn.set_visible(url.is_some());
                if url.is_none() && ui.stack.visible_child_name().as_deref() == Some("gallery") {
                    ui.gallery.close();
                    ui.stack.set_visible_child_name("home");
                }
                *ui.gallery_url.borrow_mut() = url;
                ui.pair.show_pending(&model::pairing::pending(&w));
                show_settings(&ui.set, &ui.texts, &Settings::from_state(&w));
                let (can_install, job) = model::install::state(&w);
                ui.set.advanced.set_visible(can_install);
                ui.set.advanced_row.set_visible(can_install);
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
                ui.lock.status.set_label(&ui.texts.t(key));
                ui.lock
                    .tiles
                    .show(&ui.texts, &model::home::Tiles::from_state(&w));
                let (icon, tone) = if s.level.needs_user() {
                    level_look(s.level)
                } else {
                    ("emblem-ok-symbolic", Tone::Ok)
                };
                ui.lock.badge.set_icon_name(Some(icon));
                tone.apply(&ui.lock.badge);
                ui.gallery.close();
                ui.pair.locked();
                if ui.stack.visible_child_name().as_deref() != Some("lock") {
                    ui.stack.set_visible_child_name("lock");
                    ui.lock.password.grab_focus();
                }
            }
            redraw_bar(ui);
        }
        Msg::State(Err(e)) => {
            ui.lock.status.set_label(&ui.texts.t(&e.key));
        }
        Msg::Install(m) => ui.inst.handle(m),
        Msg::Pair(m) => ui.pair.handle(m),
        Msg::Setup(m) => ui.setup.handle(m),
        Msg::Done(r) => {
            ui.set.note.set(&match r {
                Ok(_) => String::new(),
                Err(e) if e.message.is_empty() => ui.texts.t(&e.key),
                Err(e) => e.message,
            });
        }
        Msg::SignedIn(r) => {
            ui.lock.sign_in.set_sensitive(true);
            match r {
                Ok(_) => {
                    ui.lock.error.set("");
                    ui.stack.set_visible_child_name("home");
                }
                Err(e) => {
                    let text = if e.message.is_empty() {
                        ui.texts.t(&e.key)
                    } else {
                        e.message
                    };
                    ui.lock.error.set(&text);
                    ui.lock.password.grab_focus();
                }
            }
        }
    }
}

/// The icon and colour for an overall level (never colour alone).
fn level_look(level: model::Level) -> (&'static str, Tone) {
    match model::bar::Tone::of(level) {
        model::bar::Tone::Ok => ("emblem-ok-symbolic", Tone::Ok),
        model::bar::Tone::Warn => ("dialog-warning-symbolic", Tone::Warn),
        model::bar::Tone::Bad => ("dialog-error-symbolic", Tone::Bad),
    }
}

/// A settings row with a coloured icon in front.
fn icon_row(title: &str, icon: &str, tone: Tone) -> adw::ActionRow {
    let r = adw::ActionRow::builder().title(title).build();
    r.add_prefix(&style::chip(icon, tone, 20));
    r
}

/// The settings page: About, date and time, updates, phones, power. The
/// way back is the top bar's Home button.
fn settings_page(
    texts: &Texts,
    client: &Arc<Client>,
    tx: &mpsc::Sender<Msg>,
) -> (gtk4::Box, SettingsUi) {
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

    // Categories as in the 0.2.6 console: a list on the left, one page on
    // the right (each a libadwaita settings page that scrolls).
    let page = |_: ()| {
        let p = adw::PreferencesPage::new();
        p.set_vexpand(true);
        p
    };

    // Phones.
    let phones_page = page(());
    let connect_phone = style::button(
        "list-add-symbolic",
        &texts.t("phone.title"),
        &["suggested-action"],
    );
    connect_phone.set_valign(gtk4::Align::Center);
    let phones = adw::PreferencesGroup::builder()
        .header_suffix(&connect_phone)
        .build();
    phones_page.add(&phones);

    // Time & Language.
    let time_page = page(());
    let time = adw::PreferencesGroup::new();
    let clock = icon_row(
        &texts.t("set.clock"),
        "preferences-system-time-symbolic",
        Tone::Teal,
    );
    let sync = style::button("view-refresh-symbolic", &texts.t("set.sync"), &[]);
    sync.set_valign(gtk4::Align::Center);
    let f = act("live/clock/sync", json!({}));
    sync.connect_clicked(move |_| f());
    clock.add_suffix(&sync);
    time.add(&clock);
    time_page.add(&time);

    // Power.
    let power_page = page(());
    let power = adw::PreferencesGroup::new();
    let restart_row = icon_row(
        &texts.t("power.restart.yes"),
        "system-reboot-symbolic",
        Tone::Blue,
    );
    let restart = style::button("system-reboot-symbolic", &texts.t("power.restart.yes"), &[]);
    restart.set_valign(gtk4::Align::Center);
    restart_row.add_suffix(&restart);
    let off_row = icon_row(
        &texts.t("power.yes"),
        "system-shutdown-symbolic",
        Tone::Warn,
    );
    let off = style::button(
        "system-shutdown-symbolic",
        &texts.t("power.yes"),
        &["destructive-action"],
    );
    off.set_valign(gtk4::Align::Center);
    off_row.add_suffix(&off);
    power.add(&restart_row);
    power.add(&off_row);
    power_page.add(&power);
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

    // About: the version and updates.
    let about_page = page(());
    let about = adw::PreferencesGroup::new();
    let version = icon_row(&texts.t("about.version"), "help-about-symbolic", Tone::Blue);
    about.add(&version);
    let update = icon_row(
        &texts.t("upd.title"),
        "software-update-available-symbolic",
        Tone::Ok,
    );
    let update_btn = style::button("emblem-synchronizing-symbolic", "", &[]);
    update_btn.set_valign(gtk4::Align::Center);
    update.add_suffix(&update_btn);
    about.add(&update);
    about_page.add(&about);

    // Advanced: install on this computer (only while running from a stick).
    let advanced_page = page(());
    let install_btn = style::button("go-next-symbolic", &texts.t("set.install.go"), &[]);
    install_btn.set_valign(gtk4::Align::Center);
    let install_row = icon_row(
        &texts.t("install.open"),
        "drive-harddisk-symbolic",
        Tone::Teal,
    );
    install_row.set_subtitle(&texts.t("set.install.d"));
    install_row.add_suffix(&install_btn);
    let advanced = adw::PreferencesGroup::builder().visible(false).build();
    advanced.add(&install_row);
    advanced_page.add(&advanced);

    // The category list and the pages.
    let pages = gtk4::Stack::builder()
        .transition_type(gtk4::StackTransitionType::Crossfade)
        .hexpand(true)
        .build();
    let list = gtk4::ListBox::builder()
        .css_classes(["cat-list"])
        .selection_mode(gtk4::SelectionMode::Single)
        .build();
    list.update_property(&[gtk4::accessible::Property::Label(&texts.t("home.settings"))]);
    let heading = style::text("", &["title-2"], false);
    heading.set_margin_start(32);
    heading.set_margin_top(24);
    heading.set_margin_bottom(4);
    let mut advanced_row = None;
    for (key, icon, tone, p) in [
        ("phones", "phone-symbolic", Tone::Violet, &phones_page),
        (
            "time",
            "preferences-system-time-symbolic",
            Tone::Teal,
            &time_page,
        ),
        ("power", "system-shutdown-symbolic", Tone::Warn, &power_page),
        ("about", "help-about-symbolic", Tone::Blue, &about_page),
        (
            "advanced",
            "applications-engineering-symbolic",
            Tone::Grey,
            &advanced_page,
        ),
    ] {
        let name = texts.t(&format!("cat.{key}"));
        pages.add_named(p, Some(key));
        let line = gtk4::Box::builder().spacing(12).build();
        line.append(&style::chip(icon, tone, 16));
        line.append(&gtk4::Label::builder().label(&name).xalign(0.0).build());
        let row = gtk4::ListBoxRow::builder().child(&line).name(key).build();
        row.update_property(&[gtk4::accessible::Property::Label(&name)]);
        list.append(&row);
        if key == "advanced" {
            row.set_visible(false);
            advanced_row = Some(row);
        }
    }
    {
        let (pages, heading) = (pages.clone(), heading.clone());
        let names: Vec<(String, String)> = ["phones", "time", "power", "about", "advanced"]
            .iter()
            .map(|k| (k.to_string(), texts.t(&format!("cat.{k}"))))
            .collect();
        list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let key = row.widget_name();
            pages.set_visible_child_name(&key);
            if let Some((_, n)) = names.iter().find(|(k, _)| *k == key.as_str()) {
                heading.set_label(n);
            }
        });
    }
    list.select_row(list.row_at_index(0).as_ref());
    let side = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&list)
        .css_classes(["cat-side"])
        .width_request(250)
        .build();

    let note = style::Note::new("bad", "");
    note.root.set_margin_start(32);
    note.root.set_margin_end(32);
    note.root.set_margin_top(12);
    let main = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .hexpand(true)
        .build();
    main.append(&heading);
    main.append(&note.root);
    main.append(&pages);
    let both = gtk4::Box::builder().vexpand(true).build();
    both.append(&side);
    both.append(&main);
    // Settings in a white window over the wallpaper.
    let b = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    b.append(&style::window(&both, 1000));
    let advanced_row = advanced_row.expect("advanced category");
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
        advanced,
        advanced_row,
        install_btn,
    };
    (b, ui)
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
    let label = match &s.update_newer {
        Some(v) => texts.fill("upd.install", &[("v", v)]),
        None => texts.t("upd.check"),
    };
    style::set_button_text(&ui.update_btn, &label);
    if ui.last.borrow().devices != s.devices {
        for r in ui.phone_rows.borrow_mut().drain(..) {
            ui.phones.remove(&r);
        }
        for (_, name, model) in &s.devices {
            let r = adw::ActionRow::builder()
                .title(name.as_str())
                .subtitle(model.as_str())
                .build();
            r.add_prefix(&style::chip("phone-symbolic", Tone::Violet, 20));
            ui.phones.add(&r);
            ui.phone_rows.borrow_mut().push(r);
        }
    }
    *ui.last.borrow_mut() = s.clone();
}

/// One home tile: a coloured icon, a small label, a value, a note.
struct Tile {
    frame: gtk4::Box,
    chip: gtk4::Image,
    value: gtk4::Label,
    note: gtk4::Label,
    tone: Tone,
}

impl Tile {
    fn new(key: &str, icon: &str, tone: Tone, big: bool) -> Self {
        let frame = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .css_classes(["glass"])
            .build();
        if big {
            frame.add_css_class("tile-photos");
        }
        // Big tile: icon above the number. Small tiles: icon beside the
        // words, so three rows fit a 768-pixel-high screen.
        let inner = gtk4::Box::builder()
            .orientation(if big {
                gtk4::Orientation::Vertical
            } else {
                gtk4::Orientation::Horizontal
            })
            .spacing(if big { 4 } else { 16 })
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(22)
            .margin_end(22)
            .valign(gtk4::Align::Center)
            .vexpand(true)
            .build();
        let chip = style::chip(icon, tone, if big { 32 } else { 24 });
        chip.set_valign(if big {
            gtk4::Align::Center
        } else {
            gtk4::Align::Start
        });
        chip.set_margin_bottom(if big { 12 } else { 0 });
        let value = style::text("", &[if big { "big-number" } else { "title-3" }], false);
        let note = style::text("", &["dim-label"], false);
        let words = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .hexpand(true)
            .build();
        words.append(&style::text(key, &["caption-up"], false));
        words.append(&value);
        words.append(&note);
        inner.append(&chip);
        inner.append(&words);
        frame.append(&inner);
        Tile {
            frame,
            chip,
            value,
            note,
            tone,
        }
    }

    /// `alarm`: the value needs a look (amber, with a warning icon).
    fn set(&self, value: &str, note: &str, alarm: bool) {
        self.value.set_label(value);
        self.note.set_label(note);
        self.note.set_visible(!note.is_empty());
        if alarm {
            self.value.add_css_class("warning");
            Tone::Warn.apply(&self.chip);
        } else {
            self.value.remove_css_class("warning");
            self.tone.apply(&self.chip);
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
        let photos = Tile::new(
            &texts.t("status.photos"),
            "image-x-generic-symbolic",
            Tone::Blue,
            true,
        );
        let room = Tile::new(
            &texts.t("status.room"),
            "drive-harddisk-symbolic",
            Tone::Ok,
            false,
        );
        let phones = Tile::new(
            &texts.t("status.phones"),
            "phone-symbolic",
            Tone::Violet,
            false,
        );
        let health = Tile::new(
            &texts.t("lock.health"),
            "emblem-ok-symbolic",
            Tone::Ok,
            false,
        );
        let kept = Tile::new(
            &texts.t("status.kept"),
            "channel-secure-symbolic",
            Tone::Teal,
            false,
        );
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
        let bad = t.health != "ok";
        self.health
            .set(&texts.t(&format!("health.{}", t.health)), "", bad);
        self.health.chip.set_icon_name(Some(if bad {
            "dialog-warning-symbolic"
        } else {
            "emblem-ok-symbolic"
        }));
        self.kept.set(
            t.place.as_deref().unwrap_or("–"),
            &texts.t("home.kept.note"),
            false,
        );
    }
}

/// The home page, right on the wallpaper as in the 0.2.6 console: the
/// status disc, headline and buttons centred on top, the glass tiles below,
/// using the whole screen.
fn home_layout(
    home: &HomeUi,
    buttons: &[&gtk4::Button],
    tiles: &HomeTiles,
) -> gtk4::ScrolledWindow {
    let head = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .halign(gtk4::Align::Center)
        .css_classes(["on-wallpaper"])
        .build();
    home.badge.set_halign(gtk4::Align::Center);
    home.badge.set_margin_bottom(8);
    head.append(&home.badge);
    head.append(&home.title);
    head.append(&home.line);
    let actions = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .margin_top(14)
        .halign(gtk4::Align::Center)
        .build();
    for b in buttons {
        actions.append(*b);
    }
    head.append(&actions);
    let page = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(26)
        .margin_top(24)
        .margin_bottom(32)
        .margin_start(48)
        .margin_end(48)
        .build();
    page.append(&head);
    page.append(&tiles.grid);
    style::scroll(
        &adw::Clamp::builder()
            .maximum_size(1200)
            .tightening_threshold(1000)
            .child(&page)
            .build(),
    )
}
