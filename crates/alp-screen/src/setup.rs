// SPDX-License-Identifier: AGPL-3.0-or-later
//! First-start pages of the installed system (plan `0.2.8-first-start.md`,
//! step 4): choose the password, watch the archive being made on the disks
//! chosen in the installer, make the recovery code (shown once, three
//! words typed back), connect a phone, finish. When the installer kept an
//! old archive (plan `0.2.8-reopen-archive.md`), the archive step asks for
//! its 15 recovery words instead and no new code is made. Every step is an alpd call;
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

/// The number of first-start steps shown ("Step 4 of 7"): welcome,
/// network, place (install or keep the stick), password, archive, recovery
/// code, phone (0.2.6 order, owner 2026-11-26).
const STEPS: usize = 7;
/// The steps before this module's pages (welcome.rs and the install page).
const BEFORE: usize = 3;
/// The words of the recovery code (ADR 0013).
const WORDS: usize = 15;

pub enum SetupMsg {
    Password(Result<Value, ApiError>),
    PoolStarted(Result<Value, ApiError>),
    /// `archive/reopen-pool` answered (the job then shows in the state).
    Reopen(Result<Value, ApiError>),
    RecoveryMade(Result<Value, ApiError>),
    Committed(Result<Value, ApiError>),
    /// Skip (recovery) or finish: the answer only matters when it failed.
    Saved(Result<Value, ApiError>),
}

/// The pages this module adds to the stack.
const PAGES: [&str; 7] = [
    "setup-password",
    "setup-pool",
    "setup-recovery",
    "setup-confirm",
    "setup-phone",
    "setup-installed",
    "setup-words",
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
    // Old archive opened again with its recovery words.
    words: Vec<gtk4::Entry>,
    words_form: gtk4::Box,
    pp_on: gtk4::CheckButton,
    pp: gtk4::PasswordEntry,
    words_go: gtk4::Button,
    words_bar: gtk4::ProgressBar,
    words_note: style::Note,
    /// An `archive/reopen-pool` call is out.
    words_busy: Cell<bool>,
    /// The note shows the answer to the last call (newer than the job).
    words_answered: Cell<bool>,
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
    /// Opens the install pages (wired in main.rs): the place step.
    pub open_install: std::cell::RefCell<Option<Box<dyn Fn()>>>,
    phone_note: style::Note,
    /// The recovery pages were opened from Settings (a code made or
    /// replaced later), not by first-start setup.
    standalone: Cell<bool>,
    /// Where Settings goes after such a code is saved or left.
    pub on_standalone_done: std::cell::RefCell<Option<Box<dyn Fn()>>>,
}

pub(crate) fn text(t: &Texts, key: &str, fallback: &str) -> String {
    let s = t.t(key);
    if s == key { fallback.to_string() } else { s }
}

/// One first-start page: the step, a big icon, the title and the lead
/// line centred; the rest left-aligned for easy reading, in a card.
fn page(
    t: &Texts,
    step: usize,
    head: (&str, Tone),
    title: &str,
    lead: &str,
    rest: &[&gtk4::Widget],
    width: i32,
) -> gtk4::ScrolledWindow {
    page_at(t, step + BEFORE, head, title, lead, rest, width)
}

