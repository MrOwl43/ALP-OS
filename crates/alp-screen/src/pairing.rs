// SPDX-License-Identifier: AGPL-3.0-or-later
//! Phone pairing on the native screen (ADR 0017), the twin of the web
//! console's: a QR code the ALPArchive app scans (`pair/open`, valid for
//! the window alpd gives), and, when a phone has scanned it, an approval
//! page with the code the phone shows too and a slide to approve. Nothing
//! here decides: alpd checks every request again.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;

use alp_screen::api::{ApiError, Client};
use alp_screen::i18n::Texts;
use alp_screen::model::pairing::{self, Pending};
use gtk4::glib;
use gtk4::prelude::*;
use serde_json::{Value, json};

use crate::Msg;

/// The QR modules shown (side, cells), none while there is no code.
type Cells = Rc<RefCell<Option<(usize, Vec<bool>)>>>;

pub enum PairMsg {
    Opened(Result<Value, ApiError>),
    Answered(Result<Value, ApiError>),
}

pub struct PairUi {
    texts: Rc<Texts>,
    client: Arc<Client>,
    tx: mpsc::Sender<Msg>,
    stack: gtk4::Stack,
    // QR page.
    qr: gtk4::DrawingArea,
    cells: Cells,
    left: gtk4::Label,
    again: gtk4::Button,
    qr_note: gtk4::Label,
    secs: Cell<u64>,
    tick: RefCell<Option<glib::SourceId>>,
    // Approve page.
    title: gtk4::Label,
    code: gtk4::Label,
    slide: gtk4::Scale,
    approve_note: gtk4::Label,
    /// The request shown, and the page to go back to afterwards.
    current: RefCell<Option<Pending>>,
    back_to: RefCell<String>,
    /// Requests the user answered: not shown again while alpd catches up.
    answered: RefCell<Vec<String>>,
}

fn column() -> gtk4::Box {
    gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(18)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::Center)
        .width_request(520)
        .build()
}

fn label(text: &str, classes: &[&str]) -> gtk4::Label {
    gtk4::Label::builder()
        .label(text)
        .wrap(true)
        .justify(gtk4::Justification::Center)
        .css_classes(classes.to_vec())
        .build()
}

impl PairUi {
    pub fn new(
        texts: Rc<Texts>,
        client: Arc<Client>,
        tx: mpsc::Sender<Msg>,
        stack: &gtk4::Stack,
    ) -> Rc<Self> {
        let t = |k: &str| texts.t(k);

        // QR page.
        let cells: Cells = Rc::new(RefCell::new(None));
        let qr = gtk4::DrawingArea::builder()
            .content_width(320)
            .content_height(320)
            .halign(gtk4::Align::Center)
            .build();
        {
            let cells = cells.clone();
            qr.set_draw_func(move |_, cr, w, h| {
                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.paint().ok();
                let Some((n, ref m)) = *cells.borrow() else {
                    return;
                };
                // Four modules of quiet zone around the code.
                let side = f64::from(w.min(h));
                let unit = (side / (n as f64 + 8.0)).floor().max(1.0);
                let off_x = (f64::from(w) - unit * n as f64) / 2.0;
                let off_y = (f64::from(h) - unit * n as f64) / 2.0;
                cr.set_source_rgb(0.0, 0.0, 0.0);
                for (i, dark) in m.iter().enumerate() {
                    if *dark {
                        let (x, y) = ((i % n) as f64, (i / n) as f64);
                        cr.rectangle(off_x + x * unit, off_y + y * unit, unit, unit);
                    }
                }
                cr.fill().ok();
            });
        }
        let left = label("", &["dim-label"]);
        let again = gtk4::Button::builder()
            .label(t("phone.qr.new"))
            .css_classes(["pill"])
            .halign(gtk4::Align::Center)
            .visible(false)
            .build();
        let qr_note = label("", &["error"]);
        let back = gtk4::Button::builder()
            .label(t("common.back"))
            .css_classes(["pill"])
            .halign(gtk4::Align::Center)
            .build();
        let qr_page = column();
        qr_page.append(&label(&t("phone.title"), &["title-1"]));
        qr_page.append(&label(&t("pair.qr.sub"), &[]));
        qr_page.append(&qr);
        qr_page.append(&left);
        qr_page.append(&again);
        qr_page.append(&qr_note);
        qr_page.append(&back);
        stack.add_named(&qr_page, Some("pair-qr"));

        // Approve page.
        let title = label("", &["title-1"]);
        let code = label("", &["title-1", "monospace"]);
        let slide = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 100.0, 1.0);
        slide.set_draw_value(false);
        let refuse = gtk4::Button::builder()
            .label(t("pair.approve.no"))
            .css_classes(["flat"])
            .halign(gtk4::Align::Center)
            .build();
        let approve_note = label("", &["error"]);
        let ap = column();
        ap.append(&title);
        ap.append(&label(&t("pair.approve.sub"), &[]));
        ap.append(&code);
        ap.append(&slide);
        ap.append(&label(&t("pair.approve.slide"), &["dim-label"]));
        ap.append(&approve_note);
        ap.append(&refuse);
        stack.add_named(&ap, Some("pair-approve"));

