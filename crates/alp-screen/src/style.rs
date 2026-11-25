// SPDX-License-Identifier: AGPL-3.0-or-later
//! The screen's look (0.2.8 design pass 2): calm and plain, inspired by
//! the desktop computers of 2016-2020: a wallpaper behind everything,
//! light windows with dark text, filled rounded colour icons, a thin top
//! bar, rounded-rectangle buttons with one blue for the main action.
//! Inspiration only: no other system's images, icons or names are used.
//!
//! Colours and the wallpaper are a [`Theme`], so a later version can offer
//! other colour sets and a picture of the user's own as the background
//! (owner, 2026-11-25). Everything else reads the theme's colours.
//!
//! Accessibility rules kept here:
//! - Dark text on light windows (contrast at least 4.5:1; grey text is
//!   #5f5f66 or darker); white text only on the darkened top bar and on
//!   the sign-in page, which has its own shade over the wallpaper.
//! - A colour never stands alone: every status has an icon and words.
//! - Buttons and fields are at least 44 px high; base text 12 pt.
//! - A clear 3 px focus ring for the keyboard.
//! - Icon-only controls carry a spoken name ([`name`]).
//!
//! Icons are the Adwaita symbolic set (package adwaita-icon-theme).

use gtk4::prelude::*;
use libadwaita as adw;

/// A colour set and background. Only [`Theme::DEFAULT`] exists in 0.2.8.
pub struct Theme {
    /// CSS `background-image` of the whole screen: a gradient now, later
    /// also `url("file:///...")` for the user's own picture.
    pub wallpaper: &'static str,
    /// The colour of main buttons, ticks and focus rings (white text on it
    /// must reach 4.5:1).
    pub accent: &'static str,
}

impl Theme {
    pub const DEFAULT: Theme = Theme {
        wallpaper: "radial-gradient(circle at 18% 12%, rgba(255,255,255,0.28), rgba(255,255,255,0) 42%), \
                    radial-gradient(circle at 85% 90%, rgba(120,90,220,0.45), rgba(120,90,220,0) 50%), \
                    linear-gradient(160deg, #4a8ee6 0%, #2f5bbf 45%, #1d2d6b 100%)",
        accent: "#0a62d9",
    };
}

const CSS: &str = r#"
@define-color accent_bg_color @ACCENT@;
@define-color accent_fg_color #ffffff;
@define-color accent_color @ACCENT@;
@define-color window_bg_color #f2f2f5;
@define-color window_fg_color #1d1d1f;
@define-color view_bg_color #ffffff;
@define-color view_fg_color #1d1d1f;
@define-color card_bg_color #ffffff;
@define-color card_fg_color #1d1d1f;
@define-color dialog_bg_color #f6f6f8;
@define-color dialog_fg_color #1d1d1f;
@define-color popover_bg_color #ffffff;
@define-color popover_fg_color #1d1d1f;
@define-color destructive_bg_color #d70015;
@define-color destructive_fg_color #ffffff;
@define-color destructive_color #c4000f;
@define-color error_color #c4000f;
@define-color warning_color #9a4a00;
@define-color success_color #1a7f37;

window.alp {
  background-color: #2f5bbf;
  background-image: @WALLPAPER@;
  color: #1d1d1f;
  font-size: 12pt;
}

