// SPDX-License-Identifier: AGPL-3.0-or-later
//! Settings > Display (0.2.7): what the lock screen shows, the photo frame,
//! lock after, natural scrolling, text size (five steps), resolution (with
//! the 15 second "Keep this resolution?" question) and the home
//! background.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use alp_screen::set::{self, DisplayPrefs, LocalPrefs, Mode};
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};

use crate::dialogs::Ctx;
use crate::style::{self, Tone};

pub struct DisplayUi {
    ctx: Ctx,
    pub page: adw::PreferencesPage,
    lock_group: adw::PreferencesGroup,
    lock_rows: Vec<(&'static str, adw::SwitchRow)>,
    saver: adw::SwitchRow,
    idle: adw::ComboRow,
    idle_plain: adw::ActionRow,
    natural: adw::SwitchRow,
    size: gtk4::Scale,
    size_now: gtk4::Label,
    res: adw::ComboRow,
    modes: RefCell<Vec<Mode>>,
    modes_asked: Cell<bool>,
    last: RefCell<Option<(DisplayPrefs, LocalPrefs)>>,
    /// A control is being set from the state, not by the person.
    quiet: Cell<bool>,
    commit: RefCell<Option<glib::SourceId>>,
}

thread_local! {
    static SCALE_NOW: Cell<f64> = const { Cell::new(0.0) };
}

/// Sets the text and control size of the whole screen: the chosen step, or
/// alpd's automatic choice for this screen, or 1.
pub fn apply_scale(w: &Value) {
    let p = DisplayPrefs::from_state(w);
    let scale = p.scale.or(p.auto_scale).unwrap_or(1.0);
    if SCALE_NOW.with(|s| (s.get() - scale).abs() < 0.001) {
        return;
    }
    SCALE_NOW.with(|s| s.set(scale));
    if let Some(st) = gtk4::Settings::default() {
        st.set_gtk_xft_dpi((96.0 * scale * 1024.0).round() as i32);
    }
}

fn pct(scale: Option<f64>, auto: Option<f64>) -> u32 {
    (scale.or(auto).unwrap_or(1.0) * 100.0).round() as u32
}

impl DisplayUi {
    pub fn new(ctx: &Ctx) -> Rc<Self> {
        let page = adw::PreferencesPage::new();
        page.set_vexpand(true);

        // What a person who has not signed in may see.
        let lock_group = adw::PreferencesGroup::builder()
            .title(ctx.t("set.lockshow").trim_end_matches('.'))
            .description(ctx.t("set.lockshow.d"))
            .build();
        let mut lock_rows = Vec::new();
        for (k, icon) in [
            ("phones", "phone-symbolic"),
            ("network", "network-wireless-symbolic"),
            ("space", "drive-harddisk-symbolic"),
            ("backup", "emblem-ok-symbolic"),
        ] {
            let r = adw::SwitchRow::builder()
                .title(ctx.t(&format!("set.lockshow.{k}")))
                .subtitle(ctx.t(&format!("set.lockshow.{k}.d")))
                .build();
            r.add_prefix(&style::chip(icon, Tone::Blue, 20));
            lock_group.add(&r);
            lock_rows.push((k, r));
        }
        page.add(&lock_group);

        let g = adw::PreferencesGroup::new();
        let saver = adw::SwitchRow::builder()
            .title(ctx.t("set.saver"))
            .subtitle(ctx.t("set.saver.d"))
            .build();
        saver.add_prefix(&style::chip("image-x-generic-symbolic", Tone::Violet, 20));
        g.add(&saver);

        let mins = gtk4::StringList::new(&[]);
        for v in set::IDLE_CHOICES {
            mins.append(&ctx.fill("set.lockafter.min", &[("n", &(v / 60).to_string())]));
        }
        let idle = adw::ComboRow::builder()
            .title(ctx.t("set.lockafter"))
            .model(&mins)
            .build();
        idle.add_prefix(&style::chip("system-lock-screen-symbolic", Tone::Teal, 20));
        let idle_plain = crate::icon_row(
            &ctx.t("set.lockafter"),
            "system-lock-screen-symbolic",
            Tone::Teal,
        );
        g.add(&idle);
        g.add(&idle_plain);

        let natural = adw::SwitchRow::builder()
            .title(ctx.t("set.natural"))
            .subtitle(ctx.t("set.natural.d"))
            .build();
        natural.add_prefix(&style::chip("input-touchpad-symbolic", Tone::Grey, 20));
        g.add(&natural);

        // Five marked steps; the middle one is automatic.
        let size = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 4.0, 1.0);
        size.set_draw_value(false);
        size.set_round_digits(0);
        size.set_hexpand(true);
        for (i, k) in set::SCALE_NAMES.iter().enumerate() {
            size.add_mark(i as f64, gtk4::PositionType::Bottom, Some(&ctx.t(k)));
        }
        style::name(&size, &ctx.t("set.size"));
        let size_now = style::text("", &["dim-label"], false);
        // The title and its words on top, the slider across the whole row.
        let head = gtk4::Box::builder().spacing(12).build();
        head.append(&style::chip("zoom-in-symbolic", Tone::Blue, 20));
        let words = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .hexpand(true)
            .build();
        words.append(&style::text(&ctx.t("set.size"), &["heading"], false));
        words.append(&style::text(&ctx.t("set.size.d"), &["dim-label"], false));
        head.append(&words);
        head.append(&size_now);
        let size_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(16)
            .margin_end(16)
            .build();
        size_box.append(&head);
        size_box.append(&size);
        let size_row = adw::PreferencesRow::new();
        size_row.set_child(Some(&size_box));
        size_row.set_activatable(false);
        g.add(&size_row);

