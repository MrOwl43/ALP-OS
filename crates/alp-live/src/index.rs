// SPDX-License-Identifier: AGPL-3.0-or-later
//! The index on the boot stick (owner decisions 2026-10-19, ADR 0011): a
//! partition that `systemd-repart` adds in the stick's free space (the boot
//! stick only, never another disk), LUKS2 unlocked with the password the
//! user chooses at first start (cryptsetup defaults: argon2id), Btrfs with
//! zstd compression inside. It holds Immich's database and thumbnails, the
//! archive's key file and other secrets, so the stick alone no longer opens
//! the archive. No own cryptography: cryptsetup does it; the password goes
//! to cryptsetup's stdin only (never argv, environment or a log).
//!
//! TPM2 auto-unlock (owner decision 2026-10-19 item 2, ADR 0012): when the
//! computer has a TPM 2.0 chip, `systemd-cryptenroll` adds a key slot sealed
//! to PCR 7 (Secure Boot state; see `TPM_PCRS` for why not PCR 4). The
//! password slot stays as the fallback; after a password unlock the TPM slot
//! is made again. Binding to the exact system image needs signed PCR 11
//! policies, which come with image signing.
//!
//! Where the index lives is one config value (`index.toml` on the stick:
//! the partition's PARTUUID); the mount point stays the same, so moving the
//! index later is a copy plus a config change.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use alp_hw::inspect;

use crate::archive::STATE_DIR;
use crate::fail::Fail;

/// Where the unlocked index is mounted (fixed; the device is configured).
pub const MNT: &str = "/var/lib/alp-index";
const MAPPER: &str = "alp-index";
/// systemd-repart definition that adds the index partition (image tree).
pub const REPART_DIR: &str = "/usr/lib/alp/repart-index.d";
/// GPT type of the index partition (ALP-OS's own, see the repart file).
pub const TYPE_UUID: &str = "A3E1C6B2-5B7E-4F0A-9C1D-6B2F0E8A4D11";
pub const LABEL: &str = "alp-index";
pub const MIN_PASSWORD: usize = 8;
pub const MAX_PASSWORD: usize = 128;
/// PCRs the TPM key slot is sealed to: PCR 7 (Secure Boot state), the
/// systemd-cryptenroll default. PCR 4 was dropped for the alpha: with it the
/// unseal failed after a restart on CI ("Operation not permitted", run
/// 39276905738) while it passed in the run before, so its value is not
/// stable between our boots; it would also break automatic unlock after every
/// image update. Signed PCR 11 policies come with image signing.
pub const TPM_PCRS: &str = "7";
/// With a typed password, cryptsetup must not try the systemd-tpm2 token
/// (an external token plugin): `open --test-passphrase` then succeeded with
/// any password on a TPM stick, so sign-in and recovery/create took a wrong
/// one (CI run 41687210234, T-LIVE-02 TPM case). Also on reencrypt and
/// luksAddKey, which take the typed secret as proof (security review F1).
pub const NO_TOKENS: &str = "--disable-external-tokens";
/// Argon2id memory for the password slot, KiB. cryptsetup's default is up
/// to 1 GiB; Immich needs that RAM on a 4 GB computer, and every sign-in
/// checks the password with it (ADR 0012).
const PBKDF_MEMORY_KIB: &str = "262144";

fn config_path() -> PathBuf {
    Path::new(STATE_DIR).join("live/index.toml")
}

/// Path inside the unlocked index.
pub fn path(rel: &str) -> PathBuf {
    Path::new(MNT).join(rel)
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let o = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(format!(
            "{cmd} failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ))
    }
}

/// [`run_stdin`] for other modules (archive recovery keys).
pub(crate) fn run_stdin_pub(cmd: &str, args: &[&str], secret: &str) -> Result<String, String> {
    run_stdin(cmd, args, secret)
}

/// Run `cmd` with `secret` on stdin (no newline added), nothing on argv.
fn run_stdin(cmd: &str, args: &[&str], secret: &str) -> Result<String, String> {
    run_stdin_at(None, cmd, args, secret)
}

/// Where `cryptsetup reencrypt` keeps its temporary header: it writes
/// `LUKS2-temp-<uuid>.new` into its working folder, and alpd's is `/` (the
/// read-only system: CI run 41680533220 failed there). Memory only, root only.
const REENCRYPT_DIR: &str = "/run/alp/reencrypt";

fn reencrypt_dir() -> Result<&'static str, String> {
    use std::os::unix::fs::DirBuilderExt;
    let _ = fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(REENCRYPT_DIR);
    if !Path::new(REENCRYPT_DIR).is_dir() {
        return Err(format!("{REENCRYPT_DIR} could not be made"));
    }
    Ok(REENCRYPT_DIR)
}

fn run_stdin_at(
    dir: Option<&str>,
    cmd: &str,
    args: &[&str],
    secret: &str,
) -> Result<String, String> {
    let mut c = Command::new(cmd);
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let mut c = c
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if let Some(mut i) = c.stdin.take() {
        i.write_all(secret.as_bytes())
            .map_err(|e| format!("{cmd}: {e}"))?;
    }
    let o = c.wait_with_output().map_err(|e| format!("{cmd}: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(format!(
            "{cmd} failed ({}): {}",
            o.status,
            String::from_utf8_lossy(&o.stderr).trim()
        ))
    }
}

