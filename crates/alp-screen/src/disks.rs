// SPDX-License-Identifier: AGPL-3.0-or-later
//! Advanced > Disks (0.2.7 `disks`, `formatDisk`, `formatDanger`,
//! `formatProgress`): every disk with its role and lock, the partition bar,
//! health, Eject and Check, and formatting an external disk: name and type,
//! the password, the warning with the slide, then the progress.
//!
//! The server refuses a locked disk again whatever this screen sends; the
//! format call names exactly the partitions that were shown and carries the
//! server's own `confirm` token (a changed disk gives `format.confirm_stale`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use alp_screen::model::home::size;
use alp_screen::set::{self, DiskItem, Estimate};
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::dialogs::{self, Confirm, Ctx, Danger};
use crate::setup::text;
use crate::style::{self, Tone};

/// What is open on the Disks page.
struct View {
    pane: gtk4::Box,
    note: style::Note,
    items: RefCell<Vec<DiskItem>>,
    /// The line under Check (follows the check job in the state).
    check: RefCell<Option<gtk4::Label>>,
}

/// The format progress page.
struct Run {
    disk: DiskItem,
    name: String,
    title: gtk4::Label,
    bar: gtk4::ProgressBar,
    line: gtk4::Label,
    left: gtk4::Label,
    stop: gtk4::Button,
    secure_note: gtk4::Label,
    end: gtk4::Box,
    fin: Cell<bool>,
}

pub struct DisksUi {
    ctx: Ctx,
    view: RefCell<Option<Rc<View>>>,
    run: RefCell<Option<Rc<Run>>>,
    state: RefCell<Value>,
    /// Where Back goes (the Advanced list).
    back: Box<dyn Fn()>,
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn role_color(role: &str) -> (f64, f64, f64) {
    match role {
        "backup" => (0.18, 0.62, 0.45),
        "system" => (0.25, 0.45, 0.85),
        "free" => (0.86, 0.87, 0.90),
        _ => (0.55, 0.56, 0.62),
    }
}

impl DisksUi {
    pub fn new(ctx: &Ctx, back: impl Fn() + 'static) -> Rc<Self> {
        Rc::new(DisksUi {
            ctx: ctx.clone(),
            view: RefCell::new(None),
            run: RefCell::new(None),
            state: RefCell::new(Value::Null),
            back: Box::new(back),
        })
    }

    fn has(&self, f: &str) -> bool {
        set::has(&self.state.borrow(), f)
    }

    /// Follows the state: the check line and the format progress.
    pub fn update(self: &Rc<Self>, w: &Value) {
        *self.state.borrow_mut() = w.clone();
        if let Some(v) = self.view.borrow().as_ref()
            && let Some(l) = v.check.borrow().as_ref()
            && let Some(j) = set::job(w, "disk.check")
            && !j.message.is_empty()
        {
            l.set_label(&j.message);
        }
        let run = self.run.borrow().clone();
        if let Some(r) = run {
            self.follow(&r, w);
        }
    }

    /// Shows the Disks page (`select`: the disk to open first).
    pub fn open(self: &Rc<Self>, select: Option<String>) {
        *self.run.borrow_mut() = None;
        let ctx = &self.ctx;
        let list = gtk4::ListBox::builder()
            .css_classes(["cat-list"])
            .selection_mode(gtk4::SelectionMode::Single)
            .build();
        list.update_property(&[gtk4::accessible::Property::Label(&ctx.t("disks.title"))]);
        let side = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .child(&list)
            .css_classes(["cat-side"])
            .width_request(280)
            .build();
        let pane = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(12)
            .hexpand(true)
            .margin_top(8)
            .margin_start(24)
            .margin_end(24)
            .margin_bottom(16)
            .build();
        let pane_scroll = style::scroll(&pane);
        pane_scroll.set_hexpand(true);
        let note = style::Note::new("bad", "");
        note.root.set_margin_start(24);
        note.root.set_margin_end(24);
        note.root.set_margin_bottom(8);
        let both = gtk4::Box::builder().vexpand(true).build();
        both.append(&side);
        both.append(&pane_scroll);
        let body = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .vexpand(true)
            .build();
        body.append(&note.root);
        body.append(&both);
        pane.append(&style::text(&ctx.t("disks.loading"), &["dim-label"], false));

        let reload = style::button("view-refresh-symbolic", &ctx.t("disks.reload"), &[]);
        reload.set_valign(gtk4::Align::Center);
        let me = self.clone();
        let sel_now: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(select.clone()));
        {
            let (me, sel) = (me.clone(), sel_now.clone());
            reload.connect_clicked(move |_| me.open(sel.borrow().clone()));
        }
        let me2 = self.clone();
        let page = dialogs::sub_window(
            ctx,
            &ctx.t("disks.title"),
            Some(&ctx.t("disks.sub")),
            Some(reload.upcast_ref()),
            move || (me2.back)(),
            &body,
        );
        ctx.show_page("disks", &page);