        let res = adw::ComboRow::builder()
            .title(ctx.t("set.res"))
            .subtitle(ctx.t("disp.res.loading"))
            .model(&gtk4::StringList::new(&[]))
            .visible(false)
            .build();
        res.add_prefix(&style::chip("video-display-symbolic", Tone::Blue, 20));
        g.add(&res);

        let bg = crate::icon_row(
            &ctx.t("set.homebg"),
            "image-x-generic-symbolic",
            Tone::Violet,
        );
        bg.set_subtitle(&ctx.t("set.homebg.d"));
        let pick = gtk4::Box::builder()
            .spacing(6)
            .valign(gtk4::Align::Center)
            .build();
        pick.append(&style::icon("object-select-symbolic", 16));
        pick.append(&gtk4::Label::new(Some(&ctx.t("bg.blue"))));
        bg.add_suffix(&pick);
        g.add(&bg);
        page.add(&g);

        let ui = Rc::new(DisplayUi {
            ctx: ctx.clone(),
            page,
            lock_group,
            lock_rows,
            saver,
            idle,
            idle_plain,
            natural,
            size,
            size_now,
            res,
            modes: RefCell::new(Vec::new()),
            modes_asked: Cell::new(false),
            last: RefCell::new(None),
            quiet: Cell::new(false),
            commit: RefCell::new(None),
        });
        ui.connect();
        ui
    }

