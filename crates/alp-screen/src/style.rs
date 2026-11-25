// SPDX-License-Identifier: AGPL-3.0-or-later
//! The screen's look (0.2.8 design pass 3): calm, plain and modern, in
//! the spirit of the 0.2.6 console (owner, 2026-11-25): a deep blue
//! wallpaper; the home and sign-in pages sit right on it with frosted
//! glass tiles and white words; settings, install and setup open in white
//! windows with large round corners and a soft shadow; soft tinted icons
//! at the top of a page, filled colour icons in lists; one blue for the
//! main action. Inspiration only: no other system's images, icons or
//! names are used.
//!
//! Colours and the wallpaper are a [`Theme`], so a later version can offer
//! other colour sets and a picture of the user's own as the background
//! (owner, 2026-11-25). Everything else reads the theme's colours.
//!
//! Accessibility rules kept here:
//! - Dark text on white windows (contrast at least 4.5:1; grey text is
//!   #5c5c66 or darker); white text only on the deep blue wallpaper and
//!   its glass tiles (top bar, home, sign-in).
//! - A colour never stands alone: every status has an icon and words.
//! - Buttons and fields are at least 44 px high (main buttons 48); base
//!   text 12 pt.
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
        wallpaper: "radial-gradient(circle at 15% 8%, rgba(120,170,255,0.45), rgba(120,170,255,0) 45%), \
                    radial-gradient(circle at 90% 95%, rgba(90,70,200,0.40), rgba(90,70,200,0) 50%), \
                    linear-gradient(165deg, #2457e0 0%, #1a3fae 50%, #0f1f5c 100%)",
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
  background-color: #1a3fae;
  background-image: @WALLPAPER@;
  color: #1d1d1f;
  font-size: 12pt;
}

