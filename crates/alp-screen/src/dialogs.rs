// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared pieces of the settings flows (the 0.2.7 console's
//! `confirmPassword`, `slideConfirm`, `dangerScreen` and `dangerDone`):
//! the context every page gets, a password question, the slide to confirm,
//! the warning page with "what is removed / what stays" and the "done"
//! page.
//!
//! alpd calls run on a worker thread ([`Ctx::run`]) and the answer comes
//! back on the main loop; nothing here waits for alpd.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;

use alp_screen::api::{ApiError, Client};
use alp_screen::i18n::Texts;
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use alp_screen::set;

use crate::Msg;
use crate::style::{self, Tone};

/// Opens a Settings category by name.
pub type OpenSettings = Rc<RefCell<Option<Box<dyn Fn(&str)>>>>;

/// What every settings page needs.
#[derive(Clone)]
pub struct Ctx {
    pub texts: Rc<Texts>,
    pub client: Arc<Client>,
    pub stack: gtk4::Stack,
    pub tx: mpsc::Sender<Msg>,
    /// Opens a Settings category by name (set by the settings page).
    pub open_settings: OpenSettings,
    /// What this screen remembers by itself (the photo frame switch).
    pub local: Rc<Cell<set::LocalPrefs>>,
    /// The line at the top of Settings for a failed action.
    pub note: Rc<style::Note>,
    /// The messages behind the (i): a failed action is kept there too.
    pub errs: Rc<RefCell<set::ErrLog>>,
}

impl Ctx {
    pub fn t(&self, key: &str) -> String {
        self.texts.t(key)
    }

    pub fn fill(&self, key: &str, v: &[(&str, &str)]) -> String {
        self.texts.fill(key, v)
    }

    /// Plain words for an alpd error: the catalog's text for its key, else
    /// alpd's own sentence, else a general one.
    pub fn err_text(&self, e: &ApiError) -> String {
        let key = format!("err.{}", e.key);
        let known = self.texts.t(&key);
        if known != key {
            known
        } else if !e.message.is_empty() {
            e.message.clone()
        } else {
            let g = self.texts.t(&e.key);
            if g == e.key {
                self.texts.t("err.generic")
            } else {
                g
            }
        }
    }

    /// Runs `work` on a worker thread; `then` gets its answer on the main
    /// loop.
    pub fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Client) -> T + Send + 'static,
        then: impl FnOnce(T) + 'static,
    ) {
        let c = self.client.clone();
        glib::spawn_future_local(async move {
            if let Ok(v) = gio::spawn_blocking(move || work(&c)).await {
                then(v);
            }
        });
    }

    /// A POST on a worker; `then` gets the answer.
    pub fn post(
        &self,
        path: &'static str,
        body: Value,
        then: impl FnOnce(Result<Value, ApiError>) + 'static,
    ) {
        self.run(move |c| c.post(path, &body), then);
    }

    /// A failed action: the line on the page, and a message in the (i) list.
    pub fn fail(&self, e: &ApiError) {
        let text = self.err_text(e);
        self.note.say("bad", &text);
        let mut m = set::Msg::new("", set::Lv::Bad, &text);
        m.key = Some(e.key.clone());
        m.detail = (!e.message.is_empty() && e.message != text).then(|| e.message.clone());
        self.errs.borrow_mut().add(m, now());
        let _ = self.tx.send(Msg::ErrorsChanged);
    }

    /// Reads the state again soon (a setting changed).
    pub fn refresh(&self) {
        let _ = self.tx.send(Msg::Refresh);
    }

    pub fn open_cat(&self, cat: &str) {
        if let Some(f) = self.open_settings.borrow().as_ref() {
            f(cat);
        }
    }

    /// Replaces the page `name` in the stack with `page` and shows it.
    pub fn show_page(&self, name: &str, page: &impl IsA<gtk4::Widget>) {
        if let Some(old) = self.stack.child_by_name(name) {
            self.stack.remove(&old);
        }
        self.stack.add_named(page, Some(name));
        self.stack.set_visible_child_name(name);
    }

    /// Back to Settings, on the category `cat`.
    pub fn back_to(&self, cat: &str) {
        self.stack.set_visible_child_name("settings");
        self.open_cat(cat);
    }
}

/// "Fri 20 Nov, 10:00" for an RFC 3339 time, in the chosen zone.
pub fn when(iso: &str, zone: Option<&str>) -> Option<String> {
    let d = glib::DateTime::from_iso8601(iso, None).ok()?;
    Some(crate::welcome::wall(d.to_unix(), zone))
}

