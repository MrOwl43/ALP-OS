// SPDX-License-Identifier: AGPL-3.0-or-later
//! The screen's look (0.2.8 design pass): the 0.2.7 console's blue
//! gradient and system bar, coloured icons, cards, and the shared widgets
//! every page is built from. Accessibility rules kept here:
//!
//! - Text on the gradient is white or near white (contrast at least 4.5:1
//!   on the lightest blue; `dim-label` is raised from 55 % to 80 %).
//! - A colour never stands alone: every status has an icon and words.
//! - Buttons and fields are at least 44 px high; the keyboard focus ring
//!   is a thick yellow line that shows on blue and on white.
//! - Icon-only controls carry a spoken name ([`name`]).
//!
//! Icons are the Adwaita symbolic set (package adwaita-icon-theme).

use gtk4::prelude::*;
use libadwaita as adw;

const CSS: &str = r#"
@define-color window_bg_color #12307f;
@define-color window_fg_color #ffffff;
@define-color view_bg_color #0f2766;
@define-color view_fg_color #ffffff;
@define-color card_bg_color rgba(255,255,255,0.09);
@define-color card_fg_color #ffffff;
@define-color dialog_bg_color #17358a;
@define-color dialog_fg_color #ffffff;
@define-color popover_bg_color #17358a;
@define-color popover_fg_color #ffffff;
@define-color accent_bg_color #4f8cff;
@define-color accent_fg_color #ffffff;
@define-color accent_color #a9c8ff;
@define-color destructive_bg_color #d62a33;
@define-color destructive_fg_color #ffffff;
@define-color destructive_color #ffa39e;
@define-color error_color #ffa39e;
@define-color warning_color #ffd06b;
@define-color success_color #6fe3a0;

