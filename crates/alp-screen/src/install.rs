// SPDX-License-Identifier: AGPL-3.0-or-later
//! Install pages (0.2.8, ADR 0028 decision 2): choose where the system
//! lives and which disks hold the archive, confirm what is erased, watch
//! the progress. alpd runs the installer; this only shows its answers.
//!
//! - Every connected disk is listed. One that cannot be used is greyed out
//!   with the reason (an operating system, the start stick, too small...).
//! - The confirm page names every disk that will be erased and what is on
//!   it now. When a disk holds files, a strong warning and a slide to the
//!   end are needed before Install (owner decision 2026-11-24).
//! - Install sends the exact list of disks shown; the installer refuses if
//!   it differs from its own plan or if any disk changed meanwhile.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;

use alp_screen::api::{ApiError, Client};
use alp_screen::i18n::Texts;
use alp_screen::model::install::{self, Disk, Job, Plan};
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};

use crate::Msg;

pub enum InstallMsg {
    Disks(Result<Value, ApiError>),
    Plan(Result<Value, ApiError>),
    Started(Result<Value, ApiError>),
}

/// The user's choice, as alpd's `install/plan` takes it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Choice {
    /// `None`: keep running from the start stick.
    system: Option<String>,
    data: Vec<String>,
}

impl Choice {
    fn json(&self) -> Value {
        match &self.system {
            Some(s) => json!({"system": s, "data": self.data}),
            None => json!({"system": null, "keep_running": true, "data": self.data}),
        }
    }
}

pub struct InstallUi {
    texts: Rc<Texts>,
    client: Arc<Client>,
    tx: mpsc::Sender<Msg>,
    stack: gtk4::Stack,
    // Choose page.
    system_group: adw::PreferencesGroup,
    data_group: adw::PreferencesGroup,
    rows: RefCell<Vec<adw::ActionRow>>,
    /// Archive disk rows by name: the system disk cannot also be one.
    data_checks: RefCell<Vec<(String, adw::ActionRow, gtk4::CheckButton)>>,
    choice: Rc<RefCell<Choice>>,
    next: gtk4::Button,
    choose_note: gtk4::Label,
    // Confirm page.
    erase_group: adw::PreferencesGroup,
    erase_rows: RefCell<Vec<adw::ActionRow>>,
    summary: gtk4::Label,
    warning: gtk4::Label,
    slide: gtk4::Scale,
    slide_hint: gtk4::Label,
    go: gtk4::Button,
    confirm_note: gtk4::Label,
    shown: RefCell<Vec<String>>,
    // Progress page.
    bar: gtk4::ProgressBar,
    step: gtk4::Label,
    done: gtk4::Button,
}

fn text(t: &Texts, key: &str, fallback: &str) -> String {
    let s = t.t(key);
    if s == key { fallback.to_string() } else { s }
}

fn page_with(page: &adw::PreferencesPage, buttons: &[&gtk4::Widget]) -> gtk4::Box {
    let b = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    page.set_vexpand(true);
    b.append(page);
    let bar = gtk4::Box::builder()
        .spacing(12)
        .halign(gtk4::Align::End)
        .margin_top(12)
        .margin_bottom(24)
        .margin_end(24)
        .build();
    for w in buttons {
        bar.append(*w);
    }
    b.append(&bar);
    b
}

