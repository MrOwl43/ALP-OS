// SPDX-License-Identifier: AGPL-3.0-or-later
//! The first setup pages before the install choice, in the 0.2.6 order
//! (owner, 2026-11-26): Welcome (the hardware check, date, time and time
//! zone, the network line), then Network (cable or Wi-Fi). The Network and
//! time-zone pages are also opened from Settings.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, mpsc};

use alp_screen::api::{ApiError, Client};
use alp_screen::i18n::Texts;
use alp_screen::start::{self, Net, WifiNet};
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::Msg;
use crate::setup::{page_at, text};
use crate::style::{self, Tone};

pub const WELCOME: &str = "start-welcome";
pub const NETWORK: &str = "start-network";
pub const ZONE: &str = "start-zone";
const ZONE_TAB: &str = "/usr/share/zoneinfo/zone1970.tab";

pub enum StartMsg {
    Networks(Result<Value, ApiError>),
    Joined(Result<Value, ApiError>),
    ZoneSaved(Result<Value, ApiError>),
}

/// The time in `zone` (UTC when none or unknown) at `secs`.
#[allow(deprecated)] // TimeZone::new: the build machines' GLib is older than 2.68.
fn tz(zone: Option<&str>) -> glib::TimeZone {
    zone.map_or_else(glib::TimeZone::utc, |z| glib::TimeZone::new(Some(z)))
}

fn at(secs: i64, zone: Option<&str>) -> Option<glib::DateTime> {
    glib::DateTime::from_unix_utc(secs)
        .ok()?
        .to_timezone(&tz(zone))
        .ok()
}

pub fn wall(secs: i64, zone: Option<&str>) -> String {
    at(secs, zone)
        .and_then(|d| d.format("%a %e %b, %H:%M").ok())
        .map(|s| s.replace("  ", " "))
        .unwrap_or_default()
}

/// Minutes east of UTC of `zone` at `secs`.
fn offset_min(zone: Option<&str>, secs: i64) -> i64 {
    at(secs, zone).map_or(0, |d| d.utc_offset().as_minutes())
}

/// "UTC+2", "UTC-3:30", "UTC".
fn offset_text(min: i64) -> String {
    if min == 0 {
        return "UTC".into();
    }
    let sign = if min < 0 { '-' } else { '+' };
    let (h, m) = (min.abs() / 60, min.abs() % 60);
    if m == 0 {
        format!("UTC{sign}{h}")
    } else {
        format!("UTC{sign}{h}:{m:02}")
    }
}

fn button(icon: &str, s: &str, main: bool) -> gtk4::Button {
    let b = style::button(
        icon,
        s,
        if main {
            &["suggested-action", "pill"][..]
        } else {
            &["pill"][..]
        },
    );
    b.set_valign(gtk4::Align::Center);
    b
}

fn row(title: &str, icon: &str, tone: Tone) -> adw::ActionRow {
    let r = adw::ActionRow::builder().title(title).build();
    r.add_prefix(&style::chip(icon, tone, 20));
    r
}

fn group_of(rows: &[&gtk4::Widget]) -> gtk4::ListBox {
    let l = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    for r in rows {
        l.append(*r);
    }
    l
}

