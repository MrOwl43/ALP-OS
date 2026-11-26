// SPDX-License-Identifier: AGPL-3.0-or-later
//! The messages behind the (i) in the top bar, and the reminders on home
//! (0.2.7 `openErrors`, `needsYou`, `reminders`).
//!
//! - The messages: "Needs you" (one button each), then "Recent" with the
//!   time and a Resolved mark; a row opens its details (when, code, step,
//!   technical text) and, when alpd offers it, "Save report to USB".
//!   The text of a message comes from the error catalog (the state's own
//!   words, or alpd's sentence for a key), never made up here.
//! - Reminders: a step that was skipped in setup stays on home with a
//!   button that takes the person to the right place in Settings.

use std::cell::RefCell;
use std::rc::Rc;

use alp_screen::set::{self, Lv, Msg};
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};

use crate::dialogs::{self, Ctx};
use crate::style::{self, Tone};

pub struct MessagesUi {
    ctx: Ctx,
    state: RefCell<Value>,
    dialog: RefCell<Option<adw::Dialog>>,
    body: gtk4::Box,
    back: gtk4::Button,
    title: adw::WindowTitle,
    detail: RefCell<Option<Msg>>,
    /// The line under "Save report".
    saved: RefCell<Option<gtk4::Label>>,
}

/// Where a reminder or a need takes the person: the Settings category.
pub fn place_of(step: &str) -> &'static str {
    match step {
        "network" => "network",
        "where" | "disk" => "disk",
        "phone" => "phones",
        _ => "password",
    }
}

impl MessagesUi {
    pub fn new(ctx: &Ctx) -> Rc<Self> {
        let body = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(16)
            .margin_top(12)
            .margin_bottom(20)
            .margin_start(20)
            .margin_end(20)
            .build();
        let back = gtk4::Button::from_icon_name("go-previous-symbolic");
        style::name(&back, &ctx.t("common.back"));
        back.set_visible(false);
        let title = adw::WindowTitle::new(&ctx.t("errs.title"), "");
        Rc::new(MessagesUi {
            ctx: ctx.clone(),
            state: RefCell::new(Value::Null),
            dialog: RefCell::new(None),
            body,
            back,
            title,
            detail: RefCell::new(None),
            saved: RefCell::new(None),
        })
    }

    /// Follows the state (for the "Needs you" part and the report button).
    pub fn update(self: &Rc<Self>, w: &Value) {
        *self.state.borrow_mut() = w.clone();
        if self.dialog.borrow().is_some() {
            self.draw();
        }
    }

    /// The list changed (a message came or was resolved).
    pub fn changed(self: &Rc<Self>) {
        if self.dialog.borrow().is_some() {
            self.draw();
        }
    }

    pub fn is_open(&self) -> bool {
        self.dialog.borrow().is_some()
    }

    pub fn close(&self) {
        // Take it out first: closing runs the `closed` handler, which
        // borrows the same cell.
        let open = self.dialog.borrow_mut().take();
        if let Some(d) = open {
            d.close();
        }
        *self.detail.borrow_mut() = None;
    }

