// SPDX-License-Identifier: AGPL-3.0-or-later
//! `alp-screen`: the native local screen (ADR 0028 #5). First screens: sign
//! in and home. Every action is a call to alpd; nothing here decides.
//!
//! Threads: alpd calls run on worker threads and hand results back to the
//! GTK main loop through a channel polled with a timeout, so the screen never
//! waits on alpd.

use alp_screen::api::{ApiError, Client};
use alp_screen::i18n::{self, Texts};
use alp_screen::model::{Screen, Settings};
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
}

struct Ui {
    texts: Texts,
    stack: gtk4::Stack,
    status: gtk4::Label,
    password: gtk4::PasswordEntry,
    sign_in: gtk4::Button,
    error: gtk4::Label,
    home_title: gtk4::Label,
    home_line: gtk4::Label,
    set: SettingsUi,
}

/// The settings page's changing parts.
struct SettingsUi {
    version: adw::ActionRow,
    clock: adw::ActionRow,
    update: adw::ActionRow,
    update_btn: gtk4::Button,
    phones: adw::PreferencesGroup,
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
    let texts = Texts::load(Path::new(i18n::DIR), "en");
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
    let home_page = column(&[
        home_title.clone().upcast(),
        home_line.clone().upcast(),
        settings_btn.clone().upcast(),
        lock_btn.clone().upcast(),
    ]);

    let (settings_page, set, back) = settings_page(&texts, &client, &tx);

    let stack = gtk4::Stack::new();
    stack.add_named(&lock_page, Some("lock"));
    stack.add_named(&home_page, Some("home"));
    stack.add_named(&settings_page, Some("settings"));
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
        set,
    });

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
            if s.open {
                let (title, line) = s.home_keys();
                ui.home_title.set_label(&ui.texts.t(title));
                ui.home_line.set_label(&match (&s.message, line) {
                    (Some(m), _) if s.level.needs_user() => m.clone(),
                    (_, Some(k)) => ui.texts.t(k),
                    _ => String::new(),
                });
                show_settings(&ui.set, &ui.texts, &Settings::from_state(&w));
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
                ui.stack.set_visible_child_name("lock");
            }
        }
        Msg::State(Err(e)) => {
            ui.status.set_label(&ui.texts.t(&e.key));
        }
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

    let phones = adw::PreferencesGroup::builder()
        .title(texts.t("set.h.phones"))
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
    page.set_hexpand(true);
    b.append(&page);
    let ui = SettingsUi {
        version,
        clock,
        update,
        update_btn,
        phones,
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