pub struct StartUi {
    texts: Rc<Texts>,
    client: Arc<Client>,
    tx: mpsc::Sender<Msg>,
    stack: gtk4::Stack,
    /// This start: Welcome pressed, Network left (with or without one).
    welcomed: Cell<bool>,
    net_done: Cell<bool>,
    /// Where Network and Time zone go back to: the setup, or Settings.
    from_settings: Cell<bool>,
    last: RefCell<Value>,
    // Welcome.
    hw_list: gtk4::ListBox,
    hw_rows: RefCell<Vec<adw::ActionRow>>,
    hw_key: RefCell<String>,
    clock_row: adw::ActionRow,
    zone_btn: gtk4::Button,
    net_row: adw::ActionRow,
    net_icon: gtk4::Image,
    net_join: gtk4::Button,
    small: style::Note,
    welcome_go: gtk4::Button,
    // Network.
    net_note: style::Note,
    wifi_box: gtk4::Box,
    wifi_list: gtk4::ListBox,
    wifi_rows: RefCell<Vec<adw::ActionRow>>,
    nets: RefCell<Vec<WifiNet>>,
    wifi_note: style::Note,
    looking: Cell<bool>,
    chosen: RefCell<Option<WifiNet>>,
    pw_box: gtk4::Box,
    pw_label: gtk4::Label,
    pw: gtk4::PasswordEntry,
    join: gtk4::Button,
    later: style::Note,
    net_go: gtk4::Button,
    net_back: gtk4::Button,
    // Time zone.
    zones: Vec<String>,
    zone_rows: RefCell<Vec<(String, gtk4::ListBoxRow, gtk4::Label)>>,
    zone_list: gtk4::ListBox,
    zone_search: gtk4::SearchEntry,
    zone_chosen: RefCell<Option<String>>,
    zone_prev: gtk4::Label,
    zone_prev_d: gtk4::Label,
    base_box: gtk4::Box,
    base_local: gtk4::CheckButton,
    base_utc: gtk4::CheckButton,
    base_unsure: gtk4::CheckButton,
    zone_note: style::Note,
    zone_go: gtk4::Button,
}

