// SPDX-License-Identifier: AGPL-3.0-or-later
//! Settings > Password (0.2.7): change the ALP password (when alpd offers
//! it), the recovery code (check, replace, make one) and the protection
//! line with "encrypt later". Every dangerous step asks for the password
//! first, then shows a warning and a slide.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use alp_screen::api::ApiError;
use alp_screen::set::{self, Account};
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::dialogs::{self, Confirm, Ctx, Danger};
use crate::setup::SetupUi;
use crate::style::{self, Tone};

/// The Password category.
pub struct AccountUi {
    ctx: Ctx,
    setup: Rc<SetupUi>,
    pub page: adw::PreferencesPage,
    pw_row: adw::ActionRow,
    rec_row: adw::ActionRow,
    rec_check: gtk4::Button,
    rec_replace: gtk4::Button,
    rec_make: gtk4::Button,
    enc_row: adw::ActionRow,
    enc_btn: gtk4::Button,
    last: RefCell<Option<(Account, bool, bool, String)>>,
    /// The state's newest account facts, for the flows.
    now: RefCell<Account>,
    /// Progress page of "encrypt later" is on.
    watching: Cell<bool>,
    run_bar: gtk4::ProgressBar,
    run_line: style::Note,
    run_done: gtk4::Button,
    disk: RefCell<set::ArchiveDisk>,
}

pub(crate) fn local_error(key: &str) -> ApiError {
    ApiError {
        key: key.to_string(),
        message: String::new(),
        status: 0,
        retry_after: 0,
        index: None,
    }
}

fn pw_entry(ctx: &Ctx, label: &str) -> gtk4::PasswordEntry {
    let e = gtk4::PasswordEntry::builder()
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    style::name(&e, label);
    let _ = ctx;
    e
}

/// A page with a big icon, a title, a line and the given content.
fn form_page(
    ctx: &Ctx,
    name: &str,
    icon: &str,
    tone: Tone,
    title: &str,
    sub: &str,
    rest: &[&gtk4::Widget],
) {
    let hero = style::hero(icon, tone);
    let t = style::text(title, &["title-1"], true);
    let s = style::text(sub, &["lead"], true);
    let mut all: Vec<&gtk4::Widget> = vec![hero.upcast_ref(), t.upcast_ref()];
    if !sub.is_empty() {
        all.push(s.upcast_ref());
    }
    all.extend_from_slice(rest);
    let page = style::scroll(&style::panel(&all, 560));
    ctx.show_page(name, &page);
}