/// The password rules shown to the user: 8 to 128 characters, no control
/// characters (they cannot be typed reliably).
pub fn check_password(pw: &str) -> Result<(), Fail> {
    let n = pw.chars().count();
    if n < MIN_PASSWORD {
        return Err(Fail::new(
            "password.too_short",
            format!("Please use at least {MIN_PASSWORD} characters."),
        ));
    }
    if n > MAX_PASSWORD || pw.chars().any(char::is_control) {
        return Err(Fail::new(
            "password.invalid",
            "Please use only letters, digits, spaces and punctuation (at most 128).",
        ));
    }
    Ok(())
}

fn config() -> Option<toml::Table> {
    fs::read_to_string(config_path()).ok()?.parse().ok()
}

/// The configured index partition (PARTUUID), if set up.
pub fn configured() -> Option<String> {
    config()?.get("partuuid")?.as_str().map(str::to_string)
}

fn set_config(key: &str, value: toml::Value) -> Result<(), String> {
    let mut t = config().unwrap_or_default();
    t.insert(key.into(), value);
    write_atomic(&config_path(), &t.to_string())
}

/// Protection levels (ADR 0016), kept in the plain config: `none` (no
/// password, nothing encrypted), `console` (a password for the console only,
/// nothing encrypted). Anything else, or nothing, is an encrypted index
/// (`password`, or `strong` with serious encryption). A wrong value written
/// on the stick cannot expose anything: an encrypted index does not mount
/// as a plain one.
pub fn protection() -> &'static str {
    let said = config().and_then(|t| t.get("protection")?.as_str().map(str::to_string));
    match said.as_deref() {
        Some("none") if partition_is_plain() => "none",
        Some("console") if partition_is_plain() => "console",
        _ if is_open() && serious() => "strong",
        _ => "password",
    }
}

static PLAIN_CACHE: std::sync::Mutex<Option<(String, bool)>> = std::sync::Mutex::new(None);

/// The plain config is on the stick and can be edited: an unprotected level
/// is believed only when the index partition really holds no LUKS header
/// (`cryptsetup isLuks`), so a stolen encrypted stick cannot be talked into
/// skipping its password (review of T35). Cached per partition; setup and
/// reset clear it.
fn partition_is_plain() -> bool {
    let Some(uuid) = configured() else {
        return false;
    };
    // Encrypt later runs in this process (ADR 0018): the partition is being
    // changed, and `cryptsetup isLuks` would wait for `cryptsetup reencrypt`
    // (CI: every state answer then hung). Only alpd sets the flag, and
    // `protection` leaves the config when the index is locked, so the plain
    // level stands until then.
    if encrypting() {
        return true;
    }
    {
        let c = PLAIN_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((u, v)) = c.as_ref()
            && *u == uuid
        {
            return *v;
        }
    }
    let Ok((name, _)) = index_partition() else {
        return false;
    };
    // Asked without holding the cache: a slow answer holds only this call.
    // No answer in time counts as encrypted (never skip a password) and is
    // asked again next time.
    let code = Command::new("timeout")
        .args(["10", "cryptsetup", "isLuks", &format!("/dev/{name}")])
        .status()
        .ok()
        .and_then(|s| s.code());
    let plain = code == Some(1);
    if matches!(code, Some(0 | 1)) {
        *PLAIN_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((uuid, plain));
    }
    plain
}

fn clear_plain_cache() {
    *PLAIN_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Encrypt later (ADR 0018) step in progress, kept in the plain config
/// while the index changes: `index`, `archive` or `cleanup`.
pub fn encrypt_step() -> Option<String> {
    config().and_then(|t| t.get("encrypt")?.as_str().map(str::to_string))
}

pub fn set_encrypt_step(step: Option<&str>) -> Result<(), String> {
    let mut t = config().unwrap_or_default();
    match step {
        Some(s) => {
            t.insert("encrypt".into(), s.into());
        }
        None => {
            t.remove("encrypt");
        }
    }
    write_atomic(&config_path(), &t.to_string())
}

/// Encrypt later is running in this process (ADR 0018): unlock, the TPM
/// and the plain auto-mount leave the index alone meanwhile.
static ENCRYPTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_encrypting(on: bool) {
    ENCRYPTING.store(on, std::sync::atomic::Ordering::SeqCst);
    clear_plain_cache();
}

pub fn encrypting() -> bool {
    ENCRYPTING.load(std::sync::atomic::Ordering::SeqCst)
}

/// Bytes Btrfs uses of its device (`dev_item.total_bytes`).
fn btrfs_device_bytes(dev: &str) -> Result<u64, String> {
    let out = run("btrfs", &["inspect-internal", "dump-super", dev])?;
    out.lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("dev_item.total_bytes")?
                .trim()
                .parse()
                .ok()
        })
        .ok_or_else(|| "the stick's index size could not be read".into())
}

/// A plain index whose encryption was cut off before cryptsetup began
/// (ADR 0018): give Btrfs its room back and forget the step, so the stick
/// stays as it was and encrypting can start again.
pub fn abort_unstarted_encrypt() -> Result<(), String> {
    if encrypt_step().as_deref() != Some("index") {
        return Ok(());
    }
    let (name, _) = index_partition()?;
    let dev = format!("/dev/{name}");
    let luks = Command::new("cryptsetup")
        .args(["isLuks", &dev])
        .status()
        .is_ok_and(|s| s.success());
    if luks {
        return Ok(()); // started: it resumes after the password
    }
    if is_open() {
        let _ = run("btrfs", &["filesystem", "resize", "max", MNT]);
    }
    set_encrypt_step(None)
}