        let ui = Rc::new(PairUi {
            texts,
            client,
            tx,
            stack: stack.clone(),
            qr,
            cells,
            left,
            again: again.clone(),
            qr_note,
            secs: Cell::new(0),
            tick: RefCell::new(None),
            title,
            code,
            slide: slide.clone(),
            approve_note,
            current: RefCell::new(None),
            back_to: RefCell::new("home".into()),
            answered: RefCell::new(Vec::new()),
        });
        {
            let u = ui.clone();
            back.connect_clicked(move |_| {
                u.close_qr();
                u.stack.set_visible_child_name("settings");
            });
        }
        {
            let u = ui.clone();
            again.connect_clicked(move |_| u.open());
        }
        {
            let u = ui.clone();
            slide.connect_value_changed(move |s| {
                if s.value() >= 100.0 {
                    s.set_sensitive(false);
                    u.answer("live/pair/approve");
                }
            });
        }
        {
            let u = ui.clone();
            refuse.connect_clicked(move |_| u.answer("live/pair/reject"));
        }
        ui
    }

    fn call(&self, f: impl FnOnce(&Client) -> PairMsg + Send + 'static) {
        let (c, tx) = (self.client.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Pair(f(&c)));
        });
    }

    /// Shows a new pairing QR (from Settings).
    pub fn open(self: &Rc<Self>) {
        self.stop_tick();
        *self.cells.borrow_mut() = None;
        self.qr.queue_draw();
        self.left.set_label("");
        self.qr_note.set_label("");
        self.again.set_visible(false);
        self.stack.set_visible_child_name("pair-qr");
        self.call(|c| PairMsg::Opened(c.post("live/pair/open", &json!({}))));
    }

    fn stop_tick(&self) {
        if let Some(id) = self.tick.borrow_mut().take() {
            id.remove();
        }
    }

    /// Leaves the QR page: the pairing window closes at once.
    fn close_qr(&self) {
        self.stop_tick();
        *self.cells.borrow_mut() = None;
        let c = self.client.clone();
        std::thread::spawn(move || {
            let _ = c.post("live/pair/close", &json!({}));
        });
    }

    fn answer(self: &Rc<Self>, path: &'static str) {
        let Some(p) = self.current.borrow().clone() else {
            return;
        };
        self.answered.borrow_mut().push(p.id.clone());
        let id = p.id;
        self.call(move |c| PairMsg::Answered(c.post(path, &json!({ "id": id }))));
    }

    fn err(&self, e: ApiError) -> String {
        if e.message.is_empty() {
            self.texts.t(&e.key)
        } else {
            e.message
        }
    }

    pub fn handle(self: &Rc<Self>, m: PairMsg) {
        match m {
            PairMsg::Opened(Ok(w)) => {
                let uri = w["uri"].as_str().unwrap_or_default();
                *self.cells.borrow_mut() = pairing::qr(uri);
                self.qr.queue_draw();
                self.secs.set(w["expires_in"].as_u64().unwrap_or(300));
                self.show_left();
                let u = self.clone();
                let id = glib::timeout_add_seconds_local(1, move || {
                    let n = u.secs.get().saturating_sub(1);
                    u.secs.set(n);
                    u.show_left();
                    if n == 0 {
                        u.tick.borrow_mut().take();
                        *u.cells.borrow_mut() = None;
                        u.qr.queue_draw();
                        u.left.set_label(&u.texts.t("phone.qr.expired"));
                        u.again.set_visible(true);
                        return glib::ControlFlow::Break;
                    }
                    glib::ControlFlow::Continue
                });
                *self.tick.borrow_mut() = Some(id);
            }
            PairMsg::Opened(Err(e)) => {
                self.qr_note.set_label(&self.err(e));
                self.again.set_visible(true);
            }
            PairMsg::Answered(Ok(_)) => self.leave_approve(),
            PairMsg::Answered(Err(e)) => {
                self.approve_note.set_label(&self.err(e));
                self.slide.set_value(0.0);
                self.slide.set_sensitive(true);
            }
        }
    }

    fn show_left(&self) {
        let (m, s) = pairing::left(self.secs.get());
        self.left
            .set_label(&self.texts.fill("pair.qr.left", &[("m", &m), ("s", &s)]));
    }

    fn leave_approve(&self) {
        *self.current.borrow_mut() = None;
        if self.stack.visible_child_name().as_deref() == Some("pair-approve") {
            let back = self.back_to.borrow().clone();
            self.stack.set_visible_child_name(&back);
        }
    }

    /// On every state read while signed in: a phone waiting for approval
    /// is shown at once, over whatever page is open; when alpd no longer
    /// lists it (approved, refused or timed out), the page goes away.
    pub fn show_pending(self: &Rc<Self>, list: &[Pending]) {
        self.answered
            .borrow_mut()
            .retain(|id| list.iter().any(|p| p.id == *id));
        let next = list
            .iter()
            .find(|p| !self.answered.borrow().contains(&p.id))
            .cloned();
        let shown = self.current.borrow().clone();
        match (shown, next) {
            (Some(a), Some(b)) if a.id == b.id => {}
            (_, Some(p)) => {
                let page = self.stack.visible_child_name().map(|s| s.to_string());
                if page.as_deref() != Some("pair-approve") {
                    let back = match page.as_deref() {
                        // A phone that scanned the QR: afterwards to Settings.
                        Some("pair-qr") | None => "settings".to_string(),
                        Some(p) => p.to_string(),
                    };
                    *self.back_to.borrow_mut() = back;
                }
                if self.stack.visible_child_name().as_deref() == Some("pair-qr") {
                    self.close_qr();
                }
                let name = if p.name.is_empty() {
                    self.texts.t("dev.unknown")
                } else {
                    p.name.clone()
                };
                self.title
                    .set_label(&self.texts.fill("pair.approve.title", &[("name", &name)]));
                self.code.set_label(&p.code);
                self.approve_note.set_label("");
                self.slide.set_value(0.0);
                self.slide.set_sensitive(true);
                *self.current.borrow_mut() = Some(p);
                self.stack.set_visible_child_name("pair-approve");
            }
            (Some(_), None) => self.leave_approve(),
            (None, None) => {}
        }
    }

    /// The screen locked: no approval page and no open window behind it.
    pub fn locked(&self) {
        if self.stack.visible_child_name().as_deref() == Some("pair-qr") {
            self.close_qr();
        }
        *self.current.borrow_mut() = None;
    }
}
