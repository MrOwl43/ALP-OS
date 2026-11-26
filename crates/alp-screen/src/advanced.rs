// SPDX-License-Identifier: AGPL-3.0-or-later
//! Settings > Advanced (0.2.7): behind the password for 10 minutes (no
//! question when the archive has no password), then Disks, Files on other
//! disks, the Terminal and "Delete everything and start over".
//!
//! - Terminal: the password goes to alpd every time; alpd answers a
//!   one-time address on the terminal's own local port. That address is
//!   the session's only secret: it is held in this function's closure and
//!   handed to the locked viewer, never kept in a field, a message or a
//!   log. Leaving the page ends the session (alpd also ends it on lock and
//!   after a quiet time).
//! - Reset: password, two warnings with a button, then the slide, then a
//!   progress page that follows the `reset` job until setup starts again.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use alp_screen::set::{self, Account, ArchiveDisk};
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};
use webkit6::prelude::*;
use webkit6::{NetworkSession, PolicyDecisionType, WebView};
use zeroize::Zeroizing;

use crate::dialogs::{self, Confirm, Ctx, Danger};
use crate::disks::DisksUi;
use crate::files::FilesUi;
use crate::setup::text;
use crate::style::{self, Tone};

/// How long the password for Advanced holds.
const GATE: Duration = Duration::from_secs(10 * 60);

struct ResetRun {
    bar: gtk4::ProgressBar,
    line: gtk4::Label,
    end: gtk4::Box,
    fin: Cell<bool>,
    last: RefCell<String>,
}

/// An open terminal: the viewer and when it started.
struct Term {
    view: WebView,
    holder: gtk4::Box,
    started: Instant,
    seen_on: Cell<bool>,
}

pub struct AdvancedUi {
    ctx: Ctx,
    pub page: adw::PreferencesPage,
    files_row: adw::ActionRow,
    term_row: adw::ActionRow,
    until: Cell<Option<Instant>>,
    protected: Cell<bool>,
    place: RefCell<String>,
    state: RefCell<Value>,
    disks: Rc<DisksUi>,
    files: Rc<FilesUi>,
    reset: RefCell<Option<Rc<ResetRun>>>,
    term: RefCell<Option<Rc<Term>>>,
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

impl AdvancedUi {
    pub fn new(ctx: &Ctx) -> Rc<Self> {
        let page = adw::PreferencesPage::new();
        page.set_vexpand(true);
        let back = {
            let c = ctx.clone();
            move || c.back_to("advanced")
        };
        let disks = DisksUi::new(ctx, back.clone());
        let files = FilesUi::new(ctx, back);

        let row = |title: &str, sub: &str, icon: &str, tone: Tone| {
            let r = crate::icon_row(title, icon, tone);
            r.set_subtitle(&esc(sub));
            r
        };
        let open = |label: String| {
            let b = style::button("go-next-symbolic", &label, &[]);
            b.set_valign(gtk4::Align::Center);
            b
        };
        let tools = adw::PreferencesGroup::new();
        let disks_row = row(
            &ctx.t("set.disks"),
            &ctx.t("set.disks.d"),
            "drive-harddisk-symbolic",
            Tone::Teal,
        );
        let disks_btn = open(ctx.t("set.open"));
        disks_row.add_suffix(&disks_btn);
        let files_row = row(
            &ctx.t("set.files"),
            &ctx.t("set.files.d"),
            "folder-symbolic",
            Tone::Blue,
        );
        let files_btn = open(ctx.t("set.open"));
        files_row.add_suffix(&files_btn);
        let term_row = row(
            &ctx.t("set.term"),
            &ctx.t("set.term.d"),
            "utilities-terminal-symbolic",
            Tone::Grey,
        );
        let term_btn = open(ctx.t("set.open"));
        term_row.add_suffix(&term_btn);
        files_row.set_visible(false);
        term_row.set_visible(false);
        tools.add(&disks_row);
        tools.add(&files_row);
        tools.add(&term_row);
        page.add(&tools);

        let danger_group = adw::PreferencesGroup::new();
        let reset_row = row(
            &ctx.t("admin.reset"),
            &ctx.t("admin.reset.d"),
            "edit-delete-symbolic",
            Tone::Bad,
        );
        let reset_btn = style::button(
            "go-next-symbolic",
            &ctx.t("admin.reset.btn"),
            &["destructive-action"],
        );
        reset_btn.set_valign(gtk4::Align::Center);
        reset_row.add_suffix(&reset_btn);
        danger_group.add(&reset_row);
        page.add(&danger_group);

        let me = Rc::new(AdvancedUi {
            ctx: ctx.clone(),
            page,
            files_row,
            term_row,
            until: Cell::new(None),
            protected: Cell::new(true),
            place: RefCell::new(String::new()),
            state: RefCell::new(Value::Null),
            disks,
            files,
            reset: RefCell::new(None),
            term: RefCell::new(None),
        });
        {
            let m = me.clone();
            disks_btn.connect_clicked(move |_| m.disks.open(None));
        }
        {
            let m = me.clone();
            files_btn.connect_clicked(move |_| {
                let w = m.state.borrow().clone();
                m.files.open(&w);
            });
        }
        {
            let m = me.clone();
            term_btn.connect_clicked(move |_| m.start_terminal());
        }
        {
            let m = me.clone();
            reset_btn.connect_clicked(move |_| {
                let m2 = m.clone();
                dialogs::ask_password(&m.ctx, &m.page, "reset.pw.title", move |pw| {
                    m2.reset_danger(Rc::new(pw), 1);
                });
            });
        }
        {
            let m = me.clone();
            ctx.stack.connect_visible_child_notify(move |s| {
                if s.visible_child_name().as_deref() != Some("terminal") {
                    m.end_terminal();
                }
                if s.visible_child_name().as_deref() != Some("files") {
                    m.files.close();
                }
            });
        }
        me
    }

