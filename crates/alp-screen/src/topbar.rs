// SPDX-License-Identifier: AGPL-3.0-or-later
//! The top bar on every page, as in the 0.2.7 console (T18, T57): on the
//! left the way home and the name with the test-version tag; on the right
//! the network with the address phones use, the backup disk, the battery,
//! the clock, the overall status, settings and power.

use alp_screen::i18n::Texts;
use alp_screen::model::Level;
use alp_screen::model::bar::{Bar, Net};
use gtk4::glib;
use gtk4::prelude::*;

use crate::style::{self, Tone};

pub struct TopBar {
    pub root: gtk4::CenterBox,
    /// Goes home (wired in main.rs); hidden on home and when locked.
    pub home: gtk4::Button,
    pub settings: gtk4::Button,
    pub power: gtk4::Button,
    title: gtk4::Label,
    tag: gtk4::Label,
    net_icon: gtk4::Image,
    ip: gtk4::Label,
    net: gtk4::Box,
    disk: gtk4::Image,
    battery: gtk4::Box,
    battery_icon: gtk4::Image,
    battery_text: gtk4::Label,
    clock: gtk4::Label,
    status: gtk4::Image,
}

fn stat() -> gtk4::Box {
    gtk4::Box::builder()
        .spacing(6)
        .valign(gtk4::Align::Center)
        .css_classes(["stat"])
        .focusable(false)
        .build()
}

impl TopBar {
    pub fn new(texts: &Texts) -> Self {
        let home = style::button(
            "go-home-symbolic",
            &texts.t("tray.home"),
            &["home-btn", "flat"],
        );
        home.set_visible(false);
        let title = gtk4::Label::builder()
            .label(texts.t("app.name"))
            .css_classes(["brand"])
            .build();
        let tag = gtk4::Label::builder()
            .css_classes(["tag"])
            .valign(gtk4::Align::Center)
            .visible(false)
            .build();
        let start = gtk4::Box::builder().spacing(12).build();
        start.append(&home);
        start.append(&title);
        start.append(&tag);

        let net = stat();
        let net_icon = style::icon("network-wired-symbolic", 18);
        let ip = gtk4::Label::builder().css_classes(["ip"]).build();
        net.append(&net_icon);
        net.append(&ip);
        let disk = style::icon("drive-harddisk-symbolic", 18);
        disk.add_css_class("stat");
        disk.set_accessible_role(gtk4::AccessibleRole::Img);
        let battery = stat();
        let battery_icon = style::icon("battery-full-symbolic", 18);
        let battery_text = gtk4::Label::new(None);
        battery.append(&battery_icon);
        battery.append(&battery_text);
        battery.set_visible(false);
        let clock = gtk4::Label::builder().css_classes(["stat"]).build();
        let status = style::icon("emblem-ok-symbolic", 20);
        status.add_css_class("stat");
        status.set_accessible_role(gtk4::AccessibleRole::Img);
        let settings = gtk4::Button::from_icon_name("emblem-system-symbolic");
        settings.add_css_class("flat");
        style::name(&settings, &texts.t("home.settings"));
        settings.set_visible(false);
        let power = gtk4::Button::from_icon_name("system-shutdown-symbolic");
        power.add_css_class("flat");
        style::name(&power, &texts.t("tray.power"));
        power.set_visible(false);
        let end = gtk4::Box::builder().spacing(4).build();
        for w in [
            net.upcast_ref::<gtk4::Widget>(),
            disk.upcast_ref(),
            battery.upcast_ref(),
            clock.upcast_ref(),
            status.upcast_ref(),
            settings.upcast_ref(),
            power.upcast_ref(),
        ] {
            end.append(w);
        }
        let root = gtk4::CenterBox::builder()
            .css_classes(["topbar"])
            .start_widget(&start)
            .end_widget(&end)
            .build();
        TopBar {
            root,
            home,
            settings,
            power,
            title,
            tag,
            net_icon,
            ip,
            net,
            disk,
            battery,
            battery_icon,
            battery_text,
            clock,
            status,
        }
    }