/// [`page`] with the step number as shown (1 to [`STEPS`]).
pub(crate) fn page_at(
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

/// The 15 entries for typing a recovery code, laid out like the sheet it
/// was written on: three columns of five, read down each column, numbered.
/// Pasting or typing the whole code into one entry spreads it out.
pub(crate) fn word_sheet(t: &Texts) -> (gtk4::Box, Vec<gtk4::Entry>) {
    let sheet = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .css_classes(["paper"])
        .build();
    sheet.update_property(&[gtk4::accessible::Property::Label(&t.t("recovery.key"))]);
    let grid = gtk4::Grid::builder()
        .row_spacing(10)
        .column_spacing(20)
        .column_homogeneous(true)
        .halign(gtk4::Align::Center)
        .build();
    let mut entries = Vec::with_capacity(WORDS);
    for i in 0..WORDS {
        let cell = gtk4::Box::builder().spacing(8).build();
        let n = gtk4::Label::builder()
            .label(format!("{}.", i + 1))
            .css_classes(["word-n"])
            .width_chars(3)
            .xalign(1.0)
            .build();
        let e = gtk4::Entry::builder()
            .width_chars(11)
            .css_classes(["word-entry"])
            .input_hints(gtk4::InputHints::NO_SPELLCHECK | gtk4::InputHints::LOWERCASE)
            .build();
        // Its name for screen readers (no tooltip over the typing).
        e.update_property(&[gtk4::accessible::Property::Label(
            &t.fill("recovery.confirm.n", &[("n", &(i + 1).to_string())]),
        )]);
        cell.append(&n);
        cell.append(&e);
        grid.attach(&cell, (i / 5) as i32, (i % 5) as i32, 1, 1);
        entries.push(e);
    }
    // Several words in one entry (pasted, or typed with spaces): one per
    // entry from here on.
    for (i, e) in entries.iter().enumerate() {
        let all = entries.clone();
        e.connect_changed(move |e| {
            let text = e.text();
            let parts: Vec<&str> = text.split_whitespace().collect();
            if parts.len() < 2 && !text.ends_with(' ') {
                return;
            }
            let parts: Vec<Zeroizing<String>> = parts
                .iter()
                .map(|p| Zeroizing::new(p.to_string()))
                .collect();
            // The keyboard moves on at once (the next key lands in the next
            // word); the texts are set after this change is done (GTK
            // refuses edits inside it).
            let next = (i + parts.len()).min(WORDS - 1);
            if let Some(n) = all.get(next) {
                n.grab_focus();
            }
            let (e, all) = (e.clone(), all.clone());
            gtk4::glib::idle_add_local_once(move || {
                if parts.is_empty() {
                    e.set_text("");
                }
                for (k, w) in parts.iter().enumerate() {
                    if let Some(target) = all.get(i + k) {
                        target.set_text(w);
                    }
                }
                if let Some(n) = all.get(next) {
                    n.set_position(-1);
                }
            });
        });
        // Enter goes to the next word.
        if let Some(n) = entries.get(i + 1).cloned() {
            e.connect_activate(move |_| {
                n.grab_focus();
            });
        }
    }
    sheet.append(&grid);
    (sheet, entries)
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

        // 2b. An old archive opened again: its 15 recovery words.
        let (sheet, words) = word_sheet(t);
        let pp_on = gtk4::CheckButton::builder()
            .label(text(
                t,
                "reopen.pp.on",
                "My archive also has an extra passphrase (Strong protection)",
            ))
            .build();
        let pp = gtk4::PasswordEntry::builder()
            .show_peek_icon(true)
            .hexpand(true)
            .build();
        let pp_field = style::field(&text(t, "reopen.pp", "Extra passphrase"), &pp);
        pp_field.set_visible(false);
        {
            let f = pp_field.clone();
            let e = pp.clone();
            pp_on.connect_toggled(move |c| {
                f.set_visible(c.is_active());
                if c.is_active() {
                    e.grab_focus();
                } else {
                    e.set_text("");
                }
            });
        }
        let words_form = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(14)
            .build();
        words_form.append(&sheet);
        words_form.append(&pp_on);
        words_form.append(&pp_field);
        let words_go = main_button(
            "document-open-symbolic",
            &text(t, "reopen.go", "Open my archive"),
        );
        let words_bar = gtk4::ProgressBar::builder()
            .show_text(true)
            .visible(false)
            .build();
        let words_note = style::Note::new(
            "",
            &text(
                t,
                "reopen.note",
                "Nothing on the disks changes until the words are right.",
            ),
        );
        let words_foot = style::Note::new(
            "pencil",
            &text(
                t,
                "reopen.foot",
                "Lost the words? Without them nobody can open this archive, not even us. You can restart and install again without this disk.",
            ),
        );
        stack.add_named(
            &page(
                t,
                2,
                ("folder-open-symbolic", Tone::Ok),
                &text(t, "reopen.title", "Type your 15 recovery words"),
                &text(
                    t,
                    "reopen.sub",
                    "These are the words you wrote down or photographed when you made this archive. They open it again with all your photos.",
                ),
                &[
                    words_form.upcast_ref(),
                    words_note.root.upcast_ref(),
                    words_bar.upcast_ref(),
                    words_go.upcast_ref(),
                    words_foot.root.upcast_ref(),
                ],
                680,
            ),
            Some(PAGES[6]),
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

        // Installed from this stick: remove it, then restart.
        let restart = main_button("system-reboot-symbolic", &t.t("power.restart.yes"));
        stack.add_named(
            &style::scroll(&style::panel(
                &[
                    style::hero("emblem-ok-symbolic", Tone::Ok).upcast_ref(),
                    style::text(
                        &text(t, "setup.installed.title", "ALP-OS is installed"),
                        &["title-1"],
                        true,
                    )
                    .upcast_ref(),
                    style::text(
                        &text(
                            t,
                            "setup.installed.sub",
                            "Remove the USB stick, then restart. Setup goes on from the computer's own disk.",
                        ),
                        &["lead"],
                        true,
                    )
                    .upcast_ref(),
                    restart.upcast_ref(),
                ],
                560,
            )),
            Some(PAGES[5]),
        );
        {
            let c = client.clone();
            restart.connect_clicked(move |b| {
                b.set_sensitive(false);
                let c = c.clone();
                std::thread::spawn(move || {
                    let _ = c.post("live/power/off", &json!({"restart": true}));
                });
            });
        }

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
            words,
            words_form,
            pp_on,
            pp,
            words_go,
            words_bar,
            words_note,
            words_busy: Cell::new(false),
            words_answered: Cell::new(false),
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
            open_install: std::cell::RefCell::new(None),
            standalone: Cell::new(false),
            on_standalone_done: std::cell::RefCell::new(None),
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
            ui.words_go.connect_clicked(move |_| u.reopen());
            if let Some(last) = ui.words.last() {
                let u = ui.clone();
                last.connect_activate(move |_| u.reopen());
            }
            let u = ui.clone();
            ui.pp.connect_activate(move |_| u.reopen());
            let u = ui.clone();
            ui.rec_make.connect_clicked(move |_| u.make_recovery());
            let u = ui.clone();
            ui.rec_pw.connect_activate(move |_| u.make_recovery());
            let u = ui.clone();
            ui.rec_saved.connect_clicked(move |_| u.ask_words());
            let u = ui.clone();
            rec_later.connect_clicked(move |_| {
                if u.standalone.get() {
                    u.leave_standalone(true);
                    return;
                }
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

    /// Sends the typed words (lower case, one space between) and the
    /// passphrase. The words stay on the screen until the archive is open,
    /// so a typo can be fixed without typing all 15 again.
    fn reopen(&self) {
        if self.words_busy.get() {
            return;
        }
        for e in &self.words {
            e.remove_css_class("error");
        }
        let typed: Vec<Zeroizing<String>> = self
            .words
            .iter()
            .map(|e| Zeroizing::new(e.text().trim().to_lowercase()))
            .collect();
        if let Some(i) = typed.iter().position(|w| w.is_empty()) {
            self.words_note.say(
                "bad",
                &text(
                    &self.texts,
                    "reopen.missing",
                    "Please type all 15 words. Word {n} is empty.",
                )
                .replace("{n}", &(i + 1).to_string()),
            );
            self.words[i].add_css_class("error");
            self.words[i].grab_focus();
            return;
        }
        let words = Zeroizing::new(
            typed
                .iter()
                .map(|w| w.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
        let pp = Zeroizing::new(if self.pp_on.is_active() {
            self.pp.text().to_string()
        } else {
            String::new()
        });
        self.words_busy.set(true);
        self.words_answered.set(false);
        self.words_go.set_sensitive(false);
        self.words_form.set_sensitive(false);
        self.words_note.say(
            "",
            &text(
                &self.texts,
                "reopen.checking",
                "Checking your recovery words…",
            ),
        );
        self.call(move |c| {
            let mut body = json!({"words": words.as_str()});
            if !pp.is_empty() {
                body["passphrase"] = json!(pp.as_str());
            }
            let r = c.post_slow("live/archive/reopen-pool", &body);
            // The body held the words: overwrite it before it is dropped.
            if let Some(Value::String(s)) = body.get_mut("words") {
                zeroize::Zeroize::zeroize(s);
            }
            if let Some(Value::String(s)) = body.get_mut("passphrase") {
                zeroize::Zeroize::zeroize(s);
            }
            SetupMsg::Reopen(r)
        });
    }

    /// Forget the typed recovery words and passphrase.
    fn wipe_typed(&self) {
        for e in &self.words {
            e.set_text("");
            e.remove_css_class("error");
        }
        self.pp.set_text("");
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

    /// Settings > Password: the recovery pages, outside first-start setup.
    /// `answer` is a code just made by Replace (its words show at once).
    pub fn open_recovery(&self, answer: Option<Value>) {
        self.standalone.set(true);
        self.wipe_words();
        self.rec_note.set("");
        self.stack.set_visible_child_name(PAGES[2]);
        match answer {
            Some(v) => self.handle(SetupMsg::RecoveryMade(Ok(v))),
            None => {
                self.rec_pw.grab_focus();
            }
        }
    }

    /// Leaves the recovery pages for Settings; a code not yet saved is
    /// discarded (the old one keeps working).
    fn leave_standalone(&self, discard: bool) {
        self.standalone.set(false);
        self.wipe_words();
        if discard {
            self.call(|c| SetupMsg::Saved(c.post("live/recovery/discard", &json!({}))));
        }
        if let Some(f) = self.on_standalone_done.borrow().as_ref() {
            f();
        }
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
        if !matches!(step, Some(Step::Words { .. })) {
            self.wipe_typed();
        }
        if self.standalone.get() {
            if on_setup {
                return true;
            }
            self.standalone.set(false);
        }
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
            // Where ALP-OS lives: the install pages (install on a disk, or
            // keep running from the stick); they stay while in use.
            Step::Place => {
                let on_install = visible
                    .as_deref()
                    .is_some_and(|v| v == "install" || v.starts_with("install-"));
                if !on_install && let Some(f) = self.open_install.borrow().as_ref() {
                    f();
                }
                return true;
            }
            Step::Installed => PAGES[5],
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
            Step::Words {
                running,
                percent,
                message,
                failed,
            } => {
                self.words_bar.set_visible(*running);
                self.words_bar.set_fraction(f64::from(*percent) / 100.0);
                self.words_bar.set_text(Some(&format!("{percent} %")));
                let busy = *running || self.words_busy.get();
                self.words_go.set_sensitive(!busy);
                self.words_form.set_sensitive(!busy);
                if *running {
                    self.words_answered.set(false);
                    if !message.is_empty() {
                        self.words_note.say("", message);
                    }
                } else if let Some((key, why)) = failed
                    && !self.words_answered.get()
                    && !self.words_busy.get()
                {
                    let why = if key == "recovery.wrong" {
                        text(
                            &self.texts,
                            "reopen.wrong",
                            "These words do not open this archive. Nothing was changed. Please check each word and try again.",
                        )
                    } else {
                        why.clone()
                    };
                    self.words_note.say("bad", &why);
                }
                PAGES[6]
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
                w if w == PAGES[6] => {
                    self.words[0].grab_focus();
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
            SetupMsg::Reopen(r) => {
                self.words_busy.set(false);
                match r {
                    // The job runs; the state shows its progress.
                    Ok(_) => {}
                    Err(e) => {
                        self.words_answered.set(true);
                        self.words_go.set_sensitive(true);
                        self.words_form.set_sensitive(true);
                        self.words_note.say("bad", &error_text(t, &e));
                        // A typo: mark that word and go there.
                        if let Some(w) = e.index.and_then(|i| self.words.get(i.wrapping_sub(1))) {
                            w.add_css_class("error");
                            w.grab_focus();
                        }
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
                    Ok(_) if self.standalone.get() => self.leave_standalone(false),
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