        let view = Rc::new(View {
            pane,
            note,
            items: RefCell::new(Vec::new()),
            check: RefCell::new(None),
        });
        *self.view.borrow_mut() = Some(view.clone());
        let ids: Rc<RefCell<Vec<(gtk4::ListBoxRow, String)>>> = Rc::default();
        {
            let (me, view, ids, sel) = (self.clone(), view.clone(), ids.clone(), sel_now.clone());
            list.connect_row_selected(move |_, row| {
                let Some(row) = row else { return };
                let id = ids
                    .borrow()
                    .iter()
                    .find(|(r, _)| r == row)
                    .map(|(_, i)| i.clone());
                let d = id.and_then(|id| view.items.borrow().iter().find(|d| d.id == id).cloned());
                if let Some(d) = d {
                    *sel.borrow_mut() = Some(d.id.clone());
                    me.draw_pane(&view, &d);
                }
            });
        }
        let me = self.clone();
        let v = view.clone();
        ctx.run(
            |c| c.get("live/disks"),
            move |r| match r {
                Err(e) => {
                    clear(&v.pane);
                    me.ctx.fail(&e);
                    v.note.say("bad", &me.ctx.err_text(&e));
                }
                Ok(j) => {
                    let all = set::disks(&j);
                    *v.items.borrow_mut() = all.clone();
                    clear(&v.pane);
                    if all.is_empty() {
                        v.pane
                            .append(&style::text(&me.ctx.t("disks.none"), &["dim-label"], false));
                        return;
                    }
                    let mut first: Option<gtk4::ListBoxRow> = None;
                    for d in all.iter().filter(|d| !d.offline()) {
                        let row = me.disk_row(d);
                        list.append(&row);
                        ids.borrow_mut().push((row.clone(), d.id.clone()));
                        let wanted = select.as_deref() == Some(d.id.as_str());
                        if wanted || (first.is_none() && d.lock.is_none()) {
                            first = Some(row.clone());
                        }
                    }
                    for d in all.iter().filter(|d| d.offline()) {
                        list.append(&me.offline_row(d));
                    }
                    let pick = first.or_else(|| ids.borrow().first().map(|(r, _)| r.clone()));
                    match pick {
                        Some(r) => list.select_row(Some(&r)),
                        None => v.pane.append(&style::text(
                            &me.ctx.t("disks.none.open"),
                            &["dim-label"],
                            false,
                        )),
                    }
                }
            },
        );
    }

    fn disk_row(&self, d: &DiskItem) -> gtk4::ListBoxRow {
        let ctx = &self.ctx;
        let line = gtk4::Box::builder()
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        let icon = if d.kind == "stick" {
            "media-removable-symbolic"
        } else {
            "drive-harddisk-symbolic"
        };
        let tone = match d.role.as_str() {
            "backup" => Tone::Ok,
            "system" => Tone::Blue,
            _ => Tone::Grey,
        };
        line.append(&style::chip(icon, tone, 16));
        let names = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .hexpand(true)
            .build();
        names.append(
            &gtk4::Label::builder()
                .label(d.title())
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build(),
        );
        let sub = format!(
            "{} · {}",
            ctx.t(&format!("disks.role.{}", d.role)),
            size(d.size_bytes)
        );
        names.append(&style::text(&sub, &["caption", "dim-label"], false));
        line.append(&names);
        if let Some(level) = d.level.as_deref()
            && level != "ok"
        {
            line.append(&style::chip("dialog-warning-symbolic", Tone::Warn, 14));
        }
        let row = gtk4::ListBoxRow::builder().child(&line).build();
        row.update_property(&[gtk4::accessible::Property::Label(&format!(
            "{}, {sub}",
            d.title()
        ))]);
        row
    }

