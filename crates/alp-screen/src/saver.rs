// SPDX-License-Identifier: AGPL-3.0-or-later
//! The photo frame (0.2.7 `startSaver`): when the switch under Settings >
//! Display is on and nobody touched the screen for the lock time, the
//! screen locks and shows calm pictures with the time. Touching the screen
//! ends it; the lock screen is underneath. Off by default.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use alp_screen::i18n::Texts;
use gtk4::glib;
use gtk4::prelude::*;

/// Where the image keeps the scenes (the console's `screensaver` folder).
const DIR: &str = "/usr/share/alp/ui/live/screensaver";
/// Seconds each picture stays.
const EACH: u32 = 8;

pub struct SaverUi {
    pub page: gtk4::Overlay,
    pictures: gtk4::Stack,
    clock: gtk4::Label,
    zone: RefCell<Option<String>>,
    timers: RefCell<Vec<glib::SourceId>>,
    on: Cell<bool>,
    shown: Cell<usize>,
    count: Cell<usize>,
}

/// The pictures: licence-checked photos listed in `photos/index.json`
/// replace the drawn scenes one by one (names are plain file names).
pub fn pictures(dir: &Path) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = (1..=5)
        .map(|i| dir.join(format!("scene-{i}.svg")))
        .filter(|p| p.is_file())
        .collect();
    let index = std::fs::read_to_string(dir.join("photos/index.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .unwrap_or_default();
    for (i, f) in index.iter().take(5).enumerate() {
        let ok = !f.is_empty()
            && f.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        let p = dir.join("photos").join(f);
        if ok && p.is_file() {
            if i < list.len() {
                list[i] = p;
            } else {
                list.push(p);
            }
        }
    }
    list
}

impl SaverUi {
    pub fn new(texts: &Texts) -> Rc<Self> {
        let pictures_stack = gtk4::Stack::builder()
            .transition_type(gtk4::StackTransitionType::Crossfade)
            .transition_duration(1500)
            .hexpand(true)
            .vexpand(true)
            .build();
        let page = gtk4::Overlay::builder().css_classes(["saver"]).build();
        page.set_child(Some(&pictures_stack));
        let clock = gtk4::Label::builder()
            .css_classes(["saver-clock"])
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::End)
            .margin_start(40)
            .margin_bottom(32)
            .build();
        let note = gtk4::Label::builder()
            .label(texts.t("saver.note"))
            .css_classes(["saver-note"])
            .halign(gtk4::Align::End)
            .valign(gtk4::Align::End)
            .margin_end(40)
            .margin_bottom(36)
            .build();
        page.add_overlay(&clock);
        page.add_overlay(&note);
        // Decorative: the screen reader hears only that the screen is
        // resting.
        page.update_property(&[gtk4::accessible::Property::Label(&texts.t("saver.note"))]);
        Rc::new(SaverUi {
            page,
            pictures: pictures_stack,
            clock,
            zone: RefCell::new(None),
            timers: RefCell::new(Vec::new()),
            on: Cell::new(false),
            shown: Cell::new(0),
            count: Cell::new(0),
        })
    }

    pub fn running(&self) -> bool {
        self.on.get()
    }

    fn dir() -> PathBuf {
        [DIR, "ui/live/screensaver"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_dir())
            .unwrap_or_else(|| PathBuf::from(DIR))
    }

    fn tick(&self) {
        let z = self.zone.borrow().clone();
        let now = crate::dialogs::now() as i64;
        if let Some(s) = crate::welcome::at(now, z.as_deref()).and_then(|d| d.format("%H:%M").ok())
        {
            self.clock.set_label(&s);
        }
    }

    /// Starts the pictures (the caller shows the page).
    pub fn start(self: &Rc<Self>, zone: Option<String>) {
        if self.on.replace(true) {
            return;
        }
        *self.zone.borrow_mut() = zone;
        while let Some(c) = self.pictures.first_child() {
            self.pictures.remove(&c);
        }
        let list = pictures(&Self::dir());
        for (i, p) in list.iter().enumerate() {
            let pic = gtk4::Picture::for_filename(p);
            pic.set_content_fit(gtk4::ContentFit::Cover);
            pic.set_accessible_role(gtk4::AccessibleRole::Presentation);
            self.pictures.add_named(&pic, Some(&format!("p{i}")));
        }
        self.count.set(list.len());
        self.shown.set(0);
        if !list.is_empty() {
            self.pictures.set_visible_child_name("p0");
        }
        self.tick();
        let me = self.clone();
        let clock = glib::timeout_add_seconds_local(20, move || {
            me.tick();
            glib::ControlFlow::Continue
        });
        let me = self.clone();
        let rotate = glib::timeout_add_seconds_local(EACH, move || {
            let n = me.count.get();
            if n > 1 {
                let next = (me.shown.get() + 1) % n;
                me.shown.set(next);
                me.pictures.set_visible_child_name(&format!("p{next}"));
            }
            glib::ControlFlow::Continue
        });
        self.timers.borrow_mut().extend([clock, rotate]);
    }

    pub fn stop(&self) {
        self.on.set(false);
        for t in self.timers.borrow_mut().drain(..) {
            t.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_photos_replace_scenes_by_name_only() {
        let dir = std::env::temp_dir().join(format!("alp-saver-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("photos")).unwrap();
        for i in 1..=5 {
            std::fs::write(dir.join(format!("scene-{i}.svg")), "<svg/>").unwrap();
        }
        std::fs::write(dir.join("photos/hills.jpg"), "x").unwrap();
        std::fs::write(
            dir.join("photos/index.json"),
            r#"["hills.jpg", "../escape.jpg", "missing.jpg"]"#,
        )
        .unwrap();
        let l = pictures(&dir);
        assert_eq!(l.len(), 5);
        assert!(l[0].ends_with("photos/hills.jpg"));
        assert!(l[1].ends_with("scene-2.svg"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(pictures(&dir).is_empty());
    }
}
