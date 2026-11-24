// SPDX-License-Identifier: AGPL-3.0-or-later
//! The one way the installer runs a program. Programs and arguments are
//! fixed lists built from the plan (`exec`); nothing goes through a shell
//! and no text the user typed becomes an argument.

use std::io::Write;
use std::process::{Command, Stdio};

/// What a program left behind.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    /// Exit status; `None` when it was killed by a signal.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == Some(0)
    }
}

/// Runs `program` with `args` and, if given, `stdin` as its input.
/// `Err` only when the program could not be started at all.
pub trait Runner {
    fn run(
        &mut self,
        program: &str,
        args: &[String],
        stdin: Option<&[u8]>,
    ) -> std::io::Result<Output>;
}

/// The real machine. Used only by the installer binary on the installer
/// image (and in the VM tests); unit tests use a recording mock.
#[derive(Debug, Default)]
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(
        &mut self,
        program: &str,
        args: &[String],
        stdin: Option<&[u8]>,
    ) -> std::io::Result<Output> {
        let mut child = Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(input)?;
        }
        let out = child.wait_with_output()?;
        Ok(Output {
            status: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}