impl StartUi {
    pub fn new(
        texts: Rc<Texts>,
        client: Arc<Client>,
        tx: mpsc::Sender<Msg>,
        stack: &gtk4::Stack,
    ) -> Rc<Self> {
        let t = &texts;
        // Welcome.
        let hw_list = group_of(&[]);
        let clock_row = row(
            &t.t("set.clock"),
            "preferences-system-time-symbolic",
            Tone::Teal,
        );
        let zone_btn = button("go-next-symbolic", "UTC", false);
        clock_row.add_suffix(&zone_btn);
        let net_row = adw::ActionRow::builder()
            .title(t.t("hw.item.network"))
            .build();
        let net_icon = style::chip("network-wired-symbolic", Tone::Blue, 20);
        net_row.add_prefix(&net_icon);
        let net_join = button("network-wireless-symbolic", &t.t("hw.net.connect"), false);
        net_join.set_visible(false);
        net_row.add_suffix(&net_join);
        let facts = group_of(&[clock_row.upcast_ref(), net_row.upcast_ref()]);
        let small = style::Note::new("bad", "");
        let welcome_go = button("go-next-symbolic", &t.t("common.continue"), true);
        welcome_go.set_halign(gtk4::Align::End);
        let foot = style::text(&t.t("welcome.footer"), &["dim-label"], true);
        stack.add_named(
            &page_at(
                t,
                1,
                ("computer-symbolic", Tone::Blue),
                &t.t("welcome.title"),
                &t.t("welcome.sub"),
                &[
                    hw_list.upcast_ref(),
                    facts.upcast_ref(),
                    small.root.upcast_ref(),
                    foot.upcast_ref(),
                    welcome_go.upcast_ref(),
                ],
                640,
            ),
            Some(WELCOME),
        );

        // Network.
        let net_note = style::Note::new("", "");
        let wifi_list = group_of(&[]);
        let look = button("view-refresh-symbolic", &t.t("net.wifi.look"), false);
        look.set_halign(gtk4::Align::Start);
        let wifi_note = style::Note::new("", "");
        let pw_label = style::text("", &["heading"], false);
        let pw = gtk4::PasswordEntry::builder()
            .show_peek_icon(true)
            .hexpand(true)
            .build();
        pw.update_property(&[gtk4::accessible::Property::Label(
            &t.t("net.wifi.password.label"),
        )]);
        let join = button("network-wireless-symbolic", &t.t("net.wifi.connect"), true);
        join.set_halign(gtk4::Align::End);
        let pw_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(8)
            .visible(false)
            .build();
        pw_box.append(&pw_label);
        pw_box.append(&pw);
        pw_box.append(&style::text(
            &t.t("net.wifi.remember"),
            &["dim-label"],
            false,
        ));
        pw_box.append(&join);
        let wifi_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(10)
            .build();
        wifi_box.append(&style::text(&t.t("net.wifi.choose"), &["heading"], false));
        wifi_box.append(&wifi_list);
        wifi_box.append(&look);
        wifi_box.append(&pw_box);
        wifi_box.append(&wifi_note.root);
        let later = style::Note::new(
            "",
            &text(
                t,
                "net.later.settings",
                "Without a network your phone cannot find ALPArchive. You can connect later in Settings, Network.",
            ),
        );
        let net_back = button("go-previous-symbolic", &t.t("common.back"), false);
        let net_go = button("go-next-symbolic", &t.t("common.continue"), true);
        let nav = gtk4::Box::builder().spacing(12).build();
        nav.append(&net_back);
        nav.append(&gtk4::Box::builder().hexpand(true).build());
        nav.append(&net_go);
        stack.add_named(
            &page_at(
                t,
                2,
                ("network-wireless-symbolic", Tone::Blue),
                &t.t("net.title"),
                &t.t("net.sub"),
                &[
                    net_note.root.upcast_ref(),
                    wifi_box.upcast_ref(),
                    later.root.upcast_ref(),
                    nav.upcast_ref(),
                ],
                640,
            ),
            Some(NETWORK),
        );

        // Time zone.
        let zones = start::zones(&std::fs::read_to_string(ZONE_TAB).unwrap_or_default());
        let zone_search = gtk4::SearchEntry::builder()
            .placeholder_text(t.t("zone.search"))
            .build();
        zone_search.update_property(&[gtk4::accessible::Property::Label(&t.t("zone.search"))]);
        let zone_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        zone_list.update_property(&[gtk4::accessible::Property::Label(&t.t("set.zone"))]);
        let zone_none = style::text(&t.t("zone.none"), &["dim-label"], false);
        zone_list.set_placeholder(Some(&zone_none));
        let zone_scroll = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .min_content_height(200)
            .max_content_height(320)
            .child(&zone_list)
            .build();
        let zone_prev = style::text("", &["title-3"], false);
        let zone_prev_d = style::text("", &["dim-label"], false);
        let base_q = style::text(&t.t("zone.which"), &["heading"], false);
        let base_local = gtk4::CheckButton::new();
        let base_utc = gtk4::CheckButton::new();
        base_utc.set_group(Some(&base_local));
        let base_unsure = gtk4::CheckButton::with_label(&t.t("zone.neither"));
        base_unsure.set_group(Some(&base_local));
        let base_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .visible(false)
            .build();
        base_box.append(&base_q);
        base_box.append(&base_local);
        base_box.append(&base_utc);
        base_box.append(&base_unsure);
        base_box.append(&style::text(&t.t("zone.neither.d"), &["dim-label"], false));
        let zone_note = style::Note::new("bad", "");
        let zone_cancel = button("go-previous-symbolic", &t.t("common.cancel"), false);
        let zone_go = button("emblem-ok-symbolic", &t.t("zone.confirm"), true);
        let znav = gtk4::Box::builder().spacing(12).build();
        znav.append(&zone_cancel);
        znav.append(&gtk4::Box::builder().hexpand(true).build());
        znav.append(&zone_go);
        let zone_page = style::scroll(&style::panel(
            &[
                style::hero("preferences-system-time-symbolic", Tone::Teal).upcast_ref(),
                style::text(&t.t("zone.title"), &["title-1"], true).upcast_ref(),
                style::text(&t.t("zone.sub"), &["lead"], true).upcast_ref(),
                zone_search.upcast_ref(),
                zone_scroll.upcast_ref(),
                zone_prev.upcast_ref(),
                zone_prev_d.upcast_ref(),
                base_box.upcast_ref(),
                zone_note.root.upcast_ref(),
                znav.upcast_ref(),
            ],
            640,
        ));
        stack.add_named(&zone_page, Some(ZONE));

        let ui = Rc::new(StartUi {
            texts,
            client,
            tx,
            stack: stack.clone(),
            welcomed: Cell::new(false),
            net_done: Cell::new(false),
            from_settings: Cell::new(false),
            last: RefCell::new(Value::Null),
            hw_list,
            hw_rows: RefCell::new(Vec::new()),
            hw_key: RefCell::new(String::new()),
            clock_row,
            zone_btn: zone_btn.clone(),
            net_row,
            net_icon,
            net_join: net_join.clone(),
            small,
            welcome_go: welcome_go.clone(),
            net_note,
            wifi_box,
            wifi_list: wifi_list.clone(),
            wifi_rows: RefCell::new(Vec::new()),
            nets: RefCell::new(Vec::new()),
            wifi_note,
            looking: Cell::new(false),
            chosen: RefCell::new(None),
            pw_box,
            pw_label,
            pw: pw.clone(),
            join: join.clone(),
            later,
            net_go: net_go.clone(),
            net_back: net_back.clone(),
            zones,
            zone_rows: RefCell::new(Vec::new()),
            zone_list: zone_list.clone(),
            zone_search: zone_search.clone(),
            zone_chosen: RefCell::new(None),
            zone_prev,
            zone_prev_d,
            base_box,
            base_local: base_local.clone(),
            base_utc: base_utc.clone(),
            base_unsure: base_unsure.clone(),
            zone_note,
            zone_go: zone_go.clone(),
        });

        {
            let u = ui.clone();
            welcome_go.connect_clicked(move |_| {
                u.welcomed.set(true);
                u.open_network(false);
            });
            let u = ui.clone();
            net_join.connect_clicked(move |_| {
                u.welcomed.set(true);
                u.open_network(false);
            });
            let u = ui.clone();
            zone_btn.connect_clicked(move |_| u.open_zone(false));
            let u = ui.clone();
            look.connect_clicked(move |_| u.look());
            let u = ui.clone();
            join.connect_clicked(move |_| u.join());
            let u = ui.clone();
            pw.connect_activate(move |_| u.join());
            let u = ui.clone();
            net_go.connect_clicked(move |_| {
                if u.from_settings.get() {
                    u.stack.set_visible_child_name("settings");
                } else {
                    u.net_done.set(true);
                    // The next state shows the next step: read it now.
                    let _ = u.tx.send(Msg::Refresh);
                }
            });
            let u = ui.clone();
            net_back.connect_clicked(move |_| {
                if u.from_settings.get() {
                    u.stack.set_visible_child_name("settings");
                } else {
                    u.welcomed.set(false);
                    u.stack.set_visible_child_name(WELCOME);
                }
            });
            let u = ui.clone();
            wifi_list.connect_row_activated(move |_, r| u.choose(r.index()));
            let u = ui.clone();
            zone_search.connect_search_changed(move |_| u.filter_zones());
            let u = ui.clone();
            zone_list.connect_row_selected(move |_, r| {
                if let Some(r) = r {
                    let name = r.widget_name().to_string();
                    *u.zone_chosen.borrow_mut() = (name != "UTC").then_some(name);
                    u.draw_zone_preview();
                }
            });
            for b in [&base_local, &base_utc, &base_unsure] {
                let u = ui.clone();
                b.connect_toggled(move |_| u.draw_zone_preview());
            }
            let u = ui.clone();
            zone_cancel.connect_clicked(move |_| u.leave_zone());
            let u = ui.clone();
            zone_go.connect_clicked(move |_| u.save_zone());
        }
        ui
    }

