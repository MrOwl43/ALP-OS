// SPDX-License-Identifier: AGPL-3.0-or-later
//! Contract checks for errors/catalog.toml (spec Section 2.2, block 10):
//! every entry is well-formed; every code ever defined stays defined
//! (errors/codes.lock); every ALP-<letters>-<digits> token in any tracked
//! file is a valid, defined, non-retired code.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

const AREAS: [&str; 10] = [
    "INST", "BOOT", "UPD", "STO", "KEY", "NET", "PAIR", "APP", "PWR", "HW",
];
/// ADR 0023: the four levels of the (i) icon.
const SEVERITIES: [&str; 4] = ["note", "action", "now", "critical"];
const CATALOG: &str = "errors/catalog.toml";
const LOCK: &str = "errors/codes.lock";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    severity: String,
    title: String,
    action: String,
    technician: String,
    #[serde(default)]
    retired: bool,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn catalog() -> BTreeMap<String, Entry> {
    let text = std::fs::read_to_string(repo_root().join(CATALOG)).expect("catalog readable");
    // Duplicate keys are a TOML parse error, so "defined once" holds here.
    toml::from_str(&text).expect("errors/catalog.toml is valid and well-formed")
}

fn is_code(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 3
        && parts[0] == "ALP"
        && AREAS.contains(&parts[1])
        && parts[2].len() == 3
        && parts[2].bytes().all(|b| b.is_ascii_digit())
}

/// Every `ALP-<UPPERCASE LETTERS>-<DIGITS>` token in `text`.
fn code_like_tokens(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (i, _) in text.match_indices("ALP-") {
        let rest = &text[i + 4..];
        let letters = rest.bytes().take_while(u8::is_ascii_uppercase).count();
        if letters == 0 || rest.as_bytes().get(letters) != Some(&b'-') {
            continue;
        }
        let digits = rest[letters + 1..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if digits > 0 {
            found.push(text[i..i + 4 + letters + 1 + digits].to_string());
        }
    }
    found
}

/// Tracked text files (git), or a directory walk outside a git checkout.
fn tracked_files(root: &Path) -> Vec<PathBuf> {
    if let Ok(out) = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        && out.status.success()
    {
        return out
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| root.join(String::from_utf8_lossy(p).as_ref()))
            .collect();
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    files
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("readable dir") {
        let path = entry.expect("dir entry").path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if path.is_dir() {
            if !matches!(name.as_ref(), ".git" | "target" | "out") {
                walk(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

#[test]
fn every_entry_is_well_formed() {
    let cat = catalog();
    assert!(!cat.is_empty());
    for (code, e) in &cat {
        assert!(is_code(code), "{code}: not of the form ALP-<AREA>-<NNN>");
        assert!(
            SEVERITIES.contains(&e.severity.as_str()),
            "{code}: bad severity"
        );
        for (field, value) in [
            ("title", &e.title),
            ("action", &e.action),
            ("technician", &e.technician),
        ] {
            assert!(!value.trim().is_empty(), "{code}: empty {field}");
        }
    }
}

#[test]
fn codes_are_never_removed_and_every_code_is_locked() {
    let cat = catalog();
    let lock_text = std::fs::read_to_string(repo_root().join(LOCK)).expect("codes.lock readable");
    let lock: BTreeSet<&str> = lock_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    for code in &lock {
        assert!(
            cat.contains_key(*code),
            "{code} was removed from {CATALOG}; codes are never removed (mark it retired)"
        );
    }
    for code in cat.keys() {
        assert!(
            lock.contains(code.as_str()),
            "{code} is missing from {LOCK}; append it"
        );
    }
}

#[test]
fn every_code_used_anywhere_is_valid_defined_and_not_retired() {
    let cat = catalog();
    let root = repo_root();
    let mut problems = Vec::new();
    for file in tracked_files(&root) {
        let rel = file.strip_prefix(&root).unwrap_or(&file).to_path_buf();
        if rel == Path::new(CATALOG) || rel == Path::new(LOCK) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue; // binary or unreadable
        };
        for token in code_like_tokens(&text) {
            match cat.get(&token) {
                _ if !is_code(&token) => {
                    problems.push(format!("{}: malformed code {token}", rel.display()))
                }
                None => problems.push(format!("{}: {token} is not defined", rel.display())),
                Some(e) if e.retired => {
                    problems.push(format!("{}: {token} is retired", rel.display()))
                }
                Some(_) => {}
            }
        }
    }
    assert!(
        problems.is_empty(),
        "error code problems:\n{}",
        problems.join("\n")
    );
}

#[test]
fn token_scanner_catches_malformed_codes() {
    // Built at run time so this file does not itself contain malformed codes.
    let p = "ALP-";
    let text = format!("x {p}STO-01 {p}STOR-001 {p}OS {p}INST-* {p}<AREA>-<NNN> {p}HW-001.");
    let t = code_like_tokens(&text);
    let want = [
        format!("{p}STO-01"),
        format!("{p}STOR-001"),
        format!("{p}HW-001"),
    ];
    assert_eq!(t, want);
    assert!(!is_code(&t[0]) && !is_code(&t[1]) && is_code(&t[2]));
}