    /// Shows the state. `open`: the console is signed in; `setup`: the
    /// first-start pages are on; `page`: the stack's page.
    pub fn show(&self, t: &Texts, b: &Bar, page: &str, open: bool, setup: bool) {
        // Left: the way home from the pages that have no Back of their own;
        // the page's name.
        let signed_in = open && !setup;
        let everyday = signed_in && matches!(page, "settings" | "gallery");
        self.home.set_visible(everyday);
        let name = match page {
            "settings" => t.t("bar.settings"),
            "gallery" => t.t("bar.gallery"),
            _ => t.t("app.name"),
        };
        self.title.set_label(&name);
        self.tag.set_label(b.version.as_deref().unwrap_or_default());
        self.tag.set_visible(b.version.is_some() && !everyday);

        // Network and the address phones use.
        let (icon, label) = match &b.net {
            Net::Off => ("network-offline-symbolic", t.t("tray.net.off")),
            Net::Waiting => ("network-wired-acquiring-symbolic", t.t("tray.net.waiting")),
            Net::Cable => ("network-wired-symbolic", t.t("tray.net.cable")),
            Net::Wifi(name, signal) => (
                match signal {
                    Some(0) => "network-wireless-signal-none-symbolic",
                    Some(1) => "network-wireless-signal-weak-symbolic",
                    Some(2) => "network-wireless-signal-ok-symbolic",
                    Some(3) => "network-wireless-signal-good-symbolic",
                    _ => "network-wireless-signal-excellent-symbolic",
                },
                t.fill("tray.net.wifi", &[("name", name)]),
            ),
        };
        self.net_icon.set_icon_name(Some(icon));
        let online = !matches!(b.net, Net::Off | Net::Waiting);
        (if online { Tone::Ok } else { Tone::Warn }).apply(&self.net_icon);
        match &b.ip {
            Some(ip) if online => {
                self.ip.set_label(ip);
                style::name(
                    &self.net,
                    &format!("{label} · {}", t.fill("tray.ip", &[("ip", ip)])),
                );
            }
            _ => {
                self.ip.set_label(&if online {
                    String::new()
                } else {
                    t.t("tray.net.none")
                });
                style::name(&self.net, &label);
            }
        }

        // Backup disk.
        match b.disk {
            Some((key, tone)) => {
                let free = alp_screen::model::home::size(b.free_bytes);
                style::name(&self.disk, &t.fill(key, &[("free", &free)]));
                Tone::from(tone).apply(&self.disk);
                self.disk.set_visible(true);
            }
            None => self.disk.set_visible(false),
        }

        // Battery.
        match b.battery {
            Some((p, charging)) => {
                let level = match p {
                    0..=10 => "empty",
                    11..=35 => "low",
                    36..=75 => "good",
                    _ => "full",
                };
                let name = if charging {
                    format!("battery-{level}-charging-symbolic")
                } else {
                    format!("battery-{level}-symbolic")
                };
                self.battery_icon.set_icon_name(Some(&name));
                let tone = match (charging, p) {
                    (false, 0..=10) => Some(Tone::Bad),
                    (false, 11..=20) => Some(Tone::Warn),
                    _ => None,
                };
                for x in Tone::ALL {
                    self.battery.remove_css_class(x.class());
                }
                if let Some(x) = tone {
                    self.battery.add_css_class(x.class());
                }
                self.battery_text.set_label(&format!("{p}%"));
                let key = if charging {
                    "tray.battery.charging"
                } else {
                    "tray.battery"
                };
                style::name(&self.battery, &t.fill(key, &[("n", &p.to_string())]));
                self.battery.set_visible(true);
            }
            None => self.battery.set_visible(false),
        }

        // Clock (the computer's local time).
        if let Ok(now) = glib::DateTime::now_local()
            && let Ok(s) = now.format("%H:%M")
        {
            self.clock.set_label(&s);
        }

        // Overall status: icon, colour and words together.
        let (icon, tone) = match b.level {
            Level::Ok | Level::Note => ("emblem-ok-symbolic", Tone::Ok),
            Level::Action => ("dialog-warning-symbolic", Tone::Warn),
            Level::Now | Level::Critical => ("dialog-error-symbolic", Tone::Bad),
        };
        self.status.set_icon_name(Some(icon));
        tone.apply(&self.status);
        let level = match b.level {
            Level::Ok => "ok",
            Level::Note => "note",
            Level::Action => "action",
            Level::Now => "now",
            Level::Critical => "critical",
        };
        style::name(
            &self.status,
            &format!("{}: {}", t.t("tray.errors"), t.t(&format!("lv.{level}"))),
        );

        self.settings
            .set_visible(signed_in && matches!(page, "home" | "gallery"));
        // Not while disks are being written (install, archive being made).
        self.power
            .set_visible(open && !matches!(page, "install-progress" | "setup-pool"));
    }
}