window.alp {
  background-image: linear-gradient(135deg, #2054e0 0%, #173a9e 50%, #0c1f55 100%);
  color: #ffffff;
  font-size: 12pt;
}

/* Text */
.dim-label { opacity: 0.8; }
.alp .title-1 { font-size: 24pt; font-weight: 800; }
.alp .title-2 { font-size: 20pt; font-weight: 800; }
.lead { font-size: 13pt; }
.caption-up { font-size: 10pt; font-weight: 700; letter-spacing: 1px; opacity: 0.8; }
.big-number { font-size: 54pt; font-weight: 800; }
.error { color: #ffb4ae; }
.warning { color: #ffd06b; }

/* Top bar */
.topbar { background-color: rgba(4,14,48,0.35); min-height: 48px; padding: 0 12px; }
.topbar .brand { font-weight: 800; font-size: 12pt; }
.topbar .tag { background-color: #ffe9a8; color: #6b4600; border-radius: 99px;
  padding: 1px 9px; font-size: 9.5pt; font-weight: 800; }
.topbar .ip { font-family: monospace; font-size: 11pt; }
.topbar .stat { padding: 0 6px; }
.topbar button { min-height: 36px; min-width: 36px; padding: 0 10px; border-radius: 10px;
  background: transparent; color: #ffffff; }
.topbar button:hover { background-color: rgba(255,255,255,0.14); }
.topbar .home-btn { background-color: rgba(255,255,255,0.12); }

/* Tones: an icon chip or text in a status colour */
.tone-ok { color: #6fe3a0; }
.tone-warn { color: #ffd06b; }
.tone-bad { color: #ff9a92; }
.tone-blue { color: #9cc4ff; }
.tone-violet { color: #d2b3ff; }
.tone-teal { color: #79e6d6; }

.chip { border-radius: 14px; padding: 10px; min-width: 24px; min-height: 24px; }
.chip.tone-ok { background-color: rgba(111,227,160,0.16); }
.chip.tone-warn { background-color: rgba(255,208,107,0.16); }
.chip.tone-bad { background-color: rgba(255,154,146,0.16); }
.chip.tone-blue { background-color: rgba(156,196,255,0.16); }
.chip.tone-violet { background-color: rgba(210,179,255,0.16); }
.chip.tone-teal { background-color: rgba(121,230,214,0.16); }
.hero-badge { border-radius: 999px; padding: 16px; }
.hero-badge.tone-ok { background-color: rgba(111,227,160,0.18); }
.hero-badge.tone-warn { background-color: rgba(255,208,107,0.18); }
.hero-badge.tone-bad { background-color: rgba(255,154,146,0.18); }
.hero-badge.tone-blue { background-color: rgba(156,196,255,0.18); }
.hero-badge.tone-violet { background-color: rgba(210,179,255,0.18); }
.hero-badge.tone-teal { background-color: rgba(121,230,214,0.18); }

/* Cards */
.alp .card, .panel { background-color: rgba(255,255,255,0.09); border-radius: 18px;
  border: 1px solid rgba(255,255,255,0.14); box-shadow: none; color: #ffffff; }
.panel { padding: 26px 34px; }
.tile-photos { background-image: linear-gradient(135deg, rgba(255,255,255,0.18), rgba(255,255,255,0.06)); }

/* Notes with an icon in front (info, warning, danger) */
.note { border-radius: 12px; padding: 12px 14px; background-color: rgba(255,255,255,0.08); }
.note.warn { background-color: rgba(255,208,107,0.14); border: 1px solid rgba(255,208,107,0.45); }
.note.bad { background-color: rgba(255,120,110,0.16); border: 1px solid rgba(255,154,146,0.55); }

/* Buttons: white main button with blue text, as in 0.2.7 */
.alp button { min-height: 44px; font-weight: 700; }
.alp button.suggested-action { background-color: #ffffff; color: #173a9e; }
.alp button.suggested-action:hover { background-color: #e8efff; }
.alp button.suggested-action:disabled { background-color: rgba(255,255,255,0.35); color: rgba(23,58,158,0.7); }
.alp button.destructive-action { background-color: #d62a33; color: #ffffff; }
.alp button.pill { padding-left: 26px; padding-right: 26px; }
.alp button.flat { color: #ffffff; }
.alp button.wide { min-width: 320px; }

/* Fields */
.alp entry, .alp passwordentry { min-height: 44px; font-size: 13pt;
  background-color: rgba(255,255,255,0.12); color: #ffffff; border-radius: 10px; }
.field-label { font-weight: 700; }

/* Keyboard focus: a thick yellow ring, visible on blue and on white */
.alp *:focus-visible { outline: 3px solid #ffd76a; outline-offset: 2px; }

/* Progress */
.alp progressbar trough { min-height: 12px; border-radius: 99px; background-color: rgba(255,255,255,0.18); }
.alp progressbar progress { min-height: 12px; border-radius: 99px; background-color: #6fe3a0; }

/* Steps of the first start */
.step-dot { min-width: 10px; min-height: 10px; border-radius: 99px; background-color: rgba(255,255,255,0.28); }
.step-dot.done { background-color: #6fe3a0; }
.step-dot.now { background-color: #ffffff; min-width: 28px; }

/* The recovery words: a sheet of paper, dark on white */
.paper { background-color: #fffdf6; color: #172046; border-radius: 16px; padding: 22px 28px; }
.paper .word-n { color: #5b6380; font-size: 12pt; font-weight: 700; }
.paper .word { font-family: monospace; font-size: 17pt; font-weight: 800; color: #10183a; }
.paper.qr { background-color: #ffffff; padding: 10px; }
.paper .code-big { font-size: 34pt; letter-spacing: 4px; }
.word-n-chip { background-color: rgba(255,255,255,0.14); border-radius: 10px; padding: 6px 12px; font-weight: 800; }

/* Lock screen clock */
.clock-big { font-size: 64pt; font-weight: 300; }
.date-line { font-size: 15pt; opacity: 0.9; }

/* Settings and install lists on the gradient */
.alp preferencespage, .alp preferencespage > scrolledwindow, .alp preferencespage viewport { background: transparent; }
.alp .boxed-list { background-color: rgba(255,255,255,0.09); border-radius: 14px; }
.alp row { min-height: 56px; }
.alp row.danger-row { background-color: rgba(255,120,110,0.14); }
.footer-bar { background-color: rgba(4,14,48,0.30); padding: 12px 24px; }
"#;

/// Loads the look for the whole screen (once, at start).
pub fn install() {
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    let css = gtk4::CssProvider::new();
    css.load_from_string(CSS);
    if let Some(d) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &d,
            &css,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// The tone classes, in one place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Ok,
    Warn,
    Bad,
    Blue,
    Violet,
    Teal,
}

impl Tone {
    pub fn class(self) -> &'static str {
        match self {
            Tone::Ok => "tone-ok",
            Tone::Warn => "tone-warn",
            Tone::Bad => "tone-bad",
            Tone::Blue => "tone-blue",
            Tone::Violet => "tone-violet",
            Tone::Teal => "tone-teal",
        }
    }

    pub const ALL: [Tone; 6] = [
        Tone::Ok,
        Tone::Warn,
        Tone::Bad,
        Tone::Blue,
        Tone::Violet,
        Tone::Teal,
    ];

    /// Sets this tone on `w`, removing any other.
    pub fn apply(self, w: &impl IsA<gtk4::Widget>) {
        for t in Tone::ALL {
            w.remove_css_class(t.class());
        }
        w.add_css_class(self.class());
    }
}

impl From<alp_screen::model::bar::Tone> for Tone {
    fn from(t: alp_screen::model::bar::Tone) -> Self {
        use alp_screen::model::bar::Tone as B;
        match t {
            B::Ok => Tone::Ok,
            B::Warn => Tone::Warn,
            B::Bad => Tone::Bad,
        }
    }
}

/// Gives a control its spoken name (and the same text as a tooltip).
pub fn name(w: &(impl IsA<gtk4::Widget> + IsA<gtk4::Accessible>), text: &str) {
    w.update_property(&[gtk4::accessible::Property::Label(text)]);
    w.set_tooltip_text(Some(text));
}

pub fn icon(name: &str, px: i32) -> gtk4::Image {
    let i = gtk4::Image::from_icon_name(name);
    i.set_pixel_size(px);
    i.set_accessible_role(gtk4::AccessibleRole::Presentation);
    i
}

/// An icon on a tinted rounded square (tiles, rows).
pub fn chip(name: &str, tone: Tone, px: i32) -> gtk4::Image {
    let i = icon(name, px);
    i.add_css_class("chip");
    i.add_css_class(tone.class());
    i.set_halign(gtk4::Align::Start);
    i.set_valign(gtk4::Align::Center);
    i
}

/// The big round icon at the top of a page.
pub fn hero(name: &str, tone: Tone) -> gtk4::Image {
    let i = icon(name, 36);
    i.add_css_class("hero-badge");
    i.add_css_class(tone.class());
    i.set_halign(gtk4::Align::Center);
    i
}

/// A button with an icon before its words.
pub fn button(icon_name: &str, text: &str, classes: &[&str]) -> gtk4::Button {
    let content = adw::ButtonContent::builder()
        .icon_name(icon_name)
        .label(text)
        .use_underline(false)
        .build();
    gtk4::Button::builder()
        .child(&content)
        .css_classes(classes.to_vec())
        .build()
}

/// Changes the words of a [`button`].
pub fn set_button_text(b: &gtk4::Button, text: &str) {
    match b.child().and_downcast::<adw::ButtonContent>() {
        Some(c) => c.set_label(text),
        None => b.set_label(text),
    }
}

/// A text label; `center` for headings, left for reading text.
pub fn text(s: &str, classes: &[&str], center: bool) -> gtk4::Label {
    gtk4::Label::builder()
        .label(s)
        .css_classes(classes.to_vec())
        .wrap(true)
        .wrap_mode(gtk4::pango::WrapMode::WordChar)
        .justify(if center {
            gtk4::Justification::Center
        } else {
            gtk4::Justification::Left
        })
        .xalign(if center { 0.5 } else { 0.0 })
        .max_width_chars(58)
        .build()
}

/// A note with an icon in front: `kind` is "", "warn" or "bad".
pub struct Note {
    pub root: gtk4::Box,
    pub label: gtk4::Label,
    icon: gtk4::Image,
}

impl Note {
    pub fn new(kind: &str, s: &str) -> Self {
        let root = gtk4::Box::builder()
            .spacing(12)
            .css_classes(["note"])
            .build();
        let icon = icon("", 20);
        icon.set_valign(gtk4::Align::Start);
        let label = text(s, &[], false);
        label.set_hexpand(true);
        root.append(&icon);
        root.append(&label);
        let n = Note { root, label, icon };
        n.kind(kind);
        n.root.set_visible(!s.is_empty());
        n
    }

    pub fn kind(&self, kind: &str) {
        for k in ["warn", "bad"] {
            self.root.remove_css_class(k);
        }
        let (name, tone) = match kind {
            "warn" => ("dialog-warning-symbolic", Tone::Warn),
            "bad" => ("dialog-error-symbolic", Tone::Bad),
            "pencil" => ("document-edit-symbolic", Tone::Blue),
            "ok" => ("emblem-ok-symbolic", Tone::Ok),
            _ => ("dialog-information-symbolic", Tone::Blue),
        };
        if matches!(kind, "warn" | "bad") {
            self.root.add_css_class(kind);
        }
        self.icon.set_icon_name(Some(name));
        tone.apply(&self.icon);
    }

    /// Shows `s` as a note of `kind` (hidden when empty).
    pub fn say(&self, kind: &str, s: &str) {
        self.kind(kind);
        self.set(s);
    }

    /// Shows `s` (hidden when empty).
    pub fn set(&self, s: &str) {
        self.label.set_label(s);
        self.root.set_visible(!s.is_empty());
    }
}

/// "Step 2 of 4" with dots, for the first-start pages.
pub fn steps(now: usize, of: usize, words: &str) -> gtk4::Box {
    let b = gtk4::Box::builder()
        .spacing(8)
        .halign(gtk4::Align::Center)
        .build();
    let dots = gtk4::Box::builder()
        .spacing(6)
        .valign(gtk4::Align::Center)
        .build();
    dots.set_accessible_role(gtk4::AccessibleRole::Presentation);
    for i in 1..=of {
        let d = gtk4::Box::builder().css_classes(["step-dot"]).build();
        if i < now {
            d.add_css_class("done");
        } else if i == now {
            d.add_css_class("now");
        }
        dots.append(&d);
    }
    b.append(&dots);
    b.append(
        &gtk4::Label::builder()
            .label(words)
            .css_classes(["caption-up"])
            .build(),
    );
    b
}

/// A centred card, at most `width` wide, that holds a page's content.
pub fn panel(children: &[&gtk4::Widget], width: i32) -> adw::Clamp {
    let col = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(16)
        .css_classes(["panel"])
        .build();
    for c in children {
        col.append(*c);
    }
    adw::Clamp::builder()
        .maximum_size(width)
        .tightening_threshold(width)
        .child(&col)
        .valign(gtk4::Align::Center)
        .margin_top(20)
        .margin_bottom(20)
        .margin_start(24)
        .margin_end(24)
        .build()
}

/// Lets a page scroll when the screen is small (1366 x 768 laptops,
/// large text).
pub fn scroll(child: &impl IsA<gtk4::Widget>) -> gtk4::ScrolledWindow {
    gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(child)
        .vexpand(true)
        .build()
}

/// A labelled field: the label above, spoken as the field's name.
pub fn field(label: &str, entry: &(impl IsA<gtk4::Widget> + IsA<gtk4::Accessible>)) -> gtk4::Box {
    let b = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .build();
    let l = text(label, &["field-label"], false);
    l.set_mnemonic_widget(Some(entry));
    entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[l.upcast_ref()])]);
    b.append(&l);
    b.append(entry);
    b
}
