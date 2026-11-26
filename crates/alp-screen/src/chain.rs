// SPDX-License-Identifier: AGPL-3.0-or-later
//! Settings > Family chain (0.2.7): start a chain (8 words and a picture
//! to keep), join one with its 8 words, show the code again (password
//! first) or leave (password, warning, slide).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use alp_screen::api::ApiError;
use alp_screen::set;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::dialogs::{self, Confirm, Ctx, Danger};
use crate::pairing::{Cells, qr_area};
use crate::style::{self, Tone};

const WORDS: usize = 8;

pub struct ChainUi {
    ctx: Ctx,
    pub page: adw::PreferencesPage,
    row: adw::ActionRow,
    show_btn: gtk4::Button,
    leave_btn: gtk4::Button,
    setup_btn: gtk4::Button,
    member: Cell<Option<bool>>,
    // The code page (shown once, wiped when left).
    seed_words: Vec<gtk4::Label>,
    seed_cells: Cells,
    seed_qr: gtk4::DrawingArea,
    // Joining.
    entries: Vec<gtk4::Entry>,
    join_note: style::Note,
    join_go: gtk4::Button,
}

fn flat(label: &str) -> gtk4::Button {
    gtk4::Button::builder()
        .label(label)
        .css_classes(["flat", "pill"])
        .halign(gtk4::Align::Center)
        .build()
}

