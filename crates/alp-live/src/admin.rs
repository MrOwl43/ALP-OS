// SPDX-License-Identifier: AGPL-3.0-or-later
//! The maintenance login `admin` (docs/security/admin-account.md): its
//! password hash lives on the stick (`/var/lib/alp/live/admin.pwd`, 0600,
//! `admin:$y$…`), checked by pam_pwdfile on tty2 only. For the alpha the
//! password is the one that unlocks the index (ADR 0011). The hash is made
//! by `mkpasswd --method=yescrypt --stdin`; the password goes to its stdin
//! only.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::archive::STATE_DIR;

pub fn pwd_file() -> String {
    format!("{STATE_DIR}/live/admin.pwd")
}

/// One pwdfile line for `admin`, if `hash` looks like a yescrypt hash.
pub fn line(hash: &str) -> Option<String> {
    let h = hash.trim();
    (h.starts_with("$y$") && !h.contains([':', '\n'])).then(|| format!("admin:{h}\n"))
}

pub fn set_password(password: &str) -> Result<(), String> {
    crate::archive::state_ready()?;
    let mut c = Command::new("mkpasswd")
        .args(["--method=yescrypt", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("mkpasswd: {e}"))?;
    if let Some(mut i) = c.stdin.take() {
        i.write_all(password.as_bytes())
            .map_err(|e| format!("mkpasswd: {e}"))?;
    }
    let o = c.wait_with_output().map_err(|e| format!("mkpasswd: {e}"))?;
    let l = line(&String::from_utf8_lossy(&o.stdout))
        .filter(|_| o.status.success())
        .ok_or("mkpasswd gave no hash")?;
    let path = pwd_file();
    let tmp = format!("{path}.tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(l.as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    fs::File::open(Path::new(&path).parent().unwrap_or(Path::new("/")))
        .and_then(|d| d.sync_all())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_yescrypt_lines() {
        assert_eq!(
            super::line("$y$j9T$abc$def\n").as_deref(),
            Some("admin:$y$j9T$abc$def\n")
        );
        assert!(super::line("$6$salt$x").is_none());
        assert!(super::line("$y$a:b").is_none());
    }
}
