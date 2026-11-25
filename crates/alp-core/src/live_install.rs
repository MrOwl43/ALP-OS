// SPDX-License-Identifier: AGPL-3.0-or-later
//! Installing ALP-OS from the start stick (0.2.8, ADR 0028 decision 2), for
//! the native screen. `alpd` runs as root; the screen does not. So the
//! screen asks here, and this module runs the installer program
//! (`alp-installer`, crate `alp-installer`) with fixed arguments:
//!
//! - `GET install/disks`: every connected disk, greyed ones with a reason
//!   (`alp-installer disks`).
//! - `POST install/plan` `{"system": "sda" | null, "keep_running": bool,
//!   "data": ["sdb", ...]}`: what would be erased, with what is on it now
//!   (`alp-installer plan ...`). Writes nothing.
//! - `POST install/start`: the same choice plus `"confirm_erase"`, the
//!   exact list of disks the user saw on the confirm screen. The installer
//!   refuses unless it equals the plan's erase list, discovers the disks
//!   again and stops if anything changed (`ALP-INST-401`).
//!
//! Progress shows in `state.install`. Only one install runs at a time.
//! Disk names are kernel names checked here before they reach a command
//! line (letters and digits only), and the installer only uses names from
//! its own fresh discovery.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use alp_live::fail::Fail;
use serde_json::{Value, json};

/// Where the installer image ships the program.
pub const BIN: &str = "/usr/bin/alp-installer";

/// This image can install (the installer image ships the program).
pub fn available() -> bool {
    Path::new(BIN).exists()
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Job {
    running: bool,
    percent: u8,
    key: String,
    message: String,
    /// `ok`, or the `ALP-INST-*` code it stopped with.
    result: Option<String>,
    detail: String,
}

static JOB: Mutex<Option<Job>> = Mutex::new(None);

fn job() -> std::sync::MutexGuard<'static, Option<Job>> {
    JOB.lock().unwrap_or_else(|e| e.into_inner())
}

/// `state.install`.
pub fn state_json() -> Value {
    let j = job().clone();
    json!({
        "available": available(),
        "job": j.map(|j| json!({
            "running": j.running,
            "percent": j.percent,
            "key": j.key,
            "message": j.message,
            "result": j.result,
            "detail": j.detail,
        })),
    })
}

fn bad(msg: &str) -> Fail {
    Fail::new("request.invalid", msg.to_string())
}

/// A kernel disk name: `sda`, `nvme0n1`, `mmcblk0`. Nothing else reaches
/// the installer's command line.
fn disk_name(v: &Value) -> Result<String, Fail> {
    let s = v.as_str().ok_or_else(|| bad("A disk name is missing."))?;
    let ok = (2..=32).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if ok {
        Ok(s.to_string())
    } else {
        Err(bad("That is not a disk name."))
    }
}

fn names(v: &Value) -> Result<Vec<String>, Fail> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::Array(a) if a.len() <= 32 => a.iter().map(disk_name).collect(),
        _ => Err(bad("The disk list is not a list.")),
    }
}

/// The installer's arguments for a choice (`plan` or `install`).
fn choice_args(cmd: &str, b: &Value) -> Result<Vec<String>, Fail> {
    let keep = b["keep_running"].as_bool().unwrap_or(false);
    let mut args = Vec::new();
    match (&b["system"], keep) {
        (Value::Null, true) => {
            args.push(
                if cmd == "plan" {
                    "plan"
                } else {
                    "keep-running"
                }
                .to_string(),
            );
            if cmd == "plan" {
                args.push("--keep-running".into());
            }
        }
        (s, false) if !s.is_null() => {
            args.push(cmd.to_string());
            args.push("--system".into());
            args.push(disk_name(s)?);
        }
        _ => {
            return Err(bad(
                "Choose a disk for the system, or keep running from the stick.",
            ));
        }
    }
    for d in names(&b["data"])? {
        args.push("--data".into());
        args.push(d);
    }
    // Old archive disks to open again with the recovery words (never
    // erased; the installer checks they are archive disks).
    for d in names(&b["reopen"])? {
        args.push("--reopen".into());
        args.push(d);
    }
    Ok(args)
}