    /// Setup steps 1 and 2 on the stick or a new installed disk: shows
    /// Welcome or Network while not done; false once both are done.
    pub fn show(&self) -> bool {
        let visible = self.stack.visible_child_name();
        if visible.as_deref() == Some(ZONE) && !self.from_settings.get() {
            return true;
        }
        let want = if !self.welcomed.get() {
            WELCOME
        } else if !self.net_done.get() {
            NETWORK
        } else {
            return false;
        };
        if visible.as_deref() != Some(want) {
            if want == NETWORK {
                self.open_network(false);
            } else {
                self.from_settings.set(false);
                self.stack.set_visible_child_name(want);
            }
        }
        true
    }

    /// Back from the install choice to the Network step.
    pub fn back_to_network(&self) {
        self.net_done.set(false);
        self.open_network(false);
    }

    /// The Network page, from the setup or from Settings.
    pub fn open_network(&self, from_settings: bool) {
        self.from_settings.set(from_settings);
        let t = &self.texts;
        style::set_button_text(
            &self.net_go,
            &t.t(if from_settings {
                "common.done"
            } else {
                "common.continue"
            }),
        );
        self.net_back.set_visible(true);
        self.stack.set_visible_child_name(NETWORK);
        self.draw_network();
        if self.wifi_rows.borrow().is_empty() {
            self.look();
        }
    }