    // ------------------------------------------------------------- the gate

    /// True while the password for Advanced still holds (or none is set).
    pub fn is_open(&self) -> bool {
        !self.protected.get() || self.until.get().is_some_and(|t| Instant::now() < t)
    }

    /// Keeps the gate open for another 10 minutes (the person is here).
    pub fn extend(&self) {
        self.until.set(Some(Instant::now() + GATE));
    }

    pub fn close_gate(&self) {
        self.until.set(None);
    }

    /// Asks for the password (unless the gate is open); `then` runs when
    /// Advanced may be shown.
    pub fn ask(self: &Rc<Self>, parent: &impl IsA<gtk4::Widget>, then: impl Fn() + 'static) {
        if self.is_open() {
            self.extend();
            then();
            return;
        }
        let me = self.clone();
        dialogs::ask_password(&self.ctx, parent, "confirm.advanced", move |_pw| {
            me.extend();
            then();
        });
    }

    /// Follows the state: the rows that need a feature, the archive place,
    /// the reset and terminal sessions.
    pub fn update(self: &Rc<Self>, w: &Value) {
        *self.state.borrow_mut() = w.clone();
        let a = Account::from_state(w);
        self.protected
            .set(!matches!(a.protection.as_deref(), Some("none")));
        *self.place.borrow_mut() = ArchiveDisk::from_state(w)
            .place
            .map(|p| format!(" ({p})"))
            .unwrap_or_default();
        self.files_row.set_visible(set::has(w, "media_files"));
        self.term_row.set_visible(set::has(w, "terminal"));
        self.disks.update(w);
        self.files.update(w);
        let run = self.reset.borrow().clone();
        if let Some(r) = run {
            self.follow_reset(&r, w);
        }
        self.follow_terminal(w);
    }

    // ------------------------------------------------------------ terminal

    fn start_terminal(self: &Rc<Self>) {
        let me = self.clone();
        dialogs::ask_password(&self.ctx, &self.page, "term.pw", move |pw| {
            let secret = Zeroizing::new(pw.as_str().to_string());
            let m = me.clone();
            me.ctx.run(
                move |c| c.post("live/terminal/start", &json!({"password": secret.as_str()})),
                move |r| m.terminal_answer(r),
            );
        });
    }

    fn terminal_answer(self: &Rc<Self>, r: Result<Value, alp_screen::api::ApiError>) {
        let ctx = &self.ctx;
        let back = {
            let c = ctx.clone();
            move || c.back_to("advanced")
        };
        match r {
            Err(e) => {
                ctx.fail(&e);
                if e.key == "terminal.no_password" {
                    let c = ctx.clone();
                    dialogs::done_page(
                        ctx,
                        &ctx.err_text(&e),
                        None,
                        Some(&ctx.t("term.set_password")),
                        move || c.back_to("password"),
                    );
                } else {
                    dialogs::done_page(
                        ctx,
                        &ctx.err_text(&e),
                        None,
                        Some(&ctx.t("common.back")),
                        back,
                    );
                }
            }
            Ok(v) => {
                let url = v["url"].as_str().filter(|u| set::terminal_url_ok(u));
                match url {
                    None => {
                        ctx.post("live/terminal/stop", json!({}), |_| {});
                        let t = ctx.t("term.url.bad");
                        let mut m = set::Msg::new("term.url", set::Lv::Warn, &t);
                        m.key = Some("term.url".into());
                        ctx.errs.borrow_mut().add(m, dialogs::now());
                        let _ = ctx.tx.send(crate::Msg::ErrorsChanged);
                        dialogs::done_page(ctx, &t, None, Some(&ctx.t("common.back")), back);
                    }
                    Some(u) => self.show_terminal(u),
                }
            }
        }
    }