/// Does the index partition carry an unfinished LUKS2 re-encryption?
fn reencrypt_pending(dev: &str) -> bool {
    run("cryptsetup", &["luksDump", dev]).is_ok_and(|d| d.contains("online-reencrypt"))
}

/// ADR 0018 step 2-5: turn the plain index into a LUKS2 one in place with
/// cryptsetup's own re-encryption (it survives a power cut and resumes).
/// The caller stopped everything that uses the index. `say` gets progress.
pub fn encrypt_in_place(password: &str, say: &mut dyn FnMut(&str)) -> Result<(), String> {
    check_password(password).map_err(|f| f.message)?;
    let (name, _) = index_partition()?;
    let dev = format!("/dev/{name}");
    let luks = Command::new("cryptsetup")
        .args(["isLuks", &dev])
        .status()
        .is_ok_and(|s| s.success());
    if !luks {
        if !plain() {
            return Err("the index is not an unprotected one".into());
        }
        say("Making room on the stick for the lock…");
        if !is_open() {
            mount_dev(&dev)?;
        }
        // Btrfs shrinks only while mounted, and from the end: the LUKS2
        // header takes 32 MiB, moved data needs the end free.
        run("btrfs", &["filesystem", "resize", "-64M", MNT])?;
        run("sync", &["-f", MNT])?;
        run("umount", &[MNT])?;
        if is_open() {
            return Err("the index is still in use".into());
        }
        // The header's 32 MiB must be free at the end before data moves.
        let used = btrfs_device_bytes(&dev)?;
        let size: u64 = run("blockdev", &["--getsize64", &dev])?
            .trim()
            .parse()
            .map_err(|_| "the stick's index size could not be read".to_string())?;
        if used + (32 << 20) > size {
            mount_dev(&dev)?;
            let _ = run("btrfs", &["filesystem", "resize", "max", MNT]);
            return Err(
                "the index on the stick could not make room for the lock; nothing was changed"
                    .into(),
            );
        }
        say("Locking the stick with your password (this takes a while)…");
        let locked = run_stdin_at(
            Some(reencrypt_dir()?),
            "cryptsetup",
            &[
                "reencrypt",
                "--encrypt",
                "--type",
                "luks2",
                "--reduce-device-size",
                "32M",
                "--pbkdf",
                "argon2id",
                "--pbkdf-memory",
                PBKDF_MEMORY_KIB,
                "--key-file=-",
                "--batch-mode",
                &dev,
            ],
            password,
        );
        if let Err(e) = locked {
            // Nothing was locked if the header is not there: the plain index
            // comes back as it was, so the archive can be put back (CI: the
            // rollback found the index unmounted and the photos vanished).
            if !reencrypt_pending(&dev)
                && !Command::new("cryptsetup")
                    .args(["isLuks", &dev])
                    .status()
                    .is_ok_and(|s| s.success())
            {
                mount_dev(&dev)?;
                let _ = run("btrfs", &["filesystem", "resize", "max", MNT]);
            }
            return Err(e);
        }
    } else if reencrypt_pending(&dev) {
        say("Finishing the lock on the stick…");
        run_stdin_at(
            Some(reencrypt_dir()?),
            "cryptsetup",
            &[
                "reencrypt",
                "--resume-only",
                NO_TOKENS,
                "--key-file=-",
                "--batch-mode",
                &dev,
            ],
            password,
        )?;
    }
    if !Path::new(&format!("/dev/mapper/{MAPPER}")).exists() {
        if is_open() {
            run("umount", &[MNT])?;
        }
        run_stdin(
            "cryptsetup",
            &["open", NO_TOKENS, "--key-file=-", &dev, MAPPER],
            password,
        )?;
    }
    if !is_open() {
        mount()?;
    }
    run("btrfs", &["filesystem", "resize", "max", MNT])?;
    // The encrypted level from now on: the plain level's secrets go.
    let mut t = config().unwrap_or_default();
    t.remove("protection");
    t.insert("tpm".into(), false.into());
    write_atomic(&config_path(), &t.to_string())?;
    clear_plain_cache();
    let _ = fs::remove_file(console_hash_path());
    let _ = fs::remove_file(phone_password_path());
    // The TPM slot comes only when everything is done (alpd), so a cut-off
    // run always asks for the password and resumes (storage review).
    let _ = say;
    Ok(())
}

/// Unprotected (ADR 0016 levels 1 and 2): plain index and archive.
pub fn plain() -> bool {
    matches!(protection(), "none" | "console")
}

/// The console password of level 2, as `openssl passwd -6` (SHA-512 crypt)
/// keeps it, inside the (plain) index.
fn console_hash_path() -> PathBuf {
    path("alp/console.hash")
}

/// Level 1 (no password, ADR 0016): phones still sign in to Immich with a
/// password. A random one is made at setup and shown on the phone screen.
fn phone_password_path() -> PathBuf {
    path("alp/phone-password")
}

