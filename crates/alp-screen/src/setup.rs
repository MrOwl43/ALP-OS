// SPDX-License-Identifier: AGPL-3.0-or-later
//! First-start pages of the installed system (plan `0.2.8-first-start.md`,
//! step 4): choose the password, watch the archive being made on the disks
//! chosen in the installer, make the recovery code (shown once, three
//! words typed back), connect a phone, finish. Every step is an alpd call;
//! which page shows comes from alpd's state ([`model::setup::step`]), so a
//! restart continues where setup stopped.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;

use alp_screen::api::{ApiError, Client};
use alp_screen::i18n::Texts;
use alp_screen::model::setup::{self, Step};
use gtk4::prelude::*;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::Msg;

pub enum SetupMsg {
    Password(Result<Value, ApiError>),
    PoolStarted(Result<Value, ApiError>),
    RecoveryMade(Result<Value, ApiError>),
    Committed(Result<Value, ApiError>),
    /// Skip (recovery) or finish: the answer only matters when it failed.
    Saved(Result<Value, ApiError>),
}

/// The pages this module adds to the stack.
const PAGES: [&str; 5] = [
    "setup-password",
    "setup-pool",
    "setup-recovery",
    "setup-confirm",
    "setup-phone",
];

pub struct SetupUi {
    texts: Rc<Texts>,
    client: Arc<Client>,
    tx: mpsc::Sender<Msg>,
    stack: gtk4::Stack,
    // Password.
    pw1: gtk4::PasswordEntry,
    pw2: gtk4::PasswordEntry,
    pw_go: gtk4::Button,
    pw_note: gtk4::Label,
    // Archive being made.
    bar: gtk4::ProgressBar,
    pool_line: gtk4::Label,
    pool_retry: gtk4::Button,
    pool_asked: Cell<bool>,
    // Recovery code.
    rec_pw: gtk4::PasswordEntry,
    rec_make: gtk4::Button,
    rec_words: gtk4::Label,
    rec_saved: gtk4::Button,
    rec_note: gtk4::Label,
    asked: Cell<[usize; 3]>,
    confirm_labels: [gtk4::Label; 3],
    confirm_entries: [gtk4::Entry; 3],
    confirm_go: gtk4::Button,
    confirm_note: gtk4::Label,
    // Phone.
    /// Opens the pairing pages (wired in main.rs).
    pub phone_connect: gtk4::Button,
    phone_note: gtk4::Label,
}

fn text(t: &Texts, key: &str, fallback: &str) -> String {
    let s = t.t(key);
    if s == key { fallback.to_string() } else { s }
}

fn label(s: &str, classes: &[&str]) -> gtk4::Label {
    gtk4::Label::builder()
        .label(s)
        .css_classes(classes.to_vec())
        .wrap(true)
        .justify(gtk4::Justification::Center)
        .max_width_chars(60)
        .build()
}

fn button(s: &str, main: bool) -> gtk4::Button {
    let classes: &[&str] = if main {
        &["suggested-action", "pill"]
    } else {
        &["pill"]
    };
    gtk4::Button::builder()
        .label(s)
        .css_classes(classes.to_vec())
        .halign(gtk4::Align::Center)
        .build()
}

fn page(children: &[&gtk4::Widget]) -> gtk4::Box {
    let b = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(18)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::Center)
        .margin_start(48)
        .margin_end(48)
        .build();
    for c in children {
        b.append(*c);
    }
    b
}

fn error_text(t: &Texts, e: &ApiError) -> String {
    if e.message.is_empty() {
        t.t(&e.key)
    } else {
        e.message.clone()
    }
}

