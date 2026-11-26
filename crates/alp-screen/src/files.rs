// SPDX-License-Identifier: AGPL-3.0-or-later
//! Advanced > Files (0.2.7 `files`): a read-only look at the files on a USB
//! stick, memory card or disk. Names come from a disk anyone could have
//! written, so they are shown as plain text only. Nothing is ever written:
//! no rename, delete or copy. Photos and text files can be opened; a video
//! is not played on this screen yet.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use alp_screen::api::ApiError;
use alp_screen::model::home::size;
use alp_screen::set::{self, Entry, Kind, Media};
use gtk4::prelude::*;
use gtk4::{gdk, glib};
use serde_json::{Value, json};

use crate::dialogs::{self, Ctx};
use crate::setup::text;
use crate::style::{self, Tone};

/// A text file is read up to this many bytes.
const TEXT_MAX: usize = 256 * 1024;
/// A photo is read up to this many bytes.
const IMAGE_MAX: usize = 24 * 1024 * 1024;

struct View {
    left: gtk4::Box,
    right: gtk4::Box,
    note: style::Note,
    /// The opaque id of the disk being looked at.
    media: RefCell<Option<String>>,
    list: RefCell<Vec<Media>>,
    trail: RefCell<Vec<String>>,
    entries: RefCell<Vec<Entry>>,
    more: Cell<bool>,
    skipped: Cell<u64>,
    /// Bumped on every new request: a late answer is dropped.
    seq: Cell<u32>,
    opened: RefCell<HashSet<String>>,
    gone: RefCell<Option<String>>,
}

pub struct FilesUi {
    ctx: Ctx,
    view: RefCell<Option<Rc<View>>>,
    back: Box<dyn Fn()>,
}

fn label_of(ctx: &Ctx, m: &Media) -> String {
    m.label.clone().unwrap_or_else(|| ctx.t("import.device"))
}