/// The phone password of an unprotected stick without a password.
pub fn phone_password() -> Option<String> {
    (protection() == "none")
        .then(|| fs::read_to_string(phone_password_path()).ok())
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 12 letters and digits that are easy to type on a phone (no 0/o, 1/l).
pub fn easy_password(random: &[u8]) -> String {
    const A: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    random
        .iter()
        .take(12)
        .map(|b| A[*b as usize % A.len()] as char)
        .collect()
}

/// SHA-512 crypt of `password` with `salt` (`openssl passwd -6`; the
/// password on stdin only).
fn crypt(password: &str, salt: Option<&str>) -> Result<String, String> {
    // openssl's sha512-crypt has no rounds option: the standard's 5000
    // rounds. It guards the console of an unencrypted stick, not data.
    let mut args = vec!["passwd", "-6", "-stdin"];
    if let Some(s) = salt {
        args.extend(["-salt", s]);
    }
    run_stdin("openssl", &args, &format!("{password}\n"))
}

/// The salt of a `$6$salt$hash` line.
pub fn crypt_salt(hash: &str) -> Option<&str> {
    let mut f = hash.strip_prefix("$6$")?.split('$');
    let salt = f.next()?;
    f.next()?;
    Some(salt).filter(|s| !s.is_empty())
}

/// This computer has a TPM 2.0 chip the kernel can use.
pub fn tpm_present() -> bool {
    fs::read_to_string("/sys/class/tpm/tpm0/tpm_version_major").is_ok_and(|v| v.trim() == "2")
}

/// `none` (no chip), `on` (the chip opens the index at start), `off`, or
/// `disabled` (serious encryption: the password at every start).
pub fn tpm_state() -> &'static str {
    let flag = |k: &str| config().and_then(|t| t.get(k)?.as_bool()) == Some(true);
    if !tpm_present() || plain() {
        "none"
    } else if flag("tpm_disabled") {
        "disabled"
    } else if flag("tpm") {
        "on"
    } else {
        "off"
    }
}

/// Seal a new TPM key slot (the old one is wiped after the new one is
/// made). `password` unlocks the volume for systemd-cryptenroll and goes to
/// its stdin only (`--unlock-key-file=/dev/stdin`).
fn tpm_enroll(dev: &str, password: &str) -> Result<(), String> {
    run_stdin(
        "systemd-cryptenroll",
        &[
            "--wipe-slot=tpm2",
            "--tpm2-device=auto",
            &format!("--tpm2-pcrs={TPM_PCRS}"),
            // Empty: never bind to a pcrlock policy. systemd-cryptenroll 257
            // picks /var/lib/systemd/pcrlock.json by itself when it exists,
            // and on Live /var is gone after a restart ("Couldn't find
            // pcrlock policy for volume", CI run 39263126351).
            "--tpm2-pcrlock=",
            "--unlock-key-file=/dev/stdin",
            dev,
        ],
        password,
    )
    .map(drop)
}

/// After the password opened the index: seal a TPM slot again if this
/// computer has a chip. Never fails the unlock; the result is remembered.
pub fn tpm_refresh(password: &str) {
    if !tpm_present() || plain() {
        return;
    }
    if serious() {
        // Decided inside the encrypted index, so editing the plain config
        // on the stick cannot turn auto-unlock back on.
        let _ = set_config("tpm", false.into());
        return;
    }
    let Ok((name, _)) = index_partition() else {
        return;
    };
    let ok = match tpm_enroll(&format!("/dev/{name}"), password) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("ALP-OS Live: TPM auto-unlock not set: {e}");
            false
        }
    };
    let _ = set_config("tpm", ok.into());
}

fn settings_path() -> PathBuf {
    path("settings.toml")
}

/// Serious encryption chosen for this stick's archive (owner decision
/// 2026-10-24 item 4, ADR 0013)? Read inside the open index.
pub fn serious() -> bool {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| s.parse::<toml::Table>().ok())
        .and_then(|t| t.get("serious")?.as_bool())
        == Some(true)
}

/// Turn on serious encryption: remember it inside the index, then remove
/// the TPM key slot of the index (`systemd-cryptenroll --wipe-slot=tpm2`,
/// unlocked with `password` on stdin), so the password is typed at every
/// start. The caller checked the password. Fails (and the archive is not
/// made) when the slot cannot be removed.
pub fn set_serious(password: &str) -> Result<(), Fail> {
    let failed = |e: String| {
        Fail::new(
            "index.failed",
            format!("ALPArchive could not turn off automatic unlocking: {e}."),
        )
    };
    if !is_open() {
        return Err(Fail::new("locked", "Type your ALPArchive password first."));
    }
    let mut t = toml::Table::new();
    t.insert("serious".into(), true.into());
    write_atomic(&settings_path(), &t.to_string()).map_err(failed)?;
    set_config("tpm", false.into()).map_err(failed)?;
    set_config("tpm_disabled", true.into()).map_err(failed)?;
    if tpm_present() {
        let (name, _) = index_partition().map_err(failed)?;
        run_stdin(
            "systemd-cryptenroll",
            &[
                "--wipe-slot=tpm2",
                "--unlock-key-file=/dev/stdin",
                &format!("/dev/{name}"),
            ],
            password,
        )
        .map_err(failed)?;
    }
    Ok(())
}

const TPM_DEVICE: &str = "/dev/tpmrm0";
const TPM_WAIT_SECS: u64 = 30;

