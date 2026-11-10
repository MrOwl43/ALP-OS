// SPDX-License-Identifier: AGPL-3.0-or-later
//! Long-running Live jobs (archive creation): progress with bytes and speed,
//! cancelling, external programs that can be stopped and never hang the job,
//! and the journal lines and last-job file that `alpctl live diag` collects.
//!
//! Field bug 2026-10-21 (owner, ASUS laptop, 8 GB exFAT USB stick): the
//! "Creating your locked archive" screen stopped at 55 % ("Making room…").
//! See docs/plans/live-alpha-api.md (2026-10-21 change).

use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Prefix of every job line in the journal (`journalctl -b | grep ALP-JOB`).
pub const LOG_PREFIX: &str = "ALP-JOB";
/// The last job's status, as text (rewritten on every logged update).
/// On the stick's state partition when it is mounted, so it survives a
/// restart; `/run/alp` otherwise.
pub const LAST_JOB_STICK: &str = "/var/lib/alp/live/last-job.txt";
pub const LAST_JOB_RUN: &str = "/run/alp/last-job.txt";

/// One progress report of a job.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Update {
    pub step: &'static str,
    /// Plain sentence (English), ready to show.
    pub message: String,
    pub percent: u8,
    /// Bytes written to the disk so far (synced, not only cached), and of
    /// how many, while space is being reserved.
    pub done_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    /// Write speed over the last seconds.
    pub bytes_per_second: Option<u64>,
}

impl Update {
    pub fn step(step: &'static str, message: impl Into<String>, percent: u8) -> Self {
        Self {
            step,
            message: message.into(),
            percent,
            ..Self::default()
        }
    }

    /// Seconds still needed at the current speed, if known.
    pub fn seconds_left(&self) -> Option<u64> {
        let (done, total, speed) = (self.done_bytes?, self.total_bytes?, self.bytes_per_second?);
        (speed > 0).then(|| total.saturating_sub(done).div_ceil(speed))
    }
}

/// Progress callback of a job.
pub type Progress<'a> = &'a mut dyn FnMut(&Update);

/// How a job ended when it did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// Stopped on request; what it made was removed.
    Cancelled,
    /// A step failed: catalog code (errors/catalog.toml), the step, and a
    /// plain sentence with the cause.
    Failed {
        code: &'static str,
        step: &'static str,
        message: String,
    },
}

impl End {
    pub fn failed(code: &'static str, step: &'static str, message: impl Into<String>) -> Self {
        Self::Failed {
            code,
            step,
            message: message.into(),
        }
    }
}

/// Speed over a sliding window of recent samples.
#[derive(Debug, Clone)]
pub struct Meter {
    window: Duration,
    samples: std::collections::VecDeque<(Duration, u64)>,
}

impl Meter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            samples: std::collections::VecDeque::new(),
        }
    }

    /// `bytes` done at time `at` (since any fixed start).
    pub fn add(&mut self, at: Duration, bytes: u64) {
        self.samples.push_back((at, bytes));
        while self.samples.len() > 2
            && self
                .samples
                .get(1)
                .is_some_and(|(t, _)| at.saturating_sub(*t) >= self.window)
        {
            self.samples.pop_front();
        }
    }

    /// Bytes per second over the window; `None` until there is a time span.
    pub fn speed(&self) -> Option<u64> {
        let (t0, b0) = *self.samples.front()?;
        let (t1, b1) = *self.samples.back()?;
        let secs = t1.saturating_sub(t0).as_secs_f64();
        (secs >= 0.2).then(|| (b1.saturating_sub(b0) as f64 / secs) as u64)
    }
}

/// Why an external program did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    Cancelled,
    /// It did not finish in time (it was stopped).
    Timeout(String),
    Failed(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("stopped on request"),
            Self::Timeout(m) | Self::Failed(m) => f.write_str(m),
        }
    }
}

impl RunError {
    /// As a job end: cancelled stays cancelled, otherwise `code` (or the
    /// "disk stopped answering" code for a time-out).
    pub fn end(self, code: &'static str, step: &'static str) -> End {
        match self {
            Self::Cancelled => End::Cancelled,
            Self::Timeout(m) => End::failed(crate::archive::CODE_STALLED, step, m),
            Self::Failed(m) => End::failed(code, step, m),
        }
    }
}