    fn connect(self: &Rc<Self>) {
        for (k, row) in &self.lock_rows {
            let (u, k) = (self.clone(), *k);
            row.connect_active_notify(move |r| {
                if u.quiet.get() {
                    return;
                }
                let cur = u.last.borrow().as_ref().map(|l| l.0.lock_hide.clone());
                let next = set::lock_hide_after(&cur.unwrap_or_default(), k, r.is_active());
                let (u2, r2) = (u.clone(), r.clone());
                u.ctx.post(
                    "live/prefs",
                    json!({"lock_hide": next}),
                    move |res| match res {
                        Ok(_) => {
                            u2.ctx.note.set("");
                            u2.ctx.refresh();
                        }
                        Err(e) => {
                            u2.set_quiet(|| r2.set_active(!r2.is_active()));
                            u2.ctx.fail(&e);
                        }
                    },
                );
            });
        }
        let u = self.clone();
        self.saver.connect_active_notify(move |r| {
            if u.quiet.get() {
                return;
            }
            let p = LocalPrefs {
                saver: r.is_active(),
            };
            u.ctx.local.set(p);
            let _ = p.save(&set::prefs_path());
        });
        let u = self.clone();
        self.idle.connect_selected_notify(move |r| {
            if u.quiet.get() {
                return;
            }
            let Some(v) = set::IDLE_CHOICES.get(r.selected() as usize).copied() else {
                return;
            };
            let u2 = u.clone();
            u.ctx.post(
                "live/prefs",
                json!({"idle_lock_seconds": v}),
                move |res| match res {
                    Ok(_) => {
                        u2.ctx.note.set("");
                        u2.ctx.refresh();
                    }
                    Err(e) => u2.ctx.fail(&e),
                },
            );
        });
        let u = self.clone();
        self.natural.connect_active_notify(move |r| {
            if u.quiet.get() {
                return;
            }
            let (u2, r2) = (u.clone(), r.clone());
            u.ctx.post(
                "live/prefs",
                json!({"natural_scrolling": r.is_active()}),
                move |res| match res {
                    Ok(_) => {
                        u2.ctx.note.set("");
                        u2.ctx.refresh();
                    }
                    Err(e) => {
                        u2.set_quiet(|| r2.set_active(!r2.is_active()));
                        u2.ctx.fail(&e);
                    }
                },
            );
        });
        let u = self.clone();
        self.size.connect_value_changed(move |s| {
            let i = s.value().round() as usize;
            u.show_size(i);
            if u.quiet.get() {
                return;
            }
            // Saved a moment after the slider rests.
            if let Some(id) = u.commit.borrow_mut().take() {
                id.remove();
            }
            let u2 = u.clone();
            let id = glib::timeout_add_local_once(Duration::from_millis(500), move || {
                u2.commit.borrow_mut().take();
                let v = set::SCALE_STEPS[i.min(4)];
                let u3 = u2.clone();
                u2.ctx
                    .post("live/prefs", json!({"ui_scale": v}), move |res| match res {
                        Ok(_) => {
                            u3.ctx.note.set("");
                            u3.ctx.refresh();
                        }
                        Err(e) => u3.ctx.fail(&e),
                    });
            });
            *u.commit.borrow_mut() = Some(id);
        });
        let u = self.clone();
        self.res.connect_selected_notify(move |r| {
            if u.quiet.get() {
                return;
            }
            let m = u.modes.borrow().get(r.selected() as usize).cloned();
            if let Some(m) = m
                && !m.current
            {
                u.change_mode(&m);
            }
        });
    }

    fn set_quiet(&self, f: impl FnOnce()) {
        self.quiet.set(true);
        f();
        self.quiet.set(false);
    }

    fn show_size(&self, i: usize) {
        let t = &self.ctx;
        let auto = self.last.borrow().as_ref().and_then(|l| l.0.auto_scale);
        let step = set::SCALE_STEPS[i.min(4)];
        let name = if step.is_some() {
            t.t(set::SCALE_NAMES[i.min(4)])
        } else {
            t.t("disp.s.default.auto")
        };
        self.size_now
            .set_label(&format!("{name} · {}%", pct(step, auto)));
    }