    /// Opens the messages (or closes them when open already).
    pub fn toggle(self: &Rc<Self>, parent: &impl IsA<gtk4::Widget>) {
        if self.is_open() {
            self.close();
            return;
        }
        self.ctx.errs.borrow_mut().mark_seen(dialogs::now());
        let _ = self.ctx.tx.send(crate::Msg::ErrorsChanged);
        *self.detail.borrow_mut() = None;
        let header = adw::HeaderBar::builder().title_widget(&self.title).build();
        header.pack_start(&self.back);
        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&style::scroll(&self.body)));
        let d = adw::Dialog::builder()
            .title(self.ctx.t("errs.title"))
            .content_width(480)
            .content_height(620)
            .child(&tv)
            .build();
        let me = self.clone();
        d.connect_closed(move |_| {
            *me.dialog.borrow_mut() = None;
            *me.detail.borrow_mut() = None;
        });
        let me = self.clone();
        self.back.connect_clicked(move |_| {
            *me.detail.borrow_mut() = None;
            me.draw();
        });
        *self.dialog.borrow_mut() = Some(d.clone());
        self.draw();
        d.present(Some(parent));
    }

    fn zone(&self) -> Option<String> {
        dialogs::zone_of(&self.state.borrow())
    }

    fn draw(self: &Rc<Self>) {
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        *self.saved.borrow_mut() = None;
        let detail = self.detail.borrow().clone();
        self.back.set_visible(detail.is_some());
        match detail {
            Some(m) => {
                self.title.set_title(&self.ctx.t("errs.details"));
                self.draw_detail(&m);
            }
            None => {
                self.title.set_title(&self.ctx.t("errs.title"));
                self.draw_list();
            }
        }
    }

    fn draw_list(self: &Rc<Self>) {
        let ctx = &self.ctx;
        let w = self.state.borrow().clone();
        let needs = if w.is_null() {
            Vec::new()
        } else {
            set::needs_you(&w)
        };
        if !needs.is_empty() {
            self.body
                .append(&style::text(&ctx.t("errs.needs"), &["heading"], false));
            for (text, button, action) in needs {
                let row = gtk4::Box::builder()
                    .spacing(12)
                    .css_classes(["need-row"])
                    .build();
                let l = style::text(&ctx.t(text), &[], false);
                l.set_hexpand(true);
                row.append(&l);
                let b = gtk4::Button::builder().label(ctx.t(button)).build();
                let me = self.clone();
                b.connect_clicked(move |_| {
                    me.close();
                    me.ctx.stack.set_visible_child_name("settings");
                    me.ctx.open_cat(place_of(&action));
                });
                row.append(&b);
                self.body.append(&row);
            }
        }
        self.body
            .append(&style::text(&ctx.t("errs.recent"), &["heading"], false));
        let list: Vec<Msg> = ctx.errs.borrow().list.iter().take(60).cloned().collect();
        if list.is_empty() {
            self.body
                .append(&style::text(&ctx.t("errs.none"), &["dim-label"], false));
            return;
        }
        let zone = self.zone();
        let lb = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        lb.update_property(&[gtk4::accessible::Property::Label(&ctx.t("errs.recent"))]);
        for m in &list {
            let line = gtk4::Box::builder()
                .spacing(12)
                .margin_top(10)
                .margin_bottom(10)
                .margin_start(12)
                .margin_end(12)
                .build();
            let dot = gtk4::Box::builder()
                .css_classes([
                    "msg-dot",
                    match m.level {
                        Lv::Bad => "bad",
                        Lv::Warn => "warn",
                        Lv::Info => "info",
                    },
                ])
                .valign(gtk4::Align::Center)
                .build();
            line.append(&dot);
            let mut words = m.text.clone();
            if m.resolved {
                words.push_str(" · ");
                words.push_str(&ctx.t("errs.resolved"));
            }
            let l = gtk4::Label::builder()
                .label(&words)
                .xalign(0.0)
                .hexpand(true)
                .wrap(true)
                .build();
            if m.resolved {
                l.add_css_class("dim-label");
            }
            line.append(&l);
            let time = crate::welcome::at(m.time as i64, zone.as_deref())
                .and_then(|d| d.format("%H:%M").ok())
                .map(|s| s.to_string())
                .unwrap_or_default();
            line.append(
                &gtk4::Label::builder()
                    .label(&time)
                    .css_classes(["caption", "dim-label"])
                    .build(),
            );
            let row = gtk4::ListBoxRow::builder().child(&line).build();
            row.update_property(&[gtk4::accessible::Property::Label(&format!(
                "{words}, {time}"
            ))]);
            lb.append(&row);
        }
        let (me, list2) = (self.clone(), list);
        lb.connect_row_activated(move |_, row| {
            if let Some(m) = list2.get(row.index() as usize) {
                *me.detail.borrow_mut() = Some(m.clone());
                me.draw();
            }
        });
        self.body.append(&lb);
    }

    fn draw_detail(self: &Rc<Self>, m: &Msg) {
        let ctx = &self.ctx;
        self.body.append(&style::text(&m.text, &["title-3"], false));
        let zone = self.zone();
        let group = adw::PreferencesGroup::new();
        let row = |k: &str, v: &str, ltr: bool| {
            let r = adw::ActionRow::builder()
                .title(gtk4::glib::markup_escape_text(&ctx.t(k)))
                .subtitle(gtk4::glib::markup_escape_text(v))
                .subtitle_selectable(true)
                .build();
            if ltr {
                r.set_direction(gtk4::TextDirection::Ltr);
            }
            r
        };
        group.add(&row(
            "errs.when",
            &crate::welcome::wall(m.time as i64, zone.as_deref()),
            false,
        ));
        if let Some(c) = &m.code {
            group.add(&row("errs.code", c, true));
        }
        if let Some(s) = &m.step {
            group.add(&row("errs.step", s, true));
        }
        let tech: Vec<&str> = [m.key.as_deref(), m.detail.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        if !tech.is_empty() {
            group.add(&row("errs.tech", &tech.join(": "), true));
        }
        self.body.append(&group);
        let res = style::text("", &["dim-label"], false);
        res.set_visible(false);
        if set::has(&self.state.borrow(), "events") {
            let b = style::button("document-save-symbolic", &ctx.t("errs.save"), &[]);
            b.set_halign(gtk4::Align::Start);
            let (me, res2) = (self.clone(), res.clone());
            b.connect_clicked(move |_| {
                let (me, res) = (me.clone(), res2.clone());
                me.ctx
                    .clone()
                    .post("live/report/save", json!({}), move |r| {
                        res.set_visible(true);
                        match r {
                            Ok(_) => res.set_label(&me.ctx.t("errs.saved")),
                            Err(e) => {
                                res.set_label(&me.ctx.err_text(&e));
                                me.ctx.fail(&e);
                            }
                        }
                    });
            });
            self.body.append(&b);
        }
        self.body.append(&res);
        *self.saved.borrow_mut() = Some(res);
    }
}