/// How often a running program is looked at (cancel, time-out, `tick`).
const POLL: Duration = Duration::from_millis(100);
/// After a stop request, how long to wait for the program to end. A
/// program stuck on a disk that does not answer cannot be stopped at all;
/// the job then ends without it.
const KILL_GRACE: Duration = Duration::from_secs(10);

fn reader(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut p) = pipe {
            let mut buf = Vec::new();
            let _ = p.read_to_end(&mut buf);
            s = String::from_utf8_lossy(&buf).into_owned();
        }
        s
    })
}

/// Run a program; stop it when `cancel` is set or after `timeout`, and call
/// `tick` about every 100 ms while it runs (for progress). Its standard
/// output is returned, trimmed. Never waits forever.
pub fn run(
    cmd: &str,
    args: &[&str],
    cancel: &AtomicBool,
    timeout: Option<Duration>,
    tick: &mut dyn FnMut(),
) -> Result<String, RunError> {
    let done = run_full(cmd, args, None, cancel, timeout, tick)?;
    done.ok(cmd)
}

/// Like [`run`], with `input` written to the program's standard input
/// (which is then closed). For scripts (`sfdisk`), never for secrets that
/// could end up in an error message: stderr is reported.
pub fn run_input(
    cmd: &str,
    args: &[&str],
    input: &str,
    cancel: &AtomicBool,
    timeout: Option<Duration>,
) -> Result<String, RunError> {
    let done = run_full(
        cmd,
        args,
        Some(input.as_bytes()),
        cancel,
        timeout,
        &mut || (),
    )?;
    done.ok(cmd)
}

/// Like [`run`], but any exit status is an answer: the exit code (`None`
/// when killed by a signal) and the standard output. For programs whose
/// exit code is a bit mask (`smartctl`) or has several "good" values
/// (`fsck.exfat`).
pub fn run_code(
    cmd: &str,
    args: &[&str],
    cancel: &AtomicBool,
    timeout: Option<Duration>,
) -> Result<(Option<i32>, String), RunError> {
    let done = run_full(cmd, args, None, cancel, timeout, &mut || ())?;
    Ok((done.status.code(), done.stdout.trim().to_string()))
}

/// A program that ended by itself.
struct Done {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

impl Done {
    fn ok(self, cmd: &str) -> Result<String, RunError> {
        if self.status.success() {
            Ok(self.stdout.trim().to_string())
        } else {
            Err(RunError::Failed(format!(
                "{cmd} failed ({}): {}",
                self.status,
                crate::fail::display_safe(self.stderr.trim(), 300)
            )))
        }
    }
}

fn run_full(
    cmd: &str,
    args: &[&str],
    input: Option<&[u8]>,
    cancel: &AtomicBool,
    timeout: Option<Duration>,
    tick: &mut dyn FnMut(),
) -> Result<Done, RunError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(RunError::Cancelled);
    }
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RunError::Failed(format!("{cmd}: {e}")))?;
    if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
        use std::io::Write;
        // Small scripts only (they fit in the pipe); closed when dropped.
        let _ = stdin.write_all(data);
    }
    let out = reader(child.stdout.take());
    let err = reader(child.stderr.take());
    let start = Instant::now();
    let stop = |child: &mut std::process::Child| {
        let _ = child.kill();
        let t = Instant::now();
        while t.elapsed() < KILL_GRACE {
            if let Ok(Some(_)) = child.try_wait() {
                return;
            }
            std::thread::sleep(POLL);
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(Done {
                    status,
                    stdout: out.join().unwrap_or_default(),
                    stderr: err.join().unwrap_or_default(),
                });
            }
            Ok(None) => {}
            Err(e) => return Err(RunError::Failed(format!("{cmd}: {e}"))),
        }
        if cancel.load(Ordering::Relaxed) {
            stop(&mut child);
            return Err(RunError::Cancelled);
        }
        if let Some(limit) = timeout
            && start.elapsed() >= limit
        {
            stop(&mut child);
            return Err(RunError::Timeout(format!(
                "{cmd} did not finish within {} minutes; the disk may be very slow or not answering",
                limit.as_secs().div_ceil(60)
            )));
        }
        tick();
        std::thread::sleep(POLL);
    }
}

fn mb_s(bytes_per_second: u64) -> String {
    let m = bytes_per_second as f64 / 1e6;
    if m >= 10.0 {
        format!("{m:.0} MB/s")
    } else {
        format!("{m:.1} MB/s")
    }
}