    /// Reads the state.
    pub fn update(self: &Rc<Self>, w: &Value, local: LocalPrefs) {
        apply_scale(w);
        let p = DisplayPrefs::from_state(w);
        if self.last.borrow().as_ref() == Some(&(p.clone(), local)) {
            return;
        }
        *self.last.borrow_mut() = Some((p.clone(), local));
        self.set_quiet(|| {
            self.lock_group.set_visible(p.lock_hide_pref);
            for (k, r) in &self.lock_rows {
                r.set_active(!p.lock_hide.iter().any(|h| h == k));
            }
            self.saver.set_active(local.saver);
            let mins = (p.idle / 60).max(1);
            self.idle.set_visible(p.idle_pref);
            self.idle_plain.set_visible(!p.idle_pref);
            self.idle_plain.set_subtitle(
                &self
                    .ctx
                    .fill("set.lockafter.d", &[("n", &mins.to_string())]),
            );
            self.idle.set_subtitle(
                &self
                    .ctx
                    .fill("set.lockafter.d", &[("n", &mins.to_string())]),
            );
            if let Some(i) = set::IDLE_CHOICES.iter().position(|v| *v == p.idle) {
                self.idle.set_selected(i as u32);
            }
            self.natural.set_visible(p.natural_pref);
            self.natural.set_active(p.natural_scrolling != Some(false));
            let i = set::scale_index(p.scale);
            self.size.set_value(i as f64);
            self.show_size(i);
        });
        if p.modes && !self.modes_asked.get() {
            self.modes_asked.set(true);
            self.load_modes();
        }
        self.res.set_visible(p.modes);
    }

    fn load_modes(self: &Rc<Self>) {
        let u = self.clone();
        self.ctx.run(
            |c| c.get("live/display/modes"),
            move |r| match r {
                Ok(v) => u.show_modes(set::modes(&v)),
                Err(e) => {
                    u.res.set_subtitle(&u.ctx.err_text(&e));
                    u.modes_asked.set(false);
                }
            },
        );
    }

    fn show_modes(&self, modes: Vec<Mode>) {
        let list = gtk4::StringList::new(&[]);
        for m in &modes {
            let hint = if m.native {
                format!(" · {}", self.ctx.t("disp.res.native"))
            } else {
                String::new()
            };
            list.append(&format!("{}{hint}", m.text()));
        }
        self.set_quiet(|| {
            self.res.set_model(Some(&list));
            if let Some(i) = modes.iter().position(|m| m.current) {
                self.res.set_selected(i as u32);
            }
            self.res.set_subtitle(&self.ctx.t("set.res.d"));
        });
        *self.modes.borrow_mut() = modes;
    }

    /// A new resolution goes back by itself after a few seconds unless
    /// kept.
    fn change_mode(self: &Rc<Self>, m: &Mode) {
        let u = self.clone();
        self.ctx
            .post("live/display/mode", json!({"id": m.id}), move |r| match r {
                Err(e) => {
                    u.ctx.fail(&e);
                    u.load_modes();
                }
                Ok(v) => u.keep_question(set::revert_in(&v)),
            });
    }

    fn keep_question(self: &Rc<Self>, secs: u64) {
        let t = &self.ctx;
        let left = Rc::new(Cell::new(secs));
        let d = adw::AlertDialog::new(
            Some(&t.t("disp.keep.title")),
            Some(&t.fill("disp.keep.left", &[("n", &secs.to_string())])),
        );
        d.add_responses(&[("revert", &t.t("disp.revert")), ("keep", &t.t("disp.keep"))]);
        d.set_response_appearance("keep", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("keep"));
        d.set_close_response("revert");
        let answered = Rc::new(Cell::new(false));
        {
            let (u, answered) = (self.clone(), answered.clone());
            d.connect_response(None, move |_, r| {
                if answered.replace(true) {
                    return;
                }
                let path = if r == "keep" {
                    "live/display/mode/keep"
                } else {
                    "live/display/mode/revert"
                };
                let u2 = u.clone();
                u.ctx.post(path, json!({}), move |res| {
                    // display.no_pending: alpd already went back.
                    if let Err(e) = res
                        && e.key != "display.no_pending"
                    {
                        u2.ctx.fail(&e);
                    }
                    u2.load_modes();
                });
            });
        }
        let (d2, ctx) = (d.clone(), self.ctx.clone());
        glib::timeout_add_seconds_local(1, move || {
            if answered.get() {
                return glib::ControlFlow::Break;
            }
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                d2.close();
                return glib::ControlFlow::Break;
            }
            d2.set_body(&ctx.fill("disp.keep.left", &[("n", &left.get().to_string())]));
            glib::ControlFlow::Continue
        });
        d.present(Some(&self.res));
    }
}