/// Wait up to `secs` seconds for `path` to exist.
fn wait_for(path: &str, secs: u64) -> bool {
    for _ in 0..secs * 4 {
        if Path::new(path).exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Path::new(path).exists()
}

/// At start: open the index with the TPM chip, if it has a slot for it.
/// `Ok(false)`: nothing to try (no chip, no slot, already open).
pub fn tpm_unlock() -> Result<bool, String> {
    if encrypting() {
        return Ok(false);
    }
    if !is_open() && configured().is_some() && plain() {
        // Unprotected: nothing to unlock, the plain index opens at start.
        let (name, _) = index_partition()?;
        mount_dev(&format!("/dev/{name}"))?;
        return Ok(true);
    }
    if is_open() || tpm_state() != "on" {
        return Ok(false);
    }
    // alpd can start before the kernel's TPM resource manager is there
    // (/sys/class/tpm/tpm0 shows up first). Unsealing then fails, and the
    // chip must not be marked as refusing because of that.
    if !wait_for(TPM_DEVICE, TPM_WAIT_SECS) {
        return Err(format!(
            "{TPM_DEVICE} did not appear within {TPM_WAIT_SECS} s"
        ));
    }
    let (name, _) = index_partition()?;
    let dev = format!("/dev/{name}");
    if !Path::new(&format!("/dev/mapper/{MAPPER}")).exists()
        && let Err(e) = run(
            "cryptsetup",
            &[
                "open",
                "--token-only",
                "--token-type",
                "systemd-tpm2",
                &dev,
                MAPPER,
            ],
        )
    {
        // Remember it, so the screen asks for the password and the next
        // password unlock seals a new slot.
        let _ = set_config("tpm", false.into());
        return Err(e);
    }
    mount()?;
    Ok(true)
}

/// Is `password` the index password? (cryptsetup tries the key slots;
/// nothing is opened.) Used by the console sign-in and delete checks.
pub fn check(password: &str) -> Result<(), Fail> {
    match protection() {
        // No password: every check passes (the user chose it, ADR 0016).
        "none" => return Ok(()),
        "console" => {
            let wrong = || {
                Fail::new(
                    "password.wrong",
                    "That is not the password of this ALPArchive. Please try again.",
                )
            };
            let want = fs::read_to_string(console_hash_path()).map_err(|e| {
                Fail::new(
                    "index.failed",
                    format!("The console password is missing: {e}."),
                )
            })?;
            let want = want.trim();
            // One line only: openssl reads up to the first newline.
            check_password(password).map_err(|_| wrong())?;
            let salt = crypt_salt(want).ok_or_else(wrong)?;
            let got = crypt(password, Some(salt)).map_err(|e| Fail::new("index.failed", e))?;
            return if got.trim() == want {
                Ok(())
            } else {
                Err(wrong())
            };
        }
        _ => {}
    }
    let (name, _) = index_partition().map_err(|e| {
        Fail::new(
            "index.missing",
            format!("No password was chosen on this stick yet: {e}."),
        )
    })?;
    run_stdin(
        "cryptsetup",
        &[
            "open",
            "--test-passphrase",
            NO_TOKENS,
            "--key-file=-",
            &format!("/dev/{name}"),
        ],
        password,
    )
    .map(drop)
    .map_err(|e| wrong_or(e, "index.failed"))
}

fn wrong_or(e: String, key: &'static str) -> Fail {
    if e.contains("(exit status: 2)") {
        // cryptsetup: exit code 2 = no key available with this passphrase.
        Fail::new(
            "password.wrong",
            "That is not the password of this ALPArchive. Please try again.",
        )
    } else {
        Fail::new(key, format!("The index could not be opened: {e}."))
    }
}

pub fn is_open() -> bool {
    fs::read_to_string("/proc/self/mountinfo")
        .is_ok_and(|m| m.lines().any(|l| l.split_whitespace().nth(4) == Some(MNT)))
}

/// `setup` (no password chosen yet), `locked` or `open`.
pub fn lock_state() -> &'static str {
    if is_open() {
        "open"
    } else if configured().is_some() {
        "locked"
    } else {
        "setup"
    }
}

/// A stick partition: kernel name, PARTUUID, type UUID.
type StickPart = (String, String, String);

/// The stick (system disk) and its partitions.
fn stick() -> Result<(String, Vec<StickPart>), String> {
    let r = inspect::inspect(&inspect::Roots::system());
    let d = r
        .disks
        .iter()
        .find(|d| d.is_system_disk)
        .ok_or("the ALP-OS stick was not found")?;
    Ok((
        d.id.0.clone(),
        d.partitions
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.partition_uuid.clone(),
                    p.type_uuid.clone(),
                )
            })
            .collect(),
    ))
}

fn index_partition() -> Result<(String, String), String> {
    let want = configured().ok_or("no index set up")?;
    let (_, parts) = stick()?;
    parts
        .into_iter()
        .find(|(_, uuid, _)| uuid.eq_ignore_ascii_case(&want))
        .map(|(name, uuid, _)| (name, uuid))
        .ok_or_else(|| "the index partition is not on this stick".into())
}

/// First start: add the index partition on the stick, encrypt it with
/// `password`, make the filesystem, open it. `say` gets progress text.
pub fn setup(password: &str, say: &mut dyn FnMut(&str)) -> Result<(), Fail> {
    check_password(password)?;
    let (name, partuuid) = new_partition(say)?;
    let internal = |e: String| {
        Fail::new(
            "index.failed",
            format!("The stick could not be prepared: {e}."),
        )
    };
    let dev = format!("/dev/{name}");
    say("Locking it with your password…");
    run_stdin(
        "cryptsetup",
        &[
            "luksFormat",
            "--batch-mode",
            "--type",
            "luks2",
            "--pbkdf",
            "argon2id",
            "--pbkdf-memory",
            PBKDF_MEMORY_KIB,
            "--key-file=-",
            &dev,
        ],
        password,
    )
    .map_err(internal)?;
    run_stdin(
        "cryptsetup",
        &["open", NO_TOKENS, "--key-file=-", &dev, MAPPER],
        password,
    )
    .map_err(internal)?;
    say("Preparing the index…");
    let mapped = format!("/dev/mapper/{MAPPER}");
    run("mkfs.btrfs", &["--quiet", "-L", LABEL, &mapped]).map_err(internal)?;
    mount().map_err(internal)?;
    let mut t = toml::Table::new();
    t.insert("partuuid".into(), partuuid.into());
    t.insert("mount".into(), MNT.into());
    t.insert("tpm".into(), false.into());
    write_atomic(&config_path(), &t.to_string()).map_err(internal)?;
    if tpm_present() {
        say("Letting this computer's security chip open it…");
        tpm_refresh(password);
    }
    Ok(())
}