/// Skipped setup steps on home, each with a button ("Set up").
pub struct Reminders {
    pub root: gtk4::Box,
    list: gtk4::ListBox,
    ctx: Ctx,
    shown: RefCell<Vec<&'static str>>,
}

impl Reminders {
    pub fn new(ctx: &Ctx) -> Rc<Self> {
        let list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .width_request(560)
            .visible(false)
            .build();
        root.append(&list);
        Rc::new(Reminders {
            root,
            list,
            ctx: ctx.clone(),
            shown: RefCell::new(Vec::new()),
        })
    }

    pub fn show(self: &Rc<Self>, w: &Value) {
        let steps = set::reminders(w);
        let keys: Vec<&'static str> = steps.iter().map(|(_, k)| *k).collect();
        if *self.shown.borrow() == keys {
            return;
        }
        *self.shown.borrow_mut() = keys;
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        self.root.set_visible(!steps.is_empty());
        self.list
            .update_property(&[gtk4::accessible::Property::Label(&self.ctx.t("errs.needs"))]);
        let ctx = &self.ctx;
        for (step, key) in steps {
            let r = adw::ActionRow::builder()
                .title(gtk4::glib::markup_escape_text(&ctx.t(key)))
                .build();
            let more = ctx.t(&format!("{key}.d"));
            if more != format!("{key}.d") {
                r.set_subtitle(&gtk4::glib::markup_escape_text(&more));
            }
            r.add_prefix(&style::chip("dialog-information-symbolic", Tone::Warn, 20));
            let b = style::button("go-next-symbolic", &ctx.t("home.rem.do"), &[]);
            b.set_valign(gtk4::Align::Center);
            style::name(&b, &format!("{}: {}", ctx.t(key), ctx.t("home.rem.do")));
            let c = ctx.clone();
            b.connect_clicked(move |_| {
                c.stack.set_visible_child_name("settings");
                c.open_cat(place_of(step));
            });
            r.add_suffix(&b);
            self.list.append(&r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::place_of;

    #[test]
    fn every_reminder_leads_somewhere_in_settings() {
        for (step, cat) in [
            ("network", "network"),
            ("where", "disk"),
            ("disk", "disk"),
            ("phone", "phones"),
            ("recovery", "password"),
            ("pwreopen", "password"),
            ("password", "password"),
        ] {
            assert_eq!(place_of(step), cat);
        }
    }
}