/* Text */
.dim-label { opacity: 1; color: #5f5f66; }
.alp .title-1 { font-size: 22pt; font-weight: 700; }
.alp .title-2 { font-size: 17pt; font-weight: 700; }
.alp .title-3 { font-size: 14pt; font-weight: 600; }
.lead { font-size: 12.5pt; color: #3a3a40; }
.caption-up { font-size: 10pt; font-weight: 600; color: #5f5f66; }
.big-number { font-size: 46pt; font-weight: 700; }
.error { color: #c4000f; }
.warning { color: #9a4a00; }
.on-wallpaper, .on-wallpaper label { color: #ffffff; }

/* Top bar: thin, darkened, monochrome icons */
.topbar { background-color: rgba(10,16,40,0.45); min-height: 36px; padding: 0 10px; color: #ffffff; }
.topbar label { color: #ffffff; }
.topbar .brand { font-weight: 700; }
.topbar .tag { background-color: rgba(255,255,255,0.22); color: #ffffff; border-radius: 6px;
  padding: 1px 8px; font-size: 9.5pt; font-weight: 700; }
.topbar .ip { font-family: monospace; font-size: 11pt; }
.topbar .stat { padding: 0 6px; color: #ffffff; }
.topbar .stat.tone-warn { color: #ffd479; }
.topbar .stat.tone-bad { color: #ff9b94; }
.topbar button { min-height: 30px; min-width: 30px; padding: 0 8px; border-radius: 6px;
  background: transparent; color: #ffffff; box-shadow: none; }
.topbar button:hover { background-color: rgba(255,255,255,0.18); }
.topbar .home-btn { background-color: rgba(255,255,255,0.16); }
.topbar *:focus-visible { outline-color: #ffffff; }
.topbar .tone-ok { color: #ffffff; }
.topbar .tone-warn { color: #ffd479; }
.topbar .tone-bad { color: #ff9b94; }

/* Status colours (text and glyphs) */
.tone-ok { color: #1a7f37; }
.tone-warn { color: #9a4a00; }
.tone-bad { color: #c4000f; }
.tone-blue { color: @ACCENT@; }
.tone-violet { color: #7b3fc4; }
.tone-teal { color: #0b7a75; }

/* Filled rounded icons, white glyph on colour */
.chip { border-radius: 9px; padding: 8px; color: #ffffff; }
.hero-badge { border-radius: 18px; padding: 16px; color: #ffffff; }
.chip.tone-ok, .hero-badge.tone-ok { background-color: #2a9d4b; color: #ffffff; }
.chip.tone-warn, .hero-badge.tone-warn { background-color: #e08600; color: #ffffff; }
.chip.tone-bad, .hero-badge.tone-bad { background-color: #e0262f; color: #ffffff; }
.chip.tone-blue, .hero-badge.tone-blue { background-color: #1f7aeb; color: #ffffff; }
.chip.tone-violet, .hero-badge.tone-violet { background-color: #8a4fd8; color: #ffffff; }
.chip.tone-teal, .hero-badge.tone-teal { background-color: #109c96; color: #ffffff; }

/* Windows: light, rounded, a soft shadow over the wallpaper */
.alp .card, .panel, .window-card { background-color: #f6f6f8; color: #1d1d1f; border-radius: 12px;
  border: 1px solid rgba(0,0,0,0.10); box-shadow: 0 12px 36px rgba(0,0,0,0.28); }
.panel { padding: 28px 36px; }
.window-card { padding: 0; }
.tile-photos { background-color: #ffffff; }

/* Notes with an icon in front (info, warning, danger) */
.note { border-radius: 8px; padding: 10px 12px; background-color: #e9f1fe; color: #1d1d1f; }
.note.warn { background-color: #fff4dc; border: 1px solid #f1c76b; }
.note.bad { background-color: #ffeceb; border: 1px solid #f2a29d; }

/* Buttons: rounded rectangles; one blue main button */
.alp button { min-height: 44px; border-radius: 8px; font-weight: 600; }
.alp button.pill { border-radius: 8px; padding-left: 20px; padding-right: 20px; }
.alp button:not(.flat):not(.suggested-action):not(.destructive-action) {
  background-color: #ffffff; color: #1d1d1f; border: 1px solid rgba(0,0,0,0.16);
  box-shadow: 0 1px 1px rgba(0,0,0,0.06); }
.alp button:not(.flat):not(.suggested-action):not(.destructive-action):hover { background-color: #f0f0f3; }
.alp button.suggested-action { background-color: @ACCENT@; color: #ffffff; }
.alp button.suggested-action:disabled { background-color: #a9c3ea; color: #ffffff; }
.alp button.destructive-action { background-color: #d70015; color: #ffffff; }
.alp button.destructive-action:disabled { background-color: #eba3a9; color: #ffffff; }
.alp button.flat { color: @ACCENT@; }
.alp .topbar button.flat { color: #ffffff; }

/* Fields */
.alp entry, .alp passwordentry { min-height: 44px; font-size: 13pt; background-color: #ffffff;
  color: #1d1d1f; border-radius: 8px; border: 1px solid #b9b9c0; }
.field-label { font-weight: 600; }

/* Keyboard focus */
.alp *:focus-visible { outline: 3px solid alpha(@ACCENT@, 0.75); outline-offset: 2px; }

/* Progress */
.alp progressbar trough { min-height: 8px; border-radius: 99px; background-color: #dcdce2; }
.alp progressbar progress { min-height: 8px; border-radius: 99px; background-color: @ACCENT@; }
.alp progressbar text { color: #5f5f66; }

/* Steps of the first start */
.step-dot { min-width: 8px; min-height: 8px; border-radius: 99px; background-color: #c9c9d0; }
.step-dot.done { background-color: #7fa9e8; }
.step-dot.now { background-color: @ACCENT@; min-width: 22px; }

/* The recovery words: a note sheet, dark on pale yellow */
.paper { background-color: #fffbe8; color: #1d1d1f; border-radius: 10px; padding: 20px 26px;
  border: 1px solid #ecdca0; }
.paper .word-n { color: #6b6250; font-size: 12pt; font-weight: 600; }
.paper .word { font-family: monospace; font-size: 17pt; font-weight: 700; color: #111111; }
.paper.qr { background-color: #ffffff; border-color: #d6d6dc; padding: 10px; }
.paper .code-big { font-size: 32pt; letter-spacing: 4px; }
.word-n-chip { background-color: #e6e6eb; border-radius: 8px; padding: 6px 12px; font-weight: 700; }

/* Sign-in page: on the wallpaper, like a login screen */
.clock-big { font-size: 58pt; font-weight: 300; color: #ffffff; }
.date-line { font-size: 15pt; color: #ffffff; }
.avatar { border-radius: 999px; padding: 26px; background-color: rgba(255,255,255,0.28); color: #ffffff; }
.login-name { font-size: 18pt; font-weight: 600; color: #ffffff; }
.login-status { color: #ffffff; }
.login-field { min-height: 40px; border-radius: 99px; background-color: rgba(255,255,255,0.88);
  border: none; padding-left: 8px; }
.login-go { min-width: 44px; min-height: 44px; border-radius: 99px; padding: 0; }

/* Lists inside light windows */
.alp preferencespage, .alp preferencespage > scrolledwindow, .alp preferencespage viewport { background: transparent; }
.alp .boxed-list { background-color: #ffffff; border-radius: 10px; }
.alp row { min-height: 52px; }
.alp row.danger-row { background-color: #fff0ef; }
.footer-bar { background-color: #ececf0; padding: 12px 20px; border-top: 1px solid rgba(0,0,0,0.08);
  border-radius: 0 0 12px 12px; }
.window-title { padding: 16px 20px 4px 20px; }
"#;

/// Loads the look for the whole screen (once, at start).
pub fn install(theme: &Theme) {
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
    let css = gtk4::CssProvider::new();
    css.load_from_string(
        &CSS.replace("@ACCENT@", theme.accent)
            .replace("@WALLPAPER@", theme.wallpaper),
    );
    if let Some(d) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &d,
            &css,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// A light window over the wallpaper holding `child` (settings, install),
/// at most `width` wide, filling the height.
pub fn window(child: &impl IsA<gtk4::Widget>, width: i32) -> adw::Clamp {
    let card = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .css_classes(["window-card"])
        .overflow(gtk4::Overflow::Hidden)
        .vexpand(true)
        .build();
    card.append(child);
    adw::Clamp::builder()
        .maximum_size(width)
        .tightening_threshold(width)
        .child(&card)
        .vexpand(true)
        .margin_top(20)
        .margin_bottom(20)
        .margin_start(24)
        .margin_end(24)
        .build()
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