impl ChainUi {
    pub fn new(ctx: &Ctx) -> Rc<Self> {
        let page = adw::PreferencesPage::new();
        page.set_vexpand(true);
        let row = crate::icon_row(&ctx.t("set.chain"), "emblem-shared-symbolic", Tone::Warn);
        let suffix = |text: &str, icon: &str| {
            let b = style::button(icon, text, &[]);
            b.set_valign(gtk4::Align::Center);
            b
        };
        let show_btn = suffix(&ctx.t("set.chain.show"), "view-reveal-symbolic");
        let leave_btn = suffix(&ctx.t("set.chain.leave"), "system-log-out-symbolic");
        let setup_btn = suffix(&ctx.t("set.chain.setup"), "go-next-symbolic");
        row.add_suffix(&show_btn);
        row.add_suffix(&leave_btn);
        row.add_suffix(&setup_btn);
        let g = adw::PreferencesGroup::new();
        g.add(&row);
        page.add(&g);

        // The code page: 8 words in two columns, and the picture.
        let words_grid = gtk4::Grid::builder()
            .row_spacing(10)
            .column_spacing(28)
            .column_homogeneous(true)
            .halign(gtk4::Align::Center)
            .build();
        let mut seed_words = Vec::new();
        for i in 0..WORDS {
            let cell = gtk4::Box::builder().spacing(10).build();
            cell.append(
                &gtk4::Label::builder()
                    .label(format!("{}.", i + 1))
                    .css_classes(["word-n"])
                    .width_chars(3)
                    .xalign(1.0)
                    .build(),
            );
            let w = gtk4::Label::builder()
                .css_classes(["word"])
                .xalign(0.0)
                .selectable(false)
                .build();
            cell.append(&w);
            words_grid.attach(&cell, (i / 4) as i32, (i % 4) as i32, 1, 1);
            seed_words.push(w);
        }
        let sheet = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .css_classes(["paper"])
            .build();
        sheet.append(&words_grid);
        sheet.update_property(&[gtk4::accessible::Property::Label(
            &ctx.t("chain.seed.title"),
        )]);
        let seed_cells: Cells = Rc::new(RefCell::new(None));
        let seed_qr = qr_area(&seed_cells);
        seed_qr.update_property(&[gtk4::accessible::Property::Label(
            &ctx.t("chain.seed.qr.alt"),
        )]);
        let qr_frame = gtk4::Box::builder()
            .css_classes(["paper", "qr"])
            .halign(gtk4::Align::Center)
            .build();
        qr_frame.append(&seed_qr);
        let seed_done = style::button(
            "emblem-ok-symbolic",
            &ctx.t("chain.seed.done"),
            &["suggested-action", "pill"],
        );
        seed_done.set_halign(gtk4::Align::Center);
        let seed_page = style::scroll(&style::panel(
            &[
                style::hero("emblem-shared-symbolic", Tone::Warn).upcast_ref(),
                style::text(&ctx.t("chain.seed.title"), &["title-1"], true).upcast_ref(),
                style::text(&ctx.t("chain.seed.sub"), &["lead"], true).upcast_ref(),
                style::text(&ctx.t("chain.seed.write"), &[], false).upcast_ref(),
                sheet.upcast_ref(),
                qr_frame.upcast_ref(),
                seed_done.upcast_ref(),
            ],
            680,
        ));
        ctx.stack.add_named(&seed_page, Some("chain-seed"));

        // Join: eight boxes, one word each.
        let entries: Vec<gtk4::Entry> = (0..WORDS)
            .map(|i| {
                let e = gtk4::Entry::builder()
                    .placeholder_text((i + 1).to_string())
                    .hexpand(true)
                    .input_hints(gtk4::InputHints::NO_SPELLCHECK | gtk4::InputHints::LOWERCASE)
                    .build();
                style::name(
                    &e,
                    &ctx.fill("chain.join.word", &[("n", &(i + 1).to_string())]),
                );
                e
            })
            .collect();
        let grid = gtk4::Grid::builder()
            .row_spacing(10)
            .column_spacing(12)
            .column_homogeneous(true)
            .build();
        for (i, e) in entries.iter().enumerate() {
            grid.attach(e, (i % 2) as i32, (i / 2) as i32, 1, 1);
        }
        let join_note = style::Note::new("bad", "");
        let join_go = style::button(
            "go-next-symbolic",
            &ctx.t("chain.join.go"),
            &["suggested-action", "pill"],
        );
        join_go.set_halign(gtk4::Align::Center);
        let join_back = flat(&ctx.t("common.back"));
        let join_page = style::scroll(&style::panel(
            &[
                style::hero("emblem-shared-symbolic", Tone::Warn).upcast_ref(),
                style::text(&ctx.t("chain.join.title"), &["title-1"], true).upcast_ref(),
                style::text(&ctx.t("chain.join.sub2"), &["lead"], true).upcast_ref(),
                grid.upcast_ref(),
                join_note.root.upcast_ref(),
                join_go.upcast_ref(),
                join_back.upcast_ref(),
            ],
            680,
        ));
        ctx.stack.add_named(&join_page, Some("chain-join"));

        // Choose: start a new chain, or join one.
        let start = adw::ActionRow::builder()
            .title(ctx.t("chain.start"))
            .subtitle(ctx.t("chain.start.sub"))
            .activatable(true)
            .build();
        start.add_prefix(&style::chip("list-add-symbolic", Tone::Warn, 20));
        let join = adw::ActionRow::builder()
            .title(ctx.t("chain.join"))
            .subtitle(ctx.t("chain.join.sub"))
            .activatable(true)
            .build();
        join.add_prefix(&style::chip("emblem-shared-symbolic", Tone::Warn, 20));
        let choose_list = gtk4::ListBox::builder()
            .css_classes(["boxed-list"])
            .selection_mode(gtk4::SelectionMode::None)
            .build();
        choose_list.append(&start);
        choose_list.append(&join);
        let choose_note = style::Note::new("bad", "");
        let choose_back = flat(&ctx.t("common.back"));
        let choose_page = style::scroll(&style::panel(
            &[
                style::hero("emblem-shared-symbolic", Tone::Warn).upcast_ref(),
                style::text(&ctx.t("chain.title"), &["title-1"], true).upcast_ref(),
                style::text(&ctx.t("chain.sub"), &["lead"], true).upcast_ref(),
                choose_list.upcast_ref(),
                choose_note.root.upcast_ref(),
                choose_back.upcast_ref(),
            ],
            680,
        ));
        ctx.stack.add_named(&choose_page, Some("chain"));

        let ui = Rc::new(ChainUi {
            ctx: ctx.clone(),
            page,
            row,
            show_btn: show_btn.clone(),
            leave_btn: leave_btn.clone(),
            setup_btn: setup_btn.clone(),
            member: Cell::new(None),
            seed_words,
            seed_cells,
            seed_qr,
            entries,
            join_note,
            join_go: join_go.clone(),
        });
        let choose_note = Rc::new(choose_note);
        {
            let u = ui.clone();
            show_btn.connect_clicked(move |b| u.show_code(b));
            let u = ui.clone();
            leave_btn.connect_clicked(move |b| u.leave(b));
            let c = ctx.clone();
            setup_btn.connect_clicked(move |_| c.stack.set_visible_child_name("chain"));
            let c = ctx.clone();
            choose_back.connect_clicked(move |_| c.back_to("chain"));
            let (u, n) = (ui.clone(), choose_note);
            start.connect_activated(move |_| {
                n.set("");
                let (u2, n2) = (u.clone(), n.clone());
                u.ctx
                    .post("live/chain/create", json!({}), move |r| match r {
                        Ok(v) => u2.show_seed(&v),
                        Err(e) => n2.say("bad", &u2.ctx.err_text(&e)),
                    });
            });
            let u = ui.clone();
            join.connect_activated(move |_| u.open_join());
            let c = ctx.clone();
            join_back.connect_clicked(move |_| c.stack.set_visible_child_name("chain"));
            let u = ui.clone();
            join_go.connect_clicked(move |_| u.join(None));
            let u = ui.clone();
            seed_done.connect_clicked(move |_| {
                u.wipe_seed();
                u.ctx.refresh();
                u.ctx.back_to("chain");
            });
        }
        ui.spread_pastes();
        ui
    }