impl SetupUi {
    pub fn new(
        texts: Rc<Texts>,
        client: Arc<Client>,
        tx: mpsc::Sender<Msg>,
        stack: &gtk4::Stack,
    ) -> Rc<Self> {
        let t = &texts;
        // Password.
        let pw1 = gtk4::PasswordEntry::builder()
            .show_peek_icon(true)
            .width_chars(28)
            .placeholder_text(t.t("pw.label"))
            .build();
        let pw2 = gtk4::PasswordEntry::builder()
            .show_peek_icon(true)
            .width_chars(28)
            .placeholder_text(t.t("pw.again"))
            .build();
        let pw_go = button(&t.t("pw.set"), true);
        let pw_note = label(&t.t("pw.hint.rules"), &["dim-label"]);
        stack.add_named(
            &page(&[
                label(&t.t("pw.title"), &["title-1"]).upcast_ref(),
                label(&t.t("pw.sub"), &[]).upcast_ref(),
                pw1.upcast_ref(),
                pw2.upcast_ref(),
                pw_note.upcast_ref(),
                pw_go.upcast_ref(),
                label(&t.t("pw.foot"), &["dim-label"]).upcast_ref(),
            ]),
            Some(PAGES[0]),
        );

        // Archive being made.
        let bar = gtk4::ProgressBar::builder()
            .show_text(true)
            .width_request(480)
            .build();
        let pool_line = label("", &[]);
        let pool_retry = button(&t.t("common.tryagain"), true);
        pool_retry.set_visible(false);
        stack.add_named(
            &page(&[
                label(
                    &text(t, "setup.pool.title", "Making your archive"),
                    &["title-1"],
                )
                .upcast_ref(),
                label(
                    &text(
                        t,
                        "setup.pool.sub",
                        "The disks you chose are locked with a new key. This takes a few minutes; please leave the computer on.",
                    ),
                    &[],
                )
                .upcast_ref(),
                bar.upcast_ref(),
                pool_line.upcast_ref(),
                pool_retry.upcast_ref(),
            ]),
            Some(PAGES[1]),
        );

        // Recovery code: password, then the words.
        let rec_pw = gtk4::PasswordEntry::builder()
            .show_peek_icon(true)
            .width_chars(28)
            .placeholder_text(t.t("pw.label"))
            .build();
        let rec_make = button(&t.t("recovery.show"), true);
        let rec_words = gtk4::Label::builder()
            .css_classes(["title-3", "monospace"])
            .selectable(false)
            .wrap(true)
            .justify(gtk4::Justification::Center)
            .visible(false)
            .build();
        let rec_saved = button(&t.t("recovery.saved"), true);
        rec_saved.set_visible(false);
        let rec_later = button(&t.t("common.later"), false);
        let rec_note = label("", &["error"]);
        stack.add_named(
            &page(&[
                label(&t.t("recovery.title"), &["title-1"]).upcast_ref(),
                label(&t.t("recovery.intro.password"), &[]).upcast_ref(),
                label(&t.t("recovery.what"), &["dim-label"]).upcast_ref(),
                label(
                    &text(
                        t,
                        "setup.recovery.where",
                        "Write them on paper and keep it away from this computer.",
                    ),
                    &["dim-label"],
                )
                .upcast_ref(),
                rec_pw.upcast_ref(),
                rec_make.upcast_ref(),
                rec_words.upcast_ref(),
                rec_saved.upcast_ref(),
                rec_note.upcast_ref(),
                rec_later.upcast_ref(),
            ]),
            Some(PAGES[2]),
        );

        // Three words typed back.
        let confirm_labels = [label("", &[]), label("", &[]), label("", &[])];
        let confirm_entries = [
            gtk4::Entry::builder().width_chars(20).build(),
            gtk4::Entry::builder().width_chars(20).build(),
            gtk4::Entry::builder().width_chars(20).build(),
        ];
        let confirm_go = button(&t.t("recovery.confirm.go"), true);
        let confirm_again = button(&t.t("recovery.confirm.again"), false);
        let confirm_note = label("", &["error"]);
        let grid = gtk4::Grid::builder()
            .row_spacing(12)
            .column_spacing(12)
            .halign(gtk4::Align::Center)
            .build();
        for i in 0..3 {
            grid.attach(&confirm_labels[i], 0, i as i32, 1, 1);
            grid.attach(&confirm_entries[i], 1, i as i32, 1, 1);
        }
        stack.add_named(
            &page(&[
                label(&t.t("recovery.confirm.title"), &["title-1"]).upcast_ref(),
                label(&t.t("recovery.confirm.sub"), &[]).upcast_ref(),
                grid.upcast_ref(),
                confirm_go.upcast_ref(),
                confirm_note.upcast_ref(),
                confirm_again.upcast_ref(),
            ]),
            Some(PAGES[3]),
        );

        // Phone, then finish.
        let phone_connect = button(&t.t("phone.title"), true);
        let finish = button(&t.t("common.done"), false);
        let phone_note = label("", &["error"]);
        stack.add_named(
            &page(&[
                label(&t.t("phone.title"), &["title-1"]).upcast_ref(),
                label(&t.t("phone.sub"), &[]).upcast_ref(),
                phone_connect.upcast_ref(),
                finish.upcast_ref(),
                phone_note.upcast_ref(),
            ]),
            Some(PAGES[4]),
        );

        let ui = Rc::new(SetupUi {
            texts: texts.clone(),
            client,
            tx,
            stack: stack.clone(),
            pw1,
            pw2,
            pw_go,
            pw_note,
            bar,
            pool_line,
            pool_retry,
            pool_asked: Cell::new(false),
            rec_pw,
            rec_make,
            rec_words,
            rec_saved,
            rec_note,
            asked: Cell::new([0; 3]),
            confirm_labels,
            confirm_entries,
            confirm_go,
            confirm_note,
            phone_connect,
            phone_note,
        });
        {
            let u = ui.clone();
            ui.pw_go.connect_clicked(move |_| u.set_password());
            let u = ui.clone();
            ui.pw2.connect_activate(move |_| u.set_password());
            let u = ui.clone();
            ui.pool_retry.connect_clicked(move |_| {
                u.pool_retry.set_visible(false);
                u.start_pool();
            });
            let u = ui.clone();
            ui.rec_make.connect_clicked(move |_| u.make_recovery());
            let u = ui.clone();
            ui.rec_pw.connect_activate(move |_| u.make_recovery());
            let u = ui.clone();
            ui.rec_saved.connect_clicked(move |_| u.ask_words());
            let u = ui.clone();
            rec_later.connect_clicked(move |_| {
                u.call(|c| {
                    SetupMsg::Saved(
                        c.post("live/setup/progress", &json!({"skipped": ["recovery"]})),
                    )
                })
            });
            let u = ui.clone();
            ui.confirm_go.connect_clicked(move |_| u.commit_words());
            let u = ui.clone();
            confirm_again.connect_clicked(move |_| u.stack.set_visible_child_name(PAGES[2]));
            let u = ui.clone();
            finish.connect_clicked(move |_| {
                u.call(|c| SetupMsg::Saved(c.post("live/setup/finish", &json!({}))))
            });
        }
        ui
    }