/// The time zone the screen shows times in (`prefs.time_zone`).
pub fn zone_of(w: &Value) -> Option<String> {
    w["prefs"]["time_zone"].as_str().map(str::to_string)
}

/// Seconds since 1970 now.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Asks for the ALP password again (alpd checks it: the same call as
/// sign-in, with the same waiting time after wrong tries). `then` gets the
/// checked password; it is cleared from the entry at once.
pub fn ask_password(
    ctx: &Ctx,
    parent: &impl IsA<gtk4::Widget>,
    reason_key: &str,
    then: impl Fn(Zeroizing<String>) + 'static,
) {
    let entry = gtk4::PasswordEntry::builder()
        .show_peek_icon(true)
        .hexpand(true)
        .activates_default(true)
        .build();
    style::name(&entry, &ctx.t("pw.label"));
    let note = style::Note::new("bad", "");
    let go = style::button(
        "emblem-ok-symbolic",
        &ctx.t("confirm.go"),
        &["suggested-action"],
    );
    let cancel = gtk4::Button::builder()
        .label(ctx.t("common.cancel"))
        .css_classes(["flat"])
        .build();
    let buttons = gtk4::Box::builder()
        .spacing(8)
        .halign(gtk4::Align::End)
        .build();
    buttons.append(&cancel);
    buttons.append(&go);
    let col = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(12)
        .margin_top(20)
        .margin_bottom(20)
        .margin_start(24)
        .margin_end(24)
        .build();
    col.append(&style::text(&ctx.t(reason_key), &["title-3"], false));
    col.append(&style::text(&ctx.t("confirm.sub"), &[], false));
    col.append(&style::field(&ctx.t("pw.label"), &entry));
    col.append(&note.root);
    col.append(&buttons);
    let dialog = adw::Dialog::builder()
        .title(ctx.t(reason_key))
        .content_width(460)
        .child(&col)
        .default_widget(&go)
        .build();
    let note = Rc::new(note);
    let then = Rc::new(then);
    let busy = Rc::new(Cell::new(false));
    let send = {
        let (ctx, dialog, entry, go, note, busy) = (
            ctx.clone(),
            dialog.clone(),
            entry.clone(),
            go.clone(),
            note.clone(),
            busy.clone(),
        );
        move || {
            if busy.get() {
                return;
            }
            let pw = Zeroizing::new(entry.text().to_string());
            entry.set_text("");
            if pw.is_empty() {
                return;
            }
            busy.set(true);
            go.set_sensitive(false);
            note.say("", &ctx.t("pw.working"));
            let (ctx2, dialog, entry, go, note, busy, then) = (
                ctx.clone(),
                dialog.clone(),
                entry.clone(),
                go.clone(),
                note.clone(),
                busy.clone(),
                then.clone(),
            );
            let typed = pw.clone();
            ctx.run(
                move |c| c.post("live/console/signin", &json!({"password": typed.as_str()})),
                move |r| {
                    busy.set(false);
                    match r {
                        Ok(_) => {
                            dialog.close();
                            then(pw);
                        }
                        Err(e) if e.key == "password.wait" && e.retry_after > 0 => {
                            wait(&ctx2, &note, &go, e.retry_after);
                            entry.grab_focus();
                        }
                        Err(e) => {
                            go.set_sensitive(true);
                            note.say("bad", &ctx2.err_text(&e));
                            entry.grab_focus();
                        }
                    }
                },
            );
        }
    };
    let send = Rc::new(send);
    {
        let s = send.clone();
        go.connect_clicked(move |_| s());
        let s = send.clone();
        entry.connect_activate(move |_| s());
        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    dialog.present(Some(parent));
    entry.grab_focus();
}

/// Counts the waiting time after too many wrong passwords.
fn wait(ctx: &Ctx, note: &Rc<style::Note>, go: &gtk4::Button, secs: u64) {
    let left = Rc::new(Cell::new(secs));
    let (ctx, note, go) = (ctx.clone(), note.clone(), go.clone());
    go.set_sensitive(false);
    let show = {
        let (ctx, note, left) = (ctx.clone(), note.clone(), left.clone());
        move || {
            note.say(
                "warn",
                &ctx.fill("unlock.wait", &[("n", &left.get().to_string())]),
            )
        }
    };
    show();
    glib::timeout_add_seconds_local(1, move || {
        left.set(left.get().saturating_sub(1));
        if left.get() == 0 {
            note.set("");
            go.set_sensitive(true);
            return glib::ControlFlow::Break;
        }
        show();
        glib::ControlFlow::Continue
    });
}

/// A knob dragged to the end (mouse, touch or keyboard: arrow keys move
/// it, End confirms, Home goes back). Letting go early sends it back. It
/// mirrors in a right-to-left layout.
pub struct Slide {
    pub root: gtk4::Box,
    knob: gtk4::Box,
    fill: gtk4::Box,
    label: gtk4::Label,
    track: gtk4::Overlay,
    text: String,
    done_text: String,
    pos: Cell<f64>,
    done: Cell<bool>,
    on_confirm: Box<dyn Fn()>,
}

impl Slide {
    pub fn new(ctx: &Ctx, label: &str, on_confirm: impl Fn() + 'static) -> Rc<Slide> {
        let track = gtk4::Overlay::builder()
            .css_classes(["slide-track"])
            .height_request(56)
            .hexpand(true)
            .build();
        let fill = gtk4::Box::builder()
            .css_classes(["slide-fill"])
            .halign(gtk4::Align::Start)
            .can_target(false)
            .build();
        let lbl = gtk4::Label::builder()
            .label(label)
            .css_classes(["slide-text"])
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .can_target(false)
            .build();
        let knob = gtk4::Box::builder()
            .css_classes(["slide-knob"])
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::Center)
            .focusable(true)
            .build();
        knob.append(&style::icon("go-next-symbolic", 22));
        knob.set_accessible_role(gtk4::AccessibleRole::Slider);
        knob.update_property(&[
            gtk4::accessible::Property::Label(label),
            gtk4::accessible::Property::ValueMin(0.0),
            gtk4::accessible::Property::ValueMax(100.0),
            gtk4::accessible::Property::ValueNow(0.0),
            gtk4::accessible::Property::Description(&ctx.t("slide.hint")),
        ]);
        knob.set_tooltip_text(Some(&ctx.t("slide.hint")));
        track.set_child(Some(&fill));
        track.add_overlay(&lbl);
        track.add_overlay(&knob);
        let hint = style::text(&ctx.t("slide.hint"), &["dim-label"], true);
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        root.append(&track);
        root.append(&hint);
        let s = Rc::new(Slide {
            root,
            knob: knob.clone(),
            fill,
            label: lbl,
            track: track.clone(),
            text: label.to_string(),
            done_text: ctx.t("slide.done"),
            pos: Cell::new(0.0),
            done: Cell::new(false),
            on_confirm: Box::new(on_confirm),
        });
        let start = Rc::new(Cell::new(0.0));
        // The drag is read on the track (which stands still), and counts
        // only when it starts on the knob.
        let held = Rc::new(Cell::new(false));
        let drag = gtk4::GestureDrag::new();
        {
            let (s2, st, hd) = (s.clone(), start.clone(), held.clone());
            drag.connect_drag_begin(move |_, x, y| {
                let on_knob = s2.knob.compute_bounds(&s2.track).is_some_and(|b| {
                    b.contains_point(&gtk4::graphene::Point::new(x as f32, y as f32))
                });
                hd.set(on_knob && !s2.done.get());
                st.set(s2.pos.get());
                if on_knob {
                    s2.knob.grab_focus();
                }
            });
            let (s2, st, hd) = (s.clone(), start.clone(), held.clone());
            drag.connect_drag_update(move |_, dx, _| {
                if s2.done.get() || !hd.get() {
                    return;
                }
                let sign = if s2.track.direction() == gtk4::TextDirection::Rtl {
                    -1.0
                } else {
                    1.0
                };
                s2.set(st.get() + sign * dx / s2.range());
            });
            let (s2, hd) = (s.clone(), held.clone());
            drag.connect_drag_end(move |_, _, _| {
                if s2.done.get() || !hd.replace(false) {
                    return;
                }
                if s2.pos.get() >= 0.97 {
                    s2.finish();
                } else {
                    s2.set(0.0);
                }
            });
        }
        track.add_controller(drag);
        let keys = gtk4::EventControllerKey::new();
        {
            let s2 = s.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if s2.done.get() {
                    return glib::Propagation::Proceed;
                }
                let rtl = s2.track.direction() == gtk4::TextDirection::Rtl;
                use gtk4::gdk::Key;
                let (fwd, back) = if rtl {
                    (Key::Left, Key::Right)
                } else {
                    (Key::Right, Key::Left)
                };
                let p = s2.pos.get();
                match key {
                    k if k == fwd || k == Key::Up || k == Key::Page_Up => s2.set(p + 0.25),
                    k if k == back || k == Key::Down || k == Key::Page_Down => s2.set(p - 0.25),
                    Key::End => s2.set(1.0),
                    Key::Home => s2.set(0.0),
                    _ => return glib::Propagation::Proceed,
                }
                if s2.pos.get() >= 1.0 {
                    s2.finish();
                }
                glib::Propagation::Stop
            });
        }
        knob.add_controller(keys);
        s
    }

    /// The knob's travel in pixels.
    fn range(&self) -> f64 {
        f64::from((self.track.width() - self.knob.width() - 8).max(1))
    }

    fn set(&self, p: f64) {
        let p = p.clamp(0.0, 1.0);
        self.pos.set(p);
        let x = (p * self.range()).round() as i32 + 4;
        self.knob.set_margin_start(x);
        self.fill.set_width_request(x + self.knob.width().max(48));
        self.knob
            .update_property(&[gtk4::accessible::Property::ValueNow((p * 100.0).round())]);
    }

    fn finish(&self) {
        if self.done.replace(true) {
            return;
        }
        self.set(1.0);
        self.label.set_label(&self.done_text);
        self.track.add_css_class("done");
        self.knob.set_sensitive(false);
        (self.on_confirm)();
    }

    /// The action failed: the person can try again.
    pub fn rearm(&self) {
        self.done.set(false);
        self.label.set_label(&self.text);
        self.track.remove_css_class("done");
        self.knob.set_sensitive(true);
        self.set(0.0);
        self.knob.grab_focus();
    }
}