    pub fn update(&self, w: &Value) {
        let m = set::chain_member(w);
        if self.member.get() == Some(m) {
            return;
        }
        self.member.set(Some(m));
        self.show_btn.set_visible(m);
        self.leave_btn.set_visible(m);
        self.setup_btn.set_visible(!m);
        self.row.set_subtitle(&self.ctx.t(if m {
            "set.chain.member"
        } else {
            "set.chain.none"
        }));
    }

    fn show_code(self: &Rc<Self>, from: &gtk4::Button) {
        let me = self.clone();
        dialogs::ask_password(&self.ctx, from, "confirm.chain", move |_pw| {
            let me2 = me.clone();
            me.ctx.run(
                |c| c.get("live/chain/code"),
                move |r| match r {
                    Ok(v) => me2.show_seed(&v),
                    Err(e) => me2.ctx.fail(&e),
                },
            );
        });
    }

    /// The 8 words and the picture, shown once.
    fn show_seed(&self, v: &Value) {
        for (i, l) in self.seed_words.iter().enumerate() {
            l.set_label(
                v["words"]
                    .get(i)
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
        }
        *self.seed_cells.borrow_mut() = v["qr_text"]
            .as_str()
            .and_then(alp_screen::model::pairing::qr);
        self.seed_qr.queue_draw();
        self.ctx.stack.set_visible_child_name("chain-seed");
    }

    fn wipe_seed(&self) {
        for l in &self.seed_words {
            l.set_label("");
        }
        *self.seed_cells.borrow_mut() = None;
        self.seed_qr.queue_draw();
    }

    fn leave(self: &Rc<Self>, from: &gtk4::Button) {
        let me = self.clone();
        dialogs::ask_password(&self.ctx, from, "confirm.chainleave", move |_pw| {
            me.leave_danger()
        });
    }

    fn leave_danger(&self) {
        let ctx = &self.ctx;
        let (c1, c2) = (ctx.clone(), ctx.clone());
        dialogs::danger(
            ctx,
            Danger {
                step: None,
                title: ctx.t("danger.chain.title"),
                lead: ctx.t("danger.chain.lead"),
                lose: vec![ctx.t("danger.chain.lose1"), ctx.t("danger.chain.lose2")],
                keep: vec![ctx.t("danger.chain.keep1"), ctx.t("danger.chain.keep2")],
                warn: None,
                confirm: Confirm::Slide(ctx.t("danger.chain.slide")),
                working: None,
                on_cancel: Rc::new(move || c1.back_to("chain")),
                on_confirm: Rc::new(move |h| {
                    let c = c2.clone();
                    c2.post("live/chain/leave", json!({}), move |r| match r {
                        Ok(_) => {
                            c.refresh();
                            let c3 = c.clone();
                            dialogs::done_page(
                                &c,
                                &c.t("danger.chain.ok"),
                                None,
                                None,
                                move || c3.back_to("chain"),
                            );
                        }
                        Err(e) => h.fail(&e),
                    });
                }),
                extra: None,
            },
        );
    }

    fn open_join(&self) {
        for e in &self.entries {
            e.set_text("");
            e.remove_css_class("error");
        }
        self.join_note.set("");
        self.join_go.set_sensitive(true);
        self.ctx.stack.set_visible_child_name("chain-join");
        self.entries[0].grab_focus();
    }

    /// Several words in one box (pasted, or typed with spaces) spread over
    /// the boxes; a whole old 24 word code joins at once, a recovery code
    /// (15 words) is turned down.
    fn spread_pastes(self: &Rc<Self>) {
        for (i, e) in self.entries.iter().enumerate() {
            let u = Rc::downgrade(self);
            e.connect_changed(move |e| {
                e.remove_css_class("error");
                let Some(u) = u.upgrade() else { return };
                let text = e.text();
                let parts: Vec<Zeroizing<String>> = text
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|p| !p.is_empty())
                    .map(|p| Zeroizing::new(p.to_lowercase()))
                    .collect();
                if parts.len() < 2 {
                    return;
                }
                if parts.len() == 15 {
                    e.set_text("");
                    u.join_note
                        .say("bad", &u.ctx.err_text(&local("chain.recovery_code")));
                    return;
                }
                if parts.len() == 24 {
                    e.set_text("");
                    u.join(Some(parts));
                    return;
                }
                let n = parts.len();
                let (all, e) = (u.entries.clone(), e.clone());
                gtk4::glib::idle_add_local_once(move || {
                    for (k, w) in parts.iter().enumerate().take(WORDS - i) {
                        if let Some(t) = all.get(i + k) {
                            t.set_text(w);
                        }
                    }
                    if parts.is_empty() {
                        e.set_text("");
                    }
                    if let Some(t) = all.get((i + n).min(WORDS - 1)) {
                        t.grab_focus();
                        t.set_position(-1);
                    }
                });
            });
            if let Some(next) = self.entries.get(i + 1).cloned() {
                e.connect_activate(move |_| {
                    next.grab_focus();
                });
            } else {
                let u = Rc::downgrade(self);
                e.connect_activate(move |_| {
                    if let Some(u) = u.upgrade() {
                        u.join(None);
                    }
                });
            }
        }
    }

