// SPDX-License-Identifier: AGPL-3.0-or-later
//! Settings > Disk (0.2.7): where the archive is kept, how full it is, how
//! healthy the disk is, and "encrypt later" while the archive is not
//! encrypted. Without an archive the row offers "Set up".

use std::cell::RefCell;
use std::rc::Rc;

use alp_screen::model::home::{room_photos, size};
use alp_screen::set::{Account, ArchiveDisk};
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::Value;

use crate::account::AccountUi;
use crate::dialogs::Ctx;
use crate::style::{self, Tone};

pub struct DiskUi {
    ctx: Ctx,
    pub page: adw::PreferencesPage,
    place: adw::ActionRow,
    /// "Set up" while there is no archive (wired in main.rs).
    pub resume: gtk4::Button,
    bar: gtk4::ProgressBar,
    used: gtk4::Label,
    space_group: adw::PreferencesGroup,
    enc_group: adw::PreferencesGroup,
    last: RefCell<Option<(ArchiveDisk, Account)>>,
}

impl DiskUi {
    pub fn new(ctx: &Ctx, account: Rc<AccountUi>) -> Rc<Self> {
        let page = adw::PreferencesPage::new();
        page.set_vexpand(true);

        let place = crate::icon_row(&ctx.t("set.disk"), "drive-harddisk-symbolic", Tone::Ok);
        let resume = style::button("go-next-symbolic", &ctx.t("home.rem.do"), &[]);
        resume.set_valign(gtk4::Align::Center);
        style::name(&resume, &ctx.t("rem.where"));
        place.add_suffix(&resume);
        let top = adw::PreferencesGroup::new();
        top.add(&place);
        page.add(&top);

        // The space bar and its words, under the disk's row.
        let bar = gtk4::ProgressBar::new();
        bar.set_accessible_role(gtk4::AccessibleRole::Img);
        let used = style::text("", &["dim-label"], false);
        let col = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(16)
            .margin_end(16)
            .build();
        col.append(&bar);
        col.append(&used);
        let space_group = adw::PreferencesGroup::new();
        space_group.add(&col);
        page.add(&space_group);

        let enc_row = crate::icon_row(&ctx.t("enc.row"), "system-lock-screen-symbolic", Tone::Teal);
        enc_row.set_subtitle(&ctx.t("set.enc.d"));
        let enc_btn = style::button("go-next-symbolic", &ctx.t("enc.row.go"), &[]);
        enc_btn.set_valign(gtk4::Align::Center);
        enc_row.add_suffix(&enc_btn);
        let enc_group = adw::PreferencesGroup::new();
        enc_group.add(&enc_row);
        page.add(&enc_group);
        enc_btn.connect_clicked(move |_| account.start_encrypt());

        Rc::new(DiskUi {
            ctx: ctx.clone(),
            page,
            place,
            resume,
            bar,
            used,
            space_group,
            enc_group,
            last: RefCell::new(None),
        })
    }

    pub fn update(&self, w: &Value) {
        let d = ArchiveDisk::from_state(w);
        let a = Account::from_state(w);
        if self.last.borrow().as_ref() == Some(&(d.clone(), a.clone())) {
            return;
        }
        let t = &self.ctx;
        if d.state != "open" {
            self.place.set_title(&t.t("set.disk"));
            self.place
                .set_subtitle(&t.t(&format!("about.arch.{}", d.state)));
            self.resume.set_visible(d.can_resume());
            self.space_group.set_visible(false);
            self.enc_group.set_visible(false);
        } else {
            let place = d.place.clone().unwrap_or_else(|| t.t("set.disk"));
            self.place.set_title(&glib_escape(&place));
            self.place.set_subtitle(&t.fill(
                "set.disk.d",
                &[
                    ("free", &size(d.free)),
                    ("size", &size(d.size)),
                    ("health", &t.t(&format!("health.{}", d.health))),
                ],
            ));
            self.resume.set_visible(false);
            let pct = d.percent();
            let line = t.fill(
                "set.disk.used",
                &[("used", &size(d.used())), ("pct", &pct.to_string())],
            );
            self.bar.set_fraction(f64::from(pct.min(100)) / 100.0);
            self.bar
                .update_property(&[gtk4::accessible::Property::Label(&line)]);
            self.used.set_label(&format!(
                "{line} · {}",
                t.fill("where.room", &[("photos", &room_photos(d.free))])
            ));
            self.space_group.set_visible(true);
            self.enc_group
                .set_visible(matches!(a.protection.as_deref(), Some("none" | "console")));
        }
        *self.last.borrow_mut() = Some((d, a));
    }
}

fn glib_escape(s: &str) -> String {
    gtk4::glib::markup_escape_text(s).to_string()
}