/* Text */
.dim-label { opacity: 1; color: #5c5c66; }
.alp .title-1 { font-size: 23pt; font-weight: 700; letter-spacing: -0.2px; }
.alp .title-2 { font-size: 17pt; font-weight: 700; }
.alp .title-3 { font-size: 14pt; font-weight: 600; }
.lead { font-size: 12.5pt; color: #3a3a42; }
.caption-up { font-size: 10pt; font-weight: 600; color: #5c5c66; }
.big-number { font-size: 46pt; font-weight: 700; letter-spacing: -1px; }
.error { color: #c4000f; }
.warning { color: #9a4a00; }
.on-wallpaper, .on-wallpaper label { color: #ffffff; }
.on-wallpaper .lead { color: rgba(255,255,255,0.88); }

/* Top bar: thin and see-through over the wallpaper, white icons */
.topbar { background-color: rgba(6,12,40,0.28); min-height: 40px; padding: 0 12px; color: #ffffff; }
.topbar label { color: #ffffff; }
.topbar .brand { font-weight: 700; }
.topbar .tag { background-color: #fff1d6; color: #8a4b00; border-radius: 99px;
  padding: 1px 10px; font-size: 9.5pt; font-weight: 700; }
.topbar .ip { font-family: monospace; font-size: 11pt; }
.topbar .stat { padding: 0 6px; color: #ffffff; }
.topbar .stat.tone-warn { color: #ffd479; }
.topbar .stat.tone-bad { color: #ff9b94; }
.topbar button { min-height: 32px; min-width: 32px; padding: 0 10px; border-radius: 10px;
  background: transparent; color: #ffffff; box-shadow: none; border: none; }
.topbar button:hover { background-color: rgba(255,255,255,0.18); }
.topbar .home-btn { background-color: rgba(255,255,255,0.18); }
.topbar *:focus:focus-visible { outline-color: #ffffff; }
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

/* Filled rounded icons in lists and tiles, white glyph on colour */
.chip { border-radius: 11px; padding: 8px; color: #ffffff; }
.chip.tone-ok { background-color: #2a9d4b; color: #ffffff; }
.chip.tone-warn { background-color: #e08600; color: #ffffff; }
.chip.tone-bad { background-color: #e0262f; color: #ffffff; }
.chip.tone-blue { background-color: #1f7aeb; color: #ffffff; }
.chip.tone-violet { background-color: #8a4fd8; color: #ffffff; }
.chip.tone-teal { background-color: #109c96; color: #ffffff; }
.chip.tone-grey { background-color: #6e7380; color: #ffffff; }
.tone-grey { color: #55595f; }
.hero-badge.tone-grey { background-color: #eceef3; color: #55595f; }

/* The icon at the top of a page: soft tint, coloured glyph */
.hero-badge { border-radius: 24px; padding: 20px; }
.hero-badge.tone-ok { background-color: #e3f4e8; color: #1a7f37; }
.hero-badge.tone-warn { background-color: #fdf0d9; color: #9a4a00; }
.hero-badge.tone-bad { background-color: #fde6e4; color: #c4000f; }
.hero-badge.tone-blue { background-color: #e6f0fd; color: @ACCENT@; }
.hero-badge.tone-violet { background-color: #f0e8fc; color: #7b3fc4; }
.hero-badge.tone-teal { background-color: #e0f4f2; color: #0b7a75; }
/* On the wallpaper: a white disc with the coloured glyph */
.hero-badge.ring { border-radius: 999px; padding: 16px; background-color: #ffffff;
  box-shadow: 0 8px 24px rgba(0,0,0,0.25); }
.hero-badge.ring.tone-ok { background-color: #ffffff; color: #1a7f37; }
.hero-badge.ring.tone-warn { background-color: #ffffff; color: #b35c00; }
.hero-badge.ring.tone-bad { background-color: #ffffff; color: #c4000f; }

/* White windows: large round corners, a soft deep shadow, no frame */
.alp .card, .panel, .window-card { background-color: #ffffff; color: #1d1d1f; border-radius: 22px;
  border: none; box-shadow: 0 1px 2px rgba(0,0,0,0.06), 0 22px 56px rgba(8,16,60,0.35); }
.panel { padding: 32px 40px; }
.window-card { padding: 0; background-color: #f7f7fa; }

/* Frosted glass tiles on the wallpaper (home, sign-in) */
.glass { background-color: rgba(255,255,255,0.14); border-radius: 20px;
  border: 1px solid rgba(255,255,255,0.20); color: #ffffff; }
.glass label { color: #ffffff; }
.glass .caption-up, .glass .dim-label { color: rgba(255,255,255,0.82); }
.glass .warning { color: #ffd479; }
.glass.tile-photos { background-color: rgba(255,255,255,0.20); }

/* Notes: soft colour with a stripe in front */
.note { border-radius: 14px; padding: 12px 14px; background-color: #eef4fe; color: #1d1d1f;
  border-left: 4px solid #8db6f2; }
.note.warn { background-color: #fdf1dc; border-left: 4px solid #e08600; }
.note.bad { background-color: #fce8e6; border-left: 4px solid #e0262f; }
.on-wallpaper .note.bad, .login .note.bad { background-color: #ffffff; }

/* Buttons: round corners; one blue main button; soft grey others */
.alp button { min-height: 34px; padding-top: 5px; padding-bottom: 5px; border-radius: 12px; font-weight: 600; }
.alp button.pill { border-radius: 14px; padding: 8px 26px; min-height: 32px; }
.alp button:not(.flat):not(.suggested-action):not(.destructive-action):not(.glass-btn) {
  background-color: #eceef3; color: #1d1d1f; border: none; box-shadow: none; }
.alp button:not(.flat):not(.suggested-action):not(.destructive-action):not(.glass-btn):hover { background-color: #e1e4eb; }
.alp row button:not(.flat):not(.suggested-action):not(.destructive-action) { background-color: #e6f0fd; color: @ACCENT@; }
.alp row button:not(.flat):not(.suggested-action):not(.destructive-action):hover { background-color: #d6e6fb; }
.alp row button.destructive-action { background-color: #fde6e4; color: #c4000f; }
.alp row button.suggested-action { background-color: #e6f0fd; color: @ACCENT@; }
.alp button.suggested-action { background-color: @ACCENT@; color: #ffffff; box-shadow: none; }
.alp button.suggested-action:disabled { background-color: #b5cdf0; color: #ffffff; box-shadow: none; }
.alp button.destructive-action { background-color: #d70015; color: #ffffff; }
.alp button.destructive-action:disabled { background-color: #eba3a9; color: #ffffff; }
.alp button.flat { color: @ACCENT@; }
.alp .topbar button.flat { color: #ffffff; }
/* On the wallpaper: the main button is white, the others are glass */
.alp button.suggested-action.light-btn { background-color: #ffffff; color: @ACCENT@;
  box-shadow: none; }
.on-wallpaper button.light-btn label, .on-wallpaper button.light-btn image { color: @ACCENT@; }
.alp button.glass-btn { background-color: rgba(255,255,255,0.16); color: #ffffff;
  border: 1px solid rgba(255,255,255,0.28); box-shadow: none; }
.alp button.glass-btn:hover { background-color: rgba(255,255,255,0.26); }

/* Fields */
.alp entry, .alp passwordentry { min-height: 46px; font-size: 13pt; background-color: #ffffff;
  color: #1d1d1f; border-radius: 12px; border: 2px solid #d9dce3; box-shadow: none; }
.alp entry:focus-within, .alp passwordentry:focus-within { border-color: @ACCENT@; }
.field-label { font-weight: 600; }

/* Keyboard focus */
.alp *:focus:focus-visible { outline: 3px solid alpha(@ACCENT@, 0.75); outline-offset: 2px; }
.alp text:focus:focus-visible { outline: none; }
.alp entry:focus-within, .alp passwordentry:focus-within { outline: 3px solid alpha(@ACCENT@, 0.45); outline-offset: 1px; }

/* Progress */
.alp progressbar trough { min-height: 10px; border-radius: 99px; background-color: #e2e4ea; }
.alp progressbar progress { min-height: 10px; border-radius: 99px; background-color: @ACCENT@; }
.alp progressbar text { color: #5c5c66; }

/* Settings categories: a list on the left, as in the 0.2.6 console */
.cat-side { background-color: #eef0f4; border-right: 1px solid rgba(0,0,0,0.06); }
.cat-side viewport, .cat-list { background: transparent; }
.cat-list { padding: 14px 10px; }
.cat-list row { min-height: 40px; border-radius: 12px; padding: 0 10px; margin-bottom: 4px; }
.cat-list row label { font-size: 12.5pt; }
.cat-list row:hover { background-color: rgba(0,0,0,0.05); }
.cat-list row:selected { background-color: @ACCENT@; }
.cat-list row:selected label { color: #ffffff; font-weight: 600; }
.cat-list .chip { border-radius: 8px; padding: 6px; }

/* Slide to confirm */
.alp scale trough { min-height: 10px; border-radius: 99px; background-color: #e2e4ea; }
.alp scale highlight { border-radius: 99px; background-color: @ACCENT@; }
.alp scale slider { min-width: 30px; min-height: 30px; margin: -10px; background-color: #ffffff;
  border-radius: 99px; box-shadow: 0 1px 4px rgba(0,0,0,0.30); }

/* Disk kinds: a badge before the name; hard disks are recommended */
.kind-badge { font-size: 9.5pt; font-weight: 800; letter-spacing: 0.5px; border-radius: 7px;
  padding: 3px 8px; min-width: 34px; }
.kind-badge.kind-hdd { background-color: #e3f4e8; color: #17672e; }
.kind-badge.kind-ssd { background-color: #e6f0fd; color: #0a4fb0; }
.kind-badge.kind-usb, .kind-badge.kind-card { background-color: #f0e8fc; color: #6a2fb4; }
.good-tag { font-size: 10pt; font-weight: 700; background-color: #e3f4e8; color: #17672e;
  border-radius: 99px; padding: 3px 12px; }
.alp row.advice-row .subtitle { color: #8a4300; opacity: 1; }

/* Steps of the first start */
.step-dot { min-width: 8px; min-height: 8px; border-radius: 99px; background-color: #d5d7de; }
.step-dot.done { background-color: #8db6f2; }
.step-dot.now { background-color: @ACCENT@; min-width: 24px; }

/* The recovery words: a note sheet, dark on pale yellow */
.paper { background-color: #fffbe8; color: #1d1d1f; border-radius: 18px; padding: 22px 28px;
  border: 1px solid #f0e2a8; }
.paper .word-n { color: #6b6250; font-size: 12pt; font-weight: 600; }
.paper .word { font-family: monospace; font-size: 17pt; font-weight: 700; color: #111111; }
.paper entry.word-entry { font-family: monospace; font-size: 14pt; font-weight: 600; min-height: 38px;
  background-color: #ffffff; color: #111111; border-radius: 10px; border: 2px solid #eadfb4; }
.paper entry.word-entry:focus-within { border-color: @ACCENT@; }
.paper entry.word-entry.error { color: #c0141c; border-color: #e0262f; }
.paper.qr { background-color: #ffffff; border-color: #e2e4ea; padding: 12px; }
.paper .code-big { font-size: 32pt; letter-spacing: 4px; }
.word-n-chip { background-color: #eceef3; border-radius: 10px; padding: 6px 12px; font-weight: 700; }

/* Sign-in page: on the wallpaper, like a login screen */
.clock-big { font-size: 40pt; font-weight: 300; color: #ffffff; letter-spacing: -0.5px; }
.lock-status { font-size: 26pt; }
.lock-facts .title-3 { font-size: 18pt; }
.lock-facts .glass { min-height: 92px; }
.login-bar { background-color: rgba(255,255,255,0.14); border: 1px solid rgba(255,255,255,0.22);
  border-radius: 99px; padding: 8px 8px 8px 10px; }
.login-bar .avatar { padding: 10px; }
.login-bar .login-name { font-size: 13pt; margin-right: 6px; }
.date-line { font-size: 13pt; color: rgba(255,255,255,0.92); }
.avatar { border-radius: 999px; padding: 24px; background-color: rgba(255,255,255,0.22);
  color: #ffffff; border: 1px solid rgba(255,255,255,0.30); }
.login-name { font-size: 18pt; font-weight: 600; color: #ffffff; }
.login-status { color: rgba(255,255,255,0.92); }
.alp passwordentry.login-field { min-height: 44px; border-radius: 99px; background-color: rgba(255,255,255,0.92);
  border: 2px solid transparent; padding-left: 10px; }
.login-go { min-width: 46px; min-height: 46px; border-radius: 99px; padding: 0; }

/* Lists inside windows */
.alp preferencespage, .alp preferencespage > scrolledwindow, .alp preferencespage viewport { background: transparent; }
.alp .boxed-list { background-color: #ffffff; border-radius: 16px; border: none;
  box-shadow: 0 0 0 1px rgba(0,0,0,0.06), 0 2px 8px rgba(0,0,0,0.04); }
.alp row { min-height: 56px; }
.alp row.danger-row { background-color: #fff0ef; }
.footer-bar { background-color: #ffffff; padding: 14px 22px; border-top: 1px solid rgba(0,0,0,0.06);
  border-radius: 0 0 22px 22px; }
.window-title { padding: 20px 24px 6px 24px; }
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
    Grey,
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
            Tone::Grey => "tone-grey",
        }
    }

    pub const ALL: [Tone; 7] = [
        Tone::Ok,
        Tone::Warn,
        Tone::Bad,
        Tone::Blue,
        Tone::Violet,
        Tone::Teal,
        Tone::Grey,
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
