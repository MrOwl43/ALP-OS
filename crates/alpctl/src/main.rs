// SPDX-License-Identifier: AGPL-3.0-or-later
//! `alpctl`: CLI talking to `alpd` (placeholder until the `alpd` skeleton),
//! plus the ALP-OS Live commands (logic in the `alp-live` crate).

use alp_live::{archive, immich, index, live};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = match args.as_slice() {
        ["live", "report"] => live::report(),
        ["live", "screen"] => live::screen(),
        ["live", "splash"] => live::splash(),
        ["live", "protect-disks"] => live::protect_disks(),
        ["live", "diag"] => live::diag(),
        ["live", "bootlog"] => live::boot_log(),
        ["live", "term-logs"] => live::term_logs(),
        ["live", "archive", rest @ ..] => archive::cli(rest),
        ["live", "index", rest @ ..] => index::cli(rest),
        ["live", "immich", rest @ ..] => immich::cli(rest),
        ["live", "pair", rest @ ..] => alp_live::pairing::cli(rest),
        _ => {
            eprintln!(
                "usage: alpctl live report | screen | splash | protect-disks | diag | bootlog | term-logs | archive … | index … | immich …"
            );
            2
        }
    };
    std::process::exit(code);
}