/// How a warning page is confirmed.
pub enum Confirm {
    /// The slide, with its words.
    Slide(String),
    /// A plain button (the first screens of "start over").
    Button(String),
}

/// One warning page (the web console's `dangerScreen`).
pub struct Danger {
    /// "Step 1 of 3".
    pub step: Option<(usize, usize)>,
    pub title: String,
    pub lead: String,
    pub lose: Vec<String>,
    pub keep: Vec<String>,
    pub warn: Option<String>,
    pub confirm: Confirm,
    /// What the note says while the action runs.
    pub working: Option<String>,
    pub on_cancel: Rc<dyn Fn()>,
    /// Runs when confirmed; call `fail` on the handle if it did not work.
    pub on_confirm: Rc<dyn Fn(Rc<DangerHandle>)>,
    /// Something extra above the title (a link), if any.
    pub extra: Option<gtk4::Widget>,
}

/// Lets the action report back to its page.
pub struct DangerHandle {
    ctx: Ctx,
    note: style::Note,
    cancel: gtk4::Button,
    slide: RefCell<Option<Rc<Slide>>>,
    button: RefCell<Option<gtk4::Button>>,
    working: String,
}

impl DangerHandle {
    /// The action started: say so and lock the way back.
    pub fn working(&self) {
        self.note.say("", &self.working);
        self.cancel.set_sensitive(false);
        if let Some(b) = self.button.borrow().as_ref() {
            b.set_sensitive(false);
        }
    }