    /// The terminal in a locked viewer: only its own local address, no new
    /// windows, no downloads, no developer tools.
    fn show_terminal(self: &Rc<Self>, url: &str) {
        let ctx = &self.ctx;
        self.end_terminal();
        let session = NetworkSession::new_ephemeral();
        let view = WebView::builder()
            .network_session(&session)
            .vexpand(true)
            .hexpand(true)
            .build();
        if let Some(s) = WebViewExt::settings(&view) {
            s.set_enable_developer_extras(false);
        }
        view.connect_context_menu(|_, _, _| true);
        view.connect_create(|_, _| None);
        view.connect_decide_policy(|_, decision, kind| {
            if kind != PolicyDecisionType::NavigationAction {
                return false;
            }
            let uri = decision
                .downcast_ref::<webkit6::NavigationPolicyDecision>()
                .and_then(|d| d.navigation_action())
                .and_then(|a| a.request())
                .and_then(|r| r.uri())
                .map(|u| u.to_string());
            if uri.as_deref().is_some_and(set::terminal_url_ok) {
                decision.use_();
            } else {
                decision.ignore();
            }
            true
        });
        session.connect_download_started(|_, d| d.cancel());
        view.update_property(&[gtk4::accessible::Property::Label(&ctx.t("set.term"))]);
        view.load_uri(url);

        let holder = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .vexpand(true)
            .build();
        let note = style::Note::new("", &ctx.t("term.logged"));
        note.root.set_margin_start(20);
        note.root.set_margin_end(20);
        note.root.set_margin_bottom(8);
        holder.append(&note.root);
        holder.append(&view);
        let m = self.clone();
        let page = dialogs::sub_window(
            ctx,
            &ctx.t("set.term"),
            None,
            None,
            move || {
                m.end_terminal();
                m.ctx.back_to("advanced");
            },
            &holder,
        );
        *self.term.borrow_mut() = Some(Rc::new(Term {
            view: view.clone(),
            holder: holder.clone(),
            started: Instant::now(),
            seen_on: Cell::new(false),
        }));
        ctx.show_page("terminal", &page);
        view.grab_focus();
    }

    /// Closes the viewer and ends the session on alpd.
    fn end_terminal(&self) {
        if let Some(t) = self.term.borrow_mut().take() {
            t.view.try_close();
            t.holder.remove(&t.view);
            self.ctx.post("live/terminal/stop", json!({}), |_| {});
        }
    }

    fn follow_terminal(self: &Rc<Self>, w: &Value) {
        let Some(t) = self.term.borrow().clone() else {
            return;
        };
        let off = w["terminal"]["state"].as_str() == Some("off");
        if !off {
            t.seen_on.set(true);
            return;
        }
        if t.seen_on.get() || t.started.elapsed() > Duration::from_secs(8) {
            self.end_terminal();
            let c = self.ctx.clone();
            dialogs::done_page(
                &self.ctx,
                &self.ctx.t("term.ended"),
                Some(&self.ctx.t("term.ended.d")),
                Some(&self.ctx.t("common.back")),
                move || c.back_to("advanced"),
            );
        }
    }

    // --------------------------------------------------------------- reset