/// First start, unprotected (ADR 0016 levels 1 and 2): the same partition,
/// plain Btrfs, no encryption. `password` (level 2) guards the console only
/// and is kept as a hash in the index. The caller showed the warning.
pub fn setup_plain(password: Option<&str>, say: &mut dyn FnMut(&str)) -> Result<(), Fail> {
    if let Some(pw) = password {
        check_password(pw)?;
    }
    let (name, partuuid) = new_partition(say)?;
    say("Preparing the index…");
    let res = plain_format(&format!("/dev/{name}"), &partuuid, password);
    if res.is_err() && is_open() {
        // So a second try can format it again.
        let _ = run("umount", &[MNT]);
    }
    clear_plain_cache();
    res.map_err(|e| {
        Fail::new(
            "index.failed",
            format!("The stick could not be prepared: {e}."),
        )
    })
}

fn plain_format(dev: &str, partuuid: &str, password: Option<&str>) -> Result<(), String> {
    // A partition left from an earlier encrypted setup: its old header goes.
    let _ = run("wipefs", &["--all", "--quiet", dev]);
    run("mkfs.btrfs", &["--quiet", "--force", "-L", LABEL, dev])?;
    mount_dev(dev)?;
    if let Some(pw) = password {
        let h = crypt(pw, None)?;
        write_atomic(&console_hash_path(), &format!("{}\n", h.trim()))?;
    } else {
        let mut r = [0u8; 12];
        fs::File::open("/dev/urandom")
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut r))
            .map_err(|e| e.to_string())?;
        write_atomic(&phone_password_path(), &format!("{}\n", easy_password(&r)))?;
    }
    let mut t = toml::Table::new();
    t.insert("partuuid".into(), partuuid.into());
    t.insert("mount".into(), MNT.into());
    t.insert("tpm".into(), false.into());
    let level = if password.is_some() {
        "console"
    } else {
        "none"
    };
    t.insert("protection".into(), level.into());
    write_atomic(&config_path(), &t.to_string())
}

/// The index partition for a first start: made by systemd-repart in the
/// stick's free space, or the one left by an earlier setup. Kernel name and
/// PARTUUID.
fn new_partition(say: &mut dyn FnMut(&str)) -> Result<(String, String), Fail> {
    let internal = |e: String| {
        Fail::new(
            "index.failed",
            format!("The stick could not be prepared: {e}."),
        )
    };
    crate::archive::state_ready().map_err(|e| Fail::new("state.not_ready", e))?;
    if configured().is_some() {
        return Err(Fail::new(
            "index.exists",
            "A password was already chosen on this stick.",
        ));
    }
    if let Some(uuid) = wipe_pending() {
        // A reset was cut off: finish erasing the old index first.
        say("Finishing erasing the stick…");
        finish_wipe(&uuid, &mut |done, total, _| {
            say(&format!(
                "Finishing erasing the stick… {}%",
                done.saturating_mul(100) / total.max(1)
            ))
        })
        .map_err(internal)?;
    }
    say("Making room on the stick…");
    let (disk, before) = stick().map_err(internal)?;
    // Only the boot stick is ever passed to systemd-repart.
    let existing = before
        .iter()
        .find(|(_, _, t)| t.eq_ignore_ascii_case(TYPE_UUID))
        .cloned();
    let part = match existing {
        Some(p) => p,
        None => {
            run(
                "systemd-repart",
                &[
                    "--dry-run=no",
                    "--no-pager",
                    &format!("--definitions={REPART_DIR}"),
                    &format!("/dev/{disk}"),
                ],
            )
            .map_err(internal)?;
            let _ = run("udevadm", &["settle", "--timeout=30"]);
            stick()
                .map_err(internal)?
                .1
                .into_iter()
                .find(|(_, _, t)| t.eq_ignore_ascii_case(TYPE_UUID))
                .ok_or_else(|| internal("the new partition did not appear".into()))?
        }
    };
    let (name, partuuid, _) = part;
    Ok((name, partuuid))
}

fn mount() -> Result<(), String> {
    mount_dev(&format!("/dev/mapper/{MAPPER}"))
}

