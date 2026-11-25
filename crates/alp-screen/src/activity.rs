// SPDX-License-Identifier: AGPL-3.0-or-later
//! The lock screen's live activity panel (owner, 2026-11-25): network speed
//! and each disk's reading, writing and busy share, with a moving graph of
//! the last minute like a task manager. Counters come from the kernel
//! ([`alp_screen::meter`]); this draws them. An advanced view: a later
//! version may keep only part of it for the simple mode.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use alp_screen::i18n::Texts;
use alp_screen::meter::{self, Sample};
use gtk4::prelude::*;

use crate::style::{self, Tone};

/// Seconds of history in a graph.
const HISTORY: usize = 60;
/// At most this many disks are shown (the rest are left out).
const MAX_DISKS: usize = 4;

/// A small graph: one or two lines with a soft fill, newest on the right.
struct Graph {
    area: gtk4::DrawingArea,
    /// (first line, second line), each 0.0-1.0 of the graph's height.
    points: Rc<RefCell<VecDeque<(f64, f64)>>>,
}

impl Graph {
    fn new(two: bool, rgb: (f64, f64, f64)) -> Self {
        let points: Rc<RefCell<VecDeque<(f64, f64)>>> =
            Rc::new(RefCell::new(VecDeque::with_capacity(HISTORY)));
        let area = gtk4::DrawingArea::builder()
            .content_height(26)
            .hexpand(true)
            .css_classes(["meter-graph"])
            .build();
        area.set_accessible_role(gtk4::AccessibleRole::Presentation);
        let p = points.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (f64::from(w), f64::from(h));
            // Faint grid, as in a task manager.
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.12);
            cr.set_line_width(1.0);
            for i in 1..4 {
                let y = (h * f64::from(i) / 4.0).round() + 0.5;
                cr.move_to(0.0, y);
                cr.line_to(w, y);
            }
            let _ = cr.stroke();
            let pts = p.borrow();
            if pts.len() < 2 {
                return;
            }
            let step = w / (HISTORY - 1) as f64;
            let x0 = w - step * (pts.len() - 1) as f64;
            let line = |pick: &dyn Fn(&(f64, f64)) -> f64, alpha_fill: f64, alpha_line: f64| {
                cr.move_to(x0, h);
                for (i, v) in pts.iter().enumerate() {
                    cr.line_to(
                        x0 + step * i as f64,
                        h - pick(v).clamp(0.0, 1.0) * (h - 2.0),
                    );
                }
                cr.line_to(w, h);
                cr.close_path();
                cr.set_source_rgba(rgb.0, rgb.1, rgb.2, alpha_fill);
                let _ = cr.fill();
                for (i, v) in pts.iter().enumerate() {
                    let (x, y) = (
                        x0 + step * i as f64,
                        h - pick(v).clamp(0.0, 1.0) * (h - 2.0),
                    );
                    if i == 0 {
                        cr.move_to(x, y);
                    } else {
                        cr.line_to(x, y);
                    }
                }
                cr.set_source_rgba(rgb.0, rgb.1, rgb.2, alpha_line);
                cr.set_line_width(2.0);
                let _ = cr.stroke();
            };
            line(&|v| v.0, 0.30, 1.0);
            if two {
                line(&|v| v.1, 0.0, 0.55);
            }
        });
        Graph { area, points }
    }

    fn push(&self, a: f64, b: f64) {
        let mut p = self.points.borrow_mut();
        if p.len() == HISTORY {
            p.pop_front();
        }
        p.push_back((a, b));
        drop(p);
        self.area.queue_draw();
    }
}

/// One line of the panel: icon, name, the numbers, the graph.
struct Line {
    root: gtk4::Box,
    title: gtk4::Label,
    value: gtk4::Label,
    graph: Graph,
}

impl Line {
    fn new(icon: &str, tone: Tone, title: &str, two: bool, rgb: (f64, f64, f64)) -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .build();
        let head = gtk4::Box::builder().spacing(8).build();
        let chip = style::chip(icon, tone, 14);
        chip.add_css_class("chip-small");
        head.append(&chip);
        let title = gtk4::Label::builder()
            .label(title)
            .css_classes(["meter-title"])
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        let value = gtk4::Label::builder()
            .css_classes(["meter-value"])
            .xalign(1.0)
            .build();
        head.append(&title);
        head.append(&value);
        let graph = Graph::new(two, rgb);
        root.append(&head);
        root.append(&graph.area);
        Line {
            root,
            title,
            value,
            graph,
        }
    }
}

pub struct Activity {
    pub root: gtk4::Box,
    texts: Rc<Texts>,
    net: Line,
    disks_box: gtk4::Box,
    disks: RefCell<Vec<(String, Line)>>,
    last: RefCell<Option<(Instant, Sample)>>,
    /// The highest network speed in the history: the graph's top.
    net_peak: RefCell<VecDeque<u64>>,
}