fn day(iso: &str) -> String {
    glib::DateTime::from_iso8601(iso, None)
        .ok()
        .and_then(|d| d.to_local().ok())
        .and_then(|d| d.format("%e %B %Y").ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

impl AccountUi {
    pub fn new(ctx: &Ctx, setup: Rc<SetupUi>) -> Rc<Self> {
        let page = adw::PreferencesPage::new();
        page.set_vexpand(true);
        let group = adw::PreferencesGroup::new();
        let suffix = |text: &str, icon: &str| {
            let b = style::button(icon, text, &[]);
            b.set_valign(gtk4::Align::Center);
            b
        };

        let pw_row = crate::icon_row(&ctx.t("set.pw"), "dialog-password-symbolic", Tone::Bad);
        pw_row.set_subtitle(&ctx.t("set.pw.d"));
        let pw_btn = suffix(&ctx.t("set.pw.change"), "go-next-symbolic");
        pw_row.add_suffix(&pw_btn);
        group.add(&pw_row);

        let rec_row = crate::icon_row(
            &ctx.t("set.recovery"),
            "channel-secure-symbolic",
            Tone::Warn,
        );
        let rec_check = suffix(&ctx.t("set.recovery.check"), "emblem-ok-symbolic");
        let rec_replace = suffix(&ctx.t("set.recovery.replace"), "view-refresh-symbolic");
        let rec_make = suffix(&ctx.t("set.recovery.make"), "list-add-symbolic");
        rec_row.add_suffix(&rec_check);
        rec_row.add_suffix(&rec_replace);
        rec_row.add_suffix(&rec_make);
        group.add(&rec_row);

        let enc_row = crate::icon_row(&ctx.t("set.enc"), "system-lock-screen-symbolic", Tone::Teal);
        let enc_btn = suffix(&ctx.t("enc.row.go"), "go-next-symbolic");
        enc_row.add_suffix(&enc_btn);
        group.add(&enc_row);
        page.add(&group);

        let run_bar = gtk4::ProgressBar::builder().show_text(true).build();
        let run_line = style::Note::new("", "");
        let run_done = style::button(
            "emblem-ok-symbolic",
            &ctx.t("common.done"),
            &["suggested-action", "pill"],
        );
        run_done.set_halign(gtk4::Align::Center);
        run_done.set_visible(false);
        {
            let hero = style::hero("system-lock-screen-symbolic", Tone::Teal);
            let t = style::text(&ctx.t("enc.run.title"), &["title-1"], true);
            let s = style::text(&ctx.t("enc.run.sub"), &["lead"], true);
            let all: Vec<&gtk4::Widget> = vec![
                hero.upcast_ref(),
                t.upcast_ref(),
                s.upcast_ref(),
                run_bar.upcast_ref(),
                run_line.root.upcast_ref(),
                run_done.upcast_ref(),
            ];
            ctx.stack.add_named(
                &style::scroll(&style::panel(&all, 560)),
                Some("encrypt-run"),
            );
        }
        let ui = Rc::new(AccountUi {
            ctx: ctx.clone(),
            setup,
            page,
            pw_row,
            rec_row,
            rec_check: rec_check.clone(),
            rec_replace: rec_replace.clone(),
            rec_make: rec_make.clone(),
            enc_row,
            enc_btn: enc_btn.clone(),
            last: RefCell::new(None),
            now: RefCell::new(Account::default()),
            watching: Cell::new(false),
            run_bar,
            run_line,
            run_done: run_done.clone(),
            disk: RefCell::new(set::ArchiveDisk::default()),
        });
        {
            let u = ui.clone();
            pw_btn.connect_clicked(move |b| u.change_password(b));
            let u = ui.clone();
            rec_check.connect_clicked(move |b| u.check_recovery(b));
            let u = ui.clone();
            rec_replace.connect_clicked(move |b| u.replace_recovery(b));
            let u = ui.clone();
            rec_make.connect_clicked(move |_| u.setup.open_recovery(None));
            let u = ui.clone();
            enc_btn.connect_clicked(move |_| u.encrypt_step1());
            let c = ctx.clone();
            run_done.connect_clicked(move |_| c.back_to("password"));
        }
        let c = ctx.clone();
        *ui.setup.on_standalone_done.borrow_mut() = Some(Box::new(move || {
            c.refresh();
            c.back_to("password");
        }));
        ui
    }

    /// Reads the state: which rows show, and what they say.
    pub fn update(&self, w: &Value) {
        let a = Account::from_state(w);
        let change = set::has(w, "password_change");
        let commit = set::has(w, "recovery_commit");
        *self.now.borrow_mut() = a.clone();
        *self.disk.borrow_mut() = set::ArchiveDisk::from_state(w);
        self.watch_job(w);
        let key = (a.clone(), change, commit, self.ctx.t("set.pw"));
        if self.last.borrow().as_ref() == Some(&key) {
            return;
        }
        let t = &self.ctx;
        let plain = a.protection.as_deref().is_none_or(|p| p == "none");
        self.pw_row.set_visible(change && !plain);
        self.rec_row.set_visible(a.encrypted());
        self.rec_check.set_visible(a.recovery_set);
        self.rec_replace.set_visible(a.recovery_set && commit);
        self.rec_make.set_visible(!a.recovery_set);
        self.rec_row.set_subtitle(&if a.recovery_set {
            match a
                .recovery_made
                .as_deref()
                .map(day)
                .filter(|d| !d.is_empty())
            {
                Some(d) => t.fill("set.recovery.d", &[("date", &d)]),
                None => t.t("set.recovery.d0"),
            }
        } else {
            t.t("set.recovery.none")
        });
        self.enc_row.set_subtitle(&t.t(&a.protection_key()));
        self.enc_btn.set_visible(!a.encrypted());
        *self.last.borrow_mut() = Some(key);
    }

    // ------------------------------------------------ change password

    fn change_password(self: &Rc<Self>, from: &gtk4::Button) {
        let me = self.clone();
        dialogs::ask_password(&self.ctx, from, "confirm.pwchange", move |cur| {
            me.new_password_page(cur)
        });
    }

    fn new_password_page(self: &Rc<Self>, cur: Zeroizing<String>) {
        let ctx = &self.ctx;
        let (p1, p2) = (
            pw_entry(ctx, &ctx.t("pwc.new")),
            pw_entry(ctx, &ctx.t("pw.again")),
        );
        let note = Rc::new(style::Note::new("warn", ""));
        let go = style::button(
            "go-next-symbolic",
            &ctx.t("common.continue"),
            &["suggested-action", "pill"],
        );
        go.set_halign(gtk4::Align::Center);
        let cancel = gtk4::Button::builder()
            .label(ctx.t("common.cancel"))
            .css_classes(["flat", "pill"])
            .halign(gtk4::Align::Center)
            .build();
        let (f1, f2) = (
            style::field(&ctx.t("pwc.new"), &p1),
            style::field(&ctx.t("pw.again"), &p2),
        );
        form_page(
            ctx,
            "pwchange",
            "dialog-password-symbolic",
            Tone::Bad,
            &ctx.t("pwc.title"),
            &ctx.t("pwc.sub"),
            &[
                f1.upcast_ref(),
                f2.upcast_ref(),
                note.root.upcast_ref(),
                go.upcast_ref(),
                cancel.upcast_ref(),
            ],
        );
        p1.grab_focus();
        let cur = Rc::new(cur);
        let c = ctx.clone();
        cancel.connect_clicked(move |_| c.back_to("password"));
        let submit = {
            let (me, p1, p2, note, cur) = (self.clone(), p1.clone(), p2.clone(), note, cur);
            move || {
                let a = Zeroizing::new(p1.text().to_string());
                let b = Zeroizing::new(p2.text().to_string());
                if a.chars().count() < 8 {
                    note.say("warn", &me.ctx.err_text(&local_error("password.too_short")));
                    return;
                }
                if *a != *b {
                    note.say("warn", &me.ctx.t("pw.mismatch"));
                    return;
                }
                p1.set_text("");
                p2.set_text("");
                me.change_danger(cur.clone(), a);
            }
        };
        let submit = Rc::new(submit);
        let s = submit.clone();
        go.connect_clicked(move |_| s());
        p2.connect_activate(move |_| submit());
    }

    fn change_danger(self: &Rc<Self>, cur: Rc<Zeroizing<String>>, new: Zeroizing<String>) {
        let ctx = &self.ctx;
        let (c1, c2) = (ctx.clone(), ctx.clone());
        let new = Rc::new(new);
        dialogs::danger(
            ctx,
            Danger {
                step: None,
                title: ctx.t("danger.pwc.title"),
                lead: ctx.t("danger.pwc.lead"),
                lose: vec![ctx.t("danger.pwc.lose")],
                keep: vec![ctx.t("danger.pwc.keep1"), ctx.t("danger.pwc.keep2")],
                warn: None,
                confirm: Confirm::Slide(ctx.t("danger.pwc.slide")),
                working: None,
                on_cancel: Rc::new(move || c1.back_to("password")),
                on_confirm: Rc::new(move |h| {
                    let c = c2.clone();
                    let (cur, new) = (cur.clone(), new.clone());
                    c2.post(
                        "live/password/change",
                        json!({"password": cur.as_str(), "new_password": new.as_str()}),
                        move |r| match r {
                            Ok(_) => {
                                c.refresh();
                                let c3 = c.clone();
                                dialogs::done_page(
                                    &c,
                                    &c.t("danger.pwc.ok"),
                                    None,
                                    None,
                                    move || c3.back_to("password"),
                                );
                            }
                            Err(e) => h.fail(&e),
                        },
                    );
                }),
                extra: None,
            },
        );
    }

    // ------------------------------------------------ recovery code

    /// Type the words someone kept; alpd says whether they open the archive.
    fn check_recovery(self: &Rc<Self>, from: &gtk4::Button) {
        let ctx = &self.ctx;
        let (sheet, words) = crate::setup::word_sheet(&ctx.texts);
        let strong = self.now.borrow().strong();
        let pp = pw_entry(ctx, &ctx.t("where.reopen.pp"));
        let note = Rc::new(style::Note::new("", ""));
        let go = style::button(
            "emblem-ok-symbolic",
            &ctx.t("rcheck.go"),
            &["suggested-action"],
        );
        let close = gtk4::Button::builder()
            .label(ctx.t("common.close"))
            .css_classes(["flat"])
            .build();
        let buttons = gtk4::Box::builder()
            .spacing(8)
            .halign(gtk4::Align::End)
            .build();
        buttons.append(&close);
        buttons.append(&go);
        let col = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(12)
            .margin_top(20)
            .margin_bottom(20)
            .margin_start(24)
            .margin_end(24)
            .build();
        col.append(&style::text(&ctx.t("rcheck.title"), &["title-3"], false));
        col.append(&style::text(&ctx.t("rcheck.sub"), &[], false));
        col.append(&sheet);
        if strong {
            col.append(&style::field(&ctx.t("where.reopen.pp"), &pp));
        }
        col.append(&note.root);
        col.append(&buttons);
        let dialog = adw::Dialog::builder()
            .title(ctx.t("rcheck.title"))
            .content_width(760)
            .child(&col)
            .build();
        let d = dialog.clone();
        close.connect_clicked(move |_| {
            d.close();
        });
        // The typed words are forgotten when the window closes.
        let w2 = words.clone();
        let pp2 = pp.clone();
        dialog.connect_closed(move |_| {
            for e in &w2 {
                e.set_text("");
            }
            pp2.set_text("");
        });
        let (ctx2, go2) = (ctx.clone(), go.clone());
        go.connect_clicked(move |_| {
            let typed: Vec<Zeroizing<String>> = words
                .iter()
                .map(|e| Zeroizing::new(e.text().trim().to_lowercase()))
                .filter(|w| !w.is_empty())
                .collect();
            let joined = Zeroizing::new(
                typed
                    .iter()
                    .map(|w| w.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            let pass = Zeroizing::new(pp.text().to_string());
            go2.set_sensitive(false);
            note.say("", &ctx2.t("rcheck.working"));
            let body = if strong && !pass.is_empty() {
                json!({"recovery_key": joined.as_str(), "passphrase": pass.as_str()})
            } else {
                json!({"recovery_key": joined.as_str()})
            };
            let (note, go3, c) = (note.clone(), go2.clone(), ctx2.clone());
            ctx2.run(
                move |cl| cl.post_slow("live/recovery/check", &body),
                move |r| {
                    go3.set_sensitive(true);
                    match r {
                        Ok(v) if v["opens"].as_bool().unwrap_or(false) => {
                            note.say("ok", &c.t("rcheck.ok"))
                        }
                        Ok(_) => note.say("bad", &c.t("rcheck.bad")),
                        Err(e) => note.say("bad", &c.err_text(&e)),
                    }
                },
            );
        });
        dialog.present(Some(from));
    }

    /// Replace: password, warning (what stops working), slide; the old
    /// code keeps working until three words of the new one are typed back.
    fn replace_recovery(self: &Rc<Self>, from: &gtk4::Button) {
        let me = self.clone();
        dialogs::ask_password(&self.ctx, from, "confirm.recovery.replace", move |pw| {
            me.replace_danger(pw)
        });
    }

    fn replace_danger(self: &Rc<Self>, pw: Zeroizing<String>) {
        let ctx = &self.ctx;
        let made = self
            .now
            .borrow()
            .recovery_made
            .as_deref()
            .map(day)
            .filter(|d| !d.is_empty());
        let lose = match made {
            Some(d) => ctx.fill("danger.rec.lose.on", &[("date", &d)]),
            None => ctx.t("danger.rec.lose"),
        };
        let check = gtk4::Button::builder()
            .label(ctx.t("danger.rec.check"))
            .css_classes(["flat"])
            .halign(gtk4::Align::Center)
            .build();
        let me = self.clone();
        check.connect_clicked(move |b| me.check_recovery(b));
        let (c1, c2) = (ctx.clone(), ctx.clone());
        let setup = self.setup.clone();
        let pw = Rc::new(pw);
        dialogs::danger(
            ctx,
            Danger {
                step: None,
                title: ctx.t("danger.rec.title"),
                lead: ctx.t("danger.rec.lead"),
                lose: vec![lose],
                keep: vec![ctx.t("danger.rec.keep")],
                warn: None,
                confirm: Confirm::Slide(ctx.t("danger.rec.slide")),
                working: Some(ctx.t("recovery.making")),
                on_cancel: Rc::new(move || c1.back_to("password")),
                on_confirm: Rc::new(move |h| {
                    let setup = setup.clone();
                    let pw = Zeroizing::new(pw.as_str().to_string());
                    c2.run(
                        move |cl| {
                            cl.post_slow(
                                "live/recovery/create",
                                &json!({"password": pw.as_str(), "replace": true}),
                            )
                        },
                        move |r| match r {
                            Ok(v) => setup.open_recovery(Some(v)),
                            Err(e) => h.fail(&e),
                        },
                    );
                }),
                extra: Some(check.upcast()),
            },
        );
    }

    // ------------------------------------------------ encrypt later

    /// "Encrypt later": three screens (what happens, the password, a slide).
    pub fn start_encrypt(self: &Rc<Self>) {
        self.encrypt_step1();
    }

    /// Step 1: what happens.
    fn encrypt_step1(self: &Rc<Self>) {
        let ctx = &self.ctx;
        let room = alp_screen::model::home::size(self.disk.borrow().used());
        let mut items: Vec<gtk4::Widget> = Vec::new();
        for k in ["enc.1.a", "enc.1.b", "enc.1.c", "enc.1.d", "enc.1.e"] {
            let row = gtk4::Box::builder().spacing(10).build();
            row.append(&style::chip("dialog-information-symbolic", Tone::Blue, 14));
            let l = style::text(&ctx.fill(k, &[("room", &room)]), &[], false);
            l.set_hexpand(true);
            row.append(&l);
            items.push(row.upcast());
        }
        let go = style::button(
            "go-next-symbolic",
            &ctx.t("common.continue"),
            &["suggested-action", "pill"],
        );
        go.set_halign(gtk4::Align::Center);
        let cancel = gtk4::Button::builder()
            .label(ctx.t("common.cancel"))
            .css_classes(["flat", "pill"])
            .halign(gtk4::Align::Center)
            .build();
        let mut rest: Vec<&gtk4::Widget> = items.iter().collect();
        rest.push(go.upcast_ref());
        rest.push(cancel.upcast_ref());
        form_page(
            ctx,
            "encrypt",
            "system-lock-screen-symbolic",
            Tone::Teal,
            &ctx.t("enc.1.title"),
            "",
            &rest,
        );
        let c = ctx.clone();
        cancel.connect_clicked(move |_| c.back_to("password"));
        let me = self.clone();
        go.connect_clicked(move |_| me.encrypt_step2());
    }

    /// Step 2: the password. Level 2 types the screen's own, level 1 a new
    /// one twice.
    fn encrypt_step2(self: &Rc<Self>) {
        let ctx = &self.ctx;
        let console = self.now.borrow().protection.as_deref() == Some("console");
        let first = if console { "enc.2.current" } else { "pw.label" };
        let p1 = pw_entry(ctx, &ctx.t(first));
        let p2 = pw_entry(ctx, &ctx.t("pw.again"));
        let note = Rc::new(style::Note::new("bad", ""));
        let go = style::button(
            "go-next-symbolic",
            &ctx.t("common.continue"),
            &["suggested-action", "pill"],
        );
        go.set_halign(gtk4::Align::Center);
        let cancel = gtk4::Button::builder()
            .label(ctx.t("common.cancel"))
            .css_classes(["flat", "pill"])
            .halign(gtk4::Align::Center)
            .build();
        let f1 = style::field(&ctx.t(first), &p1);
        let f2 = style::field(&ctx.t("pw.again"), &p2);
        let mut rest: Vec<&gtk4::Widget> = vec![f1.upcast_ref()];
        if !console {
            rest.push(f2.upcast_ref());
        }
        rest.push(note.root.upcast_ref());
        rest.push(go.upcast_ref());
        rest.push(cancel.upcast_ref());
        form_page(
            ctx,
            "encrypt",
            "dialog-password-symbolic",
            Tone::Bad,
            &ctx.t("enc.2.title"),
            &ctx.t(if console {
                "enc.2.sub.console"
            } else {
                "enc.2.sub"
            }),
            &rest,
        );
        p1.grab_focus();
        let (p1b, p2b) = (p1.clone(), p2.clone());
        let c = ctx.clone();
        cancel.connect_clicked(move |_| c.back_to("password"));
        let submit = {
            let me = self.clone();
            move || {
                let a = Zeroizing::new(p1.text().to_string());
                let b = Zeroizing::new(p2.text().to_string());
                if a.chars().count() < 8 {
                    note.say("bad", &me.ctx.err_text(&local_error("password.too_short")));
                    return;
                }
                if !console && *a != *b {
                    note.say("bad", &me.ctx.t("pw.mismatch"));
                    return;
                }
                p1.set_text("");
                p2.set_text("");
                me.encrypt_step3(a);
            }
        };
        let submit = Rc::new(submit);
        let s = submit.clone();
        go.connect_clicked(move |_| s());
        if console {
            p1b.connect_activate(move |_| submit());
        } else {
            p2b.connect_activate(move |_| submit());
        }
    }

    /// 3. The slide, then the work.
    fn encrypt_step3(self: &Rc<Self>, pw: Zeroizing<String>) {
        let ctx = &self.ctx;
        let (c1, c2, me) = (ctx.clone(), ctx.clone(), self.clone());
        let pw = Rc::new(pw);
        dialogs::danger(
            ctx,
            Danger {
                step: None,
                title: ctx.t("enc.3.title"),
                lead: ctx.t("enc.3.sub"),
                lose: vec![],
                keep: vec![],
                warn: None,
                confirm: Confirm::Slide(ctx.t("enc.3.slide")),
                working: None,
                on_cancel: Rc::new(move || c1.back_to("password")),
                on_confirm: Rc::new(move |h| {
                    let me = me.clone();
                    let pw = Zeroizing::new(pw.as_str().to_string());
                    c2.run(
                        move |cl| {
                            cl.post_slow("live/archive/encrypt", &json!({"password": pw.as_str()}))
                        },
                        move |r| match r {
                            Ok(_) => me.encrypt_running(),
                            Err(e) => h.fail(&e),
                        },
                    );
                }),
                extra: None,
            },
        );
    }

    fn encrypt_running(self: &Rc<Self>) {
        self.run_bar.set_fraction(0.0);
        self.run_bar.set_text(Some("0 %"));
        self.run_line.set("");
        self.run_done.set_visible(false);
        self.watching.set(true);
        self.ctx.stack.set_visible_child_name("encrypt-run");
    }

    /// While the progress page shows, follows the archive job.
    fn watch_job(&self, w: &Value) {
        if !self.watching.get() {
            return;
        }
        let j = &w["archive"]["job"];
        let pct = j["percent"].as_u64().unwrap_or(0).min(100);
        self.run_bar.set_fraction(pct as f64 / 100.0);
        self.run_bar.set_text(Some(&format!("{pct} %")));
        match j["state"].as_str() {
            Some("done") => {
                self.run_line.say("ok", &self.ctx.t("enc.done"));
                self.finish_run();
            }
            Some("failed") => {
                let m = j["error"]["message"]
                    .as_str()
                    .or(j["message"].as_str())
                    .unwrap_or_default();
                self.run_line.say("bad", m);
                self.finish_run();
            }
            _ => {
                if let Some(m) = j["message"].as_str() {
                    self.run_line.say("", m);
                }
            }
        }
    }

    fn finish_run(&self) {
        self.watching.set(false);
        self.run_done.set_visible(true);
    }
}