    /// The action failed: show why and let the person try again.
    pub fn fail(&self, e: &ApiError) {
        self.note.say("bad", &self.ctx.err_text(e));
        self.cancel.set_sensitive(true);
        if let Some(b) = self.button.borrow().as_ref() {
            b.set_sensitive(true);
        }
        if let Some(s) = self.slide.borrow().as_ref() {
            s.rearm();
        }
    }
}

fn list_box(ctx: &Ctx, title_key: &str, items: &[String], lose: bool) -> gtk4::Box {
    let b = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .hexpand(true)
        .css_classes(["danger-box", if lose { "lose" } else { "keep" }])
        .build();
    b.append(&style::text(&ctx.t(title_key), &["title-3"], false));
    for it in items {
        let row = gtk4::Box::builder().spacing(10).build();
        let (icon, tone) = if lose {
            ("window-close-symbolic", Tone::Bad)
        } else {
            ("emblem-ok-symbolic", Tone::Ok)
        };
        row.append(&style::chip(icon, tone, 14));
        let l = style::text(it, &[], false);
        l.set_hexpand(true);
        row.append(&l);
        b.append(&row);
    }
    b
}

/// Shows the warning page `d` (replacing the previous one).
pub fn danger(ctx: &Ctx, d: Danger) {
    let mut kids: Vec<gtk4::Widget> = Vec::new();
    if let Some(x) = &d.extra {
        kids.push(x.clone());
    }
    kids.push(style::hero("dialog-warning-symbolic", Tone::Bad).upcast());
    if let Some((n, total)) = d.step {
        kids.push(
            style::text(
                &ctx.fill(
                    "danger.step",
                    &[("n", &n.to_string()), ("total", &total.to_string())],
                ),
                &["caption-up"],
                true,
            )
            .upcast(),
        );
    }
    kids.push(style::text(&d.title, &["title-1"], true).upcast());
    kids.push(style::text(&d.lead, &["lead"], true).upcast());
    let cols = gtk4::Box::builder().spacing(16).homogeneous(true).build();
    if !d.lose.is_empty() {
        cols.append(&list_box(ctx, "danger.lose", &d.lose, true));
    }
    if !d.keep.is_empty() {
        cols.append(&list_box(ctx, "danger.keep", &d.keep, false));
    }
    if cols.first_child().is_some() {
        kids.push(cols.upcast());
    }
    if let Some(w) = &d.warn {
        kids.push(style::text(w, &["error"], true).upcast());
    }
    let cancel = gtk4::Button::builder()
        .label(ctx.t("common.cancel"))
        .css_classes(["flat", "pill"])
        .halign(gtk4::Align::Center)
        .build();
    let handle = Rc::new(DangerHandle {
        ctx: ctx.clone(),
        note: style::Note::new("bad", ""),
        cancel: cancel.clone(),
        slide: RefCell::new(None),
        button: RefCell::new(None),
        working: d.working.clone().unwrap_or_else(|| ctx.t("danger.working")),
    });
    let go = d.on_confirm.clone();
    match &d.confirm {
        Confirm::Slide(label) => {
            let h = handle.clone();
            let s = Slide::new(ctx, label, move || {
                h.working();
                go(h.clone());
            });
            kids.push(s.root.clone().upcast());
            *handle.slide.borrow_mut() = Some(s);
        }
        Confirm::Button(label) => {
            let b = style::button("go-next-symbolic", label, &["destructive-action", "pill"]);
            b.set_halign(gtk4::Align::Center);
            let h = handle.clone();
            b.connect_clicked(move |_| go(h.clone()));
            kids.push(b.clone().upcast());
            *handle.button.borrow_mut() = Some(b);
        }
    }
    kids.push(handle.note.root.clone().upcast());
    kids.push(cancel.clone().upcast());
    let cb = d.on_cancel.clone();
    cancel.connect_clicked(move |_| cb());
    let refs: Vec<&gtk4::Widget> = kids.iter().collect();
    let page = style::scroll(&style::panel(&refs, 760));
    ctx.show_page("danger", &page);
    match &d.confirm {
        Confirm::Slide(_) => {
            if let Some(s) = handle.slide.borrow().as_ref() {
                s.knob.grab_focus();
            }
        }
        Confirm::Button(_) => {
            cancel.grab_focus();
        }
    }
}