/// The disk's name for people: its model when the kernel knows it.
fn disk_label(texts: &Texts, name: &str) -> String {
    let sys = Path::new("/sys/block").join(name);
    let model = std::fs::read_to_string(sys.join("device/model"))
        .ok()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    let kind = if std::fs::read_to_string(sys.join("removable")).is_ok_and(|r| r.trim() == "1")
        || std::fs::canonicalize(&sys).is_ok_and(|p| p.to_string_lossy().contains("/usb"))
    {
        "USB"
    } else if std::fs::read_to_string(sys.join("queue/rotational")).is_ok_and(|r| r.trim() == "1") {
        "HDD"
    } else {
        "SSD"
    };
    let kind = texts.t(&format!("disk.kind.{}", kind.to_lowercase()));
    match model {
        Some(m) => format!("{kind} · {m}"),
        None => format!("{kind} · {name}"),
    }
}

/// A port with a hardware device (not the loopback or a container link).
fn physical(port: &str) -> bool {
    Path::new("/sys/class/net")
        .join(port)
        .join("device")
        .exists()
}

fn read() -> Sample {
    let (rx, tx) = meter::net(
        &std::fs::read_to_string("/proc/net/dev").unwrap_or_default(),
        physical,
    );
    Sample {
        rx,
        tx,
        disks: meter::disks(&std::fs::read_to_string("/proc/diskstats").unwrap_or_default()),
    }
}

impl Activity {
    pub fn new(texts: Rc<Texts>) -> Rc<Self> {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(10)
            .css_classes(["glass", "activity"])
            .build();
        let head = gtk4::Label::builder()
            .label(texts.t("act.title"))
            .css_classes(["caption-up"])
            .xalign(0.0)
            .build();
        root.append(&head);
        let net = Line::new(
            "network-transmit-receive-symbolic",
            Tone::Blue,
            &texts.t("act.net"),
            true,
            (0.55, 0.85, 1.0),
        );
        root.append(&net.root);
        let disks_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(10)
            .build();
        root.append(&disks_box);
        let a = Rc::new(Activity {
            root,
            texts,
            net,
            disks_box,
            disks: RefCell::new(Vec::new()),
            last: RefCell::new(None),
            net_peak: RefCell::new(VecDeque::new()),
        });
        a.tick();
        let me = a.clone();
        gtk4::glib::timeout_add_seconds_local(1, move || {
            me.tick();
            gtk4::glib::ControlFlow::Continue
        });
        a
    }

    /// Reads the counters and moves the graphs on by one second.
    fn tick(&self) {
        let now = (Instant::now(), read());
        let prev = self.last.replace(Some(now.clone()));
        let Some((t0, s0)) = prev else { return };
        let r = meter::rate(&s0, &now.1, now.0.duration_since(t0).as_secs_f64());
        let t = &self.texts;

        // Network: download and upload, scaled to the minute's top speed.
        let mut peaks = self.net_peak.borrow_mut();
        if peaks.len() == HISTORY {
            peaks.pop_front();
        }
        peaks.push_back(r.rx_bps.max(r.tx_bps));
        let top = peaks.iter().copied().max().unwrap_or(0).max(1_000_000) as f64;
        drop(peaks);
        self.net.value.set_label(&t.fill(
            "act.net.line",
            &[
                ("down", &meter::speed(r.rx_bps)),
                ("up", &meter::speed(r.tx_bps)),
            ],
        ));
        style::name(
            &self.net.root,
            &t.fill(
                "act.net.said",
                &[
                    ("down", &meter::speed(r.rx_bps)),
                    ("up", &meter::speed(r.tx_bps)),
                ],
            ),
        );
        self.net
            .graph
            .push(r.rx_bps as f64 / top, r.tx_bps as f64 / top);

        // Disks: busy share (the graph), writing and reading speeds.
        let mut lines = self.disks.borrow_mut();
        let names: Vec<&str> = r
            .disks
            .iter()
            .take(MAX_DISKS)
            .map(|d| d.name.as_str())
            .collect();
        if lines
            .iter()
            .map(|(n, _)| n.as_str())
            .ne(names.iter().copied())
        {
            for (_, l) in lines.drain(..) {
                self.disks_box.remove(&l.root);
            }
            for n in &names {
                let l = Line::new(
                    "drive-harddisk-symbolic",
                    Tone::Ok,
                    &disk_label(t, n),
                    false,
                    (0.55, 0.95, 0.65),
                );
                self.disks_box.append(&l.root);
                lines.push((n.to_string(), l));
            }
        }
        for (d, (_, l)) in r.disks.iter().zip(lines.iter()) {
            let busy = d.busy.to_string();
            let words = t.fill(
                "act.disk",
                &[
                    ("write", &meter::speed(d.write_bps)),
                    ("read", &meter::speed(d.read_bps)),
                    ("busy", &busy),
                ],
            );
            l.value.set_label(&words);
            style::name(&l.root, &format!("{} · {words}", l.title.label()));
            l.graph.push(f64::from(d.busy) / 100.0, 0.0);
        }
    }
}
