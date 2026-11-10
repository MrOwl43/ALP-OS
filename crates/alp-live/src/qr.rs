// SPDX-License-Identifier: AGPL-3.0-or-later
//! QR codes as SVG (qrcodegen) for the phone address and the chain words.

use std::fmt::Write as _;

use qrcodegen::{QrCode, QrCodeEcc};

/// An SVG drawing of `text` as a QR code with a 4-module quiet zone, black
/// on white, scalable (the UI sets the size).
pub fn svg(text: &str) -> Option<String> {
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let border = 4;
    let size = qr.size() + border * 2;
    let mut path = String::new();
    for y in 0..qr.size() {
        for x in 0..qr.size() {
            if qr.get_module(x, y) {
                let _ = write!(path, "M{},{}h1v1h-1z", x + border, y + border);
            }
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" shape-rendering=\"crispEdges\"><rect width=\"100%\" height=\"100%\" fill=\"#fff\"/><path d=\"{path}\" fill=\"#000\"/></svg>"
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn makes_an_svg() {
        let s = super::svg("https://192.168.1.23").unwrap();
        assert!(s.starts_with("<svg") && s.ends_with("</svg>"));
        // Version 2 (25 modules) plus the quiet zone.
        assert!(s.contains("viewBox=\"0 0 33 33\""), "{s}");
    }
}