fn clear(b: &gtk4::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

impl FilesUi {
    pub fn new(ctx: &Ctx, back: impl Fn() + 'static) -> Rc<Self> {
        Rc::new(FilesUi {
            ctx: ctx.clone(),
            view: RefCell::new(None),
            back: Box::new(back),
        })
    }

    /// Leaving the page: drop anything late and the open photo.
    pub fn close(&self) {
        if let Some(v) = self.view.borrow_mut().take() {
            v.seq.set(v.seq.get() + 1);
            clear(&v.right);
        }
    }

    pub fn open(self: &Rc<Self>, w: &Value) {
        let ctx = &self.ctx;
        let left = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .margin_top(8)
            .margin_start(12)
            .margin_end(12)
            .width_request(250)
            .build();
        let right = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(10)
            .hexpand(true)
            .margin_top(8)
            .margin_start(20)
            .margin_end(20)
            .margin_bottom(16)
            .build();
        let note = style::Note::new("bad", "");
        let lock = gtk4::Box::builder()
            .spacing(8)
            .margin_start(24)
            .margin_bottom(8)
            .build();
        lock.append(&style::icon("changes-prevent-symbolic", 16));
        lock.append(&style::text(&ctx.t("files.ro"), &["dim-label"], false));
        let side = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .child(&left)
            .css_classes(["cat-side"])
            .hexpand(false)
            .width_request(260)
            .build();
        let main = style::scroll(&right);
        main.set_hexpand(true);
        let both = gtk4::Box::builder().vexpand(true).build();
        both.append(&side);
        both.append(&main);
        let body = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .vexpand(true)
            .build();
        body.append(&lock);
        body.append(&note.root);
        body.append(&both);
        let me = self.clone();
        let page = dialogs::sub_window(
            ctx,
            &ctx.t("set.files"),
            None,
            None,
            move || {
                me.close();
                (me.back)();
            },
            &body,
        );
        ctx.show_page("files", &page);
        let v = Rc::new(View {
            left,
            right,
            note,
            media: RefCell::new(None),
            list: RefCell::new(Vec::new()),
            trail: RefCell::new(Vec::new()),
            entries: RefCell::new(Vec::new()),
            more: Cell::new(false),
            skipped: Cell::new(0),
            seq: Cell::new(0),
            opened: RefCell::new(HashSet::new()),
            gone: RefCell::new(None),
        });
        *self.view.borrow_mut() = Some(v);
        self.update(w);
    }

    /// The disks plugged in change on their own: keep the list true.
    pub fn update(self: &Rc<Self>, w: &Value) {
        let Some(v) = self.view.borrow().clone() else {
            return;
        };
        let list = set::media(w);
        if *v.list.borrow() == list && v.left.first_child().is_some() {
            return;
        }
        *v.list.borrow_mut() = list.clone();
        let cur = v.media.borrow().clone();
        if let Some(id) = &cur
            && !list.iter().any(|m| &m.id == id)
        {
            *v.media.borrow_mut() = None;
            v.trail.borrow_mut().clear();
            v.entries.borrow_mut().clear();
            v.seq.set(v.seq.get() + 1);
            clear(&v.right);
            v.right.append(&style::text(
                &self.ctx.t("files.unplugged"),
                &["title-3"],
                false,
            ));
        }
        self.draw_devices(&v);
        if v.media.borrow().is_none()
            && let Some(first) = list.iter().find(|m| m.refused.is_none())
        {
            self.pick(&v, first, true);
        } else if list.is_empty() {
            self.draw_none(&v);
        }
    }

    fn draw_none(&self, v: &View) {
        clear(&v.right);
        let gone = v.gone.borrow().clone();
        let t = match gone {
            Some(n) => self.ctx.fill("files.ejected", &[("name", &n)]),
            None => self.ctx.t("files.none"),
        };
        v.right.append(&style::text(&t, &["title-3"], false));
        v.right.append(&style::text(
            &self.ctx.t("import.plug"),
            &["dim-label"],
            false,
        ));
    }

    fn draw_devices(self: &Rc<Self>, v: &Rc<View>) {
        clear(&v.left);
        v.left.append(&style::text(
            &self.ctx.t("files.devices"),
            &["title-4"],
            false,
        ));
        let list = v.list.borrow().clone();
        if list.is_empty() {
            v.left.append(&style::text(
                &self.ctx.t("import.plug"),
                &["dim-label"],
                false,
            ));
            return;
        }
        let cur = v.media.borrow().clone();
        let devs = gtk4::ListBox::builder()
            .css_classes(["cat-list"])
            .selection_mode(gtk4::SelectionMode::Single)
            .build();
        devs.update_property(&[gtk4::accessible::Property::Label(
            &self.ctx.t("files.devices"),
        )]);
        let mut chosen = None;
        for m in &list {
            let line = gtk4::Box::builder()
                .spacing(10)
                .margin_top(6)
                .margin_bottom(6)
                .build();
            line.append(&style::chip(
                if m.refused.is_some() {
                    "changes-prevent-symbolic"
                } else {
                    "drive-harddisk-symbolic"
                },
                Tone::Teal,
                16,
            ));
            let names = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .hexpand(true)
                .build();
            let name = label_of(&self.ctx, m);
            names.append(
                &gtk4::Label::builder()
                    .label(&name)
                    .xalign(0.0)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .build(),
            );
            let sub = if m.refused.is_some() {
                self.ctx.t("files.refused.short")
            } else {
                [
                    m.fs_type.as_deref().map(str::to_uppercase),
                    m.size_bytes.filter(|b| *b > 0).map(size),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ")
            };
            names.append(&style::text(&sub, &["caption", "dim-label"], false));
            line.append(&names);
            let row = gtk4::ListBoxRow::builder()
                .child(&line)
                .name(m.id.as_str())
                .build();
            row.update_property(&[gtk4::accessible::Property::Label(&format!("{name}, {sub}"))]);
            devs.append(&row);
            if cur.as_deref() == Some(m.id.as_str()) {
                chosen = Some(row);
            }
        }
        devs.select_row(chosen.as_ref());
        let (me, v2) = (self.clone(), v.clone());
        devs.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let id = row.widget_name();
            if v2.media.borrow().as_deref() == Some(id.as_str()) {
                return;
            }
            let m = v2
                .list
                .borrow()
                .iter()
                .find(|m| m.id == id.as_str())
                .cloned();
            if let Some(m) = m {
                me.pick(&v2, &m, false);
            }
        });
        v.left.append(&devs);
    }

    fn pick(self: &Rc<Self>, v: &Rc<View>, m: &Media, redraw: bool) {
        v.note.set("");
        *v.media.borrow_mut() = Some(m.id.clone());
        v.trail.borrow_mut().clear();
        v.entries.borrow_mut().clear();
        if redraw {
            self.draw_devices(v);
        }
        if let Some(why) = &m.refused {
            v.seq.set(v.seq.get() + 1);
            clear(&v.right);
            v.right
                .append(&style::hero("changes-prevent-symbolic", Tone::Grey));
            v.right.append(&style::text(
                &self
                    .ctx
                    .fill("files.refused", &[("name", &label_of(&self.ctx, m))]),
                &["title-3"],
                false,
            ));
            v.right.append(&style::text(why, &["dim-label"], false));
            return;
        }
        self.load(v, true);
    }

    fn load(self: &Rc<Self>, v: &Rc<View>, reset: bool) {
        let Some(media) = v.media.borrow().clone() else {
            return;
        };
        let my = v.seq.get() + 1;
        v.seq.set(my);
        if reset {
            v.entries.borrow_mut().clear();
            v.more.set(false);
            v.skipped.set(0);
            clear(&v.right);
            v.right.append(&style::text(
                &self.ctx.t("files.reading"),
                &["dim-label"],
                false,
            ));
        }
        let was_open = v.opened.borrow().contains(&media);
        let path = v.trail.borrow().clone();
        let offset = v.entries.borrow().len();
        let (me, v2, id) = (self.clone(), v.clone(), media.clone());
        self.ctx.run(
            move |c| -> Result<Value, ApiError> {
                if !was_open {
                    c.post("live/media/open", &json!({"media": media}))?;
                }
                let ask = || {
                    c.post(
                        "live/media/list",
                        &json!({"media": media, "path": path, "offset": offset}),
                    )
                };
                match ask() {
                    Err(e) if e.key == "media.not_open" => {
                        c.post("live/media/open", &json!({"media": media}))?;
                        ask()
                    }
                    r => r,
                }
            },
            move |r| {
                if v2.seq.get() != my {
                    return;
                }
                match r {
                    Ok(j) => {
                        v2.opened.borrow_mut().insert(id);
                        let l = set::listing(&j);
                        v2.entries.borrow_mut().extend(l.entries);
                        v2.more.set(l.truncated);
                        v2.skipped.set(v2.skipped.get() + l.skipped);
                        me.draw(&v2);
                    }
                    Err(e) => {
                        clear(&v2.right);
                        me.ctx.fail(&e);
                        v2.note.say("bad", &me.ctx.err_text(&e));
                    }
                }
            },
        );
    }

    fn crumbs(self: &Rc<Self>, v: &Rc<View>) -> gtk4::Box {
        let row = gtk4::Box::builder().spacing(4).hexpand(true).build();
        let cur = v.media.borrow().clone();
        let name = v
            .list
            .borrow()
            .iter()
            .find(|m| Some(&m.id) == cur.as_ref())
            .map(|m| label_of(&self.ctx, m))
            .unwrap_or_default();
        let top = gtk4::Button::builder()
            .label(&name)
            .css_classes(["flat"])
            .build();
        let (me, v2) = (self.clone(), v.clone());
        top.connect_clicked(move |_| {
            v2.trail.borrow_mut().clear();
            me.load(&v2, true);
        });
        row.append(&top);
        let trail = v.trail.borrow().clone();
        for (i, n) in trail.iter().enumerate() {
            row.append(&gtk4::Label::new(Some("/")));
            let b = gtk4::Button::builder()
                .label(n)
                .css_classes(["flat"])
                .build();
            let (me, v2) = (self.clone(), v.clone());
            b.connect_clicked(move |_| {
                v2.trail.borrow_mut().truncate(i + 1);
                me.load(&v2, true);
            });
            row.append(&b);
        }
        row
    }

    fn draw(self: &Rc<Self>, v: &Rc<View>) {
        let ctx = &self.ctx;
        clear(&v.right);
        let bar = gtk4::Box::builder().spacing(8).build();
        bar.append(&self.crumbs(v));
        let eject = style::button("media-eject-symbolic", &ctx.t("import.eject"), &[]);
        let (me, v2) = (self.clone(), v.clone());
        eject.connect_clicked(move |_| me.eject(&v2));
        bar.append(&eject);
        v.right.append(&bar);
        let entries = v.entries.borrow().clone();
        if entries.is_empty() {
            v.right
                .append(&style::text(&ctx.t("files.empty"), &["dim-label"], false));
        } else {
            let list = gtk4::ListBox::builder()
                .selection_mode(gtk4::SelectionMode::None)
                .css_classes(["boxed-list"])
                .build();
            list.update_property(&[gtk4::accessible::Property::Label(&ctx.t("set.files"))]);
            for e in &entries {
                let icon = if e.dir {
                    "folder-symbolic"
                } else {
                    match set::kind_of(&e.name) {
                        Kind::Image => "image-x-generic-symbolic",
                        Kind::Video => "video-x-generic-symbolic",
                        _ => "text-x-generic-symbolic",
                    }
                };
                let line = gtk4::Box::builder()
                    .spacing(12)
                    .margin_top(8)
                    .margin_bottom(8)
                    .margin_start(12)
                    .margin_end(12)
                    .build();
                line.append(&style::chip(icon, Tone::Grey, 16));
                line.append(
                    &gtk4::Label::builder()
                        .label(&e.name)
                        .xalign(0.0)
                        .hexpand(true)
                        .ellipsize(gtk4::pango::EllipsizeMode::Middle)
                        .build(),
                );
                if !e.dir
                    && let Some(b) = e.size_bytes
                {
                    line.append(
                        &gtk4::Label::builder()
                            .label(set::file_size(b))
                            .css_classes(["dim-label"])
                            .build(),
                    );
                }
                let row = gtk4::ListBoxRow::builder().child(&line).build();
                row.update_property(&[gtk4::accessible::Property::Label(&e.name)]);
                list.append(&row);
            }
            let (me, v2) = (self.clone(), v.clone());
            list.connect_row_activated(move |_, row| {
                let e = entries.get(row.index() as usize).cloned();
                if let Some(e) = e {
                    if e.dir {
                        v2.trail.borrow_mut().push(e.name);
                        me.load(&v2, true);
                    } else {
                        me.preview(&v2, &e);
                    }
                }
            });
            v.right.append(&list);
        }
        if v.more.get() {
            let b = style::button("view-more-symbolic", &ctx.t("files.more"), &[]);
            b.set_halign(gtk4::Align::Center);
            let (me, v2) = (self.clone(), v.clone());
            b.connect_clicked(move |_| me.load(&v2, false));
            v.right.append(&b);
        }
        if v.skipped.get() > 0 {
            v.right.append(&style::text(
                &ctx.fill("files.skipped", &[("n", &v.skipped.get().to_string())]),
                &["dim-label"],
                false,
            ));
        }
    }

    fn preview(self: &Rc<Self>, v: &Rc<View>, e: &Entry) {
        let ctx = &self.ctx;
        let Some(media) = v.media.borrow().clone() else {
            return;
        };
        let my = v.seq.get() + 1;
        v.seq.set(my);
        clear(&v.right);
        let bar = gtk4::Box::builder().spacing(8).build();
        bar.append(&self.crumbs(v));
        let back = style::button("go-previous-symbolic", &ctx.t("files.back"), &[]);
        let (me, v2) = (self.clone(), v.clone());
        back.connect_clicked(move |_| {
            v2.seq.set(v2.seq.get() + 1);
            me.draw(&v2);
        });
        bar.append(&back);
        v.right.append(&bar);
        v.right.append(&style::text(&e.name, &["title-3"], false));
        if let Some(b) = e.size_bytes {
            v.right
                .append(&style::text(&set::file_size(b), &["dim-label"], false));
        }
        let kind = set::kind_of(&e.name);
        let say = |t: String| v.right.append(&style::text(&t, &[], false));
        match kind {
            Kind::Other => return say(ctx.t("files.noopen")),
            Kind::Video => {
                return say(text(
                    &ctx.texts,
                    "files.novideo",
                    "Videos cannot be played on this screen yet.",
                ));
            }
            _ => {}
        }
        let waiting = style::text(&ctx.t("files.reading"), &["dim-label"], false);
        v.right.append(&waiting);
        let mut path = v.trail.borrow().clone();
        path.push(e.name.clone());
        let max = if kind == Kind::Image {
            IMAGE_MAX
        } else {
            TEXT_MAX
        };
        let (me, v2, cut) = (
            self.clone(),
            v.clone(),
            e.size_bytes.unwrap_or(0) as usize > TEXT_MAX,
        );
        ctx.run(
            move |c| c.get_file(&media, &path, max),
            move |r| {
                if v2.seq.get() != my {
                    return;
                }
                v2.right.remove(&waiting);
                let bad = || {
                    v2.right
                        .append(&style::text(&me.ctx.t("files.unreadable"), &[], false));
                };
                let Ok((ctype, bytes)) = r else {
                    return bad();
                };
                if kind == Kind::Image {
                    match gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)) {
                        Ok(t) => {
                            let pic = gtk4::Picture::for_paintable(&t);
                            pic.set_content_fit(gtk4::ContentFit::Contain);
                            pic.set_size_request(-1, 360);
                            pic.update_property(&[gtk4::accessible::Property::Label(
                                &me.ctx.t("files.devices"),
                            )]);
                            v2.right.append(&pic);
                        }
                        Err(_) => bad(),
                    }
                    return;
                }
                if !ctype.to_ascii_lowercase().starts_with("text/plain") {
                    return bad();
                }
                let t = gtk4::Label::builder()
                    .label(String::from_utf8_lossy(&bytes).as_ref())
                    .selectable(true)
                    .wrap(true)
                    .wrap_mode(gtk4::pango::WrapMode::WordChar)
                    .xalign(0.0)
                    .css_classes(["monospace"])
                    .build();
                v2.right.append(&t);
                if cut {
                    v2.right.append(&style::text(
                        &me.ctx.t("files.text.cut"),
                        &["dim-label"],
                        false,
                    ));
                }
            },
        );
    }

    fn eject(self: &Rc<Self>, v: &Rc<View>) {
        let Some(media) = v.media.borrow().clone() else {
            return;
        };
        let name = v
            .list
            .borrow()
            .iter()
            .find(|m| m.id == media)
            .map(|m| label_of(&self.ctx, m))
            .unwrap_or_default();
        v.seq.set(v.seq.get() + 1);
        clear(&v.right);
        v.right.append(&style::text(
            &self.ctx.t("files.ejecting"),
            &["dim-label"],
            false,
        ));
        let (me, v2) = (self.clone(), v.clone());
        self.ctx.post(
            "live/media/eject",
            json!({"media": media}),
            move |r| match r {
                Ok(_) => {
                    v2.opened.borrow_mut().remove(&media);
                    *v2.gone.borrow_mut() = Some(name.clone());
                    *v2.media.borrow_mut() = None;
                    v2.trail.borrow_mut().clear();
                    v2.entries.borrow_mut().clear();
                    clear(&v2.right);
                    v2.right.append(&style::text(
                        &me.ctx.fill("files.ejected", &[("name", &name)]),
                        &["title-3"],
                        false,
                    ));
                    v2.right.append(&style::text(
                        &me.ctx.t("import.ejected"),
                        &["dim-label"],
                        false,
                    ));
                    me.ctx.refresh();
                }
                Err(e) => {
                    clear(&v2.right);
                    me.ctx.fail(&e);
                    v2.note.say("bad", &me.ctx.err_text(&e));
                }
            },
        );
    }
}