    fn call(&self, f: impl FnOnce(&Client) -> SetupMsg + Send + 'static) {
        let (c, tx) = (self.client.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Setup(f(&c)));
        });
    }

    fn set_password(&self) {
        let (a, b) = (
            Zeroizing::new(self.pw1.text().to_string()),
            Zeroizing::new(self.pw2.text().to_string()),
        );
        if a.chars().count() < 8 {
            self.pw_note.set_label(&self.texts.t("pw.hint.short"));
            return;
        }
        if a != b {
            self.pw_note.set_label(&self.texts.t("pw.mismatch"));
            return;
        }
        self.pw1.set_text("");
        self.pw2.set_text("");
        self.pw_go.set_sensitive(false);
        self.pw_note.set_label(&self.texts.t("pw.working"));
        self.call(move |c| {
            SetupMsg::Password(c.post_slow("live/password/setup", &json!({"password": a.as_str()})))
        });
    }

    fn start_pool(&self) {
        self.pool_asked.set(true);
        self.call(|c| SetupMsg::PoolStarted(c.post("live/archive/create-pool", &json!({}))));
    }

    fn make_recovery(&self) {
        let pw = Zeroizing::new(self.rec_pw.text().to_string());
        if pw.is_empty() {
            self.rec_note.set_label(&self.texts.t("pw.label"));
            return;
        }
        self.rec_pw.set_text("");
        self.rec_make.set_sensitive(false);
        self.rec_note.set_label(&self.texts.t("recovery.making"));
        self.call(move |c| {
            SetupMsg::RecoveryMade(
                c.post_slow("live/recovery/create", &json!({"password": pw.as_str()})),
            )
        });
    }

    fn ask_words(&self) {
        for (i, n) in self.asked.get().iter().enumerate() {
            self.confirm_labels[i].set_label(
                &self
                    .texts
                    .fill("recovery.confirm.n", &[("n", &n.to_string())]),
            );
            self.confirm_entries[i].set_text("");
        }
        self.confirm_note.set_label("");
        self.stack.set_visible_child_name(PAGES[3]);
    }

    fn commit_words(&self) {
        let mut words = serde_json::Map::new();
        for (i, n) in self.asked.get().iter().enumerate() {
            words.insert(
                n.to_string(),
                Value::String(self.confirm_entries[i].text().trim().to_lowercase()),
            );
        }
        self.confirm_go.set_sensitive(false);
        self.confirm_note
            .set_label(&self.texts.t("recovery.confirming"));
        let body = json!({ "words": words });
        self.call(move |c| SetupMsg::Committed(c.post_slow("live/recovery/commit", &body)));
    }

    /// Forget the words shown (also from the screen).
    fn wipe_words(&self) {
        self.rec_words.set_label("");
        self.rec_words.set_visible(false);
        self.rec_saved.set_visible(false);
        self.rec_pw.set_visible(true);
        self.rec_make.set_visible(true);
        for e in &self.confirm_entries {
            e.set_text("");
        }
    }

    /// The state's setup step; `None` once setup is over. Returns whether a
    /// setup page is showing (main.rs then leaves the other pages alone).
    pub fn show(&self, step: Option<&Step>) -> bool {
        let visible = self.stack.visible_child_name();
        let on_setup = visible.as_deref().is_some_and(|v| PAGES.contains(&v));
        let Some(step) = step else {
            if on_setup {
                self.wipe_words();
                self.stack.set_visible_child_name("home");
            }
            return false;
        };
        // Pairing pages opened from the phone step stay until they close.
        if visible.as_deref().is_some_and(|v| v.starts_with("pair-")) {
            return true;
        }
        let want = match step {
            Step::Password => PAGES[0],
            Step::Pool {
                start,
                percent,
                message,
                failed,
            } => {
                self.bar.set_fraction(f64::from(*percent) / 100.0);
                self.bar.set_text(Some(&format!("{percent} %")));
                match failed {
                    Some(why) => {
                        self.pool_line.set_label(why);
                        self.pool_retry.set_visible(true);
                    }
                    None => {
                        self.pool_line.set_label(message);
                        self.pool_retry.set_visible(false);
                    }
                }
                if *start && !self.pool_asked.get() {
                    self.start_pool();
                }
                PAGES[1]
            }
            // The words page and the typing page belong to this step.
            Step::Recovery if visible.as_deref() == Some(PAGES[3]) => PAGES[3],
            Step::Recovery => PAGES[2],
            Step::Phone => {
                self.wipe_words();
                PAGES[4]
            }
        };
        if visible.as_deref() != Some(want) {
            self.stack.set_visible_child_name(want);
        }
        true
    }

    pub fn handle(&self, m: SetupMsg) {
        let t = &self.texts;
        match m {
            SetupMsg::Password(r) => {
                self.pw_go.set_sensitive(true);
                match r {
                    Ok(_) => self.pw_note.set_label(&t.t("pw.done")),
                    Err(e) => self.pw_note.set_label(&error_text(t, &e)),
                }
            }
            SetupMsg::PoolStarted(r) => {
                if let Err(e) = r {
                    // archive.busy: it runs already; the state shows it.
                    if e.key != "archive.busy" {
                        self.pool_line.set_label(&error_text(t, &e));
                        self.pool_retry.set_visible(true);
                    }
                }
            }
            SetupMsg::RecoveryMade(r) => {
                self.rec_make.set_sensitive(true);
                match r {
                    Ok(v) => match (setup::asked(&v), v["words"].as_array()) {
                        (Some(ask), Some(words)) => {
                            self.asked.set(ask);
                            let words: Vec<String> = words
                                .iter()
                                .enumerate()
                                .map(|(i, w)| format!("{}. {}", i + 1, w.as_str().unwrap_or("")))
                                .collect();
                            let rows: Vec<String> =
                                words.chunks(5).map(|c| c.join("    ")).collect();
                            self.rec_words.set_label(&rows.join("\n"));
                            self.rec_words.set_visible(true);
                            self.rec_saved.set_visible(true);
                            self.rec_pw.set_visible(false);
                            self.rec_make.set_visible(false);
                            self.rec_note.set_label(&text(
                                t,
                                "setup.recovery.keep",
                                "These words are shown only now and never again. Write them down.",
                            ));
                        }
                        _ => self.rec_note.set_label(&t.t("api.unreachable")),
                    },
                    Err(e) => self.rec_note.set_label(&error_text(t, &e)),
                }
            }
            SetupMsg::Committed(r) => {
                self.confirm_go.set_sensitive(true);
                match r {
                    // The state now says the code is set: the phone step.
                    Ok(_) => {
                        self.wipe_words();
                        self.stack.set_visible_child_name(PAGES[4]);
                    }
                    Err(e) => {
                        self.confirm_note.set_label(&error_text(t, &e));
                        if matches!(e.key.as_str(), "recovery.commit_gone" | "recovery.expired") {
                            self.wipe_words();
                            self.stack.set_visible_child_name(PAGES[2]);
                            self.rec_note.set_label(&error_text(t, &e));
                        }
                    }
                }
            }
            SetupMsg::Saved(r) => {
                if let Err(e) = r {
                    self.phone_note.set_label(&error_text(t, &e));
                    self.rec_note.set_label(&error_text(t, &e));
                }
            }
        }
    }
}
