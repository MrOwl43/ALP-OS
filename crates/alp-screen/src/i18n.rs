// SPDX-License-Identifier: AGPL-3.0-or-later
//! The screen's texts: the same `key -> text` files as the web console
//! (`ui/live/i18n/<lang>.json`), so wording and translations stay in one
//! place. `{name}` placeholders are filled by [`Texts::fill`].

use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Where the image installs the console's texts.
pub const DIR: &str = "/usr/share/alp/ui/live/i18n";

#[derive(Default)]
pub struct Texts {
    map: HashMap<String, String>,
}

impl Texts {
    /// Parses one language file. Non-text values are ignored.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let obj = v.as_object().ok_or("the texts file is not an object")?;
        Ok(Texts {
            map: obj
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect(),
        })
    }

    /// The language's file from `dir`, falling back to English.
    pub fn load(dir: &Path, lang: &str) -> Self {
        let read = |l: &str| {
            std::fs::read_to_string(dir.join(format!("{l}.json")))
                .ok()
                .and_then(|t| Self::from_json(&t).ok())
        };
        let mut texts = read("en").unwrap_or_default();
        if lang != "en"
            && let Some(other) = read(lang)
        {
            texts.map.extend(other.map);
        }
        texts
    }

    /// The text for `key`; the key itself when it is missing, so a gap is
    /// visible in testing rather than an empty label.
    pub fn t(&self, key: &str) -> String {
        self.map
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    /// The text with `{name}` placeholders replaced.
    pub fn fill(&self, key: &str, values: &[(&str, &str)]) -> String {
        let mut s = self.t(key);
        for (k, v) in values {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }
}

/// Languages written right to left (the layout mirrors for them).
pub fn is_rtl(lang: &str) -> bool {
    let base = lang
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "ar" | "he" | "fa" | "ur" | "ps" | "yi" | "dv" | "ckb" | "sd" | "ug"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_to_left_languages() {
        for l in ["ar", "he", "fa", "ar-EG", "HE_il", "ckb"] {
            assert!(is_rtl(l), "{l}");
        }
        for l in ["en", "de", "", "arn", "en-US"] {
            assert!(!is_rtl(l), "{l}");
        }
    }

    #[test]
    fn reads_the_console_texts() {
        let t = Texts::from_json(include_str!("../../../ui/live/i18n/en.json")).unwrap();
        assert_eq!(t.t("lock.msg.ok"), "All good");
        assert_eq!(t.t("lock.signin"), "Sign in");
        assert_eq!(t.t("no.such.key"), "no.such.key");
    }

    #[test]
    fn fills_placeholders() {
        let t = Texts::from_json(r#"{"a": "Wait {n} seconds", "x": 1}"#).unwrap();
        assert_eq!(t.fill("a", &[("n", "30")]), "Wait 30 seconds");
        assert_eq!(t.t("x"), "x");
    }
}