    /// Every state: redraw what is showing.
    pub fn update(&self, w: &Value) {
        *self.last.borrow_mut() = w.clone();
        match self.stack.visible_child_name().as_deref() {
            Some(WELCOME) => self.draw_welcome(),
            Some(NETWORK) => self.draw_network(),
            _ => {}
        }
    }

    fn draw_welcome(&self) {
        let w = self.last.borrow();
        let t = &self.texts;
        let (checks, pending) = start::checks(&w);
        let key = format!("{checks:?}{pending}");
        if *self.hw_key.borrow() != key {
            for r in self.hw_rows.borrow_mut().drain(..) {
                self.hw_list.remove(&r);
            }
            let mut rows = self.hw_rows.borrow_mut();
            if pending {
                let r = row(&t.t("hw.checking"), "view-refresh-symbolic", Tone::Grey);
                self.hw_list.append(&r);
                rows.push(r);
            }
            for c in &checks {
                let k = format!("hw.item.{}", c.id);
                let name = text(t, &k, &c.item);
                let (icon, tone) = match c.status.as_str() {
                    "ok" => ("emblem-ok-symbolic", Tone::Ok),
                    "warn" => ("dialog-warning-symbolic", Tone::Warn),
                    "bad" => ("dialog-error-symbolic", Tone::Bad),
                    _ => ("dialog-information-symbolic", Tone::Blue),
                };
                let r = row(&name, icon, tone);
                let mut sub = c.value.clone();
                if c.status != "ok" {
                    let advice = text(t, &format!("hw.note.{}", c.id), &c.advice);
                    if !advice.is_empty() {
                        if !sub.is_empty() {
                            sub.push_str(" · ");
                        }
                        sub.push_str(&advice);
                    }
                }
                r.set_subtitle(&glib::markup_escape_text(&sub));
                self.hw_list.append(&r);
                rows.push(r);
            }
            self.hw_list.set_visible(!rows.is_empty());
            *self.hw_key.borrow_mut() = key;
        }
        // Too little memory (under 4 GB): nothing goes on.
        let small = checks.iter().any(|c| c.id == "memory" && c.status == "bad");
        self.small.set(&if small {
            format!(
                "{} {}",
                t.t("welcome.toosmall.title"),
                t.t("welcome.toosmall.sub")
            )
        } else {
            String::new()
        });
        self.welcome_go.set_sensitive(!small);
        if let Some(c) = start::clock(&w) {
            let now = wall(c.now as i64, c.zone.as_deref());
            self.clock_row
                .set_subtitle(&glib::markup_escape_text(&if c.synced {
                    now
                } else {
                    format!("{now} · {}", t.t("clock.unsure"))
                }));
            style::set_button_text(&self.zone_btn, &start::zone_name(c.zone.as_deref()));
        }
        let (icon, value, can_join) = match start::net(&w) {
            Net::Cable => ("network-wired-symbolic", t.t("hw.net.cable"), false),
            Net::Wifi(_) => (
                "network-wireless-symbolic",
                format!("{} {}", t.t("hw.net.wifi"), t.t("hw.net.wifi.sub")),
                false,
            ),
            Net::Waiting => ("network-wired-symbolic", t.t("net.waiting_address"), false),
            Net::None { wifi_present } => {
                ("network-offline-symbolic", t.t("hw.net.none"), wifi_present)
            }
        };
        self.net_row.set_subtitle(&glib::markup_escape_text(&value));
        self.net_icon.set_icon_name(Some(icon));
        self.net_join.set_visible(can_join);
    }

