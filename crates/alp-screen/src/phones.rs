// SPDX-License-Identifier: AGPL-3.0-or-later
//! Settings > Phones (0.2.7): the phones signed in (with when they last
//! backed up) and the phones paired with the app, each with Disconnect
//! (asks for the password), "Only paired phones" (turning it off: password,
//! warning, slide) and Add phone.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use alp_screen::set::{self, Phones};
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};

use crate::dialogs::{self, Confirm, Ctx, Danger};
use crate::style::{self, Tone};

pub struct PhonesUi {
    ctx: Ctx,
    pub page: adw::PreferencesPage,
    list: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk4::Widget>>,
    empty: adw::ActionRow,
    strict: adw::SwitchRow,
    /// Opens the pairing pages (wired in main.rs).
    pub connect_phone: gtk4::Button,
    last: RefCell<Option<(Phones, Option<String>)>>,
    /// The switch is being set from the state, not by the person.
    quiet: Cell<bool>,
}

fn device_icon(kind: &str) -> &'static str {
    match kind {
        "laptop" => "computer-symbolic",
        _ => "phone-symbolic",
    }
}

impl PhonesUi {
    pub fn new(ctx: &Ctx) -> Rc<Self> {
        let page = adw::PreferencesPage::new();
        page.set_vexpand(true);
        let list = adw::PreferencesGroup::new();
        let empty = crate::icon_row(&ctx.t("dev.none"), "phone-symbolic", Tone::Violet);
        list.add(&empty);
        page.add(&list);

        let strict = adw::SwitchRow::builder()
            .title(ctx.t("pair.strict"))
            .subtitle(ctx.t("set.strict.d"))
            .build();
        strict.add_prefix(&style::chip("security-high-symbolic", Tone::Ok, 20));
        let connect_phone = style::button("list-add-symbolic", &ctx.t("set.open"), &[]);
        connect_phone.set_valign(gtk4::Align::Center);
        style::name(&connect_phone, &ctx.t("set.addphone"));
        let add = crate::icon_row(&ctx.t("set.addphone"), "list-add-symbolic", Tone::Violet);
        add.set_subtitle(&ctx.t("set.addphone.d"));
        add.add_suffix(&connect_phone);
        let more = adw::PreferencesGroup::new();
        more.add(&strict);
        more.add(&add);
        page.add(&more);

        let ui = Rc::new(PhonesUi {
            ctx: ctx.clone(),
            page,
            list,
            rows: RefCell::new(Vec::new()),
            empty,
            strict,
            connect_phone,
            last: RefCell::new(None),
            quiet: Cell::new(false),
        });
        let u = ui.clone();
        ui.strict.connect_active_notify(move |row| {
            if u.quiet.get() {
                return;
            }
            u.strict_changed(row.is_active());
        });
        ui
    }

    /// Reads the state (rebuilds the list only when it changed).
    pub fn update(self: &Rc<Self>, w: &Value) {
        let p = Phones::from_state(w);
        let zone = dialogs::zone_of(w);
        if self.last.borrow().as_ref() == Some(&(p.clone(), zone.clone())) {
            return;
        }
        for r in self.rows.borrow_mut().drain(..) {
            self.list.remove(&r);
        }
        self.empty
            .set_visible(p.devices.is_empty() && p.paired.is_empty());
        let t = &self.ctx;
        let unknown = t.t("dev.unknown");
        for d in &p.devices {
            let when = d
                .last_seen
                .as_deref()
                .and_then(|s| dialogs::when(s, zone.as_deref()))
                .unwrap_or_else(|| t.t("dev.never"));
            let r = crate::icon_row(
                &glib::markup_escape_text(&set::device_line(&d.name, &d.os, &unknown)),
                device_icon(&d.kind),
                Tone::Violet,
            );
            r.set_subtitle(&glib::markup_escape_text(
                &t.fill("set.dev.row", &[("when", &when)]),
            ));
            let id = d.id.clone();
            self.disconnect_button(&r, move |c, then| {
                c.post("live/phone/devices/remove", json!({ "id": id }), then)
            });
            self.list.add(&r);
            self.rows.borrow_mut().push(r.upcast());
        }
        for d in &p.paired {
            let line = if d.model.is_empty() {
                d.name.clone()
            } else {
                format!("{} · {}", d.name, d.model)
            };
            let r = crate::icon_row(
                &glib::markup_escape_text(&set::device_line(&line, "", &unknown)),
                "phone-symbolic",
                Tone::Violet,
            );
            r.set_subtitle(&glib::markup_escape_text(&t.t("pair.dev.paired")));
            let serial = d.serial.clone();
            self.disconnect_button(&r, move |c, then| {
                c.post("live/pair/revoke", json!({ "serial": serial }), then)
            });
            self.list.add(&r);
            self.rows.borrow_mut().push(r.upcast());
        }
        self.quiet.set(true);
        self.strict.set_active(p.strict);
        self.quiet.set(false);
        *self.last.borrow_mut() = Some((p, zone));
    }

