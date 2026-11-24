// SPDX-License-Identifier: AGPL-3.0-or-later
//! `alp-screen`: the native local screen (ADR 0028 #5). First screens: sign
//! in and home. Every action is a call to alpd; nothing here decides.
//!
//! Threads: alpd calls run on worker threads and hand results back to the
//! GTK main loop through a channel polled with a timeout, so the screen never
//! waits on alpd.

use alp_screen::api::{ApiError, Client};
use alp_screen::i18n::{self, Texts};
use alp_screen::model::Screen;
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use serde_json::{Value, json};
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
    let home_page = column(&[
        home_title.clone().upcast(),
        home_line.clone().upcast(),
        lock_btn.clone().upcast(),
    ]);

    let stack = gtk4::Stack::new();
    stack.add_named(&lock_page, Some("lock"));
    stack.add_named(&home_page, Some("home"));

    let ui = Rc::new(Ui {
        texts,
        stack: stack.clone(),
        status,
        password,
        sign_in,
        error,
        home_title,
        home_line,
    });

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
                ui.stack.set_visible_child_name("home");
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