    fn draw_network(&self) {
        let w = self.last.borrow();
        let t = &self.texts;
        let net = start::net(&w);
        let (kind, line) = match &net {
            Net::Cable => ("ok", t.t("net.connected.cable")),
            Net::Wifi(name) => ("ok", t.fill("net.connected", &[("name", name)])),
            Net::Waiting => ("warn", t.t("net.waiting_address")),
            Net::None {
                wifi_present: false,
            } => ("warn", t.t("net.nowifi")),
            Net::None { .. } => ("", String::new()),
        };
        self.net_note.say(kind, &line);
        let wifi_present = w["network"]["wifi_present"].as_bool().unwrap_or(false);
        self.wifi_box.set_visible(wifi_present);
        let online = matches!(net, Net::Cable | Net::Wifi(_));
        self.later
            .root
            .set_visible(!online && !self.from_settings.get());
        if let Some((state, msg)) = start::wifi_job(&w) {
            match state.as_str() {
                "running" => {
                    self.wifi_note.say("", &msg);
                    self.join.set_sensitive(false);
                }
                "failed" => {
                    self.wifi_note.say("bad", &msg);
                    self.join.set_sensitive(true);
                }
                _ => {
                    let mut s = t.t("net.wifi.connected");
                    if w["network"]["wpa3_off"].as_bool() == Some(true) {
                        s = format!("{s} {}", t.t("net.wifi.wpa2"));
                    }
                    self.wifi_note.say("ok", &s);
                    self.join.set_sensitive(true);
                    self.pw_box.set_visible(false);
                }
            }
        }
    }

