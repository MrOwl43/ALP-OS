// SPDX-License-Identifier: AGPL-3.0-or-later
//! The gallery view: the photo service's web pages in a locked embedded
//! viewer (ADR 0028 #5), the native twin of the web console's gallery frame.
//!
//! - Only the gallery proxy's own origin is ever loaded
//!   (`model::gallery::allowed`); every other navigation, every new window
//!   and every download is refused.
//! - The view has no credential of the photo service. It carries only the
//!   console session cookie, in a throwaway (ephemeral) web session made
//!   for each opening; nginx asks alpd with that cookie whether the console
//!   is signed in (`gallery/gate`) and adds the family key itself. Locking
//!   the console closes the gallery at once.
//! - No developer tools, no context menu.

use alp_screen::api::Client;
use alp_screen::model::gallery;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use webkit6::prelude::*;
use webkit6::{NetworkSession, PolicyDecisionType, WebView};

/// The console session cookie (alpd `COOKIE`), scoped like alpd's.
const COOKIE: &str = "alp_console";

pub struct GalleryUi {
    pub page: gtk4::Box,
    holder: gtk4::Box,
    view: RefCell<Option<WebView>>,
}

impl GalleryUi {
    /// The way back is the top bar's Home button (main.rs).
    pub fn new() -> Rc<Self> {
        let page = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();
        let holder = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .vexpand(true)
            .hexpand(true)
            .build();
        page.append(&holder);
        Rc::new(GalleryUi {
            page,
            holder,
            view: RefCell::new(None),
        })
    }

    /// Opens `url` (already checked) with the console session `token`.
    pub fn open(&self, url: &str, client: &Client) {
        self.close();
        if !gallery::allowed(url) {
            return;
        }
        let Some(token) = client.session() else {
            return;
        };
        let session = NetworkSession::new_ephemeral();
        let view = WebView::builder()
            .network_session(&session)
            .vexpand(true)
            .hexpand(true)
            .build();
        if let Some(s) = WebViewExt::settings(&view) {
            s.set_enable_developer_extras(false);
        }
        view.connect_context_menu(|_, _, _| true);
        view.connect_create(|_, _| None);
        view.connect_decide_policy(|_, decision, kind| {
            let uri = match kind {
                PolicyDecisionType::NavigationAction | PolicyDecisionType::NewWindowAction => {
                    decision
                        .downcast_ref::<webkit6::NavigationPolicyDecision>()
                        .and_then(|d| d.navigation_action())
                        .and_then(|a| a.request())
                        .and_then(|r| r.uri())
                        .map(|u| u.to_string())
                }
                // A response is shown, never saved (downloads refused).
                PolicyDecisionType::Response => {
                    let shown = decision
                        .downcast_ref::<webkit6::ResponsePolicyDecision>()
                        .is_some_and(|d| d.is_mime_type_supported());
                    if !shown {
                        decision.ignore();
                        return true;
                    }
                    return false;
                }
                _ => None,
            };
            let ok = kind == PolicyDecisionType::NavigationAction
                && uri.as_deref().is_some_and(gallery::allowed);
            if ok {
                decision.use_();
            } else {
                decision.ignore();
            }
            true
        });
        session.connect_download_started(|_, d| d.cancel());
        let host = gallery::ORIGIN.trim_start_matches("http://");
        let domain = host.split(':').next().unwrap_or("127.0.0.1");
        let mut cookie = webkit6::soup::Cookie::new(COOKIE, token.as_str(), domain, "/api/", -1);
        cookie.set_http_only(true);
        let (v, u) = (view.clone(), url.to_string());
        match session.cookie_manager() {
            Some(cm) => cm.add_cookie(&cookie, None::<&gtk4::gio::Cancellable>, move |r| {
                if r.is_ok() {
                    v.load_uri(&u);
                }
            }),
            None => return,
        }
        self.holder.append(&view);
        *self.view.borrow_mut() = Some(view);
    }

    /// Drops the view and its web session (cookie included).
    pub fn close(&self) {
        if let Some(v) = self.view.borrow_mut().take() {
            v.try_close();
            self.holder.remove(&v);
        }
    }
}