/// The installer refused: `RESULT <code>` on stdout, `<code>: detail` on
/// stderr.
fn refusal(stdout: &str, stderr: &str) -> Fail {
    let code = stdout
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("RESULT "))
        .unwrap_or("ALP-INST-400")
        .trim()
        .to_string();
    let detail = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("The installer stopped.");
    let detail = detail
        .strip_prefix(&format!("{code}: "))
        .unwrap_or(detail)
        .to_string();
    eprintln!("alpd: install: {code}: {detail}");
    Fail::new("install.refused", format!("{detail} ({code})"))
}

fn run_json(args: &[String]) -> Result<Value, Fail> {
    if !available() {
        return Err(Fail::new(
            "install.unavailable",
            "This system cannot install ALP-OS. Start from the ALP-OS stick.",
        ));
    }
    let out = Command::new(BIN)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            Fail::new(
                "install.refused",
                format!("The installer did not start: {e}"),
            )
        })?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return Err(refusal(&stdout, &String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_str(stdout.trim()).map_err(|_| {
        Fail::new(
            "install.refused",
            "The installer gave an unreadable answer.",
        )
    })
}

/// `GET install/disks`.
pub fn disks() -> Result<Value, Fail> {
    run_json(&["disks".to_string()])
}

/// `POST install/plan`.
pub fn plan(b: &Value) -> Result<Value, Fail> {
    run_json(&choice_args("plan", b)?)
}

/// One stdout line of a running install: `PROGRESS <pct> <i>/<n> <key> <message>`
/// or `RESULT <ok | code>`.
fn apply_line(j: &mut Job, line: &str) {
    if let Some(rest) = line.strip_prefix("PROGRESS ") {
        let mut it = rest.splitn(4, ' ');
        if let (Some(p), Some(_), Some(k)) = (it.next(), it.next(), it.next()) {
            j.percent = p.parse().unwrap_or(j.percent).min(100);
            j.key = k.to_string();
            j.message = it.next().unwrap_or_default().to_string();
        }
    } else if let Some(r) = line.strip_prefix("RESULT ") {
        j.result = Some(r.trim().to_string());
    }
}

/// `POST install/start`: starts the install and answers at once; progress
/// in `state.install`.
pub fn start(b: &Value) -> Result<Value, Fail> {
    let mut args = choice_args("install", b)?;
    let confirm = names(&b["confirm_erase"])?;
    if confirm.is_empty() && b["keep_running"].as_bool() != Some(true) {
        return Err(bad("Confirm which disks will be erased."));
    }
    args.push("--confirm-erase".into());
    args.push(confirm.join(","));
    // The plan's digest of what the Confirm page showed (the installer
    // recomputes it right before writing; ALP-INST-401 when it differs).
    let token = b["confirm_token"].as_str().unwrap_or_default();
    if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad("Confirm the disks again: the plan shown is missing."));
    }
    args.push("--confirm-token".into());
    args.push(token.to_string());
    if !available() {
        return Err(Fail::new(
            "install.unavailable",
            "This system cannot install ALP-OS. Start from the ALP-OS stick.",
        ));
    }
    {
        let mut g = job();
        if g.as_ref().is_some_and(|j| j.running) {
            return Err(Fail::new(
                "install.busy",
                "ALP-OS is already being installed.",
            ));
        }
        *g = Some(Job {
            running: true,
            key: "install.start".into(),
            message: "Starting…".into(),
            ..Job::default()
        });
    }
    let mut child = match Command::new(BIN)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            *job() = None;
            return Err(Fail::new(
                "install.refused",
                format!("The installer did not start: {e}"),
            ));
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    std::thread::spawn(move || {
        // The last stderr line explains a refusal; read it on its own
        // thread so a full pipe never blocks the installer.
        let err = std::thread::spawn(move || {
            let mut last = String::new();
            if let Some(e) = stderr {
                for l in BufReader::new(e).lines().map_while(Result::ok) {
                    eprintln!("alpd: installer: {l}");
                    if !l.trim().is_empty() {
                        last = l;
                    }
                }
            }
            last
        });
        if let Some(o) = stdout {
            for l in BufReader::new(o).lines().map_while(Result::ok) {
                if let Some(j) = job().as_mut() {
                    apply_line(j, &l);
                }
            }
        }
        let status = child.wait();
        let last = err.join().unwrap_or_default();
        let mut g = job();
        if let Some(j) = g.as_mut() {
            j.running = false;
            let ok = status.map(|s| s.success()).unwrap_or(false);
            match (&j.result, ok) {
                (Some(r), true) if r == "ok" => j.percent = 100,
                _ => {
                    let code = j.result.clone().unwrap_or_else(|| "ALP-INST-400".into());
                    j.detail = last
                        .strip_prefix(&format!("{code}: "))
                        .unwrap_or(&last)
                        .to_string();
                    j.result = Some(code);
                }
            }
        }
    });
    Ok(state_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_kernel_disk_names_reach_the_installer() {
        for ok in ["sda", "nvme0n1", "mmcblk0", "vdb"] {
            assert_eq!(disk_name(&json!(ok)).unwrap(), ok);
        }
        for bad in [
            "",
            "a",
            "-x",
            "--confirm-erase",
            "sda1/",
            "../sda",
            "SDA",
            "sd a",
            "1sda",
        ] {
            assert!(disk_name(&json!(bad)).is_err(), "{bad}");
        }
        assert!(disk_name(&json!(3)).is_err());
    }

    #[test]
    fn choices_become_fixed_arguments() {
        let a = choice_args("plan", &json!({"system": "sda", "data": ["sdb", "sde"]})).unwrap();
        assert_eq!(
            a,
            ["plan", "--system", "sda", "--data", "sdb", "--data", "sde"]
        );
        let a = choice_args("install", &json!({"system": "sda"})).unwrap();
        assert_eq!(a, ["install", "--system", "sda"]);
        let a = choice_args("plan", &json!({"keep_running": true, "data": ["sdb"]})).unwrap();
        assert_eq!(a, ["plan", "--keep-running", "--data", "sdb"]);
        let a = choice_args(
            "install",
            &json!({"system": "sda", "reopen": ["sdb", "sdc"]}),
        )
        .unwrap();
        assert_eq!(
            a,
            [
                "install", "--system", "sda", "--reopen", "sdb", "--reopen", "sdc"
            ]
        );
        assert!(choice_args("install", &json!({"system": "sda", "reopen": ["../x"]})).is_err());
        let a = choice_args("install", &json!({"keep_running": true, "data": ["sdb"]})).unwrap();
        assert_eq!(a, ["keep-running", "--data", "sdb"]);
        assert!(choice_args("plan", &json!({})).is_err());
        assert!(choice_args("plan", &json!({"system": "sda", "keep_running": true})).is_err());
        assert!(choice_args("plan", &json!({"system": "sda", "data": "sdb"})).is_err());
    }

    #[test]
    fn reads_the_installer_output() {
        let mut j = Job::default();
        apply_line(&mut j, "PROGRESS 40 3/7 install.copy Copying the system…");
        assert_eq!((j.percent, j.key.as_str()), (40, "install.copy"));
        assert_eq!(j.message, "Copying the system…");
        apply_line(&mut j, "PROGRESS 900 x");
        assert_eq!(j.percent, 40);
        apply_line(&mut j, "RESULT ALP-INST-401");
        assert_eq!(j.result.as_deref(), Some("ALP-INST-401"));
        let f = refusal("RESULT ALP-INST-401\n", "ALP-INST-401: sdb changed\n");
        assert_eq!(f.key, "install.refused");
        assert_eq!(f.message, "sdb changed (ALP-INST-401)");
    }
}