    fn look(&self) {
        if self.looking.replace(true) {
            return;
        }
        self.wifi_note.say("", &self.texts.t("net.wifi.looking"));
        let (c, tx) = (self.client.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Start(StartMsg::Networks(c.get("live/wifi/networks"))));
        });
    }

    fn choose(&self, i: i32) {
        let Some(n) = self.nets.borrow().get(i as usize).cloned() else {
            return;
        };
        let t = &self.texts;
        match n.security.as_str() {
            "8021x" => {
                self.wifi_note.say("bad", &t.t("net.wifi.enterprise"));
                self.pw_box.set_visible(false);
                return;
            }
            "psk" | "open" => {}
            _ => {
                self.wifi_note.say("bad", &t.t("net.wifi.wep"));
                self.pw_box.set_visible(false);
                return;
            }
        }
        self.wifi_note.set("");
        let open = n.security == "open";
        self.pw_label.set_label(&if open {
            format!(
                "{} {}",
                t.fill("net.wifi.open.title", &[("name", &n.ssid)]),
                t.t("net.wifi.open.note")
            )
        } else {
            t.fill("net.wifi.password", &[("name", &n.ssid)])
        });
        self.pw.set_visible(!open);
        self.pw.set_text("");
        self.pw_box.set_visible(true);
        if !open {
            self.pw.grab_focus();
        }
        *self.chosen.borrow_mut() = Some(n);
    }

    fn join(&self) {
        let Some(n) = self.chosen.borrow().clone() else {
            return;
        };
        let pw = Zeroizing::new(self.pw.text().to_string());
        self.pw.set_text("");
        self.join.set_sensitive(false);
        self.wifi_note.say(
            "",
            &self.texts.fill("net.wifi.connecting", &[("name", &n.ssid)]),
        );
        let (c, tx) = (self.client.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let r = c.post(
                "live/wifi/connect",
                &json!({"ssid": n.ssid, "password": pw.as_str()}),
            );
            let _ = tx.send(Msg::Start(StartMsg::Joined(r)));
        });
    }

    fn show_networks(&self, nets: &[WifiNet]) {
        let t = &self.texts;
        for r in self.wifi_rows.borrow_mut().drain(..) {
            self.wifi_list.remove(&r);
        }
        let mut rows = self.wifi_rows.borrow_mut();
        for n in nets {
            let icon = match n.signal {
                4 => "network-wireless-signal-excellent-symbolic",
                3 => "network-wireless-signal-good-symbolic",
                2 => "network-wireless-signal-ok-symbolic",
                _ => "network-wireless-signal-weak-symbolic",
            };
            let r = row(&glib::markup_escape_text(&n.ssid), icon, Tone::Blue);
            let mut sub = t.fill("net.wifi.signal", &[("n", &n.signal.to_string())]);
            if n.security == "open" {
                sub = format!("{} · {sub}", t.t("net.wifi.open"));
            }
            if n.connected {
                sub = format!("{} · {sub}", t.fill("net.connected", &[("name", &n.ssid)]));
            }
            r.set_subtitle(&glib::markup_escape_text(&sub));
            r.set_activatable(true);
            if n.security != "open" {
                r.add_suffix(&style::icon("system-lock-screen-symbolic", 16));
            }
            self.wifi_list.append(&r);
            rows.push(r);
        }
        *self.nets.borrow_mut() = nets.to_vec();
        self.wifi_list.set_visible(!rows.is_empty());
        if rows.is_empty() {
            self.wifi_note.say("warn", &t.t("net.wifi.none"));
        } else {
            self.wifi_note.say("", &t.t("net.wifi.tap"));
        }
    }

    /// The time-zone page (nothing changes before Confirm).
    pub fn open_zone(&self, from_settings: bool) {
        self.from_settings.set(from_settings);
        if self.zone_rows.borrow().is_empty() {
            self.fill_zones();
        }
        let zone = start::clock(&self.last.borrow()).and_then(|c| c.zone);
        *self.zone_chosen.borrow_mut() = zone.clone();
        let want = zone.unwrap_or_else(|| "UTC".into());
        self.zone_search.set_text("");
        self.filter_zones();
        for (z, r, _) in self.zone_rows.borrow().iter() {
            if *z == want {
                self.zone_list.select_row(Some(r));
                r.grab_focus();
            }
        }
        self.zone_note.set("");
        self.zone_go.set_sensitive(true);
        self.draw_zone_preview();
        self.stack.set_visible_child_name(ZONE);
        self.zone_search.grab_focus();
    }

    fn fill_zones(&self) {
        let now = start::clock(&self.last.borrow()).map_or(0, |c| c.now as i64);
        let mut rows = self.zone_rows.borrow_mut();
        for z in std::iter::once("UTC".to_string()).chain(self.zones.iter().cloned()) {
            let zone = (z != "UTC").then_some(z.as_str());
            let off = offset_text(offset_min(zone, now));
            let line = gtk4::Box::builder()
                .spacing(12)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(12)
                .margin_end(12)
                .build();
            line.append(
                &gtk4::Label::builder()
                    .label(start::zone_name(zone))
                    .xalign(0.0)
                    .hexpand(true)
                    .build(),
            );
            let hint = gtk4::Label::builder()
                .label(&off)
                .css_classes(["dim-label"])
                .build();
            line.append(&hint);
            let r = gtk4::ListBoxRow::builder().child(&line).name(&z).build();
            self.zone_list.append(&r);
            rows.push((z, r, hint));
        }
    }

    fn filter_zones(&self) {
        let q = self.zone_search.text().to_string();
        for (z, r, hint) in self.zone_rows.borrow().iter() {
            r.set_visible(start::zone_matches(z, &hint.label(), &q));
        }
    }

    /// The hardware clock read back as it came (before an earlier
    /// "local time" answer moved it).
    fn raw_now(&self) -> Option<(i64, bool, Option<String>)> {
        let c = start::clock(&self.last.borrow())?;
        let now = c.now as i64;
        let raw = if c.base.as_deref() == Some("local") {
            now + offset_min(c.zone.as_deref(), now) * 60
        } else {
            now
        };
        Some((raw, c.synced, c.base))
    }

    /// Ask which time is right: not from the internet yet, and the chosen
    /// zone is not UTC (a computer that also runs Windows keeps local time).
    fn ask_base(&self) -> bool {
        let chosen = self.zone_chosen.borrow().clone();
        self.raw_now()
            .is_some_and(|(raw, synced, _)| !synced && offset_min(chosen.as_deref(), raw) != 0)
    }

    fn draw_zone_preview(&self) {
        let t = &self.texts;
        let chosen = self.zone_chosen.borrow().clone();
        let Some((raw, synced, base)) = self.raw_now() else {
            return;
        };
        self.zone_prev
            .set_label(&start::zone_name(chosen.as_deref()));
        let ask = self.ask_base();
        self.base_box.set_visible(ask);
        if ask {
            self.base_local.set_label(Some(&wall(raw, None)));
            self.base_utc.set_label(Some(&wall(raw, chosen.as_deref())));
            if !self.base_local.is_active()
                && !self.base_utc.is_active()
                && !self.base_unsure.is_active()
            {
                match base.as_deref() {
                    Some("local") => self.base_local.set_active(true),
                    Some("utc") => self.base_utc.set_active(true),
                    _ => {}
                }
            }
            self.zone_prev_d.set_label("");
            self.zone_go.set_sensitive(
                self.base_local.is_active()
                    || self.base_utc.is_active()
                    || self.base_unsure.is_active(),
            );
        } else {
            let now = start::clock(&self.last.borrow())
                .map_or(raw, |c| if c.synced { c.now as i64 } else { raw });
            self.zone_prev_d.set_label(&format!(
                "{} · {}",
                wall(now, chosen.as_deref()),
                t.t(if synced {
                    "zone.prev.synced"
                } else {
                    "zone.prev.unsure"
                })
            ));
            self.zone_go.set_sensitive(true);
        }
    }

    fn save_zone(&self) {
        let chosen = self.zone_chosen.borrow().clone();
        let Some(c) = start::clock(&self.last.borrow()) else {
            return;
        };
        let ask = self.ask_base();
        let local = ask && self.base_local.is_active();
        let set_zone = chosen != c.zone;
        let set_base = !c.synced && (ask || c.base.as_deref() == Some("local"));
        self.zone_go.set_sensitive(false);
        let (cl, tx) = (self.client.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let mut r = Ok(Value::Null);
            if set_zone {
                r = cl.post("live/prefs", &json!({"time_zone": chosen}));
            }
            if r.is_ok() && set_base {
                r = cl.post("live/clock/base", &json!({"local": local}));
            }
            let _ = tx.send(Msg::Start(StartMsg::ZoneSaved(r)));
        });
    }

    fn leave_zone(&self) {
        let back = if self.from_settings.get() {
            "settings"
        } else {
            WELCOME
        };
        self.stack.set_visible_child_name(back);
        if back == WELCOME {
            self.draw_welcome();
        }
    }

    pub fn handle(&self, m: StartMsg) {
        let err = |e: &ApiError| {
            if e.message.is_empty() {
                self.texts.t(&e.key)
            } else {
                e.message.clone()
            }
        };
        match m {
            StartMsg::Networks(r) => {
                self.looking.set(false);
                match r {
                    Ok(v) => self.show_networks(&start::networks(&v)),
                    Err(e) => {
                        self.show_networks(&[]);
                        self.wifi_note.say("bad", &err(&e));
                    }
                }
            }
            StartMsg::Joined(r) => {
                if let Err(e) = r {
                    self.join.set_sensitive(true);
                    self.wifi_note.say("bad", &err(&e));
                }
            }
            StartMsg::ZoneSaved(r) => match r {
                Ok(_) => {
                    let _ = self.tx.send(Msg::Refresh);
                    self.leave_zone();
                }
                Err(e) => {
                    self.zone_go.set_sensitive(true);
                    self.zone_note.say("bad", &err(&e));
                }
            },
        }
    }
}
