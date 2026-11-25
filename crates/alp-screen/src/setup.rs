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
use crate::style::{self, Tone};

/// The number of first-start steps shown ("Step 2 of 4"): password,
/// archive, recovery code, phone.
const STEPS: usize = 4;
/// The words of the recovery code (ADR 0013).
const WORDS: usize = 15;

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
    pw_note: style::Note,
    // Archive being made.
    bar: gtk4::ProgressBar,
    pool_line: style::Note,
    pool_retry: gtk4::Button,
    pool_asked: Cell<bool>,
    // Recovery code.
    rec_pw: gtk4::PasswordEntry,
    rec_ask: gtk4::Box,
    rec_make: gtk4::Button,
    rec_what: style::Note,
    /// The sheet with the 15 words, and its word labels (shown once).
    rec_paper: gtk4::Box,
    rec_words: Vec<gtk4::Label>,
    rec_keep: style::Note,
    rec_saved: gtk4::Button,
    rec_note: style::Note,
    asked: Cell<[usize; 3]>,
    confirm_labels: [gtk4::Label; 3],
    confirm_entries: [gtk4::Entry; 3],
    confirm_go: gtk4::Button,
    confirm_note: style::Note,
    // Phone.
    /// Opens the pairing pages (wired in main.rs).
    pub phone_connect: gtk4::Button,
    phone_note: style::Note,
}

fn text(t: &Texts, key: &str, fallback: &str) -> String {
    let s = t.t(key);
    if s == key { fallback.to_string() } else { s }
}

/// One first-start page: the step, a big icon, the title and the lead
/// line centred; the rest left-aligned for easy reading, in a card.
fn page(
    t: &Texts,
    step: usize,
    (icon, tone): (&str, Tone),
    title: &str,
    lead: &str,
    rest: &[&gtk4::Widget],
    width: i32,
) -> gtk4::ScrolledWindow {
    let steps = style::steps(
        step,
        STEPS,
        &text(t, "setup.step", "Step {n} of {of}")
            .replace("{n}", &step.to_string())
            .replace("{of}", &STEPS.to_string()),
    );
    let head = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(10)
        .build();
    head.append(&style::hero(icon, tone));
    head.append(&style::text(title, &["title-1"], true));
    if !lead.is_empty() {
        head.append(&style::text(lead, &["lead"], true));
    }
    let mut all: Vec<&gtk4::Widget> = vec![steps.upcast_ref(), head.upcast_ref()];
    all.extend_from_slice(rest);
    style::scroll(&style::panel(&all, width))
}

fn main_button(icon: &str, s: &str) -> gtk4::Button {
    style::button(icon, s, &["suggested-action", "pill"])
}

fn quiet_button(s: &str) -> gtk4::Button {
    gtk4::Button::builder()
        .label(s)
        .css_classes(["flat", "pill"])
        .build()
}

fn error_text(t: &Texts, e: &ApiError) -> String {
    if e.message.is_empty() {
        t.t(&e.key)
    } else {
        e.message.clone()
    }
}

/// The recovery words on a white "sheet of paper": three columns of five,
/// read down each column (1-5, 6-10, 11-15), a large clear typeface,
/// each word with its number. Returns the sheet and the 15 word labels.
fn paper(t: &Texts) -> (gtk4::Box, Vec<gtk4::Label>) {
    let sheet = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(12)
        .css_classes(["paper"])
        .build();
    sheet.update_property(&[gtk4::accessible::Property::Label(&t.t("recovery.key"))]);
    let grid = gtk4::Grid::builder()
        .row_spacing(10)
        .column_spacing(28)
        .column_homogeneous(true)
        .halign(gtk4::Align::Center)
        .build();
    let mut words = Vec::with_capacity(WORDS);
    for i in 0..WORDS {
        let cell = gtk4::Box::builder().spacing(10).build();
        let n = gtk4::Label::builder()
            .label(format!("{}.", i + 1))
            .css_classes(["word-n"])
            .width_chars(3)
            .xalign(1.0)
            .build();
        let w = gtk4::Label::builder()
            .css_classes(["word"])
            .xalign(0.0)
            .build();
        cell.append(&n);
        cell.append(&w);
        grid.attach(&cell, (i / 5) as i32, (i % 5) as i32, 1, 1);
        words.push(w);
    }
    sheet.append(&grid);
    (sheet, words)
}

