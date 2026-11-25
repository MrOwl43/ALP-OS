// SPDX-License-Identifier: AGPL-3.0-or-later
//! `alp-installer`: scripted front end of the installer library, for the VM
//! tests (T-INST-*) until the native screen drives the same library.
//!
//!   alp-installer list
//!   alp-installer disks                      (JSON, for alpd and the screen)
//!   alp-installer plan --system <disk> [--data <disk>]...   (JSON; writes nothing)
//!   alp-installer plan --keep-running [--data <disk>]...
//!   alp-installer install --system <disk> [--data <disk>]... --confirm-erase <disk,disk,...>
//!   alp-installer keep-running --data <disk>... --confirm-erase <disk,...>
//!
//! Disk names are only looked up in the fresh discovery; `--confirm-erase`
//! must name exactly the disks the plan erases (the scripted stand-in for
//! the Confirm screen). Exit 0 on success; otherwise the `ALP-INST-*` code
//! is printed and the exit status is 1.

use std::process::ExitCode;

use alp_hw::inspect::blockdev::{Context, Limits, Reason, Verdict, discover};
use alp_installer::exec::{
    ContractDigest, ExecConfig, LSBLK_ID_ARGS, LSBLK_LIST_ARGS, execute, parse_identities,
};
use alp_installer::plan::{Choice, MIN_DATA_DISK_BYTES, ProfileChoice, StartPath, plan};
use alp_installer::run::{Runner, SystemRunner};

/// Hardware profile in the image (same file `alpctl live` reads).
const PROFILE: &str = "/usr/share/alp/profiles/generic-x86_64.toml";

fn limits() -> Result<Limits, String> {
    let text = std::fs::read_to_string(PROFILE).map_err(|e| format!("{PROFILE}: {e}"))?;
    let t: toml::Table = text.parse().map_err(|e| format!("{PROFILE}: {e}"))?;
    let get = |k: &str| {
        t.get(k)
            .and_then(toml::Value::as_integer)
            .and_then(|v| u64::try_from(v).ok())
            .ok_or(format!("{PROFILE}: missing {k}"))
    };
    Ok(Limits {
        system_and_data_bytes: get("min_single_disk_bytes")?,
        system_bytes: get("min_system_disk_bytes")?,
        data_bytes: MIN_DATA_DISK_BYTES,
    })
}

fn boot_disk() -> Option<String> {
    alp_hw::inspect::inspect(&alp_hw::inspect::Roots::system())
        .disks
        .into_iter()
        .find(|d| d.is_system_disk)
        .map(|d| d.id.0)
}

fn lsblk(r: &mut SystemRunner, args: &[&str]) -> Result<String, String> {
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    match r.run("lsblk", &args, None) {
        Ok(o) if o.ok() => Ok(o.stdout),
        Ok(o) => Err(format!("lsblk: {}", o.stderr.trim())),
        Err(e) => Err(format!("lsblk: {e}")),
    }
}

fn fail(code: &str, detail: &str) -> ExitCode {
    eprintln!("{code}: {detail}");
    println!("RESULT {code}");
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let limits = match limits() {
        Ok(l) => l,
        Err(e) => return fail("ALP-INST-400", &e),
    };
    let boot = boot_disk();
    let ctx = Context {
        boot_disk: boot.as_deref(),
        limits,
    };
    let mut runner = SystemRunner;
    let disks = match lsblk(&mut runner, LSBLK_LIST_ARGS)
        .and_then(|j| discover(&j, &ctx).map_err(|e| e.to_string()))
    {
        Ok(d) => d,
        Err(e) => return fail("ALP-INST-400", &e),
    };
    let Some(cmd) = args.first() else {
        eprintln!("usage: alp-installer list | install ... | keep-running ...");
        return ExitCode::from(2);
    };
    if cmd == "list" {
        for a in &disks {
            let what = match &a.verdict {
                Verdict::Usable { roles, contents } => format!("usable {roles:?} {contents:?}"),
                Verdict::NotUsable(r @ Reason::TooSmall { .. }) => r.text(),
                Verdict::NotUsable(r) => format!("not usable: {}", r.key()),
            };
            println!(
                "DISK {} {} {} {what}",
                a.disk.name, a.disk.size_bytes, a.disk.model
            );
        }
        return ExitCode::SUCCESS;
    }
    if cmd == "disks" {
        println!("{}", alp_installer::view::disks(&disks));
        return ExitCode::SUCCESS;
    }
    let mut system = None;
    let mut keep = false;
    let mut data = Vec::new();
    let mut confirm = Vec::new();
    let mut it = args.iter().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--keep-running" {
            keep = true;
            continue;
        }
        let Some(value) = it.next() else {
            return fail("ALP-INST-402", &format!("{flag} needs a value"));
        };
        match flag.as_str() {
            "--system" => system = Some(value.clone()),
            "--data" => data.push(value.clone()),
            "--confirm-erase" => {
                confirm = value
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            }
            _ => return fail("ALP-INST-402", &format!("unknown option {flag}")),
        }
    }
    let path = match (cmd.as_str(), system, keep) {
        ("install" | "plan", Some(s), false) => StartPath::InstallTo { system_disk: s },
        ("keep-running", None, _) | ("plan", None, true) => StartPath::KeepRunning,
        _ => {
            return fail(
                "ALP-INST-402",
                "install needs --system; keep-running takes none",
            );
        }
    };
    let choice = Choice {
        path,
        data_disks: data,
        profile: ProfileChoice::Auto,
    };
    let p = match plan(&disks, &choice) {
        Ok(p) => p,
        Err(e) => return fail(e.code, &e.detail),
    };
    if cmd == "plan" {
        println!("{}", alp_installer::view::plan(&p));
        return ExitCode::SUCCESS;
    }
    let erased: Vec<String> = p.erases.iter().map(|e| e.disk.clone()).collect();
    if erased != confirm {
        return fail(
            "ALP-INST-402",
            &format!("the plan erases {erased:?}; --confirm-erase must name exactly these"),
        );
    }
    let confirmed = match lsblk(&mut runner, LSBLK_ID_ARGS).and_then(|j| parse_identities(&j)) {
        Ok(i) => i,
        Err(e) => return fail("ALP-INST-400", &e),
    };
    let mut cfg = ExecConfig::image_defaults(boot, limits);
    cfg.image_version = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("IMAGE_VERSION="))
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .unwrap_or_default();
    cfg.roothash = std::fs::read_to_string("/proc/cmdline").ok().and_then(|c| {
        c.split_whitespace()
            .find_map(|w| w.strip_prefix("roothash="))
            .map(str::to_string)
    });
    // The installer's clock; first boot only records it (no decision rests on it).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    cfg.confirmed_at = alp_installer::seed::rfc3339_utc(now);
    let r = execute(
        &p,
        &confirmed,
        &cfg,
        &mut runner,
        &ContractDigest,
        &mut |e| {
            println!(
                "PROGRESS {} {}/{} {} {}",
                e.percent, e.step, e.steps, e.key, e.message
            );
        },
    );
    match r {
        Ok(()) => {
            println!("RESULT ok");
            ExitCode::SUCCESS
        }
        Err(e) => fail(e.code, &format!("step {}: {}", e.step, e.detail)),
    }
}
