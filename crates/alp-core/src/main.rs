// SPDX-License-Identifier: AGPL-3.0-or-later
//! `alpd`: the ALP-OS daemon. For ALP-OS Live it serves the console
//! listener (`http://127.0.0.1:8411`: the UI and `/api/v1/live/...`,
//! docs/plans/live-alpha-api.md). The rest of the alpd API follows with
//! the installed system (spec Section 9, step 6).

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use alp_core::console::Server;
use alp_core::live_api::Live;

const LISTEN: &str = "127.0.0.1:8411";
const UI_DIR: &str = "/usr/share/alp/ui";

fn main() {
    let listener = match TcpListener::bind(LISTEN) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("alpd: cannot listen on {LISTEN}: {e}");
            std::process::exit(1);
        }
    };
    let live = Live::new(alp_live::chain::Store::live());
    // Everything slow (hardware check, battery, TPM unlock) runs in the
    // background: the console answers at once (T28).
    live.watch_hardware();
    live.watch_space();
    live.watch_status();
    live.watch_display();
    live.watch_certificate();
    live.watch_immich();
    live.auto_unlock();
    let server = Arc::new(Server {
        // ALP_UI_DIR: a UI checkout for development (`ui/`).
        ui_dir: std::env::var_os("ALP_UI_DIR").map_or_else(|| PathBuf::from(UI_DIR), PathBuf::from),
        api: Box::new(move |req| live.handle(req)),
    });
    eprintln!(
        "alpd {}: console on http://{LISTEN}",
        env!("CARGO_PKG_VERSION")
    );
    server.run(listener);
}