impl SetupUi {
    pub fn new(
        texts: Rc<Texts>,
        client: Arc<Client>,
        tx: mpsc::Sender<Msg>,
        stack: &gtk4::Stack,
    ) -> Rc<Self> {
        let t = &texts;
        // 1. Password.
        let pw1 = gtk4::PasswordEntry::builder()
            .show_peek_icon(true)
            .hexpand(true)
            .build();
        let pw2 = gtk4::PasswordEntry::builder()
            .show_peek_icon(true)
            .hexpand(true)
            .build();
        let pw_go = main_button("emblem-ok-symbolic", &t.t("pw.set"));
        let pw_note = style::Note::new("", &t.t("pw.hint.rules"));
        let pw_foot = style::Note::new("", &t.t("pw.foot"));
        pw_foot.kind("pencil");
        stack.add_named(
            &page(
                t,
                1,
                ("dialog-password-symbolic", Tone::Blue),
                &t.t("pw.title"),
                &t.t("pw.sub"),
                &[
                    style::field(&t.t("pw.label"), &pw1).upcast_ref(),
                    style::field(&t.t("pw.again"), &pw2).upcast_ref(),
                    pw_note.root.upcast_ref(),
                    pw_go.upcast_ref(),
                    pw_foot.root.upcast_ref(),
                ],
                560,
            ),
            Some(PAGES[0]),
        );

        // 2. Archive being made.
        let bar = gtk4::ProgressBar::builder().show_text(true).build();
        let pool_line = style::Note::new("", "");
        let pool_retry = main_button("view-refresh-symbolic", &t.t("common.tryagain"));
        pool_retry.set_visible(false);
        stack.add_named(
            &page(
                t,
                2,
                ("drive-harddisk-symbolic", Tone::Teal),
                &text(t, "setup.pool.title", "Making your archive"),
                &text(
                    t,
                    "setup.pool.sub",
                    "The disks you chose are locked with a new key. This takes a few minutes; please leave the computer on.",
                ),
                &[bar.upcast_ref(), pool_line.root.upcast_ref(), pool_retry.upcast_ref()],
                560,
            ),
            Some(PAGES[1]),
        );

        // 3. Recovery code: the password, then the words on a "paper".
        let rec_pw = gtk4::PasswordEntry::builder()
            .show_peek_icon(true)
            .hexpand(true)
            .build();
        let rec_make = main_button("view-reveal-symbolic", &t.t("recovery.show"));
        let rec_ask = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(18)
            .build();
        rec_ask.append(&style::field(&t.t("pw.label"), &rec_pw));
        rec_ask.append(&rec_make);
        let rec_what = style::Note::new("", &t.t("recovery.what"));
        let (rec_paper, rec_words) = paper(t);
        rec_paper.set_visible(false);
        let rec_keep = style::Note::new(
            "warn",
            &text(
                t,
                "setup.recovery.keep",
                "These words are shown only now and never again. Write them on paper, in this order, and keep it away from this computer.",
            ),
        );
        rec_keep.root.set_visible(false);
        let rec_saved = main_button("emblem-ok-symbolic", &t.t("recovery.saved"));
        rec_saved.set_visible(false);
        let rec_later = quiet_button(&t.t("common.later"));
        let rec_note = style::Note::new("bad", "");
        stack.add_named(
            &page(
                t,
                3,
                ("channel-secure-symbolic", Tone::Warn),
                &t.t("recovery.title"),
                &t.t("recovery.intro.password"),
                &[
                    rec_what.root.upcast_ref(),
                    rec_ask.upcast_ref(),
                    rec_paper.upcast_ref(),
                    rec_keep.root.upcast_ref(),
                    rec_note.root.upcast_ref(),
                    rec_saved.upcast_ref(),
                    rec_later.upcast_ref(),
                ],
                680,
            ),
            Some(PAGES[2]),
        );

        // Three words typed back.
        let confirm_labels: [gtk4::Label; 3] = std::array::from_fn(|_| {
            gtk4::Label::builder()
                .css_classes(["word-n-chip"])
                .width_chars(8)
                .build()
        });
        let confirm_entries: [gtk4::Entry; 3] = std::array::from_fn(|_| {
            gtk4::Entry::builder()
                .hexpand(true)
                .input_purpose(gtk4::InputPurpose::FreeForm)
                .input_hints(gtk4::InputHints::NO_SPELLCHECK | gtk4::InputHints::LOWERCASE)
                .build()
        });
        let confirm_go = main_button("emblem-ok-symbolic", &t.t("recovery.confirm.go"));
        let confirm_again = quiet_button(&t.t("recovery.confirm.again"));
        let confirm_note = style::Note::new("bad", "");
        let grid = gtk4::Grid::builder()
            .row_spacing(12)
            .column_spacing(14)
            .build();
        for i in 0..3 {
            confirm_labels[i].set_mnemonic_widget(Some(&confirm_entries[i]));
            confirm_entries[i].update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
                confirm_labels[i].upcast_ref(),
            ])]);
            grid.attach(&confirm_labels[i], 0, i as i32, 1, 1);
            grid.attach(&confirm_entries[i], 1, i as i32, 1, 1);
        }
        stack.add_named(
            &page(
                t,
                3,
                ("document-edit-symbolic", Tone::Warn),
                &t.t("recovery.confirm.title"),
                &t.t("recovery.confirm.sub"),
                &[
                    grid.upcast_ref(),
                    confirm_note.root.upcast_ref(),
                    confirm_go.upcast_ref(),
                    confirm_again.upcast_ref(),
                ],
                560,
            ),
            Some(PAGES[3]),
        );

        // 4. Phone, then finish.
        let phone_connect = main_button("phone-symbolic", &t.t("phone.title"));
        let finish = quiet_button(&t.t("common.done"));
        let phone_note = style::Note::new("bad", "");
        stack.add_named(
            &page(
                t,
                4,
                ("phone-symbolic", Tone::Violet),
                &t.t("phone.title"),
                &t.t("phone.sub"),
                &[
                    phone_connect.upcast_ref(),
                    finish.upcast_ref(),
                    phone_note.root.upcast_ref(),
                ],
                560,
            ),
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
            rec_ask,
            rec_make,
            rec_what,
            rec_paper,
            rec_words,
            rec_keep,
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
            self.pw_note.say("bad", &self.texts.t("pw.hint.short"));
            return;
        }
        if a != b {
            self.pw_note.say("bad", &self.texts.t("pw.mismatch"));
            return;
        }
        self.pw1.set_text("");
        self.pw2.set_text("");
        self.pw_go.set_sensitive(false);
        self.pw_note.say("", &self.texts.t("pw.working"));
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
            self.rec_note.say("bad", &self.texts.t("pw.label"));
            self.rec_pw.grab_focus();
            return;
        }
        self.rec_pw.set_text("");
        self.rec_make.set_sensitive(false);
        self.rec_note.say("", &self.texts.t("recovery.making"));
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
        self.confirm_note.set("");
        self.confirm_entries[0].grab_focus();
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
            .say("", &self.texts.t("recovery.confirming"));
        let body = json!({ "words": words });
        self.call(move |c| SetupMsg::Committed(c.post_slow("live/recovery/commit", &body)));
    }

    /// Forget the words shown (also from the screen).
    fn wipe_words(&self) {
        for w in &self.rec_words {
            w.set_label("");
        }
        self.rec_paper.set_visible(false);
        self.rec_keep.root.set_visible(false);
        self.rec_what.root.set_visible(true);
        self.rec_ask.set_visible(true);
        self.rec_saved.set_visible(false);
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
                        self.pool_line.say("bad", why);
                        self.pool_retry.set_visible(true);
                    }
                    None => {
                        self.pool_line.say("", message);
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
            // The keyboard starts in the page's first field.
            match want {
                w if w == PAGES[0] => {
                    self.pw1.grab_focus();
                }
                w if w == PAGES[2] && self.rec_ask.is_visible() => {
                    self.rec_pw.grab_focus();
                }
                _ => {}
            }
        }
        true
    }

    pub fn handle(&self, m: SetupMsg) {
        let t = &self.texts;
        match m {
            SetupMsg::Password(r) => {
                self.pw_go.set_sensitive(true);
                match r {
                    Ok(_) => self.pw_note.say("ok", &t.t("pw.done")),
                    Err(e) => self.pw_note.say("bad", &error_text(t, &e)),
                }
            }
            SetupMsg::PoolStarted(r) => {
                if let Err(e) = r {
                    // archive.busy: it runs already; the state shows it.
                    if e.key != "archive.busy" {
                        self.pool_line.say("bad", &error_text(t, &e));
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
                            for (i, l) in self.rec_words.iter().enumerate() {
                                l.set_label(words.get(i).and_then(Value::as_str).unwrap_or(""));
                            }
                            self.rec_paper.set_visible(true);
                            self.rec_keep.root.set_visible(true);
                            self.rec_saved.set_visible(true);
                            self.rec_what.root.set_visible(false);
                            self.rec_ask.set_visible(false);
                            self.rec_note.set("");
                            self.rec_saved.grab_focus();
                        }
                        _ => self.rec_note.say("bad", &t.t("api.unreachable")),
                    },
                    Err(e) => self.rec_note.say("bad", &error_text(t, &e)),
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
                        self.confirm_note.say("bad", &error_text(t, &e));
                        if matches!(e.key.as_str(), "recovery.commit_gone" | "recovery.expired") {
                            self.wipe_words();
                            self.stack.set_visible_child_name(PAGES[2]);
                            self.rec_note.say("bad", &error_text(t, &e));
                        }
                    }
                }
            }
            SetupMsg::Saved(r) => {
                if let Err(e) = r {
                    self.phone_note.say("bad", &error_text(t, &e));
                    self.rec_note.say("bad", &error_text(t, &e));
                }
            }
        }
    }
}