    /// The three screens: button, button, slide (0.2.7 `resetDanger`).
    fn reset_danger(self: &Rc<Self>, pw: Rc<Zeroizing<String>>, step: usize) {
        let ctx = &self.ctx;
        let place = self.place.borrow().clone();
        let lose = vec![
            ctx.fill("reset.lose.archive", &[("name", &place)]),
            ctx.t("reset.lose.index"),
        ];
        let keep = vec![
            ctx.t("reset.keep.disk"),
            ctx.t("reset.keep.computer"),
            ctx.t("reset.keep.phones"),
        ];
        let cancel: Rc<dyn Fn()> = {
            let c = ctx.clone();
            Rc::new(move || c.back_to("advanced"))
        };
        let next = |n: usize| -> Rc<dyn Fn(Rc<dialogs::DangerHandle>)> {
            let (m, pw) = (self.clone(), pw.clone());
            Rc::new(move |_| m.reset_danger(pw.clone(), n))
        };
        let d = match step {
            1 => Danger {
                step: Some((1, 3)),
                title: ctx.t("reset.1.title"),
                lead: ctx.t("reset.1.lead"),
                lose,
                keep: vec![],
                warn: None,
                confirm: Confirm::Button(ctx.t("danger.continue")),
                working: None,
                on_cancel: cancel,
                on_confirm: next(2),
                extra: None,
            },
            2 => Danger {
                step: Some((2, 3)),
                title: ctx.t("reset.2.title"),
                lead: ctx.t("reset.2.lead"),
                lose: vec![],
                keep,
                warn: Some(ctx.t("reset.2.warn")),
                confirm: Confirm::Button(ctx.t("danger.continue")),
                working: None,
                on_cancel: cancel,
                on_confirm: next(3),
                extra: None,
            },
            _ => {
                let (m, pw) = (self.clone(), pw.clone());
                Danger {
                    step: Some((3, 3)),
                    title: ctx.t("reset.3.title"),
                    lead: ctx.t("reset.3.lead"),
                    lose,
                    keep: vec![],
                    warn: Some(ctx.t("danger.cannot")),
                    confirm: Confirm::Slide(ctx.t("reset.slide")),
                    working: Some(ctx.t("reset.working")),
                    on_cancel: cancel,
                    on_confirm: Rc::new(move |h| {
                        let secret = Zeroizing::new(pw.as_str().to_string());
                        let m = m.clone();
                        m.ctx.clone().run(
                            move |c| c.post("live/reset", &json!({"password": secret.as_str()})),
                            move |r| match r {
                                Err(e) => h.fail(&e),
                                Ok(v) => {
                                    m.close_gate();
                                    m.reset_progress(v["job"]["message"].as_str());
                                }
                            },
                        );
                    }),
                    extra: None,
                }
            }
        };
        dialogs::danger(ctx, d);
    }

    /// The reset job: erasing can take many minutes on a big stick. It is
    /// done when the lock state is `setup` again.
    fn reset_progress(self: &Rc<Self>, first: Option<&str>) {
        let ctx = &self.ctx;
        let hero = style::hero("view-refresh-symbolic", Tone::Warn);
        let title = style::text(&ctx.t("reset.run.title"), &["title-1"], true);
        let sub = style::text(&ctx.t("reset.run.sub"), &["lead"], true);
        let bar = gtk4::ProgressBar::builder().fraction(0.05).build();
        bar.set_accessible_role(gtk4::AccessibleRole::Img);
        let line = style::text(
            first.unwrap_or(&ctx.t("reset.working")),
            &["dim-label"],
            true,
        );
        let end = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(12)
            .build();
        let page = style::scroll(&style::panel(
            &[
                hero.upcast_ref(),
                title.upcast_ref(),
                sub.upcast_ref(),
                bar.upcast_ref(),
                line.upcast_ref(),
                end.upcast_ref(),
            ],
            620,
        ));
        ctx.show_page("reset", &page);
        *self.reset.borrow_mut() = Some(Rc::new(ResetRun {
            bar,
            line,
            end,
            fin: Cell::new(false),
            last: RefCell::new(String::new()),
        }));
        ctx.refresh();
    }

    fn follow_reset(self: &Rc<Self>, r: &Rc<ResetRun>, w: &Value) {
        if r.fin.get() {
            return;
        }
        let ctx = &self.ctx;
        let job = set::job(w, "reset");
        if let Some(j) = &job {
            let p = j.percent.max(2);
            r.bar.set_fraction(p as f64 / 100.0);
            r.bar
                .update_property(&[gtk4::accessible::Property::Label(&format!("{p} %"))]);
            if !j.message.is_empty() {
                r.line.set_label(&j.message);
                *r.last.borrow_mut() = j.message.clone();
            }
            if j.state == "failed" {
                r.fin.set(true);
                r.line.set_visible(false);
                let msg = j
                    .error
                    .clone()
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| text(&ctx.texts, "reset.failed", &j.message));
                r.end.append(&style::Note::new("bad", &msg).root);
                let b = style::button("go-previous-symbolic", &ctx.t("common.back"), &["pill"]);
                b.set_halign(gtk4::Align::Center);
                let c = ctx.clone();
                b.connect_clicked(move |_| c.back_to("advanced"));
                r.end.append(&b);
                return;
            }
        }
        let setup_again = w["lock"]["state"].as_str() == Some("setup");
        if setup_again && job.as_ref().is_none_or(|j| j.state == "done") {
            r.fin.set(true);
            *self.reset.borrow_mut() = None;
            let last = r.last.borrow().clone();
            let c = ctx.clone();
            dialogs::done_page(
                ctx,
                &ctx.t("reset.ok"),
                (!last.is_empty()).then_some(last.as_str()),
                Some(&ctx.t("reset.go")),
                move || c.refresh(),
            );
        }
    }
}