    /// Joins with the typed words (or a pasted old code).
    fn join(self: &Rc<Self>, legacy: Option<Vec<Zeroizing<String>>>) {
        let words: Vec<Zeroizing<String>> = legacy.unwrap_or_else(|| {
            self.entries
                .iter()
                .map(|e| Zeroizing::new(e.text().trim().to_lowercase()))
                .collect()
        });
        self.join_go.set_sensitive(false);
        self.join_note.say("", &self.ctx.t("chain.join.waiting"));
        let list: Vec<Zeroizing<String>> = words;
        let me = self.clone();
        self.ctx.run(
            move |c| {
                let w: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
                c.post_slow("live/chain/join", &json!({ "words": w }))
            },
            move |r| {
                me.join_go.set_sensitive(true);
                match r {
                    Ok(_) => {
                        for e in &me.entries {
                            e.set_text("");
                        }
                        me.join_note.say("ok", &me.ctx.t("chain.joined"));
                        me.ctx.refresh();
                        let c = me.ctx.clone();
                        gtk4::glib::timeout_add_local_once(
                            std::time::Duration::from_millis(1200),
                            move || c.back_to("chain"),
                        );
                    }
                    Err(e) => {
                        let text = match e.index {
                            Some(n) => me
                                .ctx
                                .fill("err.chain.word_unknown_n", &[("n", &n.to_string())]),
                            None => me.ctx.err_text(&e),
                        };
                        me.join_note.say("bad", &text);
                        if let Some(w) = e.index.and_then(|n| me.entries.get(n.wrapping_sub(1))) {
                            w.add_css_class("error");
                            w.grab_focus();
                        }
                    }
                }
            },
        );
    }
}

fn local(key: &str) -> ApiError {
    crate::account::local_error(key)
}
