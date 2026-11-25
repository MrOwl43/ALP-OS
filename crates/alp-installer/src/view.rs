// SPDX-License-Identifier: AGPL-3.0-or-later
//! The installer as the screen sees it: the disk list and a plan as JSON.
//! `alpd` runs `alp-installer disks` and `alp-installer plan ...` (as
//! root) and passes these documents to the native screen unchanged, so the
//! screen never guesses at a disk. Field names are a contract with
//! `alp-screen` (`model::install`).

use alp_hw::RaidProfile;
use alp_hw::inspect::blockdev::{Assessed, Contents, Role, Transport, Verdict, size_text};
use serde_json::{Value, json};

use crate::plan::InstallPlan;

fn transport(t: Transport) -> &'static str {
    match t {
        Transport::Sata => "sata",
        Transport::Nvme => "nvme",
        Transport::Usb => "usb",
        Transport::Mmc => "mmc",
        Transport::Virtio => "virtio",
        Transport::Other => "other",
    }
}

fn role(r: &Role) -> &'static str {
    match r {
        Role::SystemAndData => "system_and_data",
        Role::System => "system",
        Role::Data => "data",
    }
}

fn contents(c: &Contents) -> Value {
    match c {
        Contents::Empty => json!([]),
        Contents::Files(lines) => json!(lines),
    }
}

pub fn profile(p: RaidProfile) -> &'static str {
    match p {
        RaidProfile::Single => "single",
        RaidProfile::Dup => "dup",
        RaidProfile::Raid1 => "raid1",
        RaidProfile::Raid1c3 => "raid1c3",
        RaidProfile::Raid10 => "raid10",
    }
}

/// Every connected disk, usable or not. A disk that cannot be used carries
/// the reason's translation key and English text; the screen greys it out.
pub fn disks(list: &[Assessed]) -> Value {
    let disks: Vec<Value> = list
        .iter()
        .map(|a| {
            let d = &a.disk;
            let mut v = json!({
                "name": d.name,
                "model": d.model,
                "size_bytes": d.size_bytes,
                "size": size_text(d.size_bytes),
                "transport": transport(d.transport),
                // The row's first word, and whether the archive's long-term
                // copy is recommended there (hard disks only).
                "kind": d.kind(),
                "long_term": d.long_term(),
                "removable": d.removable,
            });
            match &a.verdict {
                Verdict::Usable { roles, contents: c } => {
                    v["usable"] = json!(true);
                    v["roles"] = json!(roles.iter().map(role).collect::<Vec<_>>());
                    v["contents"] = contents(c);
                }
                Verdict::NotUsable(r) => {
                    v["usable"] = json!(false);
                    v["reason"] = json!({"key": r.key(), "text": r.text()});
                    // An old archive disk: the screen offers "open it
                    // again" (never erase) when this is true.
                    v["reopenable"] = json!(alp_hw::inspect::blockdev::reopenable(d));
                }
            }
            v
        })
        .collect();
    json!({ "disks": disks })
}

/// A plan, for the confirm screen: what is erased (with what is on it
/// now), the protection, the estimated space, and whether the strong
/// warning and the slide are needed.
pub fn plan(p: &InstallPlan) -> Value {
    json!({
        "erases": p.erases.iter().map(|e| json!({
            "name": e.disk,
            "model": e.model,
            "size_bytes": e.size_bytes,
            "size": size_text(e.size_bytes),
            "contents": contents(&e.contents),
        })).collect::<Vec<_>>(),
        "strong_warning": p.needs_strong_warning(),
        // Old archive disks opened again with the recovery words at the
        // first start; never erased.
        "reopen": p.choice.reopen_disks,
        "pool": {
            "members": p.pool.members,
            "data_profile": profile(p.pool.data_profile),
            "metadata_profile": profile(p.pool.metadata_profile),
            "usable_bytes": p.pool.usable_bytes,
            "usable": size_text(p.pool.usable_bytes),
            "protection": match p.pool.protection {
                alp_hw::Protection::None => "none",
                alp_hw::Protection::BitRotOnly => "bit_rot_only",
                alp_hw::Protection::DiskFailure => "disk_failure",
            },
        },
    })
}