/// Plain duration: "about 3 minutes", "less than a minute".
pub fn about(secs: u64) -> String {
    match secs {
        0..=59 => "less than a minute".into(),
        60..=119 => "about 1 minute".into(),
        120..=5399 => format!("about {} minutes", secs.div_ceil(60)),
        _ => format!("about {} hours", (secs + 1800) / 3600),
    }
}

/// The message of the disk test (ADR 0014): what is written, how fast.
/// Sizes in MiB (the test is 256 MiB, shown as "256 MB").
pub fn checking_message(done: u64, total: u64, speed: Option<u64>) -> String {
    let mut s = format!("Checking the disk: {} MB of {} MB", done >> 20, total >> 20);
    if let Some(v) = speed.filter(|v| *v > 0) {
        s += &format!(" at {}", mb_s(v));
    }
    s + "…"
}

/// One journal line for a job update (no secrets: steps, numbers, and the
/// plain message only).
pub fn log_line(job: &str, u: &Update) -> String {
    let mut s = format!("{LOG_PREFIX} {job}: step={} percent={}", u.step, u.percent);
    if let (Some(d), Some(t)) = (u.done_bytes, u.total_bytes) {
        s += &format!(" bytes={d}/{t}");
    }
    if let Some(v) = u.bytes_per_second {
        s += &format!(" speed={}", mb_s(v));
    }
    s + &format!(
        " message=\"{}\"",
        crate::fail::display_safe(&u.message, 200)
    )
}

/// The journal line of a job's end.
pub fn end_line(job: &str, end: &Result<(), End>, secs: u64) -> String {
    match end {
        Ok(()) => format!("{LOG_PREFIX} {job}: done after {secs} s"),
        Err(End::Cancelled) => format!("{LOG_PREFIX} {job}: cancelled after {secs} s"),
        Err(End::Failed {
            code,
            step,
            message,
        }) => format!(
            "{LOG_PREFIX} {job}: failed after {secs} s code={code} step={step} message=\"{}\"",
            crate::fail::display_safe(message, 400)
        ),
    }
}

/// Should this update go to the journal? Every step change, and during a
/// step at most every `every`.
#[derive(Debug, Default)]
pub struct LogGate {
    step: Option<&'static str>,
    last: Option<Duration>,
}

impl LogGate {
    pub fn pass(&mut self, at: Duration, step: &'static str, every: Duration) -> bool {
        let new_step = self.step != Some(step);
        let due = self.last.is_none_or(|l| at.saturating_sub(l) >= every);
        if new_step || due {
            self.step = Some(step);
            self.last = Some(at);
            true
        } else {
            false
        }
    }
}

/// Keep the last job's status for `alpctl live diag` (best effort).
pub fn save_last(text: &str) {
    let stick = Path::new(LAST_JOB_STICK);
    let dir_ok = stick.parent().is_some_and(Path::is_dir);
    if dir_ok && crate::archive::state_ready().is_ok() && fs::write(stick, text).is_ok() {
        return;
    }
    let _ = fs::create_dir_all("/run/alp");
    let _ = fs::write(LAST_JOB_RUN, text);
}