    /// A Disconnect button on `row`: password first, then the call.
    fn disconnect_button(
        &self,
        row: &adw::ActionRow,
        call: impl Fn(&Ctx, Box<dyn FnOnce(Result<Value, alp_screen::api::ApiError>)>) + 'static,
    ) {
        let b = style::button(
            "user-trash-symbolic",
            &self.ctx.t("set.dev.disconnect"),
            &[],
        );
        b.set_valign(gtk4::Align::Center);
        style::name(
            &b,
            &format!("{}: {}", self.ctx.t("set.dev.disconnect"), row.title()),
        );
        let ctx = self.ctx.clone();
        let call = Rc::new(call);
        b.connect_clicked(move |b| {
            let (ctx2, call) = (ctx.clone(), call.clone());
            dialogs::ask_password(&ctx, b, "confirm.devremove", move |_pw| {
                let c = ctx2.clone();
                call(
                    &ctx2,
                    Box::new(move |r| match r {
                        Ok(_) => {
                            c.note.set("");
                            c.refresh();
                        }
                        Err(e) => c.fail(&e),
                    }),
                );
            });
        });
        row.add_suffix(&b);
    }

    fn strict_changed(self: &Rc<Self>, on: bool) {
        let ctx = self.ctx.clone();
        if on {
            let me = self.clone();
            ctx.post("live/pair/strict", json!({"on": true}), move |r| {
                if let Err(e) = r {
                    me.reset_switch(false);
                    me.ctx.fail(&e);
                } else {
                    me.ctx.note.set("");
                    me.ctx.refresh();
                }
            });
            return;
        }
        // Turning it off weakens protection: password, warning, slide.
        self.reset_switch(true);
        let (me, ctx2) = (self.clone(), ctx.clone());
        dialogs::ask_password(&ctx, &self.strict, "confirm.strictoff", move |_pw| {
            me.strict_off_danger(&ctx2);
        });
    }

    fn reset_switch(&self, on: bool) {
        self.quiet.set(true);
        self.strict.set_active(on);
        self.quiet.set(false);
    }

    fn strict_off_danger(&self, ctx: &Ctx) {
        let (c1, c2) = (ctx.clone(), ctx.clone());
        dialogs::danger(
            ctx,
            Danger {
                step: None,
                title: ctx.t("danger.strict.title"),
                lead: ctx.t("danger.strict.lead"),
                lose: vec![ctx.t("danger.strict.lose")],
                keep: vec![ctx.t("danger.strict.keep")],
                warn: None,
                confirm: Confirm::Slide(ctx.t("danger.strict.slide")),
                working: None,
                on_cancel: Rc::new(move || c1.back_to("phones")),
                on_confirm: Rc::new(move |h| {
                    let c = c2.clone();
                    c2.post("live/pair/strict", json!({"on": false}), move |r| match r {
                        Ok(_) => {
                            c.refresh();
                            let c3 = c.clone();
                            dialogs::done_page(
                                &c,
                                &c.t("danger.strict.ok"),
                                None,
                                None,
                                move || c3.back_to("phones"),
                            );
                        }
                        Err(e) => h.fail(&e),
                    });
                }),
                extra: None,
            },
        );
    }
}