impl InstallUi {
    /// Builds the three pages into `stack` (`install`, `install-confirm`,
    /// `install-progress`); `home` is where Back and Cancel go.
    pub fn new(
        texts: Rc<Texts>,
        client: Arc<Client>,
        tx: mpsc::Sender<Msg>,
        stack: &gtk4::Stack,
    ) -> Rc<Self> {
        let t = &texts;
        // Choose.
        let choose = adw::PreferencesPage::new();
        let system_group = adw::PreferencesGroup::builder()
            .title(text(t, "install.system.title", "Where should ALP-OS live?"))
            .description(text(
                t,
                "install.system.sub",
                "The disk you choose here is erased. Greyed-out disks are never touched.",
            ))
            .build();
        let data_group = adw::PreferencesGroup::builder()
            .title(text(t, "install.data.title", "Disks for the archive"))
            .description(text(
                t,
                "install.data.sub",
                "Optional. With two or more, every photo is kept on two disks.",
            ))
            .build();
        choose.add(&system_group);
        choose.add(&data_group);
        let choose_note = gtk4::Label::builder()
            .css_classes(["error"])
            .wrap(true)
            .build();
        let cancel = gtk4::Button::with_label(&text(t, "common.cancel", "Cancel"));
        let next = gtk4::Button::builder()
            .label(text(t, "common.next", "Next"))
            .css_classes(["suggested-action"])
            .sensitive(false)
            .build();
        let choose_box = page_with(
            &choose,
            &[
                choose_note.upcast_ref(),
                cancel.upcast_ref(),
                next.upcast_ref(),
            ],
        );

        // Confirm.
        let confirm = adw::PreferencesPage::new();
        let erase_group = adw::PreferencesGroup::builder()
            .title(text(t, "install.erase.title", "These disks will be erased"))
            .build();
        confirm.add(&erase_group);
        let summary = gtk4::Label::builder().wrap(true).xalign(0.0).build();
        let warning = gtk4::Label::builder()
            .css_classes(["error", "title-4"])
            .wrap(true)
            .xalign(0.0)
            .label(text(
                t,
                "install.erase.files",
                "Files on these disks will be deleted for good. Copy anything you need first.",
            ))
            .build();
        let slide = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 100.0, 1.0);
        slide.set_draw_value(false);
        let slide_hint = gtk4::Label::builder()
            .label(text(
                t,
                "install.slide",
                "Slide all the way to the right to erase them",
            ))
            .css_classes(["dim-label"])
            .xalign(0.0)
            .build();
        let extra = adw::PreferencesGroup::new();
        let col = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(12)
            .build();
        col.append(&summary);
        col.append(&warning);
        col.append(&slide);
        col.append(&slide_hint);
        extra.add(&col);
        confirm.add(&extra);
        let confirm_note = gtk4::Label::builder()
            .css_classes(["error"])
            .wrap(true)
            .build();
        let back = gtk4::Button::with_label(&text(t, "common.back", "Back"));
        let go = gtk4::Button::builder()
            .label(text(t, "install.go", "Erase and install"))
            .css_classes(["destructive-action"])
            .sensitive(false)
            .build();
        let confirm_box = page_with(
            &confirm,
            &[
                confirm_note.upcast_ref(),
                back.upcast_ref(),
                go.upcast_ref(),
            ],
        );

        // Progress.
        let bar = gtk4::ProgressBar::builder().show_text(true).build();
        let step = gtk4::Label::builder().wrap(true).build();
        let done = gtk4::Button::builder()
            .css_classes(["pill", "suggested-action"])
            .visible(false)
            .halign(gtk4::Align::Center)
            .build();
        let progress = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(18)
            .valign(gtk4::Align::Center)
            .margin_start(96)
            .margin_end(96)
            .build();
        progress.append(
            &gtk4::Label::builder()
                .label(text(t, "install.progress.title", "Installing ALP-OS"))
                .css_classes(["title-1"])
                .build(),
        );
        progress.append(&bar);
        progress.append(&step);
        progress.append(&done);

        stack.add_named(&choose_box, Some("install"));
        stack.add_named(&confirm_box, Some("install-confirm"));
        stack.add_named(&progress, Some("install-progress"));

        let ui = Rc::new(InstallUi {
            texts,
            client,
            tx,
            stack: stack.clone(),
            system_group,
            data_group,
            rows: RefCell::new(Vec::new()),
            data_checks: RefCell::new(Vec::new()),
            choice: Rc::new(RefCell::new(Choice::default())),
            next: next.clone(),
            choose_note,
            erase_group,
            erase_rows: RefCell::new(Vec::new()),
            summary,
            warning,
            slide: slide.clone(),
            slide_hint,
            go: go.clone(),
            confirm_note,
            shown: RefCell::new(Vec::new()),
            bar,
            step,
            done: done.clone(),
        });