/// The last job's status saved by [`save_last`], newest of the two places.
pub fn read_last() -> Option<String> {
    let at = |p: &str| {
        let m = fs::metadata(p).ok()?.modified().ok()?;
        Some((m, fs::read_to_string(p).ok()?))
    };
    match (at(LAST_JOB_STICK), at(LAST_JOB_RUN)) {
        (Some(a), Some(b)) => Some(if a.0 >= b.0 { a.1 } else { b.1 }),
        (a, b) => a.or(b).map(|x| x.1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_measures_recent_speed() {
        let mut m = Meter::new(Duration::from_secs(5));
        assert_eq!(m.speed(), None);
        m.add(Duration::from_secs(0), 0);
        assert_eq!(m.speed(), None);
        m.add(Duration::from_secs(1), 10_000_000);
        assert_eq!(m.speed(), Some(10_000_000));
        // Slows down: the window forgets the fast start.
        for s in 2..=20 {
            m.add(Duration::from_secs(s), 10_000_000 + (s - 1) * 1_000_000);
        }
        let v = m.speed().unwrap();
        assert!((900_000..=1_100_000).contains(&v), "{v}");
    }

    #[test]
    fn messages_say_bytes_speed_and_time() {
        let m = checking_message(120 << 20, 256 << 20, Some(18_000_000));
        assert_eq!(m, "Checking the disk: 120 MB of 256 MB at 18 MB/s…");
        assert_eq!(
            checking_message(0, 256 << 20, None),
            "Checking the disk: 0 MB of 256 MB…"
        );
        assert_eq!(
            checking_message(3 << 20, 256 << 20, Some(4_100_000)),
            "Checking the disk: 3 MB of 256 MB at 4.1 MB/s…"
        );
        assert_eq!(about(30), "less than a minute");
        assert_eq!(about(7200), "about 2 hours");
        let u = Update {
            done_bytes: Some(50),
            total_bytes: Some(150),
            bytes_per_second: Some(10),
            ..Update::step("checking", "x", 30)
        };
        assert_eq!(u.seconds_left(), Some(10));
        assert_eq!(Update::step("encrypting", "x", 60).seconds_left(), None);
    }

    #[test]
    fn journal_lines_have_the_prefix() {
        let u = Update {
            done_bytes: Some(1),
            total_bytes: Some(2),
            bytes_per_second: Some(3_000_000),
            ..Update::step("reserving", "Reserving\nspace", 21)
        };
        let l = log_line("archive.create", &u);
        assert!(
            l.starts_with(
                "ALP-JOB archive.create: step=reserving percent=21 bytes=1/2 speed=3.0 MB/s"
            ),
            "{l}"
        );
        assert!(!l.contains('\n'));
        let f = end_line(
            "archive.create",
            &Err(End::failed("ALP-STO-111", "reserving", "I/O error")),
            9,
        );
        assert_eq!(
            f,
            "ALP-JOB archive.create: failed after 9 s code=ALP-STO-111 step=reserving message=\"I/O error\""
        );
        assert!(end_line("j", &Err(End::Cancelled), 1).contains("cancelled"));
    }

    #[test]
    fn log_gate_passes_step_changes_and_every_interval() {
        let mut g = LogGate::default();
        let s = Duration::from_secs;
        let every = s(15);
        assert!(g.pass(s(0), "preparing", every));
        assert!(g.pass(s(1), "reserving", every));
        assert!(!g.pass(s(5), "reserving", every));
        assert!(g.pass(s(16), "reserving", every));
        assert!(g.pass(s(17), "encrypting", every));
    }

    #[test]
    fn programs_finish_fail_time_out_and_cancel() {
        let no = AtomicBool::new(false);
        let mut ticks = 0;
        assert_eq!(
            run("sh", &["-c", "echo hi"], &no, None, &mut || ticks += 1).as_deref(),
            Ok("hi")
        );
        match run("sh", &["-c", "echo bad >&2; exit 3"], &no, None, &mut || ()) {
            Err(RunError::Failed(m)) => assert!(m.contains("bad"), "{m}"),
            other => panic!("{other:?}"),
        }
        // Exit codes as answers, and a script on stdin.
        assert_eq!(
            run_code("sh", &["-c", "echo x; exit 24"], &no, None),
            Ok((Some(24), "x".to_string()))
        );
        assert_eq!(
            run_input("sh", &["-c", "cat"], "type=7\n", &no, None).as_deref(),
            Ok("type=7")
        );
        let t = Instant::now();
        let r = run(
            "sleep",
            &["30"],
            &no,
            Some(Duration::from_millis(300)),
            &mut || (),
        );
        assert!(matches!(r, Err(RunError::Timeout(_))), "{r:?}");
        assert!(t.elapsed() < Duration::from_secs(5));
        // Cancel while it runs: the tick sets the flag.
        let stop = AtomicBool::new(false);
        let t = Instant::now();
        let mut n = 0;
        let r = run("sleep", &["30"], &stop, None, &mut || {
            n += 1;
            if n == 3 {
                stop.store(true, Ordering::Relaxed);
            }
        });
        assert_eq!(r, Err(RunError::Cancelled));
        assert!(t.elapsed() < Duration::from_secs(5));
        // Already cancelled: not even started.
        assert_eq!(
            run("sh", &["-c", "exit 0"], &stop, None, &mut || ()),
            Err(RunError::Cancelled)
        );
        // A program writing a lot does not block on a full pipe.
        let big = run(
            "sh",
            &["-c", "head -c 300000 /dev/zero | tr '\\0' a"],
            &no,
            Some(Duration::from_secs(20)),
            &mut || (),
        )
        .unwrap();
        assert_eq!(big.len(), 300_000);
        assert!(matches!(
            RunError::Timeout("x".into()).end("ALP-STO-113", "encrypting"),
            End::Failed { code, .. } if code == crate::archive::CODE_STALLED
        ));
    }
}