/// What the person saw on the Confirm page, as one digest: the plan's JSON
/// (the disks erased, what is on them now, the pool) and the identity of
/// every disk it erases (by-id name, serial, size). `plan` prints it,
/// `install` must be given the same one and recomputes it from its own
/// fresh discovery right before writing, so a disk swapped, filled or
/// changed after the person confirmed is refused (ALP-INST-401). Not a
/// secret: SHA3-256 hex.
pub fn confirm_token(p: &InstallPlan, ids: &[crate::exec::Identity]) -> String {
    use sha3::{Digest, Sha3_256};
    let mut h = Sha3_256::new();
    h.update(b"alp-installer-confirm-v1\n");
    h.update(plan(p).to_string().as_bytes());
    for e in &p.erases {
        h.update(b"\n");
        match ids.iter().find(|i| i.name == e.disk) {
            Some(i) => h.update(
                format!(
                    "{}|{}|{}|{}",
                    i.name,
                    i.by_id,
                    i.serial.as_deref().unwrap_or(""),
                    i.size_bytes
                )
                .as_bytes(),
            ),
            None => h.update(format!("{}|unknown", e.disk).as_bytes()),
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Choice, MIN_DATA_DISK_BYTES, ProfileChoice, StartPath};
    use alp_hw::inspect::blockdev::{Context, Limits, discover};

    const GB: u64 = 1_000_000_000;

    fn home() -> Vec<Assessed> {
        discover(
            include_str!("../tests/fixtures/lsblk/home.json"),
            &Context {
                boot_disk: Some("sdc"),
                limits: Limits {
                    system_and_data_bytes: 64 * GB,
                    system_bytes: 32 * GB,
                    data_bytes: MIN_DATA_DISK_BYTES,
                },
            },
        )
        .expect("fixture parses")
    }

    fn by_name<'a>(v: &'a Value, name: &str) -> &'a Value {
        v["disks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == name)
            .unwrap()
    }

    #[test]
    fn every_disk_is_listed_with_a_reason_when_greyed() {
        let v = disks(&home());
        assert_eq!(v["disks"].as_array().unwrap().len(), home().len());
        let win = by_name(&v, "nvme0n1");
        assert_eq!(win["usable"], false);
        assert_eq!(win["reason"]["key"], "holds_windows");
        assert!(win["reason"]["text"].as_str().unwrap().contains("Windows"));
        assert_eq!(by_name(&v, "sdc")["reason"]["key"], "boot_stick");
        let files = by_name(&v, "sdb");
        assert_eq!(files["usable"], true);
        assert!(!files["contents"].as_array().unwrap().is_empty());
        assert_eq!(by_name(&v, "sda")["contents"], json!([]));
    }

    #[test]
    fn a_plan_names_what_it_erases() {
        let p = crate::plan::plan(
            &home(),
            &Choice {
                path: StartPath::InstallTo {
                    system_disk: "sda".into(),
                },
                data_disks: vec!["sdb".into(), "sde".into()],
                profile: ProfileChoice::Auto,
                reopen_disks: Vec::new(),
            },
        )
        .unwrap();
        let v = plan(&p);
        let names: Vec<&str> = v["erases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["sda", "sdb", "sde"]);
        assert_eq!(v["strong_warning"], true);
        assert_eq!(v["pool"]["data_profile"], "raid1");
        assert_eq!(v["pool"]["protection"], "disk_failure");
    }

    #[test]
    fn the_confirm_token_follows_disks_and_contents() {
        use crate::exec::Identity;
        let choice = Choice {
            path: StartPath::InstallTo {
                system_disk: "sda".into(),
            },
            data_disks: vec!["sdb".into(), "sde".into()],
            profile: ProfileChoice::Auto,
            reopen_disks: Vec::new(),
        };
        let p = crate::plan::plan(&home(), &choice).unwrap();
        let ids: Vec<Identity> = ["sda", "sdb", "sde"]
            .iter()
            .map(|n| Identity {
                name: n.to_string(),
                by_id: format!("ata-DISK_{n}"),
                serial: Some(format!("S{n}")),
                size_bytes: 1,
            })
            .collect();
        let t = confirm_token(&p, &ids);
        assert_eq!(t.len(), 64);
        assert_eq!(t, confirm_token(&p, &ids), "stable");
        // Another disk under the same kernel name (swapped in meanwhile).
        let mut swapped = ids.clone();
        swapped[1].serial = Some("OTHER".into());
        assert_ne!(t, confirm_token(&p, &swapped));
        // Same disk, different contents now.
        let mut disks = home();
        let sdb = disks.iter_mut().find(|d| d.disk.name == "sdb").unwrap();
        sdb.disk.size_bytes += 1;
        let p2 = crate::plan::plan(&disks, &choice).unwrap();
        assert_ne!(t, confirm_token(&p2, &ids));
    }
}