/// Mount the index filesystem on `dev` (the opened LUKS volume, or the plain
/// partition of an unprotected stick) and make its folders.
fn mount_dev(dev: &str) -> Result<(), String> {
    fs::create_dir_all(MNT).map_err(|e| e.to_string())?;
    run(
        "mount",
        &[
            "-t",
            "btrfs",
            "-o",
            "compress=zstd:3,noatime,nosuid,nodev",
            dev,
            MNT,
        ],
    )?;
    for d in ["keys", "immich", "alp"] {
        fs::create_dir_all(path(d)).map_err(|e| e.to_string())?;
    }
    fs::set_permissions(
        path("keys"),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .map_err(|e| e.to_string())
}

/// Unlock with the password. A wrong password is `password.wrong`.
pub fn unlock(password: &str) -> Result<(), Fail> {
    if encrypting() {
        return Err(Fail::new(
            "encrypt.running",
            "ALPArchive is being encrypted. Please wait until it is done.",
        ));
    }
    if plain() {
        // Unprotected: open (normally done at start) and check the console
        // password.
        if !is_open() {
            tpm_unlock().map_err(|e| {
                Fail::new(
                    "index.failed",
                    format!("The index could not be opened: {e}."),
                )
            })?;
        }
        return check(password);
    }
    if is_open() {
        return Ok(());
    }
    let (name, _) = index_partition()
        .map_err(|e| Fail::new("index.missing", format!("The index was not found: {e}.")))?;
    let dev = format!("/dev/{name}");
    if reencrypt_pending(&dev) {
        // Encrypt later was cut off (ADR 0018): finish it first.
        run_stdin_at(
            Some(reencrypt_dir().map_err(|e| Fail::new("index.failed", e))?),
            "cryptsetup",
            &[
                "reencrypt",
                "--resume-only",
                NO_TOKENS,
                "--key-file=-",
                "--batch-mode",
                &dev,
            ],
            password,
        )
        .map_err(|e| wrong_or(e, "index.failed"))?;
    }
    if !Path::new(&format!("/dev/mapper/{MAPPER}")).exists() {
        run_stdin(
            "cryptsetup",
            &["open", NO_TOKENS, "--key-file=-", &dev, MAPPER],
            password,
        )
        .map_err(|e| wrong_or(e, "index.failed"))?;
    }
    mount().map_err(|e| {
        Fail::new(
            "index.failed",
            format!("The index could not be opened: {e}."),
        )
    })
}

/// Close the index (after Immich and the archive).
pub fn close() -> Result<(), String> {
    if is_open() {
        run("umount", &[MNT])?;
    }
    if Path::new(&format!("/dev/mapper/{MAPPER}")).exists() {
        run("cryptsetup", &["close", MAPPER])?;
    }
    Ok(())
}

/// "Delete everything and start over" (owner 2026-10-24): close the index,
/// make it permanently unreadable with `cryptsetup erase` (every key slot,
/// the TPM slot too, is destroyed; the gallery data, family account, keys
/// and chain words inside are gone) and forget it, so the stick is back at
/// first start (`lock.state = setup`). The partition stays and is reused by
/// the next `setup`. Only the configured index partition of this stick is
/// touched; without one, nothing is done.
pub fn reset(progress: crate::wipe::Progress) -> Result<String, String> {
    let Some(_) = configured() else {
        return Ok("no index was set up".into());
    };
    let was_plain = plain();
    close()?;
    let (name, partuuid) = index_partition()?;
    let how = if was_plain {
        // Unprotected: the gallery database and thumbnails are plain on the
        // partition, so it is overwritten for real (owner, 2026-11-03). The
        // marker comes first and the config goes before the long wipe: a
        // power cut then lands at setup, which finishes the wipe before it
        // formats anything (`new_partition`). The marker holds the PARTUUID,
        // not the kernel name, which can change between boots (review W1).
        // A failure, or only a partial overwrite, is reported and the marker
        // stays, so the next setup tries again (review W2).
        write_atomic(&wipe_marker(), &format!("{partuuid}\n"))?;
        fs::remove_file(config_path()).map_err(|e| e.to_string())?;
        clear_plain_cache();
        finish_wipe(&partuuid, progress)?.describe().to_string()
    } else {
        run(
            "cryptsetup",
            &["erase", "--batch-mode", &format!("/dev/{name}")],
        )?;
        // Then the header itself (T-LIVE-04 checks `cryptsetup isLuks`
        // fails). A failure here must not leave a half-reset stick (review
        // F2): the key slots are gone already, so log it and go on.
        if let Err(e) = run("wipefs", &["--all", "--quiet", &format!("/dev/{name}")]) {
            eprintln!("ALP-OS Live: reset: the index header stays (no key slots): {e}");
        }
        fs::remove_file(config_path()).map_err(|e| e.to_string())?;
        clear_plain_cache();
        "encrypted: every key slot destroyed".to_string()
    };
    sync_state_dir()?;
    Ok(how)
}

fn sync_state_dir() -> Result<(), String> {
    let dir = config_path();
    fs::File::open(dir.parent().unwrap_or(Path::new("/")))
        .and_then(|d| d.sync_all())
        .map_err(|e| e.to_string())
}

/// Set while an unprotected index is being overwritten (it holds the
/// partition's PARTUUID); removed when the wipe is done.
fn wipe_marker() -> PathBuf {
    Path::new(STATE_DIR).join("live/index-wipe-pending")
}

/// A wipe cut off by a power cut (or only partly done), still to finish:
/// the PARTUUID of the partition.
pub fn wipe_pending() -> Option<String> {
    fs::read_to_string(wipe_marker())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The kernel name of this stick's index partition with PARTUUID `uuid`,
/// found by PARTUUID and GPT type, never by a stored kernel name.
fn index_part_by_uuid(parts: &[StickPart], uuid: &str) -> Option<String> {
    parts
        .iter()
        .find(|(_, u, t)| u.eq_ignore_ascii_case(uuid) && t.eq_ignore_ascii_case(TYPE_UUID))
        .map(|(n, _, _)| n.clone())
}

/// Overwrite the index partition with PARTUUID `uuid` (secure wipe), then
/// forget the marker. Only ever the stick's own index partition: it is
/// looked up on the stick by PARTUUID and its GPT type checked again. Only
/// a full overwrite counts: after a partial one (edges only) the marker
/// stays and an error is returned.
fn finish_wipe(uuid: &str, progress: crate::wipe::Progress) -> Result<crate::wipe::Method, String> {
    // The config is gone by now: check by the stick's own partition type.
    let (_, parts) = stick()?;
    let name = index_part_by_uuid(&parts, uuid).ok_or_else(|| {
        format!("the partition to erase ({uuid}) is not this stick's index partition")
    })?;
    let never = std::sync::atomic::AtomicBool::new(false);
    let m = crate::wipe::wipe(
        Path::new(&format!("/dev/{name}")),
        crate::wipe::Mode::Secure,
        &never,
        progress,
    )?;
    if !m.complete() {
        return Err(format!(
            "the stick was only partly erased ({}); it will be tried again at the next start",
            m.describe()
        ));
    }
    fs::remove_file(wipe_marker()).map_err(|e| e.to_string())?;
    sync_state_dir()?;
    Ok(m)
}

/// Size and free bytes of a mounted filesystem (`df`).
pub fn usage(at: &str) -> Option<(u64, u64)> {
    let s = run("df", &["-B1", "--output=size,avail", at]).ok()?;
    let l = s.lines().last()?.to_string();
    let mut it = l.split_whitespace().map(|v| v.parse::<u64>().unwrap_or(0));
    Some((it.next()?, it.next()?))
}

/// Space level for warnings (owner: the index must never run full):
/// `ok`, `low` (below `low_percent`) or `full` (below the reserve, where
/// uploads are paused).
pub fn space_level(size: u64, free: u64, low_percent: u64) -> &'static str {
    let reserve = (size / 50).max(512 << 20); // 2 %, at least 512 MiB
    if free < reserve {
        "full"
    } else if free.saturating_mul(100) < size.saturating_mul(low_percent) {
        "low"
    } else {
        "ok"
    }
}

fn write_atomic(path: &Path, body: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    // 0600: also the console hash and the phone password (review of T35).
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    fs::File::open(path.parent().unwrap_or(Path::new("/")))
        .and_then(|d| d.sync_all())
        .map_err(|e| e.to_string())
}

/// `alpctl live index …`: `check` (is open; for the unit) and `close`.
pub fn cli(args: &[&str]) -> i32 {
    let r = match args {
        ["check"] if is_open() => Ok(()),
        ["check"] => Err("the index is locked".to_string()),
        ["close"] => close(),
        ["tpm-unlock"] => tpm_unlock().map(|done| {
            println!("{}", if done { "opened" } else { "nothing to do" });
        }),
        _ => Err("usage: alpctl live index check | close | tpm-unlock".into()),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ALP-OS Live: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_pending_wipe_is_found_by_partuuid_and_type_only() {
        let parts = vec![
            ("sdb1".to_string(), "AAAA".to_string(), "esp".to_string()),
            (
                "sdb3".to_string(),
                "bbbb".to_string(),
                super::TYPE_UUID.to_string(),
            ),
            ("sdc1".to_string(), "CCCC".to_string(), "data".to_string()),
        ];
        // The kernel name may differ from the boot that wrote the marker.
        assert_eq!(
            super::index_part_by_uuid(&parts, "BBBB").as_deref(),
            Some("sdb3")
        );
        // Right PARTUUID but not the index type: refused.
        assert_eq!(super::index_part_by_uuid(&parts, "cccc"), None);
        assert_eq!(super::index_part_by_uuid(&parts, "sdb3"), None);
    }

    #[test]
    fn easy_passwords_have_12_plain_characters() {
        let p = super::easy_password(&[0, 31, 32, 255, 7, 8, 9, 10, 11, 12, 13, 14, 99]);
        assert_eq!(p.chars().count(), 12);
        assert!(
            p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
        assert!(!p.contains(['0', 'o', '1', 'l']));
    }

    #[test]
    fn console_hash_checks_the_password() {
        // The system's openssl, as on the stick.
        let h = super::crypt("correct horse", None).unwrap();
        let salt = super::crypt_salt(&h).unwrap();
        assert_eq!(super::crypt("correct horse", Some(salt)).unwrap(), h);
        assert_ne!(super::crypt("correct horsf", Some(salt)).unwrap(), h);
    }

    #[test]
    fn crypt_salt_reads_sha512_crypt_lines() {
        assert_eq!(super::crypt_salt("$6$abcDEF12$hashhash"), Some("abcDEF12"));
        assert_eq!(super::crypt_salt("$5$abc$x"), None);
        assert_eq!(super::crypt_salt("$6$$x"), None);
        assert_eq!(super::crypt_salt("$6$abc"), None);
    }

    use super::*;

    #[test]
    fn password_rules() {
        assert_eq!(
            check_password("short").unwrap_err().key,
            "password.too_short"
        );
        assert!(check_password("eight ch").is_ok());
        assert!(check_password("Ünïcödé-ok").is_ok());
        assert_eq!(
            check_password("tab\there!").unwrap_err().key,
            "password.invalid"
        );
        assert!(check_password(&"x".repeat(129)).is_err());
    }

    #[test]
    fn space_levels() {
        let g = 1u64 << 30;
        assert_eq!(space_level(100 * g, 50 * g, 10), "ok");
        assert_eq!(space_level(100 * g, 9 * g, 10), "low");
        assert_eq!(space_level(100 * g, g, 10), "full");
        // Small index: the reserve is at least 512 MiB.
        assert_eq!(space_level(8 * g, 400 << 20, 10), "full");
    }
}