/// After a dangerous action worked: a tick, the message and one button.
pub fn done_page(
    ctx: &Ctx,
    message: &str,
    sub: Option<&str>,
    label: Option<&str>,
    next: impl Fn() + 'static,
) {
    let go = style::button(
        "go-next-symbolic",
        &label.map_or_else(|| ctx.t("danger.done.back"), str::to_string),
        &["suggested-action", "pill"],
    );
    go.set_halign(gtk4::Align::Center);
    go.connect_clicked(move |_| next());
    let hero = style::hero("emblem-ok-symbolic", Tone::Ok);
    let title = style::text(message, &["title-1"], true);
    let mut kids: Vec<&gtk4::Widget> = vec![hero.upcast_ref(), title.upcast_ref()];
    let subl = sub.map(|s| style::text(s, &["lead"], true));
    if let Some(l) = &subl {
        kids.push(l.upcast_ref());
    }
    kids.push(go.upcast_ref());
    let page = style::scroll(&style::panel(&kids, 560));
    ctx.show_page("done", &page);
    go.grab_focus();
}

/// A page in a light window with a Back button and a title above `body`
/// (Disks, Files, Terminal). `head_extra` goes at the far end of the title
/// row.
pub fn sub_window(
    ctx: &Ctx,
    title: &str,
    sub: Option<&str>,
    head_extra: Option<&gtk4::Widget>,
    back: impl Fn() + 'static,
    body: &impl IsA<gtk4::Widget>,
) -> gtk4::Box {
    let b = style::button("go-previous-symbolic", &ctx.t("common.back"), &["flat"]);
    b.set_valign(gtk4::Align::Center);
    b.connect_clicked(move |_| back());
    let titles = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    titles.append(&style::text(title, &["title-2"], false));
    if let Some(s) = sub {
        titles.append(&style::text(s, &["dim-label"], false));
    }
    let head = gtk4::Box::builder()
        .spacing(12)
        .margin_top(20)
        .margin_bottom(12)
        .margin_start(20)
        .margin_end(20)
        .build();
    head.append(&b);
    head.append(&titles);
    if let Some(x) = head_extra {
        head.append(x);
    }
    let col = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .vexpand(true)
        .build();
    col.append(&head);
    col.append(body);
    let out = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    out.append(&style::window(&col, 1000));
    out
}