    /// The computer's own disk: listed, never chosen.
    fn offline_row(&self, d: &DiskItem) -> gtk4::ListBoxRow {
        let ctx = &self.ctx;
        let col = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        let head = gtk4::Box::builder().spacing(12).build();
        head.append(&style::chip("changes-prevent-symbolic", Tone::Grey, 16));
        head.append(
            &gtk4::Label::builder()
                .label(d.title())
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        head.append(&style::text(
            &ctx.t("disks.offline"),
            &["caption", "dim-label"],
            false,
        ));
        col.append(&head);
        col.append(&style::text(
            &ctx.t("disks.internal.line"),
            &["caption", "dim-label"],
            false,
        ));
        let row = gtk4::ListBoxRow::builder()
            .child(&col)
            .activatable(false)
            .selectable(false)
            .build();
        row.update_property(&[gtk4::accessible::Property::Label(&format!(
            "{}, {}",
            d.title(),
            ctx.t("disks.offline")
        ))]);
        row
    }

    fn draw_pane(self: &Rc<Self>, view: &Rc<View>, d: &DiskItem) {
        let ctx = &self.ctx;
        let pane = &view.pane;
        clear(pane);
        *view.check.borrow_mut() = None;
        view.note.set("");
        pane.append(&style::text(&d.title(), &["title-2"], false));
        let mut bits = vec![ctx.t(&format!("disks.kind.{}", d.kind))];
        if d.size_bytes > 0 {
            bits.push(size(d.size_bytes));
        }
        if d.safe_to_remove {
            bits.push(ctx.t("disks.safe"));
        }
        pane.append(&style::text(&bits.join(" · "), &["dim-label"], false));
        pane.append(&part_bar(ctx, d));

        let group = adw::PreferencesGroup::new();
        let fs: Vec<String> = {
            let mut v: Vec<String> = d
                .parts
                .iter()
                .filter(|p| !p.fs.is_empty())
                .map(|p| p.fs.to_uppercase())
                .collect();
            v.dedup();
            v
        };
        let fs_line = if fs.is_empty() {
            ctx.t("where.prepare.nofs")
        } else {
            fs.join(", ")
        };
        group.add(&info_row(
            &ctx.t("disks.fs"),
            &fs_line,
            "text-x-generic-symbolic",
        ));
        if let Some(h) = d.health.as_deref() {
            group.add(&info_row(
                &ctx.t("disks.health"),
                &ctx.t(&format!("disks.health.{h}")),
                "emblem-favorite-symbolic",
            ));
        }
        if let Some(t) = d.temperature_c {
            group.add(&info_row(
                &ctx.t("disks.temp"),
                &ctx.fill("disks.temp.v", &[("n", &format!("{}", t.round() as i64))]),
                "weather-clear-symbolic",
            ));
        }
        pane.append(&group);

        let acts = gtk4::Box::builder().spacing(8).build();
        let res = style::text("", &["dim-label"], false);
        res.set_visible(false);
        if d.can_format() {
            let b = style::button("edit-clear-all-symbolic", &ctx.t("fmt.go"), &[]);
            let (me, d2) = (self.clone(), d.clone());
            b.connect_clicked(move |_| me.format_start(&d2));
            acts.append(&b);
        } else {
            let hint = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(4)
                .build();
            let head = gtk4::Box::builder().spacing(8).build();
            head.append(&style::icon("changes-prevent-symbolic", 16));
            head.append(&style::text(&ctx.t("fmt.go.locked"), &["heading"], false));
            hint.append(&head);
            hint.append(&style::text(
                &ctx.t(d.lock_hint_key()),
                &["caption", "dim-label"],
                false,
            ));
            acts.append(&hint);
        }
        if self.has("disk_eject") && d.can_eject() {
            let b = style::button("media-eject-symbolic", &ctx.t("disks.eject"), &[]);
            let (me, d2, r) = (self.clone(), d.clone(), res.clone());
            b.connect_clicked(move |_| me.eject(&d2, &r));
            acts.append(&b);
        }
        if self.has("disk_check") && d.kind == "external" && d.lock.is_none() {
            let b = style::button("emblem-system-symbolic", &ctx.t("disks.check"), &[]);
            let (me, d2, r, v) = (self.clone(), d.clone(), res.clone(), view.clone());
            b.connect_clicked(move |_| me.check(&v, &d2, &r));
            acts.append(&b);
        }
        pane.append(&acts);
        pane.append(&res);
    }

    fn eject(self: &Rc<Self>, d: &DiskItem, res: &gtk4::Label) {
        res.set_visible(true);
        res.set_label(&self.ctx.t("disks.ejecting"));
        let (me, r, name) = (self.clone(), res.clone(), d.title());
        self.ctx.post(
            "live/disks/eject",
            json!({"disk_id": d.id}),
            move |out| match out {
                Ok(_) => r.set_label(&me.ctx.fill("disks.ejected", &[("name", &name)])),
                Err(e) => {
                    r.set_visible(false);
                    me.ctx.fail(&e);
                }
            },
        );
    }

    fn check(self: &Rc<Self>, view: &Rc<View>, d: &DiskItem, res: &gtk4::Label) {
        res.set_visible(true);
        res.set_label(&self.ctx.t("disks.checking"));
        *view.check.borrow_mut() = Some(res.clone());
        let (me, r) = (self.clone(), res.clone());
        self.ctx.post(
            "live/disks/check",
            json!({"disk_id": d.id}),
            move |out| match out {
                Ok(v) => {
                    let j = &v["job"];
                    if j["state"].as_str() == Some("done")
                        && let Some(m) = j["message"].as_str().filter(|m| !m.is_empty())
                    {
                        r.set_label(m);
                    }
                }
                Err(e) => {
                    r.set_visible(false);
                    me.ctx.fail(&e);
                }
            },
        );
    }

    // ------------------------------------------------------------ format

    /// Step 1: ask alpd what is on the disk, then the name and type page.
    pub fn format_start(self: &Rc<Self>, d: &DiskItem) {
        let (me, d2) = (self.clone(), d.clone());
        let body = json!({"disk_id": d.id, "partuuids": d.part_ids()});
        self.ctx
            .post("live/disks/format/estimate", body, move |r| match r {
                Ok(v) => me.format_form(&d2, set::estimate(&v)),
                Err(e) => {
                    me.ctx.fail(&e);
                    let m = me.clone();
                    let id = d2.id.clone();
                    dialogs::done_page(
                        &me.ctx,
                        &me.ctx.t("fmt.refused"),
                        None,
                        Some(&me.ctx.t("common.back")),
                        move || m.open(Some(id.clone())),
                    );
                }
            });
    }

    fn format_form(self: &Rc<Self>, d: &DiskItem, est: Estimate) {
        let ctx = &self.ctx;
        let name = d.title();
        let total = if est.size_bytes > 0 {
            est.size_bytes
        } else {
            d.size_bytes
        };
        let entry = gtk4::Entry::builder()
            .max_length(11)
            .placeholder_text("USB")
            .text(d.start_label())
            .hexpand(true)
            .build();
        entry.set_direction(gtk4::TextDirection::Ltr);
        style::name(&entry, &ctx.t("fmt.label"));
        let exfat = gtk4::CheckButton::builder()
            .label(ctx.t("fmt.fs.exfat"))
            .active(true)
            .build();
        let ext4 = gtk4::CheckButton::builder()
            .label(ctx.t("fmt.fs.ext4"))
            .build();
        ext4.set_group(Some(&exfat));
        let secure = gtk4::CheckButton::builder()
            .label(ctx.t("fmt.secure"))
            .build();
        let mins = est.secure_minutes().to_string();
        let secure_line = format!(
            "{} {}",
            est.secure_text
                .clone()
                .unwrap_or_else(|| ctx.fill("fmt.secure.d", &[("n", &mins)])),
            ctx.fill(
                if est.secure_about {
                    "fmt.time.about"
                } else {
                    "fmt.time"
                },
                &[("n", &mins)]
            )
        );
        let tech = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        if self.has("disks") {
            tech.append(&ext4);
            tech.append(&style::text(
                &ctx.t("fmt.fs.ext4.d"),
                &["caption", "dim-label"],
                false,
            ));
        }
        tech.append(&secure);
        tech.append(&style::text(&secure_line, &["caption", "dim-label"], false));
        let more = gtk4::Expander::builder()
            .label(text(&ctx.texts, "common.tech", "Technical details"))
            .child(&tech)
            .build();
        let note = style::Note::new("warn", "");
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
        let hero = style::hero("drive-harddisk-symbolic", Tone::Blue);
        let title = style::text(
            &ctx.fill("fmt.title.name", &[("name", &name)]),
            &["title-1"],
            true,
        );
        let sub = style::text(&size(total), &["lead"], true);
        let label_field = style::field(&ctx.t("fmt.label"), &entry);
        let fs_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .build();
        fs_box.append(&exfat);
        fs_box.append(&style::text(
            &ctx.t("fmt.fs.exfat.d"),
            &["caption", "dim-label"],
            false,
        ));
        let page = style::scroll(&style::panel(
            &[
                hero.upcast_ref(),
                title.upcast_ref(),
                sub.upcast_ref(),
                label_field.upcast_ref(),
                fs_box.upcast_ref(),
                more.upcast_ref(),
                note.root.upcast_ref(),
                go.upcast_ref(),
                cancel.upcast_ref(),
            ],
            620,
        ));
        ctx.show_page("format", &page);
        entry.grab_focus();
        {
            let (me, id) = (self.clone(), d.id.clone());
            cancel.connect_clicked(move |_| me.open(Some(id.clone())));
        }
        let (me, d2) = (self.clone(), d.clone());
        let parent = page.clone();
        go.connect_clicked(move |_| {
            let label = entry.text().trim().to_string();
            if !label.is_empty() && !set::label_ok(&label) {
                note.say("warn", &me.ctx.t("fmt.label.bad"));
                return;
            }
            note.set("");
            let mode = if secure.is_active() {
                "secure"
            } else {
                "quick"
            };
            let fs = if ext4.is_active() { "ext4" } else { "exfat" };
            let (me2, d3, est2) = (me.clone(), d2.clone(), est.clone());
            dialogs::ask_password(&me.ctx, &parent, "confirm.format", move |pw| {
                me2.format_danger(&d3, &est2, mode, fs, &label, pw);
            });
        });
    }

    fn format_danger(
        self: &Rc<Self>,
        d: &DiskItem,
        est: &Estimate,
        mode: &'static str,
        fs: &'static str,
        label: &str,
        pw: Zeroizing<String>,
    ) {
        let ctx = &self.ctx;
        let name = d.title();
        let mut lose = vec![match est.files {
            Some(n) => ctx.fill(
                "fmt.lose.counted",
                &[
                    ("name", &name),
                    ("n", &n.to_string()),
                    ("size", &size(est.bytes_used)),
                ],
            ),
            None => ctx.fill("fmt.lose.disk", &[("name", &name)]),
        }];
        if est.photos_not_in_archive > 0 {
            lose.push(ctx.fill(
                "fmt.lose.photos",
                &[("n", &est.photos_not_in_archive.to_string())],
            ));
        }
        for w in &est.warnings {
            let k = format!("fmt.warn.{w}");
            let t = ctx.t(&k);
            lose.push(if t == k { w.clone() } else { t });
        }
        let keep = vec![
            ctx.t("fmt.keep.archive"),
            ctx.t("fmt.keep.phones"),
            ctx.t("fmt.keep.computer"),
        ];
        let lead = if mode == "secure" {
            est.secure_text.clone()
        } else {
            est.quick_text.clone()
        }
        .unwrap_or_default();
        let pw = Rc::new(pw);
        let (me, d1, est1, label1) = (self.clone(), d.clone(), est.clone(), label.to_string());
        let (me_c, id) = (self.clone(), d.id.clone());
        dialogs::danger(
            ctx,
            Danger {
                step: None,
                title: ctx.fill("fmt.danger.title", &[("name", &name)]),
                lead,
                lose,
                keep,
                warn: Some(ctx.t("danger.cannot")),
                confirm: Confirm::Slide(ctx.t("fmt.slide")),
                working: Some(ctx.t("fmt.working")),
                on_cancel: Rc::new(move || me_c.open(Some(id.clone()))),
                on_confirm: Rc::new(move |h| {
                    let body = set::format_body(&d1, mode, &pw, &label1, fs, &est1);
                    let (me, d2, name) = (me.clone(), d1.clone(), label1.clone());
                    me.ctx
                        .clone()
                        .post("live/disks/format", body, move |r| match r {
                            Ok(_) => {
                                let shown = if name.is_empty() { d2.title() } else { name };
                                me.progress(&d2, &shown, mode == "secure");
                            }
                            Err(e) if e.key == "format.confirm_stale" => {
                                let t = text(&me.ctx.texts, "fmt.stale", "The disk changed.");
                                me.ctx.note.say("warn", &t);
                                me.format_start(&d2);
                            }
                            Err(e) => h.fail(&e),
                        });
                }),
                extra: None,
            },
        );
    }

    fn progress(self: &Rc<Self>, d: &DiskItem, name: &str, secure: bool) {
        let ctx = &self.ctx;
        let hero = style::hero("drive-harddisk-symbolic", Tone::Blue);
        let title = style::text(
            &ctx.fill("fmt.run.title", &[("name", name)]),
            &["title-1"],
            true,
        );
        let bar = gtk4::ProgressBar::builder().fraction(0.02).build();
        bar.set_accessible_role(gtk4::AccessibleRole::Img);
        let line = style::text(&ctx.t("fmt.working"), &["lead"], true);
        let left = style::text("", &["dim-label"], true);
        let secure_note = style::text(&ctx.t("fmt.stop.note"), &["caption", "dim-label"], true);
        secure_note.set_visible(secure);
        let stop = style::button("process-stop-symbolic", &ctx.t("fmt.stop"), &["pill"]);
        stop.set_halign(gtk4::Align::Center);
        stop.set_visible(false);
        let end = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(12)
            .build();
        let page = style::scroll(&style::panel(
            &[
                hero.upcast_ref(),
                title.upcast_ref(),
                bar.upcast_ref(),
                line.upcast_ref(),
                left.upcast_ref(),
                secure_note.upcast_ref(),
                stop.upcast_ref(),
                end.upcast_ref(),
            ],
            620,
        ));
        {
            let (me, parent) = (self.clone(), page.clone());
            stop.connect_clicked(move |b| {
                let (me, b) = (me.clone(), b.clone());
                dialogs::ask_password(&me.ctx.clone(), &parent, "fmt.stop.confirm", move |_| {
                    b.set_sensitive(false);
                    let (me, b) = (me.clone(), b.clone());
                    me.ctx
                        .clone()
                        .post("live/archive/cancel", json!({}), move |r| {
                            if let Err(e) = r {
                                b.set_sensitive(true);
                                me.ctx.fail(&e);
                            }
                        });
                });
            });
        }
        ctx.show_page("format", &page);
        *self.run.borrow_mut() = Some(Rc::new(Run {
            disk: d.clone(),
            name: name.to_string(),
            title,
            bar,
            line,
            left,
            stop,
            secure_note,
            end,
            fin: Cell::new(false),
        }));
        ctx.refresh();
    }

    fn follow(self: &Rc<Self>, r: &Rc<Run>, w: &Value) {
        let ctx = &self.ctx;
        let Some(j) = set::job(w, "disk.format") else {
            return;
        };
        if r.fin.get() {
            return;
        }
        let p = j.percent.max(2);
        r.bar.set_fraction(p as f64 / 100.0);
        r.bar
            .update_property(&[gtk4::accessible::Property::Label(&format!("{p} %"))]);
        let said = if j.message.is_empty() {
            ctx.t("fmt.working")
        } else {
            j.message.clone()
        };
        r.line.set_label(&said);
        r.left.set_label(&match (j.state.as_str(), j.seconds_left) {
            ("running", Some(s)) => {
                ctx.fill("fmt.left", &[("n", &s.div_ceil(60).max(1).to_string())])
            }
            _ => String::new(),
        });
        r.stop.set_visible(j.can_stop);
        if !matches!(j.state.as_str(), "done" | "failed" | "cancelled") {
            return;
        }
        r.fin.set(true);
        r.line.set_visible(false);
        r.left.set_visible(false);
        r.stop.set_visible(false);
        r.secure_note.set_visible(false);
        r.bar.set_visible(false);
        let ok = j.state == "done";
        let msg = if ok {
            ctx.fill("fmt.ready", &[("name", &r.name)])
        } else {
            j.error
                .clone()
                .filter(|e| !e.is_empty())
                .unwrap_or(j.message.clone())
        };
        if ok {
            r.title.set_label(&msg);
        }
        if !ok {
            let kind = if j.state == "failed" { "bad" } else { "warn" };
            r.end.append(&style::Note::new(kind, &msg).root);
        }
        let res = style::text("", &["dim-label"], true);
        res.set_visible(false);
        let acts = gtk4::Box::builder()
            .spacing(8)
            .halign(gtk4::Align::Center)
            .build();
        if ok && self.has("disk_eject") && r.disk.can_eject() {
            let b = style::button("media-eject-symbolic", &ctx.t("disks.eject"), &["pill"]);
            let (me, d, rs) = (self.clone(), r.disk.clone(), res.clone());
            b.connect_clicked(move |_| me.eject(&d, &rs));
            acts.append(&b);
        }
        let done = style::button(
            "emblem-ok-symbolic",
            &ctx.t("common.done"),
            &["suggested-action", "pill"],
        );
        let (me, id) = (self.clone(), r.disk.id.clone());
        done.connect_clicked(move |_| me.open(Some(id.clone())));
        acts.append(&done);
        r.end.append(&acts);
        r.end.append(&res);
        done.grab_focus();
    }
}

fn clear(b: &gtk4::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn info_row(title: &str, value: &str, icon: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder()
        .title(esc(title))
        .subtitle(esc(value))
        .build();
    r.add_prefix(&style::chip(icon, Tone::Grey, 16));
    r
}

/// The partition bar: width by size, colour by role; the keys under it.
fn part_bar(ctx: &Ctx, d: &DiskItem) -> gtk4::Box {
    let segs = set::segments(d);
    let total: u64 = segs.iter().map(|s| s.weight).sum::<u64>().max(1);
    let col = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .build();
    let draw: Vec<(&'static str, f64)> = segs
        .iter()
        .map(|s| (s.role, s.weight as f64 / total as f64))
        .collect();
    let area = gtk4::DrawingArea::builder()
        .content_height(18)
        .hexpand(true)
        .build();
    area.set_draw_func(move |a, cr, w, h| {
        let rtl = a.direction() == gtk4::TextDirection::Rtl;
        let (w, h) = (f64::from(w), f64::from(h));
        let mut x = 0.0;
        for (role, share) in &draw {
            let seg = (share * w).max(3.0);
            let (r, g, b) = role_color(role);
            cr.set_source_rgb(r, g, b);
            let left = if rtl { w - x - seg } else { x };
            cr.rectangle(left, 0.0, (seg - 2.0).max(1.0), h);
            let _ = cr.fill();
            x += seg;
        }
    });
    let keys: Vec<String> = segs
        .iter()
        .map(|s| match s.part {
            Some(i) => {
                let p = &d.parts[i];
                let name = if !p.label.is_empty() {
                    p.label.clone()
                } else if !p.name.is_empty() {
                    p.name.clone()
                } else {
                    ctx.t("disks.part")
                };
                let fs = if p.fs.is_empty() {
                    ctx.t("where.prepare.nofs")
                } else {
                    p.fs.to_uppercase()
                };
                let mut v = vec![name, fs];
                if p.size_bytes > 0 {
                    v.push(size(p.size_bytes));
                }
                v.join(" · ")
            }
            None => format!("{} · {}", ctx.t("disks.unused"), size(s.weight)),
        })
        .collect();
    area.set_accessible_role(gtk4::AccessibleRole::Img);
    area.update_property(&[gtk4::accessible::Property::Label(&keys.join(", "))]);
    col.append(&area);
    for (s, k) in segs.iter().zip(&keys) {
        let (r, g, b) = role_color(s.role);
        let hex = format!(
            "#{:02x}{:02x}{:02x}",
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8
        );
        let l = gtk4::Label::builder()
            .use_markup(true)
            .label(format!(
                "<span foreground=\"{hex}\">\u{25A0}</span>  {}",
                esc(k)
            ))
            .xalign(0.0)
            .wrap(true)
            .css_classes(["caption"])
            .build();
        l.update_property(&[gtk4::accessible::Property::Label(k)]);
        col.append(&l);
    }
    col
}