        {
            let st = stack.clone();
            cancel.connect_clicked(move |_| st.set_visible_child_name("home"));
            let st = stack.clone();
            back.connect_clicked(move |_| st.set_visible_child_name("install"));
        }
        {
            let u = ui.clone();
            next.connect_clicked(move |b| {
                b.set_sensitive(false);
                u.choose_note.set_label("");
                let body = u.choice.borrow().json();
                u.call(move |c| InstallMsg::Plan(c.post("live/install/plan", &body)));
            });
        }
        {
            // The slide only counts at the very end; letting go earlier
            // springs back.
            let u = ui.clone();
            slide.connect_value_changed(move |s| u.go.set_sensitive(s.value() >= 100.0));
        }
        {
            let u = ui.clone();
            go.connect_clicked(move |b| {
                b.set_sensitive(false);
                u.confirm_note.set_label("");
                let mut body = u.choice.borrow().json();
                body["confirm_erase"] = json!(*u.shown.borrow());
                u.call(move |c| InstallMsg::Started(c.post("live/install/start", &body)));
            });
        }
        {
            let u = ui.clone();
            done.connect_clicked(move |_| {
                // Finished: restart into the installed system. Failed: back
                // to a fresh disk list.
                if u.done.has_css_class("suggested-action") {
                    let c = u.client.clone();
                    std::thread::spawn(move || {
                        let _ = c.post("live/power/off", &json!({"restart": true}));
                    });
                } else {
                    u.open();
                }
            });
        }
        ui
    }

    fn call(&self, f: impl FnOnce(&Client) -> InstallMsg + Send + 'static) {
        let (c, tx) = (self.client.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Install(f(&c)));
        });
    }

    /// Opens the disk list (fresh from alpd).
    pub fn open(self: &Rc<Self>) {
        self.choose_note.set_label("");
        self.next.set_sensitive(false);
        self.stack.set_visible_child_name("install");
        self.call(|c| InstallMsg::Disks(c.get("live/install/disks")));
    }

    pub fn handle(self: &Rc<Self>, m: InstallMsg) {
        let err = |e: ApiError| {
            if e.message.is_empty() {
                self.texts.t(&e.key)
            } else {
                e.message
            }
        };
        match m {
            InstallMsg::Disks(Ok(w)) => self.show_disks(&install::disks(&w)),
            InstallMsg::Disks(Err(e)) => self.choose_note.set_label(&err(e)),
            InstallMsg::Plan(Ok(w)) => self.show_plan(&install::plan(&w)),
            InstallMsg::Plan(Err(e)) => {
                self.next.set_sensitive(true);
                self.choose_note.set_label(&err(e));
            }
            InstallMsg::Started(Ok(_)) => {
                self.bar.set_fraction(0.0);
                self.done.set_visible(false);
                self.stack.set_visible_child_name("install-progress");
            }
            InstallMsg::Started(Err(e)) => {
                self.go.set_sensitive(true);
                self.confirm_note.set_label(&err(e));
            }
        }
    }

    fn show_disks(self: &Rc<Self>, disks: &[Disk]) {
        for r in self.rows.borrow_mut().drain(..) {
            if let Some(g) = r
                .parent()
                .and_then(|p| p.ancestor(adw::PreferencesGroup::static_type()))
            {
                g.downcast::<adw::PreferencesGroup>().unwrap().remove(&r);
            }
        }
        *self.choice.borrow_mut() = Choice::default();
        self.data_checks.borrow_mut().clear();
        let t = &self.texts;
        let mut rows = self.rows.borrow_mut();
        let mut first: Option<gtk4::CheckButton> = None;
        let subtitle = |d: &Disk| {
            let mut s = d.size.clone();
            if !d.contents.is_empty() {
                s += &format!(
                    " · {}: {}",
                    text(t, "install.has", "Has"),
                    d.contents.join(", ")
                );
            }
            s
        };
        let title = |d: &Disk| {
            let kind = if d.usb { " (USB)" } else { "" };
            format!(
                "{}{kind}",
                if d.model.is_empty() {
                    &d.name
                } else {
                    &d.model
                }
            )
        };

        // Where the system lives: one choice among the disks that can hold
        // it, or keep running from the stick.
        let radio = |row: &adw::ActionRow, group: &Option<gtk4::CheckButton>| {
            let c = gtk4::CheckButton::new();
            if let Some(g) = group {
                c.set_group(Some(g));
            }
            row.add_prefix(&c);
            row.set_activatable_widget(Some(&c));
            c
        };
        for d in disks {
            let row = adw::ActionRow::builder()
                .title(title(d))
                .subtitle(match &d.reason {
                    Some((key, en)) => text(t, &format!("install.reason.{key}"), en),
                    None => subtitle(d),
                })
                .build();
            if d.can_be_system {
                let c = radio(&row, &first);
                first.get_or_insert(c.clone());
                let (u, name) = (self.clone(), d.name.clone());
                c.connect_toggled(move |c| {
                    if c.is_active() {
                        u.pick_system(Some(name.clone()));
                    }
                });
            } else {
                row.set_sensitive(false);
            }
            self.system_group.add(&row);
            rows.push(row);
        }
        let keep = adw::ActionRow::builder()
            .title(text(t, "install.keep", "Keep running from this USB stick"))
            .subtitle(text(
                t,
                "install.keep.sub",
                "Nothing is installed on this computer. The stick must stay plugged in.",
            ))
            .build();
        let c = radio(&keep, &first);
        let u = self.clone();
        c.connect_toggled(move |c| {
            if c.is_active() {
                u.pick_system(None);
            }
        });
        self.system_group.add(&keep);
        rows.push(keep);

        // Archive disks: any number of the usable ones.
        for d in disks.iter().filter(|d| d.can_be_data) {
            let row = adw::ActionRow::builder()
                .title(title(d))
                .subtitle(subtitle(d))
                .build();
            let c = gtk4::CheckButton::new();
            row.add_prefix(&c);
            row.set_activatable_widget(Some(&c));
            let (u, name) = (self.clone(), d.name.clone());
            c.connect_toggled(move |c| {
                let mut ch = u.choice.borrow_mut();
                ch.data.retain(|n| *n != name);
                if c.is_active() {
                    ch.data.push(name.clone());
                }
            });
            self.data_group.add(&row);
            self.data_checks
                .borrow_mut()
                .push((d.name.clone(), row.clone(), c));
            rows.push(row);
        }
    }

    fn pick_system(&self, system: Option<String>) {
        // The system disk already holds the archive when no other disk is
        // chosen; it cannot also be a separate archive disk.
        for (name, row, check) in self.data_checks.borrow().iter() {
            let is_system = system.as_deref() == Some(name.as_str());
            if is_system {
                check.set_active(false);
            }
            row.set_sensitive(!is_system);
        }
        self.choice.borrow_mut().system = system;
        self.next.set_sensitive(true);
    }

    fn show_plan(&self, p: &Plan) {
        let t = &self.texts;
        for r in self.erase_rows.borrow_mut().drain(..) {
            self.erase_group.remove(&r);
        }
        let mut rows = self.erase_rows.borrow_mut();
        for (name, model, size, contents) in &p.erases {
            let what = if contents.is_empty() {
                text(t, "install.blank", "Blank")
            } else {
                contents.join(", ")
            };
            let row = adw::ActionRow::builder()
                .title(format!(
                    "{} · {size}",
                    if model.is_empty() { name } else { model }
                ))
                .subtitle(what)
                .build();
            if !contents.is_empty() {
                row.add_css_class("error");
            }
            self.erase_group.add(&row);
            rows.push(row);
        }
        *self.shown.borrow_mut() = p.erases.iter().map(|e| e.0.clone()).collect();
        let protection = match p.protection.as_str() {
            "disk_failure" => text(
                t,
                "install.prot.two",
                "Every photo is kept on two disks: one disk can fail.",
            ),
            "bit_rot_only" => text(
                t,
                "install.prot.one",
                "One disk: damaged files are repaired, but a broken disk loses the archive. Keep a backup.",
            ),
            _ => text(
                t,
                "install.prot.none",
                "No protection against a broken disk. Keep a backup.",
            ),
        };
        self.summary.set_label(&format!(
            "{}: {}\n{protection}",
            text(t, "install.space", "Space for photos"),
            p.usable
        ));
        // Files on a disk: the strong warning and the slide. Blank disks:
        // the button alone.
        let strong = p.strong_warning;
        self.warning.set_visible(strong);
        self.slide.set_visible(strong);
        self.slide_hint.set_visible(strong);
        self.slide.set_value(0.0);
        self.go.set_sensitive(!strong);
        self.confirm_note.set_label("");
        self.next.set_sensitive(true);
        self.stack.set_visible_child_name("install-confirm");
    }

    /// Progress from `state.install` (polled with the rest of the state).
    pub fn show_job(&self, job: &Option<Job>) {
        let Some(j) = job else { return };
        if self.stack.visible_child_name().as_deref() != Some("install-progress") {
            if !j.running {
                return;
            }
            // An install already running (the screen restarted).
            self.stack.set_visible_child_name("install-progress");
        }
        let t = &self.texts;
        self.bar.set_fraction(f64::from(j.percent) / 100.0);
        match j.result.as_deref() {
            None => {
                self.step.set_label(&text(t, &j.key, &j.message));
                self.done.set_visible(false);
            }
            Some("ok") => {
                self.step.set_label(&text(
                    t,
                    "install.done",
                    "ALP-OS is installed. Remove the USB stick, then restart.",
                ));
                self.done
                    .set_label(&text(t, "power.restart.yes", "Restart now"));
                self.done.add_css_class("suggested-action");
                self.done.set_visible(true);
            }
            Some(code) => {
                self.step.set_label(&format!(
                    "{} {} ({code})",
                    text(t, "install.failed", "The install stopped."),
                    j.detail
                ));
                self.done.set_label(&text(t, "common.back", "Back"));
                self.done.remove_css_class("suggested-action");
                self.done.set_visible(true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_to_alpd() {
        let c = Choice {
            system: Some("sda".into()),
            data: vec!["sdb".into()],
        };
        assert_eq!(c.json(), json!({"system": "sda", "data": ["sdb"]}));
        let c = Choice::default();
        assert_eq!(c.json()["keep_running"], true);
    }
}
