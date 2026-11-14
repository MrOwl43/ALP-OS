// SPDX-License-Identifier: AGPL-3.0-or-later
// ALP-OS Live setup assistant. Plain JS, no build step, no network except alpd on 127.0.0.1.
(function () {
  "use strict";
  const params = new URLSearchParams(location.search);
  let T = {};
  let S = null;                 // last server state
  // Interim laptop build (owner, 2026-10-19): the phone step only said it
  // comes in the next version. Immich is in the image again (T03, ADR 0015).
  const PHONE_NEXT = false;
  let setupPw = null;   // see password(): only between setting the password and making the archive
  const L = { welcomed: false, phoneDone: false, recoveryDone: false, finished: false, mode: "setup", signed: false, view: null, pair: null, wifi: null, seed: null,
    skip: {}, exited: false, net: { chosen: null, pw: "", nets: null }, disk: { partuuid: null, size: null }, pwd: { a: "", b: "" }, netFrom: false, netBack: null };
  // No "Prepare this disk": the owner decided ALPArchive never erases an external disk.
  let powerOffDone = false;
  let lastAct = Date.now(), saverOn = false, clockTimer = null;
  let current = null, pollTimer = null, offline = 0;

  /* ---------- helpers ---------- */
  const t = (k, v) => String(T[k] !== undefined ? T[k] : k).replace(/\{(\w+)\}/g, (_, n) => (v && v[n] !== undefined ? v[n] : ""));
  function h(tag, attrs, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === false || v == null) continue;
      if (k === "class") e.className = v;
      else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) e.append(c.nodeType ? c : document.createTextNode(c));
    return e;
  }
  const fmtGB = (b) => (b >= 1e12 ? (b / 1e12).toFixed(1).replace(/\.0$/, "") + " TB" : Math.round(b / 1e9) + " GB");
  const num = (n) => new Intl.NumberFormat(document.documentElement.lang || "en").format(n);
  // Server SVG (QR codes) is shown only as <img src="data:image/svg+xml,...">, never innerHTML or inline <svg>:
  // scripts inside an SVG image never run (security review U5).
  const svgImg = (svg, cls, alt) => h("img", { class: cls, alt, src: "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg) }); // img: scripts in SVG never run
  const ICON = {
    check: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>',
    wifi: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.7 16a5 5 0 0 1 6.6 0"/><circle cx="12" cy="19.2" r="1" fill="currentColor"/></svg>',
    disk: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"><rect x="3" y="6" width="18" height="12" rx="3"/><circle cx="17" cy="12" r="1" fill="currentColor"/></svg>',
    phone: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"><rect x="7" y="2.5" width="10" height="19" rx="2.5"/><path d="M11 18.5h2" stroke-linecap="round"/></svg>',
    link: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/></svg>',
    lock: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"><rect x="5" y="10.5" width="14" height="10" rx="2.5"/><path d="M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5"/></svg>',
    key: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M16 7l3 3M14 9l2 2"/></svg>',
    heart: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.600-7 10-7 10z"/></svg>',
    logo: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"><path d="M12 3l8 3v6c0 4.5-3.2 7.8-8 9-4.8-1.2-8-4.5-8-9V6z"/><path d="M8.7 12l2.3 2.3 4.3-4.6" stroke-linecap="round"/></svg>',
  };
  ICON.chevron = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg>';
  ICON.usb = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><rect x="7" y="9" width="10" height="12" rx="2"/><path d="M9 9V3h6v6M11 5.5h.01M13 5.5h.01"/></svg>';
  ICON.trackpad = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M3 15h18M12 15v5"/></svg>';
  ICON.textsize = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M3 19l5.5-14L14 19M5 14h7"/><path d="M15 19l3-7.5 3 7.5M16 16.5h4"/></svg>';
  ICON.erase = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M7 20h13M4.5 14.5l9-9a2 2 0 0 1 2.8 0l2.2 2.2a2 2 0 0 1 0 2.8L12 17l-3 3H7.5l-3-3a1.8 1.8 0 0 1 0-2.5z"/><path d="M9 10l6 6"/></svg>';
  ICON.power = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 3v8M6.8 6.6a8 8 0 1 0 10.4 0"/></svg>';
  ICON.wired = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="4" width="16" height="9" rx="2"/><path d="M8 13v3h8v-3M12 16v4M9 8h.01M12 8h.01M15 8h.01"/></svg>';
  ICON.wifioff = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.7 16a5 5 0 0 1 6.6 0"/><circle cx="12" cy="19.2" r="1" fill="currentColor"/><path d="M4 3l16 18" stroke-width="2.4"/></svg>';
  ICON.warn = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3.5l9.5 16.5h-19z"/><path d="M12 10v4.5M12 17.4h.01"/></svg>';
  ICON.arrow = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14M13 6l6 6-6 6"/></svg>';
  // Standard action icons (T20) and the three code icons (T24). Defined once, inline: the kiosk loads nothing else.
  const SV = (body, extra) => '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"' + (extra || "") + ">" + body + "</svg>";
  Object.assign(ICON, {
    refresh: SV('<path d="M20 11a8 8 0 0 0-14.6-4.5M4 4v3.5h3.5M4 13a8 8 0 0 0 14.6 4.5M20 20v-3.5h-3.5"/>'),
    back: SV('<path d="M19 12H5M11 6l-6 6 6 6"/>'),
    next: SV('<path d="M5 12h14M13 6l6 6-6 6"/>'),
    home: SV('<path d="M3.5 11L12 4l8.5 7"/><path d="M6 9.5V20h4.5v-5.5h3V20H18V9.5"/>'),
    info: SV('<circle cx="12" cy="12" r="9"/><path d="M12 11v5.5M12 7.5h.01"/>'),
    settings: SV('<circle cx="12" cy="12" r="3"/><path d="M12 2.8v2.4M12 18.8v2.4M2.8 12h2.4M18.8 12h2.4M5.5 5.5l1.7 1.7M16.8 16.8l1.7 1.7M5.5 18.5l1.7-1.7M16.8 7.2l1.7-1.7"/><circle cx="12" cy="12" r="6.6"/>'),
    close: SV('<path d="M6 6l12 12M18 6L6 18"/>'),
    later: SV('<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>'),
    gallery: SV('<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><circle cx="9" cy="10" r="1.8"/><path d="M4 18l5.5-5 4 3.5 2.5-2 4.5 3.5"/>'),
    print: SV('<path d="M7 9V3.5h10V9"/><rect x="3.5" y="9" width="17" height="7.5" rx="2"/><path d="M7 14h10v6.5H7z"/>'),
    eye: SV('<path d="M2 12s3.6-6.5 10-6.5S22 12 22 12s-3.6 6.5-10 6.5S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>'),
    chip: SV('<rect x="6" y="6" width="12" height="12" rx="2"/><rect x="9.5" y="9.5" width="5" height="5" rx=".8"/><path d="M9 2.5V6M15 2.5V6M9 18v3.5M15 18v3.5M2.5 9H6M2.5 15H6M18 9h3.5M18 15h3.5"/>'),
    screen: SV('<rect x="2.5" y="4" width="19" height="13" rx="2"/><path d="M8.5 21h7M12 17v4"/>'),
    wrench: SV('<path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3.5 17.5l3 3 5.8-5.8a4 4 0 0 0 5.4-5.4l-2.4 2.4-2.4-.6-.6-2.4z"/>'),
    folder: SV('<path d="M3 6.5A1.5 1.5 0 0 1 4.5 5H10l2 2.5h7.5A1.5 1.5 0 0 1 21 9v9.5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z"/>'),
    video: SV('<rect x="3" y="6" width="13" height="12" rx="2"/><path d="M16 10.5l5-3v9l-5-3z"/>'),
    file: SV('<path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/>'),
    eject: SV('<path d="M12 5l7 8H5z"/><path d="M5 18h14"/>'),
    log: SV('<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M7 9.5l3 2.5-3 2.5M12.5 15H17"/>'),
    download: SV('<path d="M12 3.5v11M7 10l5 5 5-5M4.5 20h15"/>'),
    qr: SV('<rect x="3.5" y="3.5" width="6.5" height="6.5" rx="1"/><rect x="14" y="3.5" width="6.5" height="6.5" rx="1"/><rect x="3.5" y="14" width="6.5" height="6.5" rx="1"/><path d="M14 14h2.5v2.5H14zM18 18h2.5v2.5H18zM14 20.5h1.5M20.5 14v1.5"/>'),
    type: SV('<rect x="2.5" y="6" width="19" height="12" rx="2"/><path d="M6 10h.01M9 10h.01M12 10h.01M15 10h.01M18 10h.01M7.5 14.5h9"/>'),
    battery: SV('<rect x="2.5" y="7" width="17" height="10" rx="2.2"/><path d="M21.5 10.5v3"/>'),
    // Password and code colours (T24): ALP password = blue mountain, recovery code = red owl, chain code = gold elephant.
    mountain: SV('<path d="M2 20L9.5 6.5l4 6.5 2.3-3.4L22 20z"/><path d="M7.4 10.3l1.6 1.2 1.7-1.5"/>'),
    owl: SV('<path d="M6 3.5L8.5 6.3h7L18 3.5V13a6 6 0 0 1-12 0z"/><circle cx="9.5" cy="10" r="1.9"/><circle cx="14.5" cy="10" r="1.9"/><path d="M11 13.4l1 1.3 1-1.3M10 19.3v2M14 19.3v2"/>'),
    elephant: SV('<path d="M9.6 7.4A5 5 0 0 1 12.5 6.2h4.2A4.6 4.6 0 0 1 21.3 10.8V17.5h-2.6v-3h-4.6v3h-2.6v-3.6"/><path d="M9.8 7.2a4.2 4.2 0 0 0-6.3 3.6v7.4a1.2 1.2 0 0 0 2.1.7"/><path d="M8.2 8.6c0 2.4 1 3.9 3 4.5"/><circle cx="6.2" cy="10" r=".6" fill="currentColor"/>'),
  });
  // Every action button gets a standard icon (T20). By default from its text key; data-ico="name" or data-ico="" overrides.
  const KEY_ICON = { "common.continue": "next", "common.back": "back", "common.later": "later", "common.tryagain": "refresh", "common.done": "check", "common.cancel": "close",
    "net.wifi.look": "refresh", "net.cable.again": "refresh", "status.lookagain": "refresh", "home.settings": "settings", "home.lock": "lock", "home.gallery": "gallery",
    "common.poweroff": "power", "power.yes": "power", "status.off": "power", "status.off.yes": "power", "set.open": "next", "set.change": "settings",
    "danger.done.back": "check", "net.wifi.connect": "wifi", "phone.app.have": "next", "where.create": "lock", "unlock.open": "mountain", "lock.signin": "mountain", "pw.set": "mountain",
    "confirm.go": "next", "gallery.back": "home", "net.another": "wifi", "done.go": "home", "home.rem.do": "next", "needs.disk.go": "disk", "where.needpw.go": "mountain", "phone.needs_net.go": "wifi" };
  let ICON_BY_TEXT = null;
  function decorate(root) {
    if (!ICON_BY_TEXT && T["common.back"]) { ICON_BY_TEXT = {}; for (const [k, v] of Object.entries(KEY_ICON)) if (T[k]) ICON_BY_TEXT[T[k]] = v; }
    root.querySelectorAll("button:not([data-ico-done]):not(.tbtn):not(.dotb), a.btn:not([data-ico-done])").forEach((b) => {
      b.setAttribute("data-ico-done", "");
      const name = b.hasAttribute("data-ico") ? b.getAttribute("data-ico") : (ICON_BY_TEXT || {})[b.textContent.trim()] || (b.hasAttribute("data-back") && b.textContent.trim() === t("common.back") ? "back" : "");
      if (!name || !ICON[name] || b.querySelector(".bi")) return;
      const i = h("span", { class: "bi" + (name === "back" || name === "next" ? " flip" : ""), "aria-hidden": "true" }); i.innerHTML = ICON[name];
      b.prepend(i);
    });
  }
  new MutationObserver(() => decorate(document.body)).observe(document.body, { childList: true, subtree: true });
  const bars = (n) => '<svg viewBox="0 0 24 24" fill="currentColor">' + [0, 1, 2, 3].map((i) => `<rect x="${3 + i * 5}" y="${16 - i * 4.2}" width="3.2" height="${5 + i * 4.2}" rx="1.1" opacity="${i < n ? 1 : 0.25}"/>`).join("") + "</svg>";
  const hero = (name) => { const d = h("div", { class: "hero", "aria-hidden": "true" }); d.innerHTML = ICON[name]; return d; };
  // T24: each secret has its own colour and icon. alp = ALP password (blue mountain), rec = recovery code (red owl), chain = chain code (gold elephant).
  const CODE_ICON = { alp: "mountain", rec: "owl", chain: "elephant" };
  const codeIcon = (kind) => { const i = h("span", { class: "ci c-" + kind, "aria-hidden": "true" }); i.innerHTML = ICON[CODE_ICON[kind]]; return i; };
  const codeLabel = (kind, forId, text) => h("label", { for: forId, class: "codelabel c-" + kind }, codeIcon(kind), text);
  const codeHero = (kind) => { const d = hero(CODE_ICON[kind]); d.classList.add("c-" + kind); return d; };

  // err: {key, message, code} (ApiError or a job/archive error object). Own wording first, then the server's sentence.
  // UX spec 3: on the screen only what the person needs to go on (a wrong field, an action that just failed);
  // every failure is also written to the error manager behind (i) with its code and the step.
  function banner(kind, err, extraText) {
    const key = "err." + ((err && err.key) || "");
    const text = extraText || (T[key] ? t(key) : (err && (err.serverMessage || err.message)) || t("err.generic"));
    if (kind === "bad") Errs.add({ level: "bad", text, code: err && err.code, key: err && err.key, step: current ? current.name : null, detail: err && err.serverMessage && err.serverMessage !== text ? err.serverMessage : null });
    return h("div", { class: "banner " + kind, role: "alert" }, h("div", {}, text, err && err.code ? h("span", { class: "code" }, t("err.code", { code: err.code })) : null));
  }
  // replaceChildren() with null or an array would insert the text "null" or "[object …]": drop and flatten first.
  const fill = (el, ...kids) => el.replaceChildren(...kids.flat(2).filter((k) => k != null && k !== false));
  const has = (f) => !!(S && S.features && S.features.has(f));   // a server capability (api.js, features)

  /* ---------- the error manager behind (i) (UX spec 3) ---------- */
  // Entries: {id, time, level: "bad" | "warn" | "info", text, code, key, step, detail, resolved}. Three sources: what the
  // state says (kept in sync by sync(): an entry that disappears is marked Resolved), what the console itself saw (a
  // failed action, alpd not answering, a blocked navigation), and the server's own events (feature "events").
  const Errs = (() => {
    const list = []; let seen = 0, seq = 0;
    const find = (id) => list.find((e) => e.id === id);
    function add(e) {
      const id = e.id || "c" + (++seq);
      const old = find(id);
      if (old) { if (old.resolved || old.text !== e.text) { Object.assign(old, e, { resolved: false, time: Date.now() }); bump(); } return old; }
      const item = Object.assign({ time: Date.now(), level: "warn", resolved: false }, e, { id });
      list.unshift(item); if (list.length > 200) list.pop(); bump(); return item;
    }
    function resolve(id) { const e = find(id); if (e && !e.resolved) { e.resolved = true; e.resolvedAt = Date.now(); paint(); } }
    // The state's problems, by a stable id each.
    function sync(s) {
      const now = new Set();
      const put = (id, level, text, extra) => { now.add(id); add(Object.assign({ id, level, text }, extra || {})); };
      warnings(s).forEach(([tone, key]) => put("st:" + key, tone, t(key)));
      const msg = (e) => (e && (e.message || e.key)) || null;
      if (msg(s.arch.error)) put("st:arch.error", "bad", t("about.err.archive"), { code: s.arch.error.code, key: s.arch.error.key, detail: msg(s.arch.error), step: s.arch.job && s.arch.job.step });
      if (msg(s.immich.error)) put("st:immich.error", "bad", t("about.err.gallery"), { code: s.immich.error.code, key: s.immich.error.key, detail: msg(s.immich.error) });
      (s.hw.checks || []).filter((c) => c.status === "fail").forEach((c) => put("st:hw:" + (c.id || c.item), "bad", (c.item || c.id) + ": " + (c.value || ""), { code: c.code, detail: c.advice }));
      (s.events || []).forEach((e) => { if (e && e.id) { now.add("sv:" + e.id); const o = add({ id: "sv:" + e.id, level: e.level === "error" ? "bad" : e.level === "info" ? "info" : e.level || "warn", text: e.message || e.key || "", code: e.code, key: e.key, step: e.step, detail: e.detail, time: e.time ? Date.parse(e.time) || Date.now() : Date.now() }); if (e.resolved) resolve(o.id); } });
      list.forEach((e) => { if ((e.id.startsWith("st:") || e.id.startsWith("sv:")) && !now.has(e.id)) resolve(e.id); });
    }
    const unseen = () => list.filter((e) => !e.resolved && e.time > seen && e.level !== "info");
    function tone() { const u = unseen(); return u.some((e) => e.level === "bad") ? "bad" : u.length ? "warn" : ""; }
    let painter = null;
    function bump() { paint(); }
    function paint() { const d = document.getElementById("errdot"); if (d) d.className = "errdot " + tone(); if (painter) painter(); }
    return { add, resolve, sync, list, tone, markSeen: () => { seen = Date.now(); paint(); }, onChange: (f) => { painter = f; }, paint };
  })();
  // UX spec 2: nothing may feel like a browser. No context menu, no dragging of images and links, no page zoom,
  // and no browser dialogs (alert/confirm/prompt/print open Chromium's own windows; they go to (i) instead).
  document.addEventListener("contextmenu", (ev) => ev.preventDefault());
  document.addEventListener("dragstart", (ev) => ev.preventDefault());
  document.addEventListener("wheel", (ev) => { if (ev.ctrlKey) ev.preventDefault(); }, { passive: false });
  document.addEventListener("keydown", (ev) => { if ((ev.ctrlKey || ev.metaKey) && ["+", "-", "=", "0", "p", "P", "s", "S", "o", "O"].includes(ev.key)) ev.preventDefault(); }, true);
  ["gesturestart", "gesturechange"].forEach((n) => document.addEventListener(n, (ev) => ev.preventDefault()));
  window.alert = (m) => { Errs.add({ level: "info", text: String(m) }); };
  window.confirm = () => false; window.prompt = () => null; window.print = () => {};
  // An external link never opens (the kiosk policy blocks it with a browser page): note it in (i) instead.
  document.addEventListener("click", (ev) => {
    const a = ev.target.closest && ev.target.closest("a[href]");
    if (a && /^(https?:)?\/\//i.test(a.getAttribute("href")) && new URL(a.href, location.href).origin !== location.origin) { ev.preventDefault(); Errs.add({ level: "info", text: t("err.external") }); }
  }, true);
  document.addEventListener("securitypolicyviolation", (ev) => Errs.add({ level: "info", text: t("err.blocked"), detail: (ev.effectiveDirective || "") + " " + (ev.blockedURI || "") }));
  // T21: the status line of every waiting screen. It says what happens now (el.now) and, when it takes longer than
  // expected, why in plain words: the server's own sentence (el.why) or, after slowAfter seconds, the text slowKey.
  function waitLine(text, slowKey, slowAfter) {
    const now = h("span", { class: "wl-now" }, text || ""), why = h("span", { class: "wl-why hidden" });
    const el = h("div", { class: "waitline", role: "status", "aria-live": "polite" }, h("span", { class: "spinner" }), h("span", { class: "wl-text" }, now, why));
    const t0 = Date.now(); let forced = false, seen = false;
    el.now = (txt) => { if (txt && now.textContent !== txt) now.textContent = txt; };
    el.why = (txt) => { forced = !!txt; if (txt) { if (why.textContent !== txt) why.textContent = txt; why.classList.remove("hidden"); } else why.classList.add("hidden"); };
    const timer = setInterval(() => {
      if (el.isConnected) seen = true; else if (seen || Date.now() - t0 > 60000) { clearInterval(timer); return; }
      if (!forced && slowKey && Date.now() - t0 > (slowAfter || 30) * 1000) { why.textContent = t(slowKey); why.classList.remove("hidden"); }
    }, 1000);
    return el;
  }
  const slug = (s) => String(s || "").toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
  const roomPhotos = (bytes) => num(Math.round(bytes / 4.3e6 / 1000) * 1000); // about 4.3 MB per photo

  /* ---------- top bar (every screen, T18): home, IP address, battery, disk and network status, settings, power ---------- */
  const trayBtn = (svg, label, onclick, cls, text) => { const b = h("button", { class: "tbtn " + (cls || ""), type: "button", title: label, "aria-label": label, onclick }); b.innerHTML = svg; if (text) b.append(h("span", {}, text)); return b; };
  // Backup disk status for the top bar: tone ok (green) | warn (amber) | bad (red), or null when there is nothing to say.
  function diskStatus(s) {
    const a = s.arch.state;
    if (a === "open") {
      const full = s.arch.space === "full" || s.index.space === "full", low = s.arch.space === "low" || s.index.space === "low";
      if (full) return { tone: "bad", label: t("tray.disk.full") };
      if (low || health(s) === "warn") return { tone: "warn", label: t("tray.disk.low") };
      if (health(s) === "bad") return { tone: "bad", label: t("tray.disk.bad") };
      return { tone: "ok", label: t("tray.disk.ok", { free: fmtGB(s.arch.free) }) };
    }
    if (a === "missing") return { tone: "bad", label: t("tray.disk.missing") };
    if (a === "creating") return { tone: "warn", label: t("tray.disk.creating") };
    if (a === "none" || a === "failed") return { tone: "warn", label: t("tray.disk.none") };
    return null;   // locked: nothing is known yet
  }
  // Feature clock: times on screen use the chosen time zone (UTC until one is chosen); the browser knows the zones.
  // A zone name it does not know falls back to UTC.
  function zoneOf(s) { const z = s && s.prefs && s.prefs.timeZone; if (!z) return "UTC"; try { new Intl.DateTimeFormat("en", { timeZone: z }); return z; } catch { return "UTC"; } }
  function clockText(s, long) {
    const d = new Date(s.clock.now * 1000), z = zoneOf(s);
    const o = long ? { weekday: "short", day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit", hour12: false, timeZone: z, timeZoneName: "shortOffset" }
      : { hour: "2-digit", minute: "2-digit", hour12: false, timeZone: z };
    return new Intl.DateTimeFormat("en-GB", o).format(d).replace("GMT", "UTC");
  }
  function clockEl(s) {
    const label = clockText(s, true) + (s.clock.synced ? "" : " · " + t("clock.unsure"));
    return h("span", { class: "tstat clock" + (s.clock.synced ? "" : " unsure"), role: "img", title: label, "aria-label": label, dir: "ltr" }, clockText(s, false));
  }
  function batteryEl(b) {
    const p = Math.max(0, Math.min(100, b.percent)), tone = b.charging ? "" : p <= 10 ? "st-bad" : p <= 20 ? "st-warn" : "";
    const label = t(b.charging ? "tray.battery.charging" : "tray.battery", { n: p });
    const e = h("span", { class: "tstat battery " + tone, role: "img", title: label, "aria-label": label });
    e.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="7" width="17" height="10" rx="2.2"/><path d="M21.5 10.5v3"/>' +
      `<rect x="4.6" y="9.1" width="${(12.8 * p / 100).toFixed(1)}" height="5.8" rx="1" fill="currentColor" stroke="none"/>` + (b.charging ? '<path d="M11.8 7.6l-2.6 4.6h3.4l-2.4 4.4" stroke="var(--card)" stroke-width="1.6"/>' : "") + "</svg>";
    e.append(h("span", { dir: "ltr" }, p + "%"));
    return e;
  }
  const goHome = () => { closeModal(); L.view = null; route(true); };
  // UX spec 2: the system bar is the only permanent element. Left: the screen's name (with the way home on an
  // everyday screen); right, icons only: network, disk, battery, (i), settings, power.
  const EVERYDAY = ["home", "gallery", "settings", "computer", "disks", "admin", "import", "format", "files", "terminal"];
  function renderTray() {
    const tray = document.getElementById("tray"), lead = document.getElementById("lead"); if (!tray) return;
    tray.replaceChildren(); if (lead) lead.replaceChildren();
    const brand = document.getElementById("brand");
    if (!S || powerOffDone || !current || current.name === "starting") { if (brand) brand.replaceChildren(t("app.name")); return; }
    const gated = L.mode === "everyday" && !L.signed || current.name === "unlock";
    const everyday = L.mode === "everyday" && !gated;
    // The screen's name; on an everyday screen other than home it is also the way home.
    const barName = current.name === "danger" && current.bar ? current.bar : current.name;   // a warning screen keeps its task's name (design review 1)
    const title = everyday && barName !== "home" && EVERYDAY.includes(barName) ? t("bar." + barName) : current.name === "lock" ? "" : t("app.name");
    if (brand) brand.replaceChildren(title, current.name === "home" || L.mode === "setup" ? versionTag(S.sys.release) || "" : "");
    if (lead && everyday && current.name !== "home") lead.append(trayBtn(ICON.back, t("tray.home"), goHome, "homeb"));
    if (L.mode === "setup" && current.name !== "toosmall" && current.name !== "unlock") tray.append(h("button", { class: "tbtn exit", type: "button", onclick: exitSetup }, t("setup.exit")));
    const imp = S.importJob;
    if (imp && imp.state === "running") tray.append(h("span", { class: "tstat import", role: "status", title: imp.message || "" }, h("progress", { max: 100, value: imp.percent || 0 }), t("import.bar", { n: imp.percent || 0 })));
    const ips = (S.net.addresses || []).map((a) => a.ip);
    const wifi = S.net.online && S.net.kind === "wifi";
    const label = (!S.net.online ? (S.net.waiting ? t("tray.net.waiting") : t("tray.net.off")) : wifi ? t("tray.net.wifi", { name: S.net.ssid || "" }) : t("tray.net.cable")) + (ips.length ? " · " + ips.join(", ") : "");
    // T57: the address phones use, shown beside the network icon (alpd lists it first); "No network" in grey.
    const nb = trayBtn(!S.net.online ? ICON.wifioff : wifi ? bars(S.net.signal === undefined ? 4 : S.net.signal) : ICON.wired, label,
      () => (gated ? netStatus(label) : openNetwork()), "netb " + (S.net.online ? "st-ok" : "st-off"));   // T54: only the legend colours status
    nb.append(S.net.online && ips.length ? h("span", { class: "ip", dir: "ltr" }, ips[0]) : h("span", { class: "ip none" }, S.net.online ? "" : t("tray.net.none")));
    tray.append(nb);
    const ds = diskStatus(S);
    if (ds) tray.append(trayBtn(ICON.disk, ds.label, gated ? () => netStatus(ds.label, "tray.disk.signin") : L.mode === "setup" ? () => jumpStep(4) : () => openSettings("disk"), ds.tone === "ok" ? "st-ok" : "st-off"));
    if (S.sys.battery) tray.append(batteryEl(S.sys.battery));
    if (has("clock") && S.clock) tray.append(clockEl(S));
    // (i): a quiet grey icon; a small dot when something new happened (orange: needs attention, red: not working).
    const lvl = statusLevel(S);
    const ib = h("button", { class: "tbtn errb lvb" + (errPanel ? " on" : ""), type: "button", title: t("tray.errors") + ": " + t("lv." + lvl), "aria-label": t("tray.errors") + ": " + t("lv." + lvl), onclick: openErrors }, legendIcon(lvl, { word: false }));
    ib.append(h("span", { id: "errdot", class: "errdot " + Errs.tone(), "aria-hidden": "true" }));
    criticalBar(); syncChrome();
    tray.append(ib);
    if (everyday) tray.append(trayBtn(ICON.settings, t("home.settings"), () => navigate("settings"), current.name === "settings" ? "on" : ""));
    tray.append(trayBtn(ICON.power, t("tray.power"), askPowerOff));
  }
  // The side panel behind (i): "Needs you" (one button each), then "Recent"; a row opens its details.
  let errPanel = null, errDetail = null;
  function needsYou(s) {
    const out = [];
    if (s.arch.state === "missing") out.push([t("needs.disk"), t("needs.disk.do"), () => { closeErrors(); goHome(); }]);
    if (s.arch.state === "open" && encrypted(s) && s.arch.recovery !== "set" && L.signed) out.push([t("needs.recovery"), t("needs.recovery.do"), () => { closeErrors(); L.mode === "setup" ? jumpStep(5) : navigate("recovery"); }]);
    if (L.mode === "everyday") reminders(s).filter(([st]) => st !== "recovery").forEach(([st, key]) => out.push([t(key), t("home.rem.do"), () => { closeErrors(); resumeSetup(st); }]));
    return out;
  }
  function closeErrors() { if (errPanel) { errPanel.remove(); errPanel = null; errDetail = null; Errs.onChange(null); renderTray(); } }
  function openErrors() {
    if (errPanel) return closeErrors();
    Errs.markSeen();
    errPanel = h("aside", { class: "errpanel", role: "dialog", "aria-label": t("errs.title") });
    document.body.append(errPanel);
    const draw = () => {
      if (!errPanel) return;
      const head = h("div", { class: "ep-head" }, h("h2", {}, errDetail ? t("errs.details") : t("errs.title")),
        h("button", { class: "tbtn", type: "button", "data-back": "", "aria-label": t("common.close"), onclick: () => { if (errDetail) { errDetail = null; draw(); } else closeErrors(); } }, iconEl(errDetail ? "back" : "close")));
      if (errDetail) {
        const e = errDetail, res = h("div");
        const dl = h("dl", { class: "specs" }, h("dt", {}, t("errs.when")), h("dd", {}, new Date(e.time).toLocaleString()),
          e.code ? [h("dt", {}, t("errs.code")), h("dd", { dir: "ltr" }, e.code)] : null, e.step ? [h("dt", {}, t("errs.step")), h("dd", { dir: "ltr" }, e.step)] : null,
          e.detail || e.key ? [h("dt", {}, t("errs.tech")), h("dd", { class: "tech", dir: "ltr" }, [e.key, e.detail].filter(Boolean).join(": "))] : null);
        fill(errPanel, head, h("p", { class: "ep-text" }, e.text), dl, res,
          has("events") ? h("div", { class: "row" }, h("button", { class: "secondary", type: "button", onclick: async () => {
            try { await alp.reportSave(); res.replaceChildren(h("div", { class: "banner ok", role: "status" }, t("errs.saved"))); } catch (x) { res.replaceChildren(banner("bad", x)); } } }, t("errs.save"))) : null);
        return;
      }
      const needs = S ? needsYou(S) : [];
      const recent = Errs.list.slice(0, 60);
      fill(errPanel, head,
        needs.length ? h("section", {}, h("h3", {}, t("errs.needs")), needs.map(([txt, btn, fn]) => h("div", { class: "ep-need" }, h("span", {}, txt), h("button", { class: "secondary", type: "button", onclick: fn }, btn)))) : null,
        h("section", {}, h("h3", {}, t("errs.recent")),
          recent.length ? h("ul", { class: "ep-list" }, recent.map((e) => h("li", { class: "lv-" + e.level + (e.resolved ? " resolved" : "") },
            h("button", { type: "button", class: "ep-row", onclick: () => { errDetail = e; draw(); } },
              h("span", { class: "ep-dot", "aria-hidden": "true" }), h("span", { class: "ep-t" }, e.text, e.resolved ? h("span", { class: "ep-res" }, " · " + t("errs.resolved")) : null),
              h("span", { class: "ep-time" }, new Date(e.time).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }))))))
            : h("p", { class: "fine" }, t("errs.none"))));
    };
    Errs.onChange(draw); draw(); renderTray();
  }
  // "alpha 0.2.4" from "ALP-OS Live alpha 0.2.4".
  function versionTag(release) {
    const m = /(alpha|beta)\s+[\d.]+/i.exec(release || "");
    return m ? h("span", { class: "ver" }, m[0].toLowerCase()) : null;
  }
  // T41 (ADR 0018): encrypt an unprotected ALPArchive later, behind three screens: what happens, the password, a slide.
  function encryptLater() {
    const level = S.lock.protection;
    const step1 = () => sheet(h("h2", { style: "margin-top:0" }, t("enc.1.title")),
      h("ul", { class: "warnlist" }, ["enc.1.a", "enc.1.b", "enc.1.c", "enc.1.d", "enc.1.e"].map((k) => h("li", {}, t(k, { room: fmtGB(S.arch.size - S.arch.free) })))),
      h("div", { class: "row" }, h("button", { class: "primary", type: "button", onclick: step2 }, t("common.continue")), h("button", { class: "secondary", type: "button", "data-back": "", onclick: closeModal }, t("common.cancel"))));
    const step2 = () => {
      const p1 = h("input", { type: "password", id: "ep1", autocomplete: "new-password", maxlength: 128, "data-focus": "" });
      const p2 = h("input", { type: "password", id: "ep2", autocomplete: "new-password", maxlength: 128 }), res = h("div");
      const f = h("form", { onsubmit: (ev) => { ev.preventDefault();
        if (p1.value.length < 8) { res.replaceChildren(banner("bad", { key: "password.too_short" })); return; }
        if (level === "none" && p1.value !== p2.value) { res.replaceChildren(h("div", { class: "banner bad" }, t("pw.mismatch"))); return; }
        step3(p1.value); } },
        h("div", { class: "field c-alp" }, codeLabel("alp", "ep1", t(level === "console" ? "enc.2.current" : "pw.label")), p1),
        level === "none" ? h("div", { class: "field c-alp" }, codeLabel("alp", "ep2", t("pw.again")), p2) : null, res,
        h("div", { class: "row" }, h("button", { class: "primary", type: "submit" }, t("common.continue")), h("button", { class: "secondary", type: "button", "data-back": "", onclick: closeModal }, t("common.cancel"))));
      sheet(h("h2", { style: "margin-top:0" }, t("enc.2.title")), h("p", { class: "fine" }, t(level === "console" ? "enc.2.sub.console" : "enc.2.sub")), f);
      p1.focus();
    };
    const step3 = (pw) => {
      const res = h("div");
      sheet(h("h2", { style: "margin-top:0" }, t("enc.3.title")), h("p", {}, t("enc.3.sub")), res,
        slideConfirm(t("enc.3.slide"), async () => {
          try { await alp.encryptArchive(pw); pw = null; watch(); } catch (e) { res.replaceChildren(banner("bad", e)); }
        }),
        h("div", { class: "row" }, h("button", { class: "secondary", type: "button", "data-back": "", onclick: () => { pw = null; closeModal(); } }, t("common.cancel"))));
    };
    const watch = () => {
      const line = h("p", { class: "fine" }), bar = h("progress", { max: 100, value: 0 });
      sheet(h("h2", { style: "margin-top:0" }, t("enc.run.title")), h("p", { class: "fine" }, t("enc.run.sub")), bar, line);
      modalEl.dataset.encrypt = "1";
      const timer = setInterval(() => {
        if (!modalEl || !modalEl.dataset.encrypt) { clearInterval(timer); return; }
        const j = S.arch.job;
        if (j) { line.textContent = j.message || ""; bar.value = j.percent || 0; }
        if (j && j.state === "done") { clearInterval(timer); line.textContent = t("enc.done"); bar.value = 100; }
        if (j && j.state === "failed") { clearInterval(timer); line.replaceChildren(banner("bad", j.error || { message: j.message })); }
      }, 1000);
    };
    step1();
  }
  // T40: the recovery code is never stored, so it cannot be shown again; this checks the words someone kept.
  function checkRecovery() {
    const words = h("textarea", { id: "rcw", rows: 3, autocomplete: "off", spellcheck: "false", "data-focus": "" });
    const pp = h("input", { type: "password", id: "rcp", autocomplete: "off" });
    const res = h("div"), go = h("button", { class: "primary", type: "submit" }, t("rcheck.go"));
    const strong = S.lock.protection === "strong" || S.arch.serious;
    const f = h("form", { onsubmit: async (ev) => {
      ev.preventDefault(); go.disabled = true; res.replaceChildren(waitLine(t("rcheck.working"), "wait.generic.slow", 30));
      try { const r = await alp.recoveryCheck(words.value, strong ? pp.value : ""); res.replaceChildren(h("div", { class: "banner " + (r.opens ? "ok" : "bad"), role: "status" }, t(r.opens ? "rcheck.ok" : "rcheck.bad"))); }
      catch (e) { res.replaceChildren(banner("bad", e)); }
      go.disabled = false;
    } },
      h("div", { class: "field c-rec" }, codeLabel("rec", "rcw", t("rcheck.words")), words),
      strong ? h("div", { class: "field c-rec" }, codeLabel("rec", "rcp", t("where.reopen.pp")), pp) : null, res,
      h("div", { class: "row" }, go, h("button", { class: "secondary", type: "button", "data-back": "", onclick: closeModal }, t("common.close"))));
    sheet(h("h2", { style: "margin-top:0" }, t("rcheck.title")), h("p", { class: "fine" }, t("rcheck.sub")), f);
    words.focus();
  }
  // The hardware specs as a list (Settings > About and the About panel).
  function specList() {
    const sp = S.hw.specs, rows = [];
    const add = (label, v) => { const vals = (Array.isArray(v) ? v : [v]).filter((x) => x !== null && x !== undefined && x !== ""); if (vals.length) rows.push(h("dt", {}, label), h("dd", {}, vals.map((x) => h("div", {}, x)))); };
    const mem = (b) => (b / 1073741824).toFixed(1).replace(/\.0$/, "") + " GB";
    const yn = (v, yes, no) => (v === true ? t(yes) : v === false ? t(no) : t("pc.unknown"));
    add(t("pc.machine"), S.hw.machine);
    if (sp) {
      add(t("pc.cpu"), sp.cpu && [sp.cpu.model, sp.cpu.threads ? t("pc.threads", { n: sp.cpu.threads }) : null].filter(Boolean).join(" · "));
      add(t("pc.memory"), sp.memory_bytes ? mem(sp.memory_bytes) : null);
      add(t("pc.disks"), (sp.disks || []).map((d) => [d.model || d.name, d.size_bytes ? fmtGB(d.size_bytes) : null, T["pc.disk." + d.kind] ? t("pc.disk." + d.kind) : d.kind,
        d.system ? t("pc.disk.system") : d.internal ? t("pc.disk.internal") : t("pc.disk.external")].filter(Boolean).join(" · ")));
      add(t("pc.graphics"), (sp.graphics || []).map((g) => [g.vendor, g.pci_id].filter(Boolean).join(" · ")));
      add(t("pc.network"), (sp.network || []).map((n) => [n.name, t(n.kind === "wifi" ? "pc.net.wifi" : "pc.net.wired"), n.driver, n.usb ? "USB" : null, n.working === false ? t("pc.net.notworking") : null].filter(Boolean).join(" · ")));
      add(t("pc.tpm"), sp.tpm ? t("pc.tpm.v", { v: sp.tpm }) : t("pc.tpm.none"));
      add(t("pc.uefi"), yn(sp.uefi, "pc.uefi.yes", "pc.uefi.no"));
      add(t("pc.secure_boot"), yn(sp.secure_boot, "pc.on", "pc.off"));
      add(t("pc.battery"), sp.battery ? [sp.battery.model, sp.battery.health_percent != null ? t("pc.battery.health", { n: sp.battery.health_percent }) : null].filter(Boolean).join(" · ") : t("pc.battery.none"));
    } else {   // older alpd: the start-up check only
      S.hw.checks.forEach((c) => add(T["hw.item." + (c.id || slug(c.item))] ? t("hw.item." + (c.id || slug(c.item))) : c.item, c.value || t("hw." + c.status)));
    }
    if (S.sys.battery) add(t("pc.battery.now"), t(S.sys.battery.charging ? "tray.battery.charging" : "tray.battery", { n: S.sys.battery.percent }));
    add(t("pc.addresses"), S.net.addresses.map((a) => a.ip + (a.kind ? " · " + t(a.kind === "wifi" ? "pc.net.wifi" : "pc.net.wired") : "")));
    return h("dl", { class: "specs" }, rows);
  }
  function sheet(...kids) { closeModal(); modalEl = h("div", { class: "modal", role: "dialog", "aria-modal": "true" }, h("div", { class: "sheet" }, ...kids)); document.body.append(modalEl); const b = modalEl.querySelector(".primary") || modalEl.querySelector("button"); if (b) b.focus(); }
  function askPowerOff() {
    sheet(h("h2", { style: "margin-top:0" }, t("power.title")), h("p", { class: "fine", style: "margin:0" }, t("power.sub")),
      h("div", { class: "row" }, h("button", { class: "primary", onclick: () => { closeModal(); powerOff(); } }, t("power.yes")),
        h("button", { class: "secondary", "data-back": "", onclick: closeModal }, t("common.cancel"))));
  }
  function netStatus(label, hint) {
    sheet(h("h2", { style: "margin-top:0" }, label), h("p", { class: "fine", style: "margin:0" }, t(hint || "tray.net.signin")),
      h("div", { class: "row" }, h("button", { class: "primary", "data-back": "", onclick: closeModal }, t("modal.ok"))));
  }
  function openNetwork() {
    closeModal();
    if (current && current.name === "network") return;
    L.netFrom = true; L.netBack = L.mode === "everyday" ? L.view || null : null;
    L.view = "network"; route(true);
  }
  // T09: setup progress is kept by alpd (feature setup_progress), so a reboot continues where it stopped and a
  // finished setup never comes back. Skipped steps become reminders on the home screen.
  function saveSetup(body) { if (has("setup_progress")) alp.setupProgress(body).then(() => refresh()).catch((e) => Errs.add({ level: "warn", text: t("err.setup.save"), code: e.code, key: e.key })); }
  function skipStep(step) {
    L.skip[step] = true;
    saveSetup({ skipped: Object.keys(L.skip).filter((k) => L.skip[k]) });
    goNext();
  }
  function finishSetup() {
    L.finished = true;
    if (!has("setup_progress")) { alp.setupFinish().catch(() => {}); return; }   // T09: today's alpd keeps only "finished"
    saveSetup({ complete: true, skipped: Object.keys(L.skip).filter((k) => L.skip[k]) });
  }
  function exitSetup() {
    closeModal(); L.exited = true; L.mode = "everyday"; L.view = null; finishSetup();
    L.signed = S.console.state === "open" || S.lock.state === "setup";
    if (L.signed) touch();
    route(true);
  }
  // Steps still open, from the real state (so they survive a reload); each has the wizard step to resume.
  function reminders(s) {
    const r = [];
    if (!s.net.online) r.push(["network", "rem.network"]);
    if (s.lock.state === "setup") r.push(["password", "rem.password"]);
    if (s.arch.state === "none" || s.arch.state === "failed") r.push(["where", "rem.where"]);
    if (s.arch.state === "open" && !PHONE_NEXT && !s.phone.paired && !s.phone.url) r.push(["phone", "rem.phone"]);
    if (s.arch.state === "open" && s.lock.protection === "strong" && s.arch.recovery !== "set") r.push(["recovery", "rem.recovery"]);
    // Level 3: the password could not yet be tied to the disk (storage review): one correct password fixes it.
    if (s.arch.state === "open" && s.lock.protection === "password" && !s.arch.passwordReopen) r.push(["pwreopen", "rem.pwreopen"]);
    return r;
  }
  function resumeSetup(step) {
    if (step === "pwreopen") {   // a correct password binds password.conf on the server (ADR 0016 level 3)
      confirmPassword("confirm.pwreopen").then(async (ok) => { if (ok) { await sleep(4000); await refresh(); route(true); } });
      return;
    }
    // T32: setup that was left is not entered again. Its missing steps open as everyday screens (from the home
    // reminders, Settings or the top bar) and Back returns to where they were opened.
    if (L.mode === "everyday") {
      if (step === "network") return openNetwork();
      if (step === "phone") L.phoneDone = false;
      if (step === "recovery") L.recoveryDone = false;
      return navigate(step);
    }
    L.skip[step] = false; L.exited = false; L.mode = "setup"; L.welcomed = true; L.view = null;
    if (step === "phone") L.phoneDone = false;
    if (step === "recovery") L.recoveryDone = false;
    route(true);
  }
  function reminderList(s, rem) {
    return h("div", { class: "reminders" }, rem.map(([step, key]) => h("div", { class: "rem" }, h("div", {}, h("b", {}, t(key)), h("span", { class: "d" }, t(key + ".d"))),
      h("button", { class: "secondary", type: "button", onclick: () => resumeSetup(step) }, t("home.rem.do")))));
  }

  /* ---------- screen frame ---------- */
  // T22: connecting the phone is the last step before "All set", after the recovery code.
  function stepOf(name) {
    return L.mode === "setup" ? { welcome: 1, toosmall: 1, network: 2, protect: 3, password: 3, where: 4, progress: 4, recovery: 5, phone: 6, done: 7 }[name] : undefined;
  }
  const STEP_VIEW = { 1: "welcome", 2: "network", 3: "password", 4: "where", 5: "recovery", 6: "phone", 7: "done" };
  const STEP_SKIP = { 2: "network", 3: "password", 4: "where", 5: "recovery", 6: "phone" };
  const STEP_ICON = { 1: "chip", 2: "wifi", 3: "mountain", 4: "disk", 5: "owl", 6: "phone", 7: "check" };
  // Is a step finished? Taken from the real state, so it also holds after a reload.
  function stepDone(i) {
    if (!S) return false;
    return [null, L.welcomed, S.net.online, S.lock.state !== "setup", S.arch.state === "open", S.arch.recovery === "set", PHONE_NEXT ? L.phoneDone : S.phone.paired, L.finished][i];
  }
  // UX spec 1: the recovery code exists only for an encrypted archive (password or strong). At the unencrypted
  // levels the step is not offered at all, not even in the step row.
  const encrypted = (s) => s.lock.protection === "password" || s.lock.protection === "strong";
  const recoveryStep = () => (S.lock.state === "setup" ? L.encrypt !== false : encrypted(S));
  const visibleSteps = () => [1, 2, 3, 4, 5, 6, 7].filter((i) => i !== 5 || recoveryStep());
  // Jump to any step. Steps that need an earlier one show what they need first.
  function jumpStep(i) {
    closeModal();
    if (STEP_SKIP[i]) L.skip[STEP_SKIP[i]] = false;
    if (i === 6) L.phoneDone = false;
    if (i === 5) L.recoveryDone = false;
    if (i > 1) L.welcomed = true;
    L.view = i === 4 && S.arch.state === "creating" ? "progress" : STEP_VIEW[i];
    route(true);
  }
  // T19: the step row is centred under the top bar; every step has a short name and an icon.
  function renderSteps(name) {
    const n = stepOf(name), box = document.getElementById("steps");
    box.replaceChildren();
    if (!n) return;
    const canJump = name !== "toosmall" && name !== "unlock" && name !== "starting";
    const row = h("ol", { class: "stepper" }), vis = visibleSteps(), total = vis.length, at = vis.indexOf(n) + 1;
    for (const i of vis) {
      const st = i === n ? "now" : stepDone(i) ? "done" : "todo", nm = t("step." + i);
      const tip = t("step.tip", { name: nm, status: t("step.status." + st) });
      const ic = h("span", { class: "st-ico", "aria-hidden": "true" }); ic.innerHTML = ICON[STEP_ICON[i]];
      if (st === "done") { const b = h("span", { class: "st-done" }); b.innerHTML = ICON.check; ic.append(b); }   // the legend's blue check, never green
      row.append(h("li", { class: "st " + st },
        h("button", { class: "stepb", type: "button", disabled: !canJump || i === n, title: tip, "aria-label": t("step.of", { n: vis.indexOf(i) + 1, total }) + ": " + tip, "aria-current": i === n ? "step" : null, onclick: () => jumpStep(i) },
          ic, h("span", { class: "st-name" }, nm))));
    }
    box.append(h("span", { class: "sr" }, t("step.of", { n: at, total }) + " · " + t("step." + n)), row);
  }
  function show(name, build, opts) {
    const card = document.getElementById("card");
    document.body.dataset.view = name;
    if (name !== "home") delete document.body.dataset.health;
    // UX spec 2: everyday screens fill the area under the system bar; step screens are a centred column on an even
    // background; going forward in setup slides left.
    document.body.dataset.kind = EVERYDAY.includes(name) ? "full" : name === "lock" ? "lock" : "step";
    const fwd = L.mode === "setup" && current && (stepOf(name) || 0) > (stepOf(current.name) || 0);
    const go = () => {
      if (powerOffDone && name !== "off") return;   // a late screen change never covers the shutdown screen
      if (current && current.stop) current.stop();
      card.replaceChildren(); card.className = "card" + (opts && opts.wide ? " wide" : "") + (opts && opts.cls ? " " + opts.cls : "");
      card.classList.toggle("fwd", !!fwd);
      card.style.animation = "none"; void card.offsetWidth; card.style.animation = "";
      current = { name, update() {}, stop() {} };
      const ctx = build(card, current) || {};
      Object.assign(current, ctx);
      renderSteps(name); renderTray();
      if (window.useMock && params.get("showerr") && card.querySelector(".sub")) card.querySelector(".sub").after(banner("bad", { key: params.get("showerr"), code: params.get("showcode") }));  // demo only
      document.getElementById("foot").textContent = (opts && opts.foot) || t("keys.hint");
      const f = card.querySelector("[data-focus]") || card.querySelector("input:checked") || card.querySelector(".primary:not([disabled])");
      if (f) f.focus({ preventScroll: true });
      if (current.update && S) current.update(S);
    };
    if (current && !(opts && opts.instant) && card.firstChild) { card.classList.add("leaving"); setTimeout(go, 180); } else go();
  }
  // T32: Back goes to the screen you came from, like an operating system. Everyday screens keep a short history;
  // setup keeps its own order (goNext).
  const hist = [];
  function navigate(view) {
    if (L.mode === "everyday" && current && current.name !== view) { hist.push(current.name); if (hist.length > 20) hist.shift(); }
    L.view = view; route(true);
  }
  function goBack(fallback) {
    let v = hist.pop();
    // Never back into a screen that no longer fits: one behind the lock, a finished step, a secret code.
    const dead = (n) => n === current.name || ["lock", "confirm", "progress", "password", "welcome", "chain-seed"].includes(n) || (n === "where" && S.arch.state === "open");
    while (v && dead(v)) v = hist.pop();
    const to = v || fallback || "home";
    L.view = to === "home" ? null : to; route(true);
  }

  /* ---------- routing ---------- */
  function computeScreen() {
    if (!S.hw.canRun && L.mode === "setup") return "toosmall";
    if (S.lock.state === "locked" && !L.signed) return "unlock";
    if (L.mode === "setup") {
      if (L.view) return L.view;
      if (!L.welcomed) return "welcome";
      if (!S.net.online && !L.skip.network) return "network";
      // T35 (ADR 0016): first the one question "encrypt your backup disk?", then the password that fits the answer.
      if (S.lock.state === "setup" && L.encrypt === undefined) return "protect";
      if (S.lock.state === "setup" && !L.skip.password) return "password";
      const st = S.arch.state;
      if (st === "creating") return "progress";
      if ((st === "none" || st === "failed") && !L.skip.where) return "where";
      if (st === "missing") return "home";
      if (st === "open") {
        if (encrypted(S) && !L.recoveryDone && !L.skip.recovery && S.arch.recovery !== "set") return "recovery";
        if (!L.phoneDone && !S.phone.paired && !L.skip.phone) return "phone";
      }
      if (!L.finished) return "done";
      return "home";
    }
    if (!L.signed) return L.view === "confirm" ? "confirm" : "lock";
    // A backup disk chosen after setup was left: show its progress there too (T32).
    if (L.view === "where" && S.arch.state === "creating") return "progress";
    if (L.view === "where" && S.arch.state === "open") L.view = null;   // made: home shows what is left
    if (L.view === "progress" && S.arch.state !== "creating") L.view = null;
    return L.view || "home";
  }
  function route(force) {
    if (powerOffDone) return;   // the shutdown screen stays
    const name = computeScreen();
    if (!force && current && current.name === name) { current.update(S); return; }
    ({ welcome, toosmall, unlock, protect, password, network, where, progress, phone, recovery, done, lock, home, settings, disks, admin, chain, "chain-seed": chainSeed, "chain-join": chainJoin, confirm: confirmPage, gallery, computer, import: importScreen, format: disks, files })[name]();
  }
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const goNext = () => { L.view = null; route(true); };
  // Back on a setup step: the step before it during setup; once setup was left, where it was opened from (T32).
  const stepBack = (prev) => (L.mode === "everyday" ? goBack("home") : navigate(prev));

  /* ---------- 1. welcome + hardware ---------- */
  // A "Technical details" row: collapsed by default, a chevron, the plain sentence stays outside (0.2.5 addendum).
  function techDetails(...kids) {
    const d = h("details", { class: "techd" }, h("summary", {}, iconEl("chevron"), h("span", {}, t("common.tech"))), h("div", { class: "techd-body" }, ...kids));
    return d;
  }
  const HW_ICON = { memory: "chip", storage: "disk", tpm: "key", security_chip: "key", cpu: "chip", uefi: "power", secure_boot: "lock", graphics: "screen" };
  const HW_LEVEL = { ok: "ok", note: "note", fail: "now" };
  // T55: network is a normal passed line, never a warning: Wi-Fi works (a cable is better), cable is best, none yet = a note.
  function netCheck(s) {   // the row pattern: "Network", a short value, the sentence under it in grey
    if (s.net.online && s.net.kind === "wired") return { level: "ok", icon: "wired", value: t("hw.net.v.cable"), sub: t("hw.net.cable") };
    if (s.net.online) return { level: "ok", icon: "wifi", value: t("hw.net.v.wifi"), sub: t("hw.net.wifi") + " " + t("hw.net.wifi.sub") };
    return { level: "note", icon: "wifioff", value: t("hw.net.v.none"), sub: t("hw.net.none") };
  }
  function welcome() {
    show("welcome", (card) => {
      const list = h("ul", { class: "checks" }), notes = h("div", { class: "notes" }), tech = h("dl", { class: "specs" }), net0 = S.net.online + "|" + S.net.kind;
      let netRow = null;
      S.hw.checks.forEach((c, i) => {
        const k = c.id || slug(c.item), name = T["hw.item." + k] ? t("hw.item." + k) : c.item, delay = "animation-delay:" + (0.25 + i * 0.25) + "s";
        tech.append(h("dt", {}, name), h("dd", {}, [c.status, c.value].filter(Boolean).join(" · ")));
        if (k === "wired_network" || k === "network") {
          // Owner, 0.2.5-test1: the network line comes last; with no network it offers Wi-Fi right there
          // (the next step is the network screen while there is none).
          const n = netCheck(S), delayN = "animation-delay:" + (0.25 + (S.hw.checks.length - 1) * 0.25) + "s";
          const join = !S.net.online && S.net.wifiPresent ? h("button", { class: "secondary", type: "button", "data-ico": "wifi", onclick: () => { L.welcomed = true; L.skip.network = false; goNext(); } }, t("hw.net.connect")) : null;
          netRow = h("li", { style: delayN }, legendIcon(n.level), iconEl(n.icon), h("span", { class: "name" }, h("span", {}, name), h("span", { class: "fine" }, n.sub)), join || h("span", { class: "state" }, n.value));
          return;
        }
        list.append(h("li", { style: delay },
          legendIcon(HW_LEVEL[c.status] || "note"), iconEl(HW_ICON[k] || "chip"), h("span", { class: "name" }, name),
          h("span", { class: "state" }, c.status !== "ok" && c.value && c.value.length < 14 ? c.value : t("hw." + c.status))));
        if (c.status === "note") notes.append(h("p", {}, T["hw.note." + k] ? t("hw.note." + k) : c.advice || ""));
      });
      if (netRow) list.append(netRow);
      // alpd answers before its hardware check is done (T28): show that it runs, then redraw.
      const pending = S.hw.pending;
      if (pending) list.append(h("li", {}, iconEl("refresh"), h("span", { class: "name" }, t("hw.checking"))));
      else if (!S.hw.reportSaved) notes.append(h("p", {}, t("err.hw.report")));
      card.append(hero("logo"), h("h1", {}, t("welcome.title")), h("p", { class: "sub" }, t("welcome.sub")), list, notes,
        pending ? null : techDetails(tech), h("p", { class: "fine" }, t("welcome.footer")),
        h("div", { class: "actions" }, h("button", { class: "primary", onclick: () => { L.welcomed = true; goNext(); } }, t("common.continue"))));
      return { update(s) { if (pending && !s.hw.pending) route(true); else if (s.net.online + "|" + s.net.kind !== net0) route(true); } };
    });
  }
  function toosmall() {
    show("toosmall", (card) => {
      card.append(hero("logo"), h("h1", {}, t("welcome.toosmall.title")), h("p", { class: "sub" }, t("welcome.toosmall.sub")),
        h("div", { class: "actions" }, h("button", { class: "primary", onclick: powerOff }, t("common.poweroff"))));
    });
  }
  // Shutdown (T15): "Shutting down…" with the steps of the power.off job. alpd stops last, so requests are expected
  // to fail at the end; the last message stays. If the screen is still on long after, say it is safe to switch off.
  const OFF_STEPS = ["immich", "archive", "index", "saving", "off"];
  async function powerOff() {
    closeModal(); stopSaver(); stopPolling(); setConn(false);
    if (current && current.stop) current.stop();
    current = { name: "off", update() {}, stop() {} }; powerOffDone = true;
    document.body.dataset.view = "off"; document.getElementById("steps").replaceChildren(); renderTray();
    document.getElementById("foot").textContent = "";
    const card = document.getElementById("card"); card.className = "card";
    let job = null;
    const list = h("ol", { class: "offsteps" }), line = waitLine(t("off.now"), "off.slow", 100), end = h("div");
    const draw = () => {
      const at = job ? OFF_STEPS.indexOf(job.step) : -1, fin = job && job.state === "done";
      list.replaceChildren(...OFF_STEPS.map((st, i) => { const k = fin || at > i ? "done" : at === i ? "now" : "todo";
        return h("li", { class: k }, h("span", { class: "badge " + (k === "done" ? "ok" : "todo"), "aria-hidden": "true" }, k === "done" ? "✓" : k === "now" ? "•" : ""), t("off.step." + st), h("span", { class: "sr" }, " " + t("step.status." + k))); }));
      if (job && job.message) line.now(job.message);
    };
    card.replaceChildren(hero("power"), h("h1", {}, t("off.title")), list, line, end); draw();
    try { const r = await alp.powerOff(); if (r && r.job) job = r.job; }
    catch (e) {
      if (e.key !== "api.unreachable") {   // refused: say so, and let the person go back
        powerOffDone = false; line.remove();
        end.replaceChildren(banner("bad", e), h("div", { class: "actions" }, h("button", { class: "primary", type: "button", onclick: powerOff }, t("common.tryagain")),
          h("button", { class: "link", type: "button", "data-back": "", onclick: () => { L.view = null; route(true); pollTimer = setTimeout(loop, 500); } }, t("common.back"))));
        return;
      }
    }
    draw();
    for (let gone = 0, t0 = Date.now(); ;) {
      await sleep(1000);
      try { const s = await alp.state(); gone = 0; S = s; if (s.power.job) job = s.power.job; } catch (e) { gone++; }
      draw();
      // Still on: alpd has been gone for 20 s (it stops last), or two minutes passed (the system forces off after 90 s).
      if (gone >= 20 || Date.now() - t0 > 120000) {
        line.remove(); list.querySelectorAll("li").forEach((li) => { li.className = "done"; li.firstChild.className = "badge ok"; li.firstChild.textContent = "✓"; });
        end.replaceChildren(h("div", { class: "banner ok", role: "status" }, h("div", {}, h("b", {}, t("off.final")), h("br"), t("off.final.sub"))));
        return;
      }
    }
  }

  /* ---------- 2. password (first start) and unlock (every start) ---------- */
  function strength(p) {
    if (!p) return null;
    const classes = [/[a-z]/, /[A-Z]/, /[0-9]/, /[^a-zA-Z0-9]/].filter((r) => r.test(p)).length;
    if (p.length < 8) return ["weak", "pw.hint.short"];
    if (p.length >= 14 || (p.length >= 11 && classes >= 3)) return ["good", "pw.hint.good"];
    return ["ok", "pw.hint.ok"];
  }
  const eyeBtn = (inputs) => { const b = h("button", { type: "button", class: "secondary", onclick: () => { const s = inputs[0].type === "password"; inputs.forEach((i) => (i.type = s ? "text" : "password")); b.textContent = s ? t("common.hide") : t("common.show"); } }, t("common.show")); return b; };
  // T35 (ADR 0016): one question first. Yes (the default) leads to the ALP password and the encrypted archive; No shows
  // plainly what that means and asks for a slide, then offers a password for this screen only.
  function protect() {
    show("protect", (card) => {
      const yes = h("input", { type: "radio", name: "enc", value: "yes", checked: L.encrypt !== false });
      const no = h("input", { type: "radio", name: "enc", value: "no", checked: L.encrypt === false });
      const opt = (inp, title, sub, tag) => h("label", { class: "choice" }, inp, h("span", { class: "main" }, h("span", { class: "t" }, title, tag ? h("span", { class: "tag" }, tag) : null), h("span", { class: "s" }, sub)));
      const f = h("form", { onsubmit: (ev) => { ev.preventDefault(); if (yes.checked) { L.encrypt = true; goNext(); } else warn(); } },
        h("div", { class: "choices" }, opt(yes, t("protect.yes"), t("protect.yes.sub"), t("protect.recommended")), opt(no, t("protect.no"), t("protect.no.sub"))),
        h("div", { class: "actions" }, h("button", { class: "primary", type: "submit", "data-focus": "" }, t("common.continue")),
          h("button", { class: "link", type: "button", "data-back": "", onclick: () => navigate("network") }, t("common.back"))));
      function warn() {
        card.replaceChildren(hero("warn"), h("h1", { class: "unprotected" }, t("protect.warn.title")), h("p", { class: "sub" }, t("protect.warn.sub")),
          h("ul", { class: "warnlist" }, h("li", {}, t("protect.warn.1")), h("li", {}, t("protect.warn.2")), h("li", {}, t("protect.warn.3"))),
          slideConfirm(t("protect.warn.slide"), () => { L.encrypt = false; goNext(); }),
          h("div", { class: "actions" }, h("button", { class: "link", type: "button", "data-back": "", onclick: () => route(true) }, t("protect.warn.back"))));
      }
      card.append(hero("lock"), h("h1", {}, t("protect.title")), h("p", { class: "sub" }, t("protect.sub")), f);
    });
  }
  function password() {
    show("password", (card, cur) => {
      if (S.lock.state !== "setup") {   // already chosen: never ask again
        card.append(codeHero("alp"), h("h1", {}, t("pw.title")), h("p", { class: "sub" }, t("pw.done")),
          h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: goNext }, t("common.continue")),
            h("button", { class: "link", "data-back": "", onclick: () => navigate("network") }, t("common.back"))));
        return;
      }
      const plain = L.encrypt === false;   // T35: unprotected; the password only guards this screen
      const p1 = h("input", { type: "password", id: "p1", autocomplete: "new-password", maxlength: 128, "data-focus": "" });
      const p2 = h("input", { type: "password", id: "p2", autocomplete: "new-password", maxlength: 128 });
      const hint = h("p", { class: "fine pwhint", "aria-live": "polite" }), res = h("div");
      const go = h("button", { class: "primary", type: "submit", disabled: true }, t("pw.set"));
      const check = () => {
        const s = strength(p1.value); hint.className = "fine pwhint " + (s ? s[0] : ""); hint.textContent = s ? t(s[1]) : t("pw.hint.rules");
        const same = p1.value === p2.value;
        go.disabled = !(p1.value.length >= 8 && same);
        if (p2.value && !same) hint.textContent += " " + t("pw.mismatch");
      };
      p1.value = L.pwd.a; p2.value = L.pwd.b;     // typed text survives Back/Forward within this session
      p1.addEventListener("input", () => { L.pwd.a = p1.value; check(); }); p2.addEventListener("input", () => { L.pwd.b = p2.value; check(); });
      const f = h("form", { onsubmit: async (ev) => {
        ev.preventDefault(); if (go.disabled) return; go.disabled = true;
        res.replaceChildren(waitLine(t("pw.working"), "wait.pw.slow", 30));
        // Level 3 (ADR 0016): kept in memory until the archive is made, so the password can reopen it right away.
        try { await alp.setupPassword(p1.value, plain ? "console" : undefined); setupPw = plain ? null : p1.value; p1.value = p2.value = ""; L.pwd = { a: "", b: "" }; await refresh(); L.signed = true; goNext(); }
        catch (e) { res.replaceChildren(banner("bad", e)); go.disabled = false; }
      } },
        h("div", { class: "field c-alp" }, codeLabel("alp", "p1", t("pw.label")), h("div", { class: "input-row" }, p1, eyeBtn([p1, p2]))),
        h("div", { class: "field c-alp" }, codeLabel("alp", "p2", t("pw.again")), p2), hint,
        h("div", { class: "banner warn" }, h("div", {}, t(plain ? "pw.plain.warning" : "pw.warning"))), res,
        h("div", { class: "actions" }, go, h("div", { class: "row" }, h("button", { class: "link", type: "button", "data-back": "", onclick: () => { L.encrypt = undefined; navigate("protect"); } }, t("common.back")),
          plain ? h("button", { class: "link later", type: "button", onclick: nopw }, t("pw.plain.none")) : null)));
      // Level 1: no password at all (the warning was confirmed on the screen before).
      async function nopw() {
        res.replaceChildren(waitLine(t("pw.plain.working"), "wait.pw.slow", 30));
        try { await alp.setupPassword("", "none"); L.pwd = { a: "", b: "" }; await refresh(); L.signed = true; goNext(); }
        catch (e) { res.replaceChildren(banner("bad", e)); }
      }
      card.append(codeHero("alp"), h("h1", {}, t(plain ? "pw.plain.title" : "pw.title")), h("p", { class: "sub" }, t(plain ? "pw.plain.sub" : "pw.sub")), f); check();
    }, { foot: t("pw.foot") });
  }
  function unlock() {
    show("unlock", (card, cur) => {
      const pw = h("input", { type: "password", id: "pw", autocomplete: "current-password", maxlength: 128, "data-focus": "" });
      const res = h("div"), go = h("button", { class: "primary", type: "submit" }, t("unlock.open"));
      let timer = null, until = 0;
      cur.stop = () => clearInterval(timer);
      const wait = (secs) => {
        clearInterval(timer); until = Date.now() + secs * 1000;
        const tick = () => { const n = Math.ceil((until - Date.now()) / 1000);
          if (n <= 0) { clearInterval(timer); res.replaceChildren(); go.disabled = false; pw.focus(); return; }
          go.disabled = true; res.replaceChildren(h("div", { class: "banner warn", role: "alert" }, t("unlock.wait", { n }))); };
        tick(); timer = setInterval(tick, 1000);
      };
      const f = h("form", { onsubmit: async (ev) => {
        ev.preventDefault(); if (go.disabled || !pw.value) return; go.disabled = true;
        res.replaceChildren(waitLine(t("unlock.working"), "wait.unlock.slow", 30));
        try { await alp.unlock(pw.value); pw.value = ""; await refresh(); L.welcomed = true; L.signed = true; detectMode(); touch(); goNext(); }
        catch (e) {
          pw.value = ""; go.disabled = false;
          if (e.key === "password.wait" && e.retryAfter) return wait(e.retryAfter);
          res.replaceChildren(banner("bad", e)); pw.focus();
        }
      } }, h("div", { class: "field c-alp" }, codeLabel("alp", "pw", t("pw.label")), h("div", { class: "input-row" }, pw, eyeBtn([pw]))), res,
        h("div", { class: "actions" }, go), h("p", { class: "fine" }, S.lock.tpm === "off" ? t("unlock.tpm_off") : t("unlock.forgot")));
      card.append(codeHero("alp"), h("h1", {}, t("unlock.title")), h("p", { class: "sub" }, t("unlock.sub")), f);
      if (S.lock.wait > 0) wait(S.lock.wait);
    }, { foot: t("unlock.foot") });
  }

  /* ---------- 3. network ---------- */
  function network() {
    show("network", (card, cur) => {
      const body = h("div");
      card.append(hero("wifi"), h("h1", {}, t("net.title")), h("p", { class: "sub" }, t("net.sub")), body);
      let nets = L.net.nets, chosen = null, token = 0;
      cur.busy = false;
      const leaveNet = () => { if (L.netFrom) { L.netFrom = false; L.view = L.netBack || null; route(true); } else goNext(); };
      const backFn = () => { if (L.netFrom) leaveNet(); else { L.welcomed = false; goNext(); } };
      const later = () => { skipStep("network"); };
      const laterBtn = () => (L.mode === "setup" && !L.netFrom ? h("button", { class: "link later", type: "button", onclick: later }, t("common.later")) : null);
      const laterNote = () => (L.mode === "setup" && !L.netFrom ? h("p", { class: "fine later-note" }, t("net.later.note")) : null);
      function showConnected() {
        body.replaceChildren(h("div", { class: "banner ok" }, S.net.kind === "wifi" && S.net.ssid ? t("net.connected", { name: S.net.ssid }) : t("net.connected.cable")),
          h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: leaveNet }, t("common.continue")),
            h("div", { class: "row" }, S.net.wifiPresent ? h("button", { class: "link", onclick: () => { cur.change = true; nets ? renderList() : look(); } }, t("net.another")) : null,
              h("button", { class: "link", "data-back": "", onclick: backFn }, t("common.back")))));
        body.querySelector(".primary").focus();
      }
      const waiting = (txt, slowKey, after) => waitLine(txt, slowKey || "wait.generic.slow", after || 45);

      async function look() {
        const my = ++token;
        body.replaceChildren(waiting(t("net.wifi.looking"), "wait.wifi.look.slow", 15));
        try { nets = L.net.nets = await alp.wifiNetworks(); } catch (e) { if (my === token) { nets = []; renderList(e); } return; }
        if (my === token) renderList();
      }
      // T20: every control sits in one toolbar at the top of the Wi-Fi page, above the list.
      // T17: one tap (or click) on a network chooses it and opens the password step; a double click must not do more.
      function renderList(err) {
        const list = [...(nets || [])].sort((a, b) => b.signal - a.signal);
        const okNet = (n) => n.security === "open" || n.security === "psk";
        let first = list.findIndex((n) => okNet(n) && n.ssid === L.net.chosen);
        if (first < 0) first = list.findIndex(okNet);
        let picked = false;
        const choose = (ssid) => {
          if (picked || !ssid) return; picked = true;
          chosen = list.find((n) => n.ssid === ssid); L.net.chosen = ssid; askPassword();
        };
        const f = h("form", { onsubmit: (ev) => { ev.preventDefault(); choose(f.elements.net && f.elements.net.value); } });
        const fs = h("fieldset", { class: "list" }, h("legend", { class: "sr" }, t("net.wifi.choose")));
        list.forEach((n, i) => {
          const ok = okNet(n);
          const sub = (n.security === "open" ? t("net.wifi.open") + " · " : "") + (ok ? t("net.wifi.signal", { n: n.signal }) : t(n.security === "8021x" ? "net.wifi.enterprise" : "net.wifi.wep"));
          // detail 0 = a click made by the keyboard (arrow keys move the choice, Enter continues): only a real tap or click opens the next step.
          fs.append(h("label", { class: "choice" + (ok ? "" : " off"), onclick: (ev) => { if (ok && ev.detail > 0) choose(n.ssid); } },
            h("input", { type: "radio", name: "net", value: n.ssid, checked: i === first, disabled: !ok }),
            h("span", { class: "main" }, h("span", { class: "t" }, n.ssid), h("span", { class: "s" }, sub)),
            h("span", { class: "signal", dir: "ltr", "aria-hidden": "true" }, "●".repeat(n.signal) + "○".repeat(4 - n.signal))));
        });
        f.append(h("div", { class: "toolbar" },
          h("button", { class: "secondary", type: "button", "data-back": "", onclick: backFn }, t("common.back")),
          h("button", { class: "secondary", type: "button", onclick: look }, t("net.wifi.look")),
          laterBtn(), first >= 0 ? h("button", { class: "primary", type: "submit" }, t("common.continue")) : null));
        if (err) f.append(banner("bad", err));
        if (list.length) f.append(h("p", { class: "fine tap-hint" }, t("net.wifi.tap")), fs); else if (!err) f.append(h("p", { class: "fine" }, t("net.wifi.none")));
        const note = laterNote(); if (note) f.append(note);
        body.replaceChildren(f);
        (f.querySelector("input:checked") || f.querySelector(".toolbar button")).focus();
      }
      function askPassword(err) {
        const open = chosen.security === "open";
        const pw = open ? null : h("input", { type: "password", id: "pw", autocomplete: "off", maxlength: 63, "data-focus": "" });
        if (pw) {
          if (L.net.chosen === chosen.ssid) pw.value = L.net.pw;      // kept in memory for this session only
          pw.addEventListener("input", () => { L.net.pw = pw.value; });
        }
        const eye = pw ? eyeBtn([pw]) : null;
        const go = h("button", { class: "primary", type: "submit" }, t("net.wifi.connect"));
        const f = h("form", { onsubmit: (ev) => { ev.preventDefault(); connect(chosen, pw ? pw.value : ""); } },
          h("div", { class: "toolbar" }, h("button", { class: "secondary", type: "button", "data-back": "", onclick: () => renderList() }, t("common.back")), go),
          h("h2", {}, open ? t("net.wifi.open.title", { name: chosen.ssid }) : t("net.wifi.password", { name: chosen.ssid })),
          pw ? h("div", { class: "field" }, h("label", { for: "pw" }, t("net.wifi.password.label")), h("div", { class: "input-row" }, pw, eye)) : h("p", { class: "fine" }, t("net.wifi.open.note")),
          pw ? h("p", { class: "fine" }, t("net.wifi.remember")) : null, err ? banner("bad", err) : null);
        // The second click of a double click must not land on the new step's buttons.
        body.style.pointerEvents = "none"; setTimeout(() => { body.style.pointerEvents = ""; }, 450);
        body.replaceChildren(f); (pw || go).focus();
      }
      async function connect(net, password) {
        cur.busy = true;
        body.replaceChildren(waiting(t("net.wifi.connecting", { name: net.ssid }), "wait.wifi.connect.slow", 20));
        const fail = (e) => { cur.busy = false; net.security === "open" ? renderList(e) : askPassword(e); };
        try { await alp.wifiConnect(net.ssid, password); } catch (e) { return fail(e); }
        for (let i = 0; i < 90; i++) {            // the job runs up to about 60 s; progress is in the state
          await sleep(1000); if (current !== cur) return;
          if (!(await refresh())) continue;
          const job = S.net.job;
          if (job && job.state === "failed") return fail(job.error);
          if (S.net.online && (!job || job.state !== "running")) { body.replaceChildren(h("div", { class: "banner ok" }, t("net.wifi.connected"))); L.net.pw = ""; await sleep(1000); return leaveNet(); }
        }
        fail({ key: "wifi.no_address" });
      }
      if (!S.net.wifiPresent) {
        body.append(h("div", { class: "banner warn" }, t("net.nowifi")), h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: async () => { await refresh(); route(true); } }, t("net.cable.again")),
          h("button", { class: "link", "data-back": "", onclick: backFn }, t("common.back")), laterBtn()), laterNote());
      } else if (S.net.online) {
        showConnected();
      } else if (nets && L.net.chosen && L.net.pw && nets.find((n) => n.ssid === L.net.chosen)) {
        chosen = nets.find((n) => n.ssid === L.net.chosen); askPassword();
      } else if (nets) {
        renderList();
      } else {
        look();
      }
      return { update(s) { if (!cur.busy && s.net.online && !L.view && !cur.change) route(true); else if (!cur.busy && s.net.waiting && !body.querySelector(".waiting-addr")) { /* cable in, no address yet: keep list; show hint once */
        body.append(h("p", { class: "fine waiting-addr" }, t("net.waiting_address"))); } } };
    });
  }

  /* ---------- 3. where to keep the archive + progress ---------- */
  function where() {
    show("where", (card, cur) => {
      if (S.lock.state === "setup") {   // the archive is locked with the password, so it cannot exist before it
        card.append(codeHero("alp"), h("h1", {}, t("where.needpw.title")), h("p", { class: "sub" }, t("where.needpw.sub")),
          h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: () => { L.skip.password = false; navigate("password"); } }, t("where.needpw.go")),
            h("div", { class: "row" }, h("button", { class: "link", "data-back": "", onclick: () => stepBack("password") }, t("common.back")),
              h("button", { class: "link later", onclick: () => { setupPw = null; skipStep("where"); } }, t("common.later")))));
        return;
      }
      if (S.arch.state === "open") {   // already done: never ask again
        const nm = S.arch.place ? S.arch.place.label || S.arch.place.name : "";
        card.append(hero("disk"), h("h1", {}, t("where.done.title")), h("p", { class: "sub" }, t("where.done.sub", { name: nm })),
          h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: goNext }, t("common.continue")),
            h("button", { class: "link", "data-back": "", onclick: () => stepBack("password") }, t("common.back"))));
        return;
      }
      const body = h("div");
      card.append(hero("disk"), h("h1", {}, t("where.title")), h("p", { class: "sub" }, t("where.sub")), body);
      let cands = [];
      const initialErr = S.arch.state === "failed" ? (S.arch.error || (S.arch.job && S.arch.job.error) || { key: "archive.failed" }) : null;
      let stopped = false, picking = false, sig = "";
      cur.stop = () => { stopped = true; };
      body.append(waitLine(t("where.looking"), "wait.generic.slow", 30));
      async function scan() {   // every few seconds until a disk is chosen, so a disk plugged in later shows up by itself
        while (!stopped && !picking) {
          try {
            const r = await alp.candidates(); const s = JSON.stringify(r.map((c) => [c.partuuid, c.usable, c.free_bytes]));
            if (stopped) return;
            if (s !== sig && !picking) { sig = s; cands = r; list(initialErr); }
          } catch (e) { if (!sig) { body.replaceChildren(banner("bad", e)); } }
          await sleep(4000);
        }
      }
      scan();
      function list(err) {
        card.classList.remove("frame-rec");
        const f = h("form", { onsubmit: (ev) => { ev.preventDefault(); const c = cands.find((x) => x.partuuid === f.elements.c.value); if (c) { picking = true; L.disk.partuuid = c.partuuid; confirmStep(c); } } });
        const fs = h("fieldset", { class: "list" }, h("legend", { class: "sr" }, t("where.title")));
        const firstOk = cands.findIndex((c) => c.usable), blocked = [];
        picking = false;
        const pick = cands.findIndex((c) => c.usable && c.partuuid === L.disk.partuuid);
        const sel = pick >= 0 ? pick : firstOk;
        const line = (c) => [c.disk_model, fmtGB(c.size_bytes), c.fs && String(c.fs).toUpperCase()].filter(Boolean).join(" · ");
        cands.forEach((c, i) => {
          if (c.usable) {
            fs.append(h("label", { class: "choice" }, h("input", { type: "radio", name: "c", value: c.partuuid, checked: i === sel }),
              h("span", { class: "main" }, h("span", { class: "t" }, c.label || c.name),
                h("span", { class: "s" }, line(c) + " · " + t("where.free", { free: fmtGB(c.free_bytes) }) + " · " + t("where.room", { photos: roomPhotos(c.max_size_bytes || c.free_bytes) })),
                c.note ? h("span", { class: "hint" }, c.note) : null),   // storage v2: "not removed safely", repaired first
              c.recommended ? h("span", { class: "tag" }, t("where.recommended")) : null));
            return;
          }
          // Every disk that cannot be used is shown with the server's reason (one sentence, includes what to do) and its code. ALPArchive never erases it.
          blocked.push(h("div", { class: "choice blocked" }, iconEl("disk"),
            h("span", { class: "main" }, h("span", { class: "t" }, c.label || c.name), h("span", { class: "s" }, line(c)),
              h("span", { class: "why" }, c.reason || t("where.blocked")), c.code ? h("span", { class: "s", dir: "ltr", style: "font-family:'DejaVu Sans Mono',monospace;font-size:.8em" }, t("where.code", { code: c.code })) : null,
              c.has_archive ? h("span", { class: "hint" }, t("where.hint.archive")) : null,
              c.can_prepare ? h("span", { class: "hint" }, t("where.prepare.hint")) : null),
            c.has_archive ? h("span", { class: "side" }, h("button", { class: "secondary", type: "button", onclick: () => { picking = true; reopenPwStep(c); } }, t("where.reopen.short")))
              : c.can_prepare && !blocked.some((b) => b.dataset.disk === c.disk) ? h("span", { class: "side" }, h("button", { class: "secondary", type: "button", onclick: () => prepareDisk(c) }, t("where.prepare"))) : null));
          blocked[blocked.length - 1].dataset.disk = c.disk;
        });
        if (!cands.length) f.append(h("div", { class: "plug" }, h("div", { class: "plugicon", "aria-hidden": "true" }, "\u23CF"), h("p", { class: "t" }, t("where.nodisk")), h("p", { class: "fine" }, t("where.plug")), h("p", { class: "waiting" }, h("span", { class: "spinner" }), t("where.plug.waiting"))));
        else {
          if (cands.some((c) => c.usable)) f.append(fs); else f.append(h("div", { class: "banner warn" }, t("where.none.usable")));
          if (blocked.length) f.append(h("div", { class: "list", style: "margin-block-start:.6rem" }, blocked));
          f.append(h("p", { class: "fine" }, t("where.plug.more")));
        }
        if (err) f.append(banner("bad", err));
        f.append(h("div", { class: "actions" }, firstOk >= 0 ? h("button", { class: "primary", type: "submit" }, t("common.continue")) : null,
          h("div", { class: "row" }, h("button", { class: "link", type: "button", "data-back": "", onclick: () => stepBack("password") }, t("common.back")),
          h("button", { class: "link later", type: "button", onclick: () => { setupPw = null; skipStep("where"); } }, t("common.later")))), h("p", { class: "fine later-note" }, t("where.later.note")));
        body.replaceChildren(f); (f.querySelector("input:checked") || f.querySelector(".link")).focus();
      }
      // Level 3 (ADR 0016): the ALP password the archive was used with reopens it; the recovery code is the other way.
      function reopenPwStep(c) {
        const pw = h("input", { type: "password", id: "rop", autocomplete: "off", "data-focus": "" }), res = h("div");
        const f = h("form", { onsubmit: async (ev) => { ev.preventDefault(); if (!pw.value) return;
          try { await alp.reopenWithPassword(c.partuuid, pw.value); pw.value = ""; await refresh(); route(true); }
          catch (e) { res.replaceChildren(banner("bad", e)); } } },
          h("h2", {}, t("where.reopen")), h("p", { class: "fine" }, t("where.reopenpw.sub", { name: c.label || c.name })),
          h("div", { class: "field c-alp" }, codeLabel("alp", "rop", t("where.reopenpw.label")), pw), res,
          h("div", { class: "actions" }, h("button", { class: "primary", type: "submit" }, t("where.reopen.go")),
            h("div", { class: "row" }, h("button", { class: "link", type: "button", onclick: () => recoverStep(c) }, t("where.reopenpw.words")),
              h("button", { class: "link", type: "button", "data-back": "", onclick: () => { sig = ""; scan(); } }, t("common.back")))));
        body.replaceChildren(f); pw.focus();
      }
      function recoverStep(c) {   // reopen an existing archive on another disk with its recovery key
        const key = h("textarea", { id: "rk", rows: 3, autocomplete: "off", autocapitalize: "none", spellcheck: "false", dir: "ltr", "data-focus": "" }), res = h("div");
        const pp = h("input", { type: "password", id: "rpp", autocomplete: "off" });
        const f = h("form", { onsubmit: async (ev) => { ev.preventDefault(); const v = key.value.trim(); if (!v) return;
          const tw = v.toLowerCase().split(/[\s,]+/).filter(Boolean);
          if (tw.length === 8 && !tw.every((x) => /^[cbdefghijklnrtuv]{8}$/.test(x))) { res.replaceChildren(banner("bad", { key: "recovery.chain_code" })); return; }   // a chain code, not a recovery code
          try { await alp.recoverArchive(c.partuuid, v, pp.value); await refresh(); route(true); }
          catch (e) { res.replaceChildren(banner("bad", e, e.key === "recovery.word_unknown" && e.index ? t("err.recovery.word_unknown_n", { n: e.index }) : null)); } } },
          h("h2", {}, t("where.reopen")), h("p", { class: "fine" }, t("where.reopen.sub", { name: c.label || c.name })),
          h("div", { class: "field c-rec" }, codeLabel("rec", "rk", t("recovery.key")), key),
          h("div", { class: "field c-rec" }, codeLabel("rec", "rpp", t("where.reopen.pp")), pp), res,
          h("div", { class: "actions" }, h("button", { class: "primary", type: "submit" }, t("where.reopen.go")), h("button", { class: "link", type: "button", "data-back": "", onclick: () => { sig = ""; scan(); } }, t("common.back"))));
        card.classList.add("frame-rec");   // recovery screens have their own look (T24)
        body.replaceChildren(f); key.focus();
      }
      // Storage v2 (ADR 0014): no size to choose; the archive grows into the free space. The disk is checked first.
      function confirmStep(c) {
        const spw = h("div", { class: "field c-alp", hidden: true }, codeLabel("alp", "spw", t("where.serious.pw")), h("input", { type: "password", id: "spw", name: "spw", autocomplete: "off" }));
        const f = h("form", { onsubmit: (ev) => { ev.preventDefault(); create(c, !!(f.elements.serious && f.elements.serious.checked), f.elements.spw.value); } },
          h("h2", {}, t("where.use.title", { name: c.label || c.name })),
          h("p", {}, t("where.grow", { free: fmtGB(c.free_bytes), photos: roomPhotos(c.max_size_bytes || c.free_bytes) })),
          c.note ? h("p", { class: "fine" }, c.note) : null,
          h("p", { class: "fine" }, t("where.confirm", { name: c.label || c.name }) + " " + t("where.check.note")),
          h("div", { class: "banner warn" }, h("div", {}, t("key.warning"))),
          S.lock.protection === "none" || S.lock.protection === "console"
            ? h("div", { class: "banner warn" }, h("div", {}, t("where.plain.note")))
            : h("label", { class: "choice" }, h("input", { type: "checkbox", name: "serious", onchange: (ev) => { spw.hidden = !ev.target.checked; } }),
              h("span", { class: "main" }, h("span", { class: "t" }, t("where.serious")), h("span", { class: "s" }, t("where.serious.sub")))),
          spw, h("div", { id: "cerr" }),
          h("div", { class: "actions" }, h("button", { class: "primary", type: "submit" }, t("where.create")),
            h("button", { class: "link", type: "button", "data-back": "", onclick: () => { sig = ""; scan(); } }, t("common.back"))));
        body.replaceChildren(f); f.querySelector(".primary").focus();
      }
      // "Prepare this disk": password, a warning listing every partition that will be erased, slide to confirm.
      async function prepareDisk(c) {
        picking = true;
        const parts = cands.filter((x) => x.disk === c.disk);
        const pw = await confirmPassword("where.prepare.pw");
        if (!pw) { sig = ""; scan(); return; }
        const item = (x) => [x.label || x.name, x.fs ? String(x.fs).toUpperCase() : t("where.prepare.nofs"), fmtGB(x.size_bytes)].join(" · ");
        dangerScreen({ title: t("danger.prepare.title", { name: c.disk_model || c.disk }), lead: t("danger.prepare.lead"),
          lose: parts.map(item), keep: [t("danger.prepare.keep1"), t("danger.prepare.keep2")], warn: t("danger.cannot"),
          slide: t("danger.prepare.slide"), working: t("danger.prepare.working"), onCancel: () => route(true),
          onConfirm: async () => {
            await alp.prepareDisk(c.disk_id, parts.map((x) => x.partuuid), pw);
            for (const until = Date.now() + 20 * 60000; ;) {   // job disk.prepare (about a minute, cannot be cancelled)
              if (Date.now() > until) throw { key: "disk.failed" };
              await sleep(1000); await refresh();
              const j = S.arch.job;
              if (!j || j.id !== "disk.prepare" || j.state === "running") continue;
              if (j.state === "failed") throw j.error || { key: "disk.failed" };
              break;
            }
            dangerDone(t("danger.prepare.ok"), () => route(true));
          } });
      }
      async function create(c, serious, pw) {
        const btn = body.querySelector(".primary"); btn.disabled = true; L.disk = { partuuid: c.partuuid, size: null };
        try { await alp.createArchive(c.partuuid, serious, pw, serious ? null : setupPw); setupPw = null; await refresh(); route(true); }
        catch (e) { btn.disabled = false; body.querySelector("#cerr").replaceChildren(banner("bad", e)); }
      }
    });
  }
  function progress() {
    show("progress", (card) => {
      const bar = h("i"), pct = waitLine(t("progress.step.preparing"), "wait.archive.slow", 240);
      const wrap = h("div", { class: "progress", role: "progressbar", "aria-valuemin": 0, "aria-valuemax": 100 }, bar);
      const cerr = h("div"), cancel = h("button", { class: "secondary", type: "button", onclick: async () => {
        cancel.disabled = true; cancel.textContent = t("progress.cancelling");
        try { await alp.cancelArchive(); await refresh(); L.view = null; route(true); }
        catch (e) { cancel.disabled = false; cancel.textContent = t("progress.cancel"); cerr.replaceChildren(banner("bad", e)); }
      } }, t("progress.cancel"));
      card.append(hero("lock"), h("h1", {}, t("progress.title")), h("p", { class: "sub" }, t("progress.sub")), wrap, pct, h("div", { class: "banner warn" }, h("div", {}, t("key.warning"))), cerr,
        h("div", { class: "actions" }, cancel, h("p", { class: "fine later-note" }, t("progress.cancel.note"))));
      return { update(s) {
        const j = s.arch.job || {}, p = Math.max(5, j.percent || 0);
        bar.style.inlineSize = p + "%"; wrap.setAttribute("aria-valuenow", p);
        // Lead fix 2026-10-21 (field bug): while reserving, or when the disk is slow to answer, show the server's sentence (bytes, speed, time left, or "still working").
        // Storage v2: the repair note (preparing) and the disk test (checking: MB written and speed) also come from the server.
        const live = j.message && (j.step === "reserving" || (j.id === "archive.create" && (j.step === "preparing" || j.step === "checking")) || (j.seconds_since_progress || 0) >= 60);
        const key = j.id === "archive.create" && j.step === "checking" ? "progress.step.disk_check" : "progress.step." + j.step;
        const stepTxt = T[key] ? t(key) : j.message || t("danger.working");
        pct.now(stepTxt); pct.why(live && j.message !== stepTxt ? j.message : null);   // T21: what happens now, and the server's detail
      } };
    }, { foot: "" });
  }

  /* ---------- 4. phone ---------- */
  function needsDisk(card, step) {   // a step reached early: say what it needs first
    card.append(hero(step === 5 ? "owl" : "phone"), h("h1", {}, t("step." + step)), h("p", { class: "sub" }, t("needs.disk.sub")),
      h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: () => jumpStep(4) }, t("needs.disk.go")),
        h("button", { class: "link", "data-back": "", onclick: () => jumpStep(step - 1) }, t("common.back"))));
  }
  // T22: the last setup step. First which app to get, then (once) the certificate, then how to connect:
  // scan the QR code in the Immich app or type the address, then sign in with the email and the ALP password (T23).
  // phone/pair always answers with the addresses (2026-10-26), also while the photo service is still starting.
  // ADR 0017: a phone that scanned the pairing QR waits here; the same code shows on the phone. Slide to approve.
  function approvePhone(r) {
    const err = h("div");
    sheet(h("h2", { style: "margin-top:0" }, t("pair.approve.title", { name: r.name || r.model || t("dev.unknown") })),
      h("p", { class: "fine" }, t("pair.approve.sub")), h("div", { class: "pcode", dir: "ltr" }, r.code), err,
      slideConfirm(t("pair.approve.slide"), async () => {
        try { await alp.pairApprove(r.id); closeModal(); await refresh(); if (current) current.update(S); }
        catch (e) { err.replaceChildren(banner("bad", e)); }
      }),
      h("div", { class: "row" }, h("button", { class: "link", type: "button", "data-back": "", onclick: async () => { try { await alp.pairReject(r.id); } catch (e) { /* gone already */ } closeModal(); } }, t("pair.approve.no"))));
  }
  // The pairing QR (ADR 0017), for the ALPArchive app; it lives 5 minutes.
  function phone() {
    show("phone", (card, cur) => {
      const setup = L.mode === "setup";
      const backFn = () => (setup ? navigate(recoveryStep() ? "recovery" : "where") : (L.phoneDone = true, goBack("settings")));
      const laterBtn = () => (setup ? h("button", { class: "secondary later", type: "button", onclick: () => { skipStep("phone"); } }, t("common.later")) : null);
      if (S.arch.state !== "open") return needsDisk(card, 6);
      if (!S.net.online) {   // the phone finds ALPArchive through the home network
        card.append(hero("phone"), h("h1", {}, t("phone.title")), h("p", { class: "sub" }, t("phone.needs_net")),
          h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: () => { L.skip.network = false; navigate("network"); } }, t("phone.needs_net.go")),
            h("div", { class: "row" }, h("button", { class: "link", "data-back": "", onclick: backFn }, t("common.back")), laterBtn())));
        return;
      }
      const go = h("button", { class: "primary", type: "button", disabled: setup && !S.phone.paired, onclick: () => { L.phoneDone = true; setup ? goNext() : goBack("settings"); } }, setup ? t("common.continue") : t("common.done"));
      const stageBack = h("button", { class: "secondary", type: "button", "data-back": "", onclick: () => (stage > 0 ? render(stage - 1) : backFn()) }, t("common.back"));
      const body = h("div", { class: "phonebody" });
      const status = waitLine(t("phone.waiting"), "phone.slow", 300);
      const help = h("div", { class: "banner warn hidden" }, h("div", {}, t("phone.help")));
      const okBox = h("div", { class: "banner ok hidden", role: "status" });
      // 0.2.5: one QR code on the screen, the one the ALPArchive app scans (pair/open). The free photo app's way
      // (its server address QR, the certificate QR and the typed address) sits under "Other ways to connect".
      const appQr = h("div", { class: "appqr-code" }), appLeft = h("p", { class: "fine" });
      const appBox = h("figure", { class: "appqr" }, appQr,
        h("figcaption", {}, h("span", { class: "qrtag" }, iconEl("phone"), t("pair.box.title")), h("span", { class: "fine" }, t("pair.qr.sub")), appLeft));
      const others = h("details", { class: "techd others" }, h("summary", {}, iconEl("chevron"), h("span", {}, t("phone.others"))),
        h("div", { class: "techd-body" }, h("p", { class: "fine" }, t("pair.temporary")), body));
      card.append(hero("phone"), h("h1", {}, t("phone.title")), h("p", { class: "sub" }, t(setup ? "phone.sub" : "phone.sub.more")), appBox, status, okBox, others,
        h("div", { class: "row" }, h("button", { class: "link", type: "button", "data-ico": "", onclick: () => help.classList.toggle("hidden") }, t("phone.help.toggle"))), help,
        h("div", { class: "toolbar bottom" }, stageBack, laterBtn(), go));
      let stopped = false, R = null, stage = 0, pairT = null, pwAsked = false, sig = "", qrTick = null, qrOpen = false;
      cur.stop = () => { stopped = true; clearTimeout(pairT); clearInterval(qrTick); if (qrOpen) alp.pairClose().catch(() => {}); };
      async function openAppQr() {
        clearInterval(qrTick); appLeft.textContent = "";
        try {
          const r = await alp.pairOpen(); if (stopped) { alp.pairClose().catch(() => {}); return; }
          qrOpen = true;
          appQr.replaceChildren(r.qr_svg ? svgImg(r.qr_svg, "qr", t("pair.qr.alt")) : h("p", { dir: "ltr", class: "sha" }, r.uri || ""));
          let n = r.expires_in || 300;
          qrTick = setInterval(() => {
            n -= 1;
            if (n > 0) { appLeft.textContent = t("pair.qr.left", { m: Math.floor(n / 60), s: String(n % 60).padStart(2, "0") }); return; }
            clearInterval(qrTick); qrOpen = false;
            appQr.replaceChildren(h("button", { class: "secondary", type: "button", onclick: openAppQr }, t("phone.qr.new")));
            appLeft.textContent = t("phone.qr.expired");
          }, 1000);
        } catch (e) {   // an older alpd has no app pairing: the other ways are the way
          appQr.replaceChildren(banner("bad", e)); others.open = true;
        }
      }
      openAppQr();
      const certStage = () => !!(R && R.certificate_qr_svg && /^https:/.test(R.server_url || ""));
      const addresses = () => {
        const a = R && Array.isArray(R.addresses) && R.addresses.length ? R.addresses : S.net.addresses.map((x) => x.url || "https://" + x.ip);
        return a.length ? a : R && R.server_url ? [R.server_url] : [];
      };
      // Ask the server again until a phone can sign in (the photo service may still be starting).
      async function pair(password) {
        clearTimeout(pairT);
        try { R = await alp.pair(password); if (R.ready === undefined) R.ready = true; }   // an older alpd answers only when ready
        catch (e) {
          if (e.key === "password.wrong" || e.key === "password.wait") { askPassword(e); return; }
          R = Object.assign({}, R || {}, { ready: false, immich: { state: e.key === "immich.starting" ? "starting" : S.immich.state, reason: e.key === "immich.starting" ? null : (T["err." + e.key] ? t("err." + e.key) : e.serverMessage || t("err.generic")) } });
        }
        if (stopped) return;
        if (R.needs_password && !pwAsked) { askPassword(); return; }
        const nsig = JSON.stringify([R.addresses, R.qr_text, R.email, R.password, R.certificate_sha256]);
        if (body.querySelector("#ppw") || (stage === 2 && nsig !== sig)) render(stage);   // new address or QR: draw the step again
        sig = nsig;
        cur.update(S);
        if (!R.ready) pairT = setTimeout(() => pair(), 5000);
      }
      function askPassword(err) {   // the family account does not use the ALP password yet: ask for it once
        pwAsked = true;
        const pw = h("input", { type: "password", id: "ppw", autocomplete: "current-password", maxlength: 128 });
        const f = h("form", { onsubmit: (ev) => { ev.preventDefault(); if (pw.value) { const v = pw.value; pw.value = ""; pair(v); } } },
          h("p", {}, t("phone.needs_pw")), h("div", { class: "field c-alp" }, codeLabel("alp", "ppw", t("pw.label")), h("div", { class: "input-row" }, pw, eyeBtn([pw]))),
          err ? banner("bad", err) : null, h("div", { class: "actions" }, h("button", { class: "primary", type: "submit" }, t("confirm.go"))));
        body.replaceChildren(f); pw.focus();
      }
      const appTile = (os, store) => h("div", { class: "apptile" }, iconEl("download"), h("div", {}, h("b", {}, t("phone.app." + os)), h("span", { class: "fine" }, t("phone.app.store", { store, app: t("phone.app.name") }))));
      function render(n) {
        stage = n; card.classList.toggle("phone-connect", n > 0);   // more room for the QR code and the address
        if (n === 0) {   // which app
          body.replaceChildren(h("h2", {}, t("phone.app.title")), h("div", { class: "apps" }, appTile("android", "Google Play"), appTile("iphone", "App Store")),
            h("p", { class: "fine" }, t("phone.app.which")),
            h("div", { class: "actions" }, h("button", { class: "primary", type: "button", "data-focus": "", onclick: () => render(certStage() ? 1 : 2) }, t("phone.app.have"))));
        } else if (n === 1) {   // once: let the phone trust ALPArchive's own certificate
          body.replaceChildren(h("div", { class: "pair" }, svgImg(R.certificate_qr_svg, "qr", t("phone.cert.qr.alt")),
            h("div", {}, h("h2", { style: "margin-top:0" }, t("phone.cert.title")),
              h("ol", { class: "steps-list" }, h("li", {}, t("phone.cert.step1"), " ", h("span", { dir: "ltr" }, R.certificate_url || "")), h("li", {}, t("phone.cert.android")), h("li", {}, t("phone.cert.ios"))),
              h("button", { class: "primary", type: "button", "data-focus": "", onclick: () => render(2) }, t("phone.cert.next")))));
        } else {   // connect: QR code or the address typed, then email and ALP password
          const list = addresses();
          const pwBox = R && R.password && R.uses_main_password !== true ? h("div", { class: "addr pw", dir: "ltr" }, R.password)   // older alpd: family password
            : h("div", { class: "pwnote c-alp" }, codeIcon("alp"), t("phone.use_alp"));
          body.replaceChildren(h("div", { class: "pair" },
            R && R.qr_svg ? h("figure", { class: "qrbox" }, svgImg(R.qr_svg, "qr", t("phone.qr.alt")), h("figcaption", { class: "fine" }, t("phone.qr.caption"))) : h("div", { class: "qr qr-none" }, iconEl("qr")),
            h("div", {}, h("h2", { style: "margin-top:0" }, t("phone.connect.title")),
              h("ol", { class: "steps-list" },
                h("li", {}, t("phone.connect.open")),
                h("li", {}, t("phone.connect.address"), list.length ? h("div", { class: "addrs" }, list.map((u) => h("div", { class: "addr", dir: "ltr" }, u))) : h("div", { class: "banner warn" }, t("phone.noaddress"))),
                h("li", {}, t("phone.connect.signin"), h("dl", { class: "kv" }, h("dt", {}, t("phone.email")), h("dd", { dir: "ltr" }, (R && R.email) || S.phone.account || "family@alparchive.local"),
                  h("dt", {}, t("phone.password")), h("dd", {}, pwBox))),
                h("li", {}, t("phone.connect.backup"))))));
          if (R && R.certificate_sha256 && !help.querySelector(".sha")) help.firstChild.append(" ", t("phone.cert"), " ", h("span", { class: "sha", dir: "ltr" }, R.certificate_sha256));
        }
      }
      render(window.useMock && params.get("pstage") === "2" ? 2 : 0); pair();   // &pstage=2: QA screenshot of the connect step
      return { update(s) {
        if (s.phone.paired) {
          status.classList.add("hidden"); okBox.classList.remove("hidden");
          const d = s.phone.devices[0];
          okBox.textContent = (d ? t("phone.connected.dev", { name: deviceLine(d) }) : t("phone.connected")) + " " + (s.phone.photos > 0 ? t("phone.uploading", { done: num(s.phone.photos) }) : t("phone.autobackup"))
            + (s.phone.now.length ? " " + t("dev.now") + " " + s.phone.now.join(", ") : "");
          go.disabled = false; if (document.activeElement === document.body) go.focus(); return;
        }
        if (!R) return;
        if (R.ready) { status.now(t("phone.waiting")); status.why(null); return; }
        const im = s.immich.state === "ready" ? (R.immich || {}) : { state: s.immich.state, reason: (R.immich || {}).reason };
        status.now(im.state === "starting" ? t("phone.gallery_starting") : im.state === "failed" ? t("phone.gallery_failed") : t("phone.preparing"));
        status.why(s.immich.detail || im.reason || null);
      } };
    }, { wide: true });
  }

  /* ---------- 5. chain ---------- */
  function chain() {
    show("chain", (card) => {
      const f = h("form", { onsubmit: (ev) => { ev.preventDefault(); f.elements.c.value === "start" ? startChain() : navigate("chain-join"); } },
        h("fieldset", { class: "list" }, h("legend", { class: "sr" }, t("chain.title")),
          ...[["start", "chain.start", "chain.start.sub"], ["join", "chain.join", "chain.join.sub"]].map(([v, a, b], i) =>
            h("label", { class: "choice" }, h("input", { type: "radio", name: "c", value: v, checked: i === 0 }), h("span", { class: "main" }, h("span", { class: "t" }, t(a)), h("span", { class: "s" }, t(b)))))),
        h("div", { id: "cerr" }),
        h("div", { class: "actions" }, h("button", { class: "primary", type: "submit" }, t("common.continue")),
          h("button", { class: "link", type: "button", "data-back": "", onclick: () => navigate("settings") }, t("common.back"))));
      card.append(codeHero("chain"), h("h1", {}, t("chain.title")), h("p", { class: "sub" }, t("chain.sub")), f);
      async function startChain() {
        try { L.seed = await alp.chainCreate(); navigate("chain-seed"); } catch (e) { f.querySelector("#cerr").replaceChildren(banner("bad", e)); }
      }
    }, { cls: "frame-chain" });
  }
  function chainSeed() {
    show("chain-seed", (card) => {
      const words = h("ol", { class: "words", dir: "ltr" }, L.seed.words.map((w) => h("li", {}, w)));
      card.append(codeHero("chain"), h("h1", {}, t("chain.seed.title")), h("p", { class: "sub" }, t("chain.seed.sub")),
        h("div", { class: "seed" }, h("div", {}, h("p", { class: "fine", style: "margin:0" }, t("chain.seed.write")), words), svgImg(L.seed.qr_svg, "qr", t("chain.seed.qr.alt"))),
        h("div", { class: "actions" }, h("button", { class: "primary", onclick: async () => { L.seed = null; await refresh(); navigate("settings"); } }, t("chain.seed.done"))));
    }, { wide: true, cls: "frame-chain" });
  }
  function chainJoin() {
    show("chain-join", (card) => {
      const N = 8; let legacy = null;   // a pasted 24-word code of a chain made before 2026-10-24
      const ins = Array.from({ length: N }, (_, i) => h("input", { type: "text", autocomplete: "off", autocapitalize: "none", spellcheck: "false", dir: "ltr", "aria-label": t("chain.join.word", { n: i + 1 }), placeholder: String(i + 1) }));
      const res = h("div");
      ins.forEach((inp, i) => {
        inp.addEventListener("input", () => inp.removeAttribute("aria-invalid"));
        inp.addEventListener("paste", (ev) => {
          const w = (ev.clipboardData.getData("text") || "").trim().toLowerCase().split(/[\s,]+/).filter(Boolean);
          if (w.length === 15) { ev.preventDefault(); res.replaceChildren(banner("bad", { key: "chain.recovery_code" })); return; }   // word counts tell the codes apart
          if (w.length === 24) { ev.preventDefault(); legacy = w; f.requestSubmit(); return; }
          if (w.length > 1) { ev.preventDefault(); w.slice(0, N).forEach((x, j) => (ins[j].value = x)); }
        });
        inp.addEventListener("keydown", (ev) => { if (ev.key === " " && inp.value && ins[i + 1]) { ev.preventDefault(); ins[i + 1].focus(); } });
      });
      const f = h("form", { onsubmit: async (ev) => {
        ev.preventDefault(); const words = legacy || ins.map((x) => x.value.trim().toLowerCase()); legacy = null;
        res.replaceChildren(waitLine(t("chain.join.waiting"), "wait.generic.slow", 30));
        try { await alp.chainJoin(words); res.replaceChildren(h("div", { class: "banner ok" }, t("chain.joined"))); await refresh(); await sleep(1200); navigate("settings"); }
        catch (e) {
          res.replaceChildren(banner("bad", e, e.index ? t("err.chain.word_unknown_n", { n: e.index }) : null));
          if (e.index && ins[e.index - 1]) { ins[e.index - 1].setAttribute("aria-invalid", "true"); ins[e.index - 1].focus(); }
        }
      } }, h("div", { class: "wordin" }, ins), res, h("div", { class: "actions" }, h("button", { class: "primary", type: "submit" }, t("chain.join.go")),
        h("button", { class: "link", type: "button", "data-back": "", onclick: () => navigate("chain") }, t("common.back"))));
      card.append(codeHero("chain"), h("h1", {}, t("chain.join.title")), h("p", { class: "sub" }, t("chain.join.sub2")), f);
      ins[0].setAttribute("data-focus", "");
    }, { wide: true, cls: "frame-chain" });
  }

  /* ---------- helpers for everyday screens ---------- */
  function detectMode() {
    if (L.exited) { L.mode = "everyday"; return; }
    if (has("setup_progress") && !(window.useMock && params.get("screen"))) {   // T09: the server knows
      S.setup.skipped.forEach((k) => { if (L.skip[k] === undefined) L.skip[k] = true; });
      if (S.setup.complete) L.finished = true;
      if (S.lock.state === "setup") { L.mode = "setup"; return; }
      if (S.setup.complete) { L.mode = "everyday"; return; }
      // Not marked complete (a stick set up before this was kept, or setup still running): the state decides, below.
    }
    const a = S.arch.state;
    // T09: setup finished or left before a restart stays finished (kept on the stick); a locked stick asks for the
    // password on the everyday lock screen, not inside setup.
    if (S.setup.finished && S.lock.state !== "setup") { L.mode = "everyday"; return; }
    L.mode = S.lock.state === "setup" || ["none", "creating", "failed"].includes(a) || (a === "open" && (PHONE_NEXT ? S.arch.recovery !== "set" : !S.phone.paired && !S.phone.url)) ? "setup" : "everyday";
  }
  const PREFS_KEY = "alp.ui.prefs";
  function prefs() {
    let p = {};
    try { p = JSON.parse(localStorage.getItem(PREFS_KEY) || "{}"); } catch (e) { /* storage may be blocked */ }
    // ui_scale: the server's choice wins when alpd keeps prefs (T56); null = automatic.
    const scale = has("prefs") && S && S.prefs.uiScale !== undefined ? S.prefs.uiScale : p.scale;
    return { saver: p.saver === true, scale: SCALES.includes(scale) ? scale : null };
  }
  function savePrefs(p) { try { localStorage.setItem(PREFS_KEY, JSON.stringify(p)); } catch (e) { /* ignore */ } }
  // The server decides (console.idle_lock_seconds). While the gallery frame is shown, taps inside it never reach the
  // console, so the lock waits 30 minutes there (docs/plans/live-ux.md) and the server session is kept awake meanwhile.
  const GALLERY_IDLE_MS = 30 * 60 * 1000;
  const inGallery = () => !!current && current.name === "gallery";
  const idleMs = () => {
    const sec = S && S.console ? S.console.idle : 300;
    const base = sec * 1000; return inGallery() ? Math.max(base, GALLERY_IDLE_MS) : base;
  };
  let lastPing = 0;
  function touch() {
    lastAct = Date.now(); if (saverOn) stopSaver();
    if (L.signed && Date.now() - lastPing > 30000) { lastPing = Date.now(); alp.activity().catch(() => {}); }   // keeps the server session awake
  }
  // Mask a device name for the lock screen: "Dana's iPhone" -> "D•••'s iPhone".
  function maskName(n) {
    n = String(n || "");
    if (n.includes("•")) return n;
    return n.replace(/^([^\s']+)/, (w) => w.charAt(0) + "•".repeat(Math.min(3, Math.max(1, w.length - 1))));
  }
  function health(s) {
    if (s.arch.health) return s.arch.health;
    if (s.arch.state !== "open" || s.arch.space === "full" || s.index.space === "full" || s.immich.state === "failed") return "bad";
    return s.arch.space === "low" || s.index.space === "low" ? "warn" : "ok";
  }
  /* ---------- T54: one status legend (top bar, home, (i), setup) ---------- */
  // ok: blue check · note: white (i), blue frame · action: yellow (i) "Action" · now: red ! in a triangle "Now" ·
  // critical: red ! in an octagon "Stop", yellow frame, pulsing (steady with reduced motion). Colour is never the
  // only signal: each level has its own shape and, from "action" on, a word.
  const LEVELS = ["ok", "note", "action", "now", "critical"];
  function statusLevel(s) {
    if (!s) return "ok";
    if (has("status_levels") && s.status && LEVELS.includes(s.status.level)) return s.status.level;
    const w = warnings(s);   // an older alpd: the console's own reading
    if (w.some((x) => x[0] === "bad")) return "now";
    if (w.length) return L.mode === "setup" ? "note" : "action";   // T55: during setup the person can always go on
    return L.mode === "everyday" && reminders(s).length ? "note" : "ok";
  }
  const isCritical = () => statusLevel(S) === "critical";
  const LV_SVG = {
    ok: '<circle cx="12" cy="12" r="9.5"/><path d="M7.5 12.3l3 3 6-6.3"/>',
    note: '<circle cx="12" cy="12" r="9.5"/><path d="M12 11v5.5M12 7.6h.01"/>',
    action: '<circle cx="12" cy="12" r="9.5"/><path d="M12 11v5.5M12 7.6h.01"/>',
    now: '<path d="M12 3.2l9.3 16.3H2.7z"/><path d="M12 9.5v4.6M12 17h.01"/>',
    critical: '<path d="M8.3 2.8h7.4l5.5 5.5v7.4l-5.5 5.5H8.3l-5.5-5.5V8.3z"/><path d="M12 7.5v5.6M12 16.4h.01"/>',
  };
  function legendIcon(level, opts) {
    const lv = LEVELS.includes(level) ? level : "ok", o = opts || {};
    const e = h("span", { class: "lv lv-" + lv, role: "img", "aria-label": t("lv." + lv) });
    const g = h("span", { class: "lv-ico", "aria-hidden": "true" });
    g.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">' + LV_SVG[lv] + "</svg>";
    e.append(g);
    if (o.word !== false && ["action", "now", "critical"].includes(lv)) e.append(h("span", { class: "lv-word" }, t("lv.word." + lv)));
    return e;
  }
  // Critical: a full-width bar under the top bar on every screen; import, delete, format and phone backup pause.
  function criticalBar() {
    let bar = document.getElementById("critbar");
    if (!isCritical() || !L.signed && L.mode === "everyday") { if (bar) bar.remove(); return; }
    if (!bar) { bar = h("div", { id: "critbar", class: "critbar", role: "alert" }); document.querySelector(".top").after(bar); }
    const msg = (S.status && S.status.message) || "";
    fill(bar, legendIcon("critical", { word: false }), h("span", { class: "t" }, t("crit.bar")), msg ? h("span", { class: "d" }, msg) : null,
      h("button", { class: "primary", type: "button", onclick: openErrors }, t("crit.fix")));
  }
  // Opens only when the person taps a paused action, and names it; the bar under the top bar stays the main signal.
  function refusedWhileCritical(what) {
    if (!isCritical()) return false;
    sheet(legendIcon("critical", { word: false }), h("h2", {}, t("crit.paused.what." + (what || "import"))), h("p", { class: "sub" }, t("crit.paused")),
      h("div", { class: "actions" }, h("button", { class: "primary", type: "button", onclick: closeModal }, t("common.close")),
        h("button", { class: "link", type: "button", onclick: () => { closeModal(); openErrors(); } }, t("crit.what"))));
    return true;
  }
  function warnings(s) {
    const w = [];
    if (s.arch.state === "missing") { w.push(["bad", "status.bad.archive"]); return w; }
    if (!s.net.online) w.push(["warn", "status.warn.network"]);
    const sp = [s.arch.space, s.index.space];
    if (sp.includes("full") || s.immich.state === "paused") w.push(["bad", "status.bad.full"]);
    else if (sp.includes("low")) w.push(["warn", "status.warn.lowspace"]);
    if (s.immich.state === "failed") w.push(["bad", "status.bad.gallery"]);
    return w;
  }
  const DEVICE_ICON = { phone: "phone", tablet: "phone", laptop: "disk" };
  const iconEl = (name) => { const d = h("span", { class: "ico", "aria-hidden": "true" }); d.innerHTML = ICON[name] || ICON.phone; return d; };

  function lockNow() { closeModal(); alp.lock().catch(() => {}); L.signed = false; L.view = null; hist.length = 0; if (L.mode === "everyday") route(true); }
  let modalEl = null;
  function closeModal() { if (modalEl) { modalEl.remove(); modalEl = null; } }
  // Ask for the admin password again. Resolves true when the server accepts it.
  function confirmPassword(reasonKey) {
    return new Promise((resolve) => {
      closeModal();
      const pw = h("input", { type: "password", id: "mpw", autocomplete: "current-password", maxlength: 128 });
      const res = h("div"), go = h("button", { class: "primary", type: "submit" }, t("confirm.go"));
      let timer = null;
      const end = (v) => { clearInterval(timer); closeModal(); resolve(v); };
      const wait = (secs) => {
        let until = Date.now() + secs * 1000; clearInterval(timer);
        const tick = () => { const n = Math.ceil((until - Date.now()) / 1000); if (n <= 0) { clearInterval(timer); res.replaceChildren(); go.disabled = false; return; }
          go.disabled = true; res.replaceChildren(h("div", { class: "banner warn", role: "alert" }, t("unlock.wait", { n }))); };
        tick(); timer = setInterval(tick, 1000);
      };
      const f = h("form", { onsubmit: async (ev) => {
        ev.preventDefault(); if (go.disabled || !pw.value) return; go.disabled = true;
        try { const typed = pw.value; await alp.verify(typed); pw.value = ""; end(typed); }
        catch (e) { pw.value = ""; go.disabled = false; if (e.key === "password.wait" && e.retryAfter) return wait(e.retryAfter); res.replaceChildren(banner("bad", e)); pw.focus(); }
      } }, h("h2", { style: "margin-top:0" }, t(reasonKey)), h("p", { class: "fine", style: "margin:0 0 .6rem" }, t("confirm.sub")),
        h("div", { class: "field c-alp", style: "margin-top:0" }, codeLabel("alp", "mpw", t("pw.label")), h("div", { class: "input-row" }, pw, eyeBtn([pw]))), res,
        h("div", { class: "row", style: "margin-top:1.2rem" }, go, h("button", { class: "secondary", type: "button", "data-back": "", onclick: () => end(false) }, t("common.cancel"))));
      modalEl = h("div", { class: "modal", role: "dialog", "aria-modal": "true" }, h("div", { class: "sheet" }, f));
      document.body.append(modalEl); pw.focus();
    });
  }


  // Serious encryption: a new recovery code needs the extra passphrase too (typed twice). Resolves it, or null.
  function askNewPassphrase() {
    return new Promise((resolve) => {
      const a = h("input", { type: "password", id: "npa", autocomplete: "new-password" }), b = h("input", { type: "password", id: "npb", autocomplete: "new-password" }), res = h("div");
      const end = (v) => { closeModal(); resolve(v); };
      sheet(h("form", { onsubmit: (ev) => { ev.preventDefault(); if (a.value.length < 8) return res.replaceChildren(banner("warn", null, t("err.recovery.passphrase"))); if (a.value !== b.value) return res.replaceChildren(banner("warn", null, t("recovery.pp.mismatch"))); end(a.value); } },
        h("h2", { style: "margin-top:0" }, t("recovery.pp.title")), h("p", { class: "fine" }, t("recovery.pp.sub")),
        h("div", { class: "field c-rec" }, codeLabel("rec", "npa", t("recovery.pp.a")), a), h("div", { class: "field c-rec" }, codeLabel("rec", "npb", t("recovery.pp.b")), b), res,
        h("div", { class: "row" }, h("button", { class: "primary", type: "submit" }, t("common.continue")), h("button", { class: "secondary", type: "button", "data-back": "", onclick: () => end(null) }, t("common.cancel")))));
      a.focus();
    });
  }

  /* ---------- slide to confirm (reusable) + danger screens ---------- */
  // A knob dragged fully to the end. Mouse and touch (pointer events), keyboard (arrow keys move it, End confirms, Home resets)
  // and screen readers (role=slider, value text). Letting go early sends it back. Works left-to-right and right-to-left.
  function slideConfirm(label, onConfirm) {
    const knob = h("div", { class: "sl-knob", role: "slider", tabindex: "0", "aria-label": label, "aria-valuemin": "0", "aria-valuemax": "100", "aria-valuenow": "0", "aria-valuetext": t("slide.hint") });
    knob.innerHTML = ICON.arrow;
    const txt = h("div", { class: "sl-text", "aria-hidden": "true" }, label);
    const track = h("div", { class: "slide-confirm" }, h("div", { class: "sl-fill" }), txt, knob);
    let v = 0, done = false, drag = null;
    const set = (x) => { v = Math.max(0, Math.min(1, x)); track.style.setProperty("--p", v); knob.setAttribute("aria-valuenow", Math.round(v * 100)); };
    const sign = () => (getComputedStyle(track).direction === "rtl" ? -1 : 1);
    const finish = () => { if (done) return; done = true; set(1); track.classList.add("done"); track.classList.remove("drag"); txt.textContent = t("slide.done"); knob.setAttribute("aria-valuetext", t("slide.done")); knob.setAttribute("aria-disabled", "true"); onConfirm(); };
    const back = () => { track.classList.add("back"); set(0); setTimeout(() => track.classList.remove("back"), 300); };
    knob.addEventListener("pointerdown", (ev) => {
      if (done) return; ev.preventDefault(); knob.focus();
      const pad = 0.15 * parseFloat(getComputedStyle(document.documentElement).fontSize);
      drag = { x: ev.clientX, v, range: Math.max(1, track.getBoundingClientRect().width - knob.getBoundingClientRect().width - 2 * pad - 4) };
      track.classList.add("drag"); track.classList.remove("back"); knob.setPointerCapture(ev.pointerId);
    });
    knob.addEventListener("pointermove", (ev) => { if (drag) set(drag.v + ((ev.clientX - drag.x) * sign()) / drag.range); });
    const up = () => { if (!drag) return; drag = null; track.classList.remove("drag"); v >= 0.97 ? finish() : back(); };
    knob.addEventListener("pointerup", up); knob.addEventListener("pointercancel", up);
    knob.addEventListener("keydown", (ev) => {
      if (done) return;
      const fwd = ev.key === "ArrowUp" || ev.key === "PageUp" || ev.key === (sign() > 0 ? "ArrowRight" : "ArrowLeft");
      const bwd = ev.key === "ArrowDown" || ev.key === "PageDown" || ev.key === (sign() > 0 ? "ArrowLeft" : "ArrowRight");
      if (fwd) set(v + 0.25); else if (bwd) set(v - 0.25); else if (ev.key === "End") set(1); else if (ev.key === "Home") set(0); else return;
      ev.preventDefault(); ev.stopPropagation();
      if (v >= 1) finish();
    });
    // Called when the action failed, so the person can try again.
    track.rearm = () => { done = false; track.classList.remove("done"); txt.textContent = label; knob.removeAttribute("aria-disabled"); knob.setAttribute("aria-valuetext", t("slide.hint")); back(); };
    const wrap = h("div", { class: "slwrap" }, track, h("p", { class: "fine" }, t("slide.hint")));
    wrap.rearm = track.rearm; wrap.knob = knob;
    return wrap;
  }
  // "What is removed" / "What stays": each item a text (or [icon, text, ...more]) with an icon from the one set.
  const dlist = (cls, titleKey, items) => h("div", { class: "dbox " + cls }, h("h2", {}, iconEl(cls === "lose" ? "close" : "check"), t(titleKey)),
    h("ul", {}, items.map((x) => { const [ic, ...rest] = Array.isArray(x) ? x : [cls === "lose" ? "close" : "check", x]; return h("li", {}, iconEl(ic), h("span", {}, ...rest)); })));
  // One warning screen. cfg: {title, lead, lose[], keep[], warn, step {n,total}, button (label) or slide (label), onConfirm(), onCancel()}
  function dangerScreen(cfg) {
    show("danger", (card, cur) => {
      if (cfg.bar) { cur.bar = cfg.bar; renderTray(); }
      card.append(...[h("div", { class: "hero danger", "aria-hidden": "true" }), cfg.step ? h("p", { class: "dstep" }, t("danger.step", cfg.step)) : null, h("h1", {}, cfg.title)].filter(Boolean)); card.append(h("p", { class: "sub" }, cfg.lead));
      card.querySelector(".hero").innerHTML = ICON.warn;
      card.append(h("div", { class: "dcols" }, cfg.lose && cfg.lose.length ? dlist("lose", "danger.lose", cfg.lose) : null, cfg.keep && cfg.keep.length ? dlist("keep", "danger.keep", cfg.keep) : null));
      if (cfg.warn) card.append(h("p", { class: "dcannot" }, cfg.warn));
      if (cfg.extra) card.querySelector("h1").before(h("div", { class: "dextra" }, cfg.extra));
      const res = h("div"), cancel = h("button", { class: "link", type: "button", "data-back": "", "data-focus": "", onclick: cfg.onCancel }, t("common.cancel"));
      let control;
      if (cfg.slide) {
        control = slideConfirm(cfg.slide, async () => {
          res.replaceChildren(waitLine(cfg.working || t("danger.working"), "wait.generic.slow", 90)); cancel.disabled = true;
          try { await cfg.onConfirm(); }
          catch (e) { cancel.disabled = false; res.replaceChildren(banner("bad", e)); control.rearm(); control.knob.focus(); }
        });
        card.append(control, res, h("div", { class: "actions", style: "margin-block-start:.6rem" }, cancel));
      } else {
        control = h("button", { class: "primary btn-danger", type: "button", onclick: cfg.onConfirm }, cfg.button);
        card.append(h("div", { class: "actions" }, control, cancel));
      }
    }, { foot: "" });
  }
  // After a dangerous action worked.
  function dangerDone(message, next, label, sub) {
    show("danger-done", (card) => {
      card.append(h("div", { class: "big-dot", "aria-hidden": "true" }, "✓"), h("h1", {}, message), sub ? h("p", { class: "sub" }, sub) : "", h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: next }, label || t("danger.done.back"))));
    }, { foot: "" });
  }

  /* ---------- photo-frame screensaver (optional, off by default) ---------- */
  function startSaver() {
    if (saverOn) return; saverOn = true;
    const slides = [1, 2, 3, 4, 5].map((i) => h("div", { class: "slide s" + i, style: "animation-delay:" + ((i - 1) * 8) + "s" }));
    // Hook: licence-checked photos listed in screensaver/photos/index.json replace the SVG scenes (see screensaver/README.md).
    fetch("screensaver/photos/index.json", { cache: "no-store" }).then((r) => (r.ok ? r.json() : [])).then((l) => { if (Array.isArray(l)) l.slice(0, 5).forEach((f, i) => { if (/^[\w.-]+$/.test(f)) slides[i].style.backgroundImage = "url(screensaver/photos/" + f + ")"; }); }).catch(() => {});
    const clock = h("div", { class: "saver-clock" });
    const upd = () => { clock.textContent = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }); }; upd();
    const saverEl = h("div", { class: "saver", id: "saver", "aria-hidden": "true" }, ...slides, clock, h("div", { class: "saver-note" }, t("saver.note")));
    saverEl._t = setInterval(upd, 20000);
    document.body.append(saverEl);
    if (L.mode === "everyday" && L.signed) lockNow();
  }
  function stopSaver() { const e = document.getElementById("saver"); if (e) { clearInterval(e._t); e.remove(); } saverOn = false; }
  setInterval(() => {
    if (!powerOffDone && L.mode === "everyday" && L.signed && S && S.lock.state !== "setup" && !saverOn && Date.now() - lastAct > idleMs()) { if (prefs().saver) startSaver(); else lockNow(); }
    else if (!powerOffDone && L.signed && inGallery() && Date.now() - lastPing > 60000) { lastPing = Date.now(); alp.activity().catch(() => {}); }   // gallery: the server would lock first
  }, 2000);
  ["pointerdown", "keydown", "touchstart", "wheel"].forEach((ev) => document.addEventListener(ev, touch, { passive: true, capture: true }));
  document.addEventListener("mousemove", () => { if (Date.now() - lastAct > 1000) touch(); }, { passive: true });

  /* ---------- 5. recovery code (setup step 5, also from Settings > Password & Recovery) ---------- */
  // UX spec 1 and 7, security review 1 (T42, T43):
  // - a code is never made without a tap: the screen explains first;
  // - strong protection: required (Later leaves a reminder); password level: optional, "Make it" or "Not now";
  // - making one needs the ALP password; replacing one is password, warning, slide (replaceRecovery);
  // - with feature recovery_commit the new words count only after 3 of them are typed back; until then (and after
  //   leaving, a restart or a power cut) the old code keeps working;
  // - the words live only in this screen: leaving it wipes them, and they can never be shown again.
  let handoff = null;   // a code just made by Replace, handed to recovery() once, never kept
  function recovery() {
    show("recovery", (card, cur) => {
      const body = h("div"), setup = L.mode === "setup";
      if (S.arch.state !== "open") return needsDisk(card, 5);
      const strong = S.lock.protection === "strong" || S.arch.serious;
      let code = handoff; handoff = null;        // {words, qr_svg, confirm, recovery_key} while on screen only
      let committed = false;
      const wipe = () => { if (code) { if (code.words) code.words.fill(""); code = null; } body.replaceChildren(); };
      cur.stop = () => { if (code && !committed && has("recovery_commit")) alp.recoveryDiscard().catch(() => {}); wipe(); card.classList.remove("frame-rec"); };
      const leave = () => { if (setup) { L.recoveryDone = true; goNext(); } else goBack("settings"); };
      const later = () => skipStep("recovery");
      card.append(codeHero("rec"), h("h1", {}, t("recovery.title")), body);
      const made = S.recoveryMade ? new Date(S.recoveryMade).toLocaleDateString() : null;
      function intro() {
        if (!encrypted(S)) {   // never offered at the unencrypted levels
          body.replaceChildren(h("p", { class: "sub" }, t("recovery.none")), h("div", { class: "actions" }, h("button", { class: "primary", "data-back": "", onclick: leave }, setup ? t("common.continue") : t("common.back"))));
          return;
        }
        if (S.arch.recovery === "set") {   // a code exists: never make another one from here
          body.replaceChildren(h("p", { class: "sub" }, made ? t("recovery.exists.on", { date: made }) : t("recovery.exists")),
            h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: leave }, setup ? t("common.continue") : t("common.done")),
              h("button", { class: "secondary", type: "button", onclick: checkRecovery }, t("set.recovery.check")),
              setup ? h("button", { class: "link", "data-back": "", onclick: () => navigate("where") }, t("common.back")) : null));
          return;
        }
        body.replaceChildren(h("p", { class: "sub" }, t(strong ? "recovery.intro.strong" : "recovery.intro.password")),
          h("ul", { class: "warnlist" }, ["recovery.what", "recovery.why", "recovery.where"].map((k) => h("li", {}, t(k)))),
          h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: () => make() }, t(strong ? "recovery.show" : "recovery.make")),
            setup ? h("button", { class: "link later", type: "button", onclick: later }, t(strong ? "common.later" : "recovery.notnow"))
              : h("button", { class: "link", type: "button", "data-back": "", onclick: leave }, t(strong ? "common.back" : "recovery.notnow")),
            setup && strong ? h("p", { class: "fine later-note" }, t("recovery.later.note")) : null));
      }
      function askPassphrase(pw, err) {   // serious encryption: the code also needs an extra passphrase
        const a = h("input", { type: "password", id: "ppa", autocomplete: "new-password", "data-focus": "" }), b = h("input", { type: "password", id: "ppb", autocomplete: "new-password" });
        const f = h("form", { onsubmit: (ev) => { ev.preventDefault();
          if (a.value !== b.value) { f.querySelector("#pperr").replaceChildren(banner("warn", null, t("recovery.pp.mismatch"))); return; }
          create(pw, a.value); } },
          h("h2", {}, t("recovery.pp.title")), h("p", { class: "fine" }, t("recovery.pp.sub")),
          h("div", { class: "field c-rec" }, codeLabel("rec", "ppa", t("recovery.pp.a")), a), h("div", { class: "field c-rec" }, codeLabel("rec", "ppb", t("recovery.pp.b")), b),
          h("div", { id: "pperr" }, err ? banner("bad", err) : null),
          h("div", { class: "actions" }, h("button", { class: "primary", type: "submit" }, t("recovery.pp.go")), h("button", { class: "link", type: "button", "data-back": "", onclick: intro }, t("common.back"))));
        body.replaceChildren(f); a.focus();
      }
      async function make() {
        const pw = await confirmPassword("confirm.recovery.make");
        if (!pw) return;
        if (S.arch.serious) return askPassphrase(pw);
        create(pw);
      }
      async function create(pw, passphrase) {
        body.replaceChildren(waitLine(t("recovery.making"), "wait.generic.slow", 30));
        try { code = await alp.recoveryCreate({ password: has("recovery_commit") ? pw : undefined, passphrase }); pw = null; showWords(); }
        catch (e) {
          if (S.arch.serious && e.key === "recovery.passphrase") return askPassphrase(pw, e);
          body.replaceChildren(banner("bad", e), h("div", { class: "actions" }, h("button", { class: "primary", onclick: intro }, t("common.tryagain")),
            setup ? h("button", { class: "link later", onclick: later }, t("common.later")) : h("button", { class: "link", "data-back": "", onclick: leave }, t("common.back"))));
        }
      }
      // The words once, with the QR to photograph. No printing (the kiosk cannot print, security review 1.5).
      function showWords() {
        card.classList.add("frame-rec");
        const shown = code.words ? h("ol", { class: "words selectable", dir: "ltr" }, code.words.map((w) => h("li", {}, w)))   // 15 words (ADR 0013)
          : h("div", { class: "rkeys selectable", dir: "ltr" }, String(code.recovery_key).split("-").map((g) => h("span", { class: "rk" }, g)));
        const ok = h("input", { type: "checkbox", id: "saved" });
        const go = h("button", { class: "primary", disabled: true, onclick: () => (Array.isArray(code.confirm) && code.confirm.length && has("recovery_commit") ? confirmWords() : finish()) }, t("common.continue"));
        ok.addEventListener("change", () => { go.disabled = !ok.checked; });
        body.replaceChildren(
          h("div", { class: "seed recov" }, h("div", {}, h("div", { class: "codelabel c-rec", style: "margin:0 0 .4rem" }, codeIcon("rec"), t("recovery.key")), shown,
            h("p", { class: "fine" }, t("recovery.keep")), S.arch.serious ? h("p", { class: "fine" }, t("recovery.pp.note")) : null),
            h("figure", { class: "qrbox" }, svgImg(code.qr_svg, "qr", t("recovery.qr.alt")), h("figcaption", { class: "fine" }, t("recovery.photo")))),
          h("div", { class: "actions" }, h("label", { class: "check" }, ok, " ", t("recovery.saved")), go));
      }
      // Proof the words were kept: the server picks 3 positions; only then is the new code swapped in.
      function confirmWords(err) {
        const pos = code.confirm.slice(0, 3);
        const ins = pos.map((n) => h("input", { type: "text", autocomplete: "off", autocapitalize: "none", spellcheck: "false", dir: "ltr", id: "cw" + n }));
        const res = h("div", {}, err ? banner("bad", err) : null);
        const f = h("form", { onsubmit: async (ev) => { ev.preventDefault();
          const words = {}; pos.forEach((n, i) => { words[String(n)] = ins[i].value.trim().toLowerCase(); });
          res.replaceChildren(waitLine(t("recovery.confirming"), "wait.generic.slow", 30));
          try { await alp.recoveryCommit(words); committed = true; await refresh(); finish(); }
          catch (e) { confirmWords(e); } } },
          h("h2", {}, t("recovery.confirm.title")), h("p", { class: "fine" }, t("recovery.confirm.sub")),
          h("div", { class: "wordin three" }, pos.map((n, i) => h("div", { class: "field c-rec" }, h("label", { for: "cw" + n, class: "codelabel c-rec" }, t("recovery.confirm.n", { n })), ins[i]))), res,
          h("div", { class: "actions" }, h("button", { class: "primary", type: "submit" }, t("recovery.confirm.go")),
            h("button", { class: "link", type: "button", onclick: showWords }, t("recovery.confirm.again"))));
        body.replaceChildren(f); ins[0].focus();
      }
      function finish() {
        committed = true; wipe(); card.classList.remove("frame-rec");
        body.replaceChildren(h("div", { class: "banner ok", role: "status" }, h("div", {}, t("recovery.done"))),
          h("div", { class: "actions" }, h("button", { class: "primary", "data-focus": "", onclick: leave }, setup ? t("common.continue") : t("common.done"))));
        body.querySelector(".primary").focus();
      }
      if (code) showWords(); else intro();
    }, { wide: true, foot: "" });
  }
  // UX spec 7: Replace = ALP password, a warning that says what stops working, a slide. The old code keeps working
  // until 3 words of the new one are typed back (feature recovery_commit).
  // Only with the two-phase server (security review U1): an older alpd rotates at once on create, so the old code
  // would stop working before the new one is confirmed. Without recovery_commit there is no Replace at all.
  async function replaceRecovery() {
    if (!has("recovery_commit")) return;
    let pw = await confirmPassword("confirm.recovery.replace");
    if (!pw) return;
    let passphrase;
    if (S.arch.serious) { passphrase = await askNewPassphrase(); if (!passphrase) return; }
    const made = S.recoveryMade ? new Date(S.recoveryMade).toLocaleDateString() : null;
    dangerScreen({ title: t("danger.rec.title"), lead: t("danger.rec.lead"),
      lose: [made ? t("danger.rec.lose.on", { date: made }) : t("danger.rec.lose")], keep: [t("danger.rec.keep")],
      extra: h("button", { class: "link", type: "button", onclick: checkRecovery }, t("danger.rec.check")),
      slide: t("danger.rec.slide"), working: t("recovery.making"), onCancel: () => openSettings("password"),
      onConfirm: async () => {
        handoff = await alp.recoveryCreate({ password: pw, replace: true, passphrase });
        pw = null; passphrase = null;
        navigate("recovery");
      } });
  }
  /* ---------- 7. all set ---------- */
  function done() {
    show("done", (card) => {
      const n = h("p", { class: "fine" }), rem = reminders(S), part = rem.length > 0;
      card.append(h("div", { class: "big-dot" + (part ? " warn" : ""), "aria-hidden": "true" }, part ? "!" : "✓"), h("h1", {}, t(part ? "done.partial.title" : "done.title")), h("p", { class: "sub" }, t(part ? "done.partial.sub" : "done.sub")), part ? reminderList(S, rem) : n,
        h("div", { class: "actions" }, h("button", { class: "primary", onclick: () => { finishSetup(); L.mode = "everyday"; L.signed = true; touch(); L.view = null; route(true); } }, t("done.go"))));
      return { update(s) { if (!n.isConnected) return; n.textContent = s.phone.photos > 0 ? t("phone.uploading", { done: num(s.phone.photos) }) : t("done.waiting"); } };
    });
  }

  /* ---------- everyday: lock screen ---------- */
  function lock() {
    show("lock", (card, cur) => {
      card.classList.add("lockcard");
      const time = h("div", { class: "lk-time" }), date = h("div", { class: "lk-date" }), msg = h("p", { class: "lk-msg" }), chips = h("div", { class: "lk-chips" });
      const tickClock = () => { const d = new Date(); time.textContent = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }); date.textContent = d.toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" }); };
      tickClock(); const ct = setInterval(tickClock, 10000);
      const pw = h("input", { type: "password", id: "lpw", class: "c-alp", autocomplete: "current-password", maxlength: 128, "aria-label": t("pw.label"), placeholder: t("pw.label") });
      const res = h("div"), go = h("button", { class: "primary", type: "submit" }, t("lock.signin"));
      let timer = null;
      const wait = (secs) => { clearInterval(timer); const until = Date.now() + secs * 1000;
        const tick = () => { const n = Math.ceil((until - Date.now()) / 1000); if (n <= 0) { clearInterval(timer); res.replaceChildren(); go.disabled = false; return; }
          go.disabled = true; res.replaceChildren(h("div", { class: "banner warn", role: "alert" }, t("unlock.wait", { n }))); }; tick(); timer = setInterval(tick, 1000); };
      const form = h("form", { class: "lk-form hidden", onsubmit: async (ev) => {
        ev.preventDefault(); if (go.disabled || !pw.value) return; go.disabled = true;
        try { await alp.verify(pw.value); pw.value = ""; L.signed = true; touch(); L.view = null; route(true); }
        catch (e) { pw.value = ""; go.disabled = false; if (e.key === "password.wait" && e.retryAfter) return wait(e.retryAfter); res.replaceChildren(banner("bad", e)); pw.focus(); }
      } }, h("div", { class: "lk-user" }, h("span", { class: "lk-avatar" }, iconEl("logo")), h("b", {}, t("lock.user"))),
        h("div", { class: "input-row" }, pw, go), res);
      const reveal = (first) => { form.classList.remove("hidden"); hint.classList.add("hidden"); pw.focus(); if (first) pw.value = first; };
      const hint = h("button", { class: "lk-hint", type: "button", "data-focus": "", onclick: () => reveal() }, t("lock.hint"));
      const onkey = (ev) => { if (form.classList.contains("hidden") && ev.key.length === 1 && !ev.ctrlKey && !ev.metaKey) { ev.preventDefault(); reveal(ev.key); } else if (form.classList.contains("hidden") && ev.key === "Enter") reveal(); };
      document.addEventListener("keydown", onkey);
      cur.stop = () => { clearInterval(ct); clearInterval(timer); document.removeEventListener("keydown", onkey); card.classList.remove("lockcard"); };
      card.append(h("div", { class: "lk-top" }, time, date, msg), chips, h("div", { class: "lk-bottom" }, hint, form));
      const chip = (icon, label, value, tone) => h("div", { class: "chip" + (tone ? " " + tone : "") }, iconEl(icon), h("div", {}, h("div", { class: "k" }, label), h("div", { class: "v" }, value)));
      return { update(s) {
        const hl = health(s), w = warnings(s);
        msg.textContent = t(hl === "bad" || w.some((x) => x[0] === "bad") ? "lock.msg.bad" : w.length ? "lock.msg.warn" : "lock.msg.ok");
        const devs = s.phone.devices.length ? s.phone.devices : Array.from({ length: s.phone.phones }, () => ({ kind: "phone", name: "" }));
        const devNode = devs.length ? h("div", { class: "devs" }, devs.map((d) => h("span", { class: "dev" }, iconEl(DEVICE_ICON[d.kind] || "phone"), d.name ? maskName(d.name) : t("lock.device")))) : t("lock.nodevices");
        const spaceTone = s.arch.space === "full" || s.index.space === "full" ? "bad" : s.arch.space === "low" || s.index.space === "low" ? "warn" : "";
        const net = s.net.online ? (s.net.kind === "wifi" && s.net.ssid ? t("lock.net.wifi", { name: maskName(s.net.ssid) }) : t("lock.net.cable")) : t("lock.net.off");
        chips.replaceChildren(
          chip("phone", t("lock.devices"), devNode),
          chip("wifi", t("status.network"), net, s.net.online ? "" : "warn"),
          chip("disk", t("status.room"), s.arch.state === "open" ? t("lock.free", { free: fmtGB(s.arch.free) }) : "–", spaceTone),
          chip("heart", t("lock.health"), t("health." + hl), hl === "ok" ? "" : hl),
          s.phone.last ? chip("check", t("lock.last"), new Date(s.phone.last).toLocaleString()) : chip("check", t("status.photos"), num(s.phone.photos)));
      } };
    }, { foot: "" });
  }

  /* ---------- everyday: home ---------- */
  function home() {
    show("home", (card) => {
      const dot = h("div", { class: "homelv" }), title = h("h1"), warn = h("div"), tiles = h("div", { class: "tiles" });
      const sub = h("p", { class: "sub", style: "margin-bottom:0" });
      const lookAgain = h("button", { class: "primary hidden", onclick: async () => { await refresh(); route(true); } }, t("status.lookagain"));
      const confirmOther = async () => { if (await confirmPassword("status.other")) forgetDanger(); };
      const other = h("button", { class: "link hidden", onclick: confirmOther }, t("status.other"));
      // T16: the gallery opens inside the console, below the top bar (home button = the way back).
      const gal = h("button", { class: "primary", type: "button", "data-focus": "", onclick: () => navigate("gallery") }, t("home.gallery"));
      const galWait = waitLine(t("home.gallery.starting"), "wait.gallery.slow", 240);
      const remBox = h("div");
      const menu = h("div", { class: "menu" }, gal,
        h("button", { class: "secondary", onclick: () => navigate("settings") }, t("home.settings")),
        has("media_import") ? h("button", { class: "secondary", type: "button", "data-ico": "download", onclick: () => refusedWhileCritical() || navigate("import") }, t("import.go")) : null,
        S.lock.state === "setup" ? null : h("button", { class: "secondary", onclick: lockNow }, t("home.lock")));
      card.append(dot, title, sub, warn, remBox, lookAgain, other, galWait, tiles, menu);
      const tile = (k, v, small, note) => h("div", { class: "tile" }, h("div", { class: "k" }, k), h("div", { class: "v" + (small ? " small" : ""), dir: small ? "ltr" : null }, v), note ? h("div", { class: "k" }, note) : null);
      return { update(s) {
        const rem = reminders(s), empty = ["none", "failed", "creating"].includes(s.arch.state);
        const w = warnings(s).filter((x) => !(rem.length && x[1] === "status.warn.network")), bad = w.some((x) => x[0] === "bad");
        remBox.replaceChildren(...(s.arch.state === "creating" ? [h("div", { class: "banner ok" }, t("home.creating"))] : []), ...(rem.length ? [reminderList(s, rem)] : []));
        // T53/T54: blue when the legend is the check or a white (i), red from "action" on; the headline says it too.
        const lvl = statusLevel(s), trouble = ["action", "now", "critical"].includes(lvl);
        document.body.dataset.health = trouble ? "bad" : "ok";
        dot.replaceChildren(legendIcon(lvl));
        title.textContent = t(trouble ? "home.title.attn" : rem.length ? "home.open.title" : "home.title.ok");
        const said = has("status_levels") && s.status && s.status.message;   // the server's one plain sentence, as text
        sub.textContent = trouble ? said || "" : rem.length && !w.length ? t("home.open.sub") : !w.length ? t("home.sub.ok") : "";
        // UX spec 3: the details of a problem live behind (i); the home screen says only that there is one.
        warn.replaceChildren(...(w.length || trouble ? [h("button", { class: "link", type: "button", "data-ico": "info", onclick: openErrors }, t("home.see.errors"))] : []));
        const miss = s.arch.state === "missing"; lookAgain.classList.toggle("hidden", !miss); other.classList.toggle("hidden", !miss); tiles.classList.toggle("hidden", miss || empty);
        gal.classList.toggle("hidden", miss || empty || s.immich.state !== "ready" && s.immich.state !== "paused");
        galWait.classList.toggle("hidden", miss || empty || s.immich.state !== "starting");
        galWait.why(s.immich.detail);   // T21: e.g. "Unpacking the photo service for the first time…"
        const place = s.arch.place ? (s.arch.place.label || s.arch.place.name) : "–";
        const hl = health(s);
        // Design review 1: the tile behind a problem carries the legend mark in its corner.
        const mark = (el, on, lv) => { if (on) { el.classList.add("tile-lv"); el.append(legendIcon(lv, { word: false })); } return el; };
        const spaceLv = s.arch.space === "full" ? "now" : s.arch.space === "low" ? "action" : null;
        tiles.replaceChildren(
          tile(t("status.photos"), num(s.phone.photos)),
          mark(tile(t("status.room"), fmtGB(s.arch.free), false, t("where.room", { photos: roomPhotos(s.arch.free) })), !!spaceLv, spaceLv),
          tile(t("status.phones"), s.phone.phones ? t("status.phones.n", { n: s.phone.phones }) : t("status.phones.none"), false, s.phone.last ? t("status.last", { when: new Date(s.phone.last).toLocaleString() }) : null),
          mark(tile(t("lock.health"), t("health." + hl)), hl !== "ok" && !spaceLv, hl === "bad" ? "now" : "action"),
          tile(t("status.kept"), place, true, t("home.kept.note")));
      } };
    }, { wide: true, foot: "" });
  }

  /* ---------- everyday: Photos, full screen (UX spec 4, T46) ---------- */
  // The gallery fills the whole area under the system bar, on its own origin (security review 2: never the
  // console's), with the session cookie alpd set after the console sign-in: no second sign-in screen. alpd announces
  // it with immich.gallery_url (feature gallery_session) only when that layer is ready; until then a plain
  // "Photos are coming to this screen" page with the number of photos, never the old frame with its sign-in line.
  // Security review U2: the frame keeps allow-same-origin for the gallery's own cookie, so it must never be the
  // console's origin (it could then drop its sandbox and reach the console API). Only http://127.0.0.1:<port>
  // on another origin is framed; anything else is noted in (i) and shows the "Photos are coming" page.
  // The same check for every frame on another local origin, each kind on its own fixed port (review N5).
  const FRAME_PORT = { gallery: "8412", terminal: "8413" };
  function localFrameOk(raw, kind) {
    let u = null;
    try { u = new URL(raw); } catch { u = null; }
    return u && u.protocol === "http:" && u.hostname === "127.0.0.1" && u.port === FRAME_PORT[kind] && u.origin !== location.origin ? u.href : null;
  }
  function galleryOk(raw) {
    if (!raw) return null;
    const ok = localFrameOk(raw, "gallery");
    if (!ok) Errs.add({ id: "gallery.url", level: "warn", text: t("gallery.url.bad") });
    return ok;
  }
  function gallery() {
    show("gallery", (card, cur) => {
      const url = has("gallery_session") ? galleryOk(S.immich.galleryUrl) : null;
      const msg = h("div", { class: "galmsg" });
      const soon = (key) => {
        if (frame) frame.remove();
        msg.replaceChildren(hero("gallery"), h("h1", {}, t(key === "soon" ? "gallery.soon.title" : "gallery.fail.title")),
          h("p", { class: "sub" }, key === "soon" ? t("gallery.soon.sub", { n: num(S.phone.photos) }) : t(key)),
          h("div", { class: "actions" }, h("button", { class: "primary", type: "button", "data-back": "", onclick: goHome }, t("gallery.back"))));
        card.classList.add("galempty");
      };
      let frame = null;
      if (!url) { soon("soon"); card.append(msg); return; }
      frame = h("iframe", { class: "galframe", src: url, title: t("bar.gallery"), referrerpolicy: "no-referrer",
        sandbox: "allow-scripts allow-same-origin allow-forms" });   // no top-navigation, popups or downloads (review U3)
      let loaded = false;
      frame.addEventListener("load", () => { loaded = true; });
      const timer = setTimeout(() => { if (!loaded) { Errs.add({ level: "warn", text: t("gallery.fail.slow") }); soon("gallery.fail.slow"); } }, 20000);
      cur.stop = () => clearTimeout(timer);
      card.append(frame, msg);
      const check = (s) => { if (frame.isConnected && s.immich.state !== "ready" && s.immich.state !== "paused") { Errs.add({ level: "warn", text: t(s.immich.state === "starting" ? "gallery.fail.starting" : "gallery.fail.off") }); soon(s.immich.state === "starting" ? "gallery.fail.starting" : "gallery.fail.off"); } };
      check(S);
      return { update: check };
    }, { cls: "gallerycard", foot: "", instant: true });
  }
  // "Back to the gallery" on the delete guard pages: inside the gallery frame go back there, else open Photos.
  function backToGallery() { const g = S && galleryOk(S.immich.galleryUrl); if (window.top !== window && g) location.href = g; else { L.view = "gallery"; route(true); } }

  /* ---------- everyday: Import from a disk, memory card or camera (UX spec 4, T47) ---------- */
  // Our own full-screen picker instead of the browser's file window: devices on the left, folders and photos on the
  // right. The browser only ever sends opaque ids (security review 3). alpd reads the files read-only and uploads
  // them through the photo service; the progress shows in the system bar so the person can keep working. Names of
  // devices, folders and files come from external disks: text nodes only, never HTML.
  function importScreen() {
    show("import", (card, cur) => {
      const left = h("nav", { class: "imp-dev", "aria-label": t("import.devices") }), right = h("div", { class: "imp-main" }), err = h("div");
      card.append(h("div", { class: "importer" }, left, h("div", { class: "imp-right" }, err, right)));
      let media = L.importMedia || null, folder = null, trail = [], items = [], chosen = new Set(), listing = null, prog = null;
      const fail = (e) => err.replaceChildren(banner("bad", e));
      function drawDevices(s) {
        const list = s.media || [];
        if (media && !list.some((m) => m.id === media)) { media = null; listing = null; }
        if (!media && list.length) media = list[0].id;
        if (media && !listing && !browsing) browse(null);
        fill(left, h("h2", {}, t("import.devices")), list.length ? list.map((m) => h("button", { type: "button", class: "imp-d" + (m.id === media ? " on" : ""), onclick: () => { media = m.id; trail = []; browse(null); drawDevices(S); } },
          iconEl("disk"), h("span", {}, h("b", {}, m.label || t("import.device")), h("span", { class: "d" }, [m.fs_type ? String(m.fs_type).toUpperCase() : null, m.size_bytes ? fmtGB(m.size_bytes) : null].filter(Boolean).join(" · "))))) : h("p", { class: "d" }, t("import.plug")));
        if (!list.length) right.replaceChildren(h("div", { class: "plug" }, h("p", { class: "t" }, t("import.none")), h("p", { class: "fine" }, t("import.plug"))));
      }
      let browsing = false;
      async function browse(f) {
        browsing = true; folder = f; chosen.clear(); right.replaceChildren(waitLine(t("import.reading"), "wait.generic.slow", 30));
        try { listing = await alp.mediaBrowse(media, f ? f.id : null, 0); items = listing.entries || []; drawItems(); }
        catch (e) { right.replaceChildren(); fail(e); }
        browsing = false;
      }
      function drawItems() {
        const go = h("button", { class: "primary", type: "button", disabled: !items.length, onclick: start }, t("import.start"));
        const upd = () => { go.textContent = chosen.size ? t("import.start.n", { n: num(chosen.size) }) : t("import.start.all"); };
        const crumbs = h("div", { class: "crumbs" }, h("button", { class: "link", type: "button", onclick: () => { trail = []; browse(null); } }, t("import.top")),
          trail.map((f, i) => [" / ", h("button", { class: "link", type: "button", onclick: () => { trail = trail.slice(0, i + 1); browse(f); } }, f.name)]));
        const grid = h("div", { class: "imp-grid" }, items.map((it) => {
          const box = h("input", { type: "checkbox", "aria-label": it.name });
          box.addEventListener("change", () => { box.checked ? chosen.add(it.id) : chosen.delete(it.id); upd(); });
          return h("div", { class: "imp-item" + (it.kind === "folder" ? " folder" : "") }, box,
            h("button", { type: "button", class: "imp-open", disabled: it.kind !== "folder", onclick: () => { trail.push(it); browse(it); } },
              iconEl(it.kind === "folder" ? "folder" : it.kind === "video" ? "video" : "gallery"), h("span", { class: "nm" }, it.name),
              h("span", { class: "d" }, it.kind === "folder" ? (it.count != null ? t("import.items", { n: num(it.count) }) : "") : it.size_bytes ? (it.size_bytes < 1e9 ? (it.size_bytes / 1e6).toFixed(1) + " MB" : fmtGB(it.size_bytes)) : "")));
        }));
        fill(right, h("div", { class: "imp-bar" }, crumbs, go), listing && listing.known ? h("p", { class: "fine" }, t("import.known", { n: num(listing.known) })) : null,
          items.length ? grid : h("p", { class: "fine" }, t("import.empty")), listing && listing.truncated ? h("p", { class: "fine" }, t("import.more")) : null);
        upd();
      }
      async function start() {
        if (refusedWhileCritical()) return;
        const pick = chosen.size ? [...chosen] : folder ? [folder.id] : items.map((x) => x.id);
        try { await alp.importStart(media, pick); await refresh(); progress(); } catch (e) { fail(e); }
      }
      function progress() {
        const bar = h("progress", { max: 100, value: 0 }), line = h("p", { class: "fine" }), end = h("div");
        fill(right, h("h2", {}, t("import.running")), bar, line, h("p", { class: "fine" }, t("import.keepworking")), end);
        prog = (s) => {
          drawDevicesQuiet(s);
          const j = s.importJob; if (!j) return;
          bar.value = j.percent || 0; line.textContent = j.message || "";
          if (["done", "failed", "cancelled"].includes(j.state) && !end.firstChild) {
            end.replaceChildren(j.state === "done" ? h("div", { class: "banner ok", role: "status" }, h("div", {}, t("import.done", { photos: num(j.photos || 0), videos: num(j.videos || 0), known: num(j.skipped || 0) }))) : banner("bad", j.error || { message: j.message }),
              h("div", { class: "row" }, h("button", { class: "primary", type: "button", onclick: eject }, t("import.eject")), h("button", { class: "secondary", type: "button", onclick: () => navigate("gallery") }, t("import.see"))));
          }
        };
      }
      const drawDevicesQuiet = (s) => { if (!s.media.some((m) => m.id === media)) drawDevices(s); };
      async function eject() { try { await alp.mediaEject(media); media = null; await refresh(); drawDevices(S); err.replaceChildren(h("div", { class: "banner ok", role: "status" }, t("import.ejected"))); } catch (e) { fail(e); } }
      L.importMedia = null;
      drawDevices(S);
      if (S.importJob && S.importJob.state === "running") progress();
      return { update: (s) => { if (prog) return prog(s); const ids = (s.media || []).map((m) => m.id).join(); if (ids !== cur.ids) { cur.ids = ids; drawDevices(s); } } };
    }, { foot: "" });
  }
  // A disk, memory card or camera was plugged in: "Photos found on SanDisk 64 GB. Import them?" (once per device).
  const mediaSeen = new Set();
  function offerImport(s) {
    if (!has("media_import") || !L.signed || L.mode !== "everyday" || modalEl || !current || ["import", "lock", "danger", "format"].includes(current.name)) return;
    const m = (s.media || []).find((x) => !mediaSeen.has(x.id) && x.photos);
    (s.media || []).forEach((x) => { if (!x.photos) mediaSeen.add(x.id); });
    if (!m) return;
    mediaSeen.add(m.id);
    sheet(h("h2", { style: "margin-top:0" }, t("import.found", { name: m.label || t("import.device") })), h("p", { class: "fine" }, t("import.found.sub")),
      h("div", { class: "row" }, h("button", { class: "primary", type: "button", onclick: () => { closeModal(); if (refusedWhileCritical()) return; L.importMedia = m.id; navigate("import"); } }, t("import.found.yes")),
        h("button", { class: "secondary", type: "button", "data-back": "", onclick: closeModal }, t("recovery.notnow"))));
  }

  /* ---------- everyday: settings (UX spec 5, T51) ---------- */
  // Like macOS System Settings: a sidebar of nine categories, the chosen one on the right. Each row: a title, one or
  // two grey lines that say what it does for the person now, and one control.
  function deviceLine(d) { return [d.name || t("dev.unknown"), d.os].filter(Boolean).join(" · "); }
  const SET_CATS = [["phones", "phone", "blue"], ["password", "key", "red"], ["disk", "disk", "green"], ["chain", "elephant", "gold"], ["network", "wifi", "sky"],
    ["display", "screen", "purple"], ["power", "power", "grey"], ["about", "logo", "black"], ["advanced", "wrench", "slate"]];
  function openSettings(cat) { L.setCat = cat; if (current && current.name === "settings") route(true); else navigate("settings"); }
  // T26: Advanced asks for the ALP password once; the permission ends after 10 minutes without use.
  let advancedUntil = 0;
  const ADV_MS = 10 * 60 * 1000;
  async function advancedGate() {
    if (Date.now() < advancedUntil || S.lock.protection === "none") { advancedUntil = Date.now() + ADV_MS; return true; }
    if (!(await confirmPassword("confirm.advanced"))) return false;
    advancedUntil = Date.now() + ADV_MS; return true;
  }
  function settings() {
    if (!L.setCat) L.setCat = "phones";
    if (L.setCat === "advanced" && Date.now() >= advancedUntil && S.lock.protection !== "none") L.setCat = "phones";
    show("settings", (card, cur) => {
      const err = h("div"), pane = h("div", { class: "setpane" });
      const side = h("nav", { class: "setside", "aria-label": t("home.settings") }, SET_CATS.map(([k, ic, col]) =>
        h("button", { type: "button", class: "setcat" + (L.setCat === k ? " on" : ""), "aria-current": L.setCat === k ? "page" : null, onclick: async () => {
          if (k === "advanced" && !(await advancedGate())) return;
          L.setCat = k; route(true); } },
          (() => { const i = h("span", { class: "sq sq-" + col, "aria-hidden": "true" }); i.innerHTML = ICON[ic] || ICON.settings; return i; })(), h("span", {}, t("cat." + k)))));
      card.append(h("div", { class: "settings" }, side, h("div", { class: "setmain" }, h("h1", {}, t("cat." + L.setCat)), err, pane)));
      // 0.2.5 addendum: every setting row has an icon from the one set.
      const ROW_ICON = { "about.build": "log", "about.version": "info", "admin.reset": "refresh", "enc.row": "lock", "pair.strict": "lock", "pc.battery.now": "battery",
        "set.addphone": "phone", "set.disk": "disk", "set.disks": "disk", "set.enc": "key", "set.files": "folder", "set.lockafter": "lock", "set.natural": "trackpad",
        "set.saver": "gallery", "set.term": "wrench", "status.network": "wifi", "status.off": "power" };
      const rowIcon = (label) => { const k = typeof label === "string" && Object.keys(ROW_ICON).find((x) => T[x] === label); return k ? [iconEl(ROW_ICON[k]), label] : label; };
      const row = (label, desc, ...ctl) => h("div", { class: "setrow" }, h("div", { class: "lbl" }, h("b", {}, rowIcon(label)), desc ? h("span", { class: "d" }, desc) : null), h("div", { class: "ctl" }, ...ctl));
      const group = (...rows) => h("div", { class: "setgroup" }, ...rows.filter(Boolean));
      const btn = (label, fn, cls) => h("button", { class: cls || "secondary", type: "button", onclick: fn }, label);
      const fail = (e) => err.replaceChildren(banner("bad", e));
      const sw = (on, label, onchange) => { const i = h("input", { type: "checkbox", role: "switch", checked: !!on }); i.addEventListener("change", () => onchange(i)); return h("label", { class: "switch" }, i, h("span", { class: "knob" }), h("span", { class: "sr" }, label)); };
      if (L.setCat === "advanced") advancedUntil = Date.now() + ADV_MS;
      const P = {
        phones() {
          const when = (d) => (d.last_seen ? new Date(d.last_seen).toLocaleString() : t("dev.never"));
          const devs = S.phone.devices || [];
          const disconnect = (fn) => async () => { if (!(await confirmPassword("confirm.devremove"))) return; try { await fn(); await refresh(); route(true); } catch (e) { fail(e); } };
          return [group(...(devs.length || S.pairing.devices.length ? [
            ...devs.map((d) => row([iconEl(DEVICE_ICON[d.kind] || "phone"), deviceLine(d)], t("set.dev.row", { when: when(d) }), btn(t("set.dev.disconnect"), disconnect(() => alp.removeDevice(d.id)), "link"))),
            ...S.pairing.devices.map((d) => row([iconEl("phone"), deviceLine(d)], t("pair.dev.paired"), btn(t("set.dev.disconnect"), disconnect(() => alp.pairRevoke(d.serial)), "link")))]
            : [h("p", { class: "d" }, t("dev.none"))])),
          group(row(t("pair.strict"), t("set.strict.d"), sw(S.pairing.strict, t("pair.strict"), async (i) => {
            if (i.checked) { try { await alp.pairStrict(true); await refresh(); } catch (e) { i.checked = false; fail(e); } return; }
            i.checked = true;   // turning it off weakens protection: password, warning, slide (UX spec 7)
            if (await confirmPassword("confirm.strictoff")) strictOffDanger();
          })),
          row(t("set.addphone"), t("set.addphone.d"), btn(t("set.open"), () => { L.phoneDone = false; navigate("phone"); })))];
        },
        password() {
          const enc = encrypted(S), made = S.recoveryMade ? new Date(S.recoveryMade).toLocaleDateString() : null;
          return [group(
            S.lock.protection !== "none" && has("password_change") ? row([codeIcon("alp"), t("set.pw")], t("set.pw.d"), btn(t("set.pw.change"), changePassword)) : null,
            enc ? (S.arch.recovery === "set"
              ? row([codeIcon("rec"), t("set.recovery")], made ? t("set.recovery.d", { date: made }) : t("set.recovery.d0"), btn(t("set.recovery.check"), checkRecovery), has("recovery_commit") ? btn(t("set.recovery.replace"), replaceRecovery, "link") : null)
              : row([codeIcon("rec"), t("set.recovery")], t("set.recovery.none"), btn(t("set.recovery.make"), () => navigate("recovery"))))
              : null,
            row(t("set.enc"), t("about.protection." + (S.lock.protection || "none")), enc ? null : btn(t("enc.row.go"), encryptLater)))];
        },
        disk() {
          const a = S.arch, place = a.place ? a.place.label || a.place.name : null, used = Math.max(0, a.size - a.free);
          const hl = health(S);
          if (a.state !== "open") return [group(row(t("set.disk"), t("about.arch." + (a.state || "none")), a.state === "none" || a.state === "failed" ? btn(t("home.rem.do"), () => resumeSetup("where")) : null))];
          const pct = a.size ? Math.round((used / a.size) * 100) : 0;
          return [group(row([iconEl("disk"), place || t("set.disk")], t("set.disk.d", { free: fmtGB(a.free), size: fmtGB(a.size), health: t("health." + hl) })),
            h("div", { class: "spacebar", role: "img", "aria-label": t("set.disk.used", { used: fmtGB(used), pct }) }, h("i", { style: "inline-size:" + pct + "%" })),
            h("p", { class: "d" }, t("set.disk.used", { used: fmtGB(used), pct }) + " · " + t("where.room", { photos: roomPhotos(a.free) }))),
            S.lock.protection === "none" || S.lock.protection === "console" ? group(row(t("enc.row"), t("set.enc.d"), btn(t("enc.row.go"), encryptLater))) : null];
        },
        chain() {
          const member = S.chain.state === "member";
          const showChain = async () => { if (!(await confirmPassword("confirm.chain"))) return; try { L.seed = await alp.chainCode(); navigate("chain-seed"); } catch (e) { fail(e); } };
          const leaveChain = async () => { if (await confirmPassword("confirm.chainleave")) chainLeaveDanger(); };
          return [group(member ? row([codeIcon("chain"), t("set.chain")], t("set.chain.member"), btn(t("set.chain.show"), showChain), btn(t("set.chain.leave"), leaveChain, "link"))
            : row([codeIcon("chain"), t("set.chain")], t("set.chain.none"), btn(t("set.chain.setup"), () => navigate("chain"))))];
        },
        network() {
          const ips = S.net.addresses.map((x) => x.ip).join(", ");
          return [group(row(t("status.network"), S.net.online ? t("set.net.d", { name: S.net.kind === "wifi" ? S.net.ssid || "" : t("lock.net.cable"), address: ips || "–" }) : t("lock.net.off"), btn(t("set.change"), openNetwork)))];
        },
        display() {
          const p = prefs();
          const saver = sw(p.saver, t("set.saver"), (i) => savePrefs({ ...prefs(), saver: i.checked }));
          // T56: five marked steps; "Default" is the automatic choice for this screen.
          const steps = [1, null, 1.5, 1.75, 2], names = ["disp.s.smaller", "disp.s.default", "disp.s.larger", "disp.s.larger2", "disp.s.largest"];
          const at = Math.max(0, steps.indexOf(p.scale)), auto = autoScale();
          const out = h("output", { class: "scale-now" });
          const show = (i) => { out.textContent = (steps[i] ? t(names[i]) : t("disp.s.default.auto")) + " · " + Math.round((steps[i] || auto) * 100) + "%"; };
          const slider = h("input", { type: "range", min: 0, max: 4, step: 1, value: at, list: "scale-marks", "aria-label": t("set.size") });
          slider.addEventListener("input", () => show(+slider.value));
          slider.addEventListener("change", () => setScale(steps[+slider.value]));
          show(at);
          const size = h("div", { class: "scale" }, slider, h("datalist", { id: "scale-marks" }, steps.map((_, i) => h("option", { value: i, label: t(names[i]) }))),
            h("div", { class: "scale-marks", "aria-hidden": "true" }, names.map((k) => h("span", {}, t(k)))), out);
          const modes = has("display_modes") ? h("div", { class: "ctl-wait" }, waitLine(t("disp.res.loading"))) : null;
          if (modes) alp.displayModes().then((ms) => {
            const curM = ms.find((m) => m.current) || {};
            modes.replaceChildren(choiceMenu(t("set.res"), ms.map((m) => ({ value: m.id, text: m.width + " × " + m.height + (m.refresh ? " · " + Math.round(m.refresh) + " Hz" : ""), hint: m.native ? t("disp.res.native") : null })),
              curM.id, (id) => changeMode(id, curM.id)));
          }).catch((e) => modes.replaceChildren(h("span", { class: "d" }, banner("bad", e).textContent)));
          return [group(row(t("set.saver"), t("set.saver.d"), saver),
            // Lock after: a choice when alpd keeps it (feature idle_lock_pref). Never is not offered: the idle lock is the
            // only guard on a signed-in console (security review, 2026-11-08); alpd accepts only these values.
            has("idle_lock_pref") ? row(t("set.lockafter"), t("set.lockafter.d", { n: Math.round(S.console.idle / 60) }),
              choiceMenu(t("set.lockafter"), [60, 300, 900, 1800].map((v) => ({ value: v, text: t("set.lockafter.min", { n: v / 60 }) })), S.console.idle,
                async (v) => { try { await alp.setPrefs({ idle_lock_seconds: v }); await refresh(); openSettings("display"); } catch (e) { fail(e); } }))
              : row(t("set.lockafter"), t("set.lockafter.d", { n: Math.round(S.console.idle / 60) })),
            // Feature clock: the time as the computer has it, whether the network set it, and the zone times are shown in.
            has("clock") && S.clock ? row(t("set.clock"), clockText(S, true) + ". " + t(S.clock.synced ? "set.clock.synced" : "set.clock.unsure"),
              choiceMenu(t("set.zone"), zoneOptions(), S.prefs.timeZone,
                async (v) => { try { await alp.setPrefs({ time_zone: v }); await refresh(); openSettings("display"); } catch (e) { fail(e); } })) : null,
            has("natural_scrolling") ? row(t("set.natural"), t("set.natural.d"), sw(S.prefs.naturalScrolling !== false, t("set.natural"), async (i) => { try { await alp.setPrefs({ natural_scrolling: i.checked }); await refresh(); } catch (e) { i.checked = !i.checked; fail(e); } })) : null,
            row([iconEl("textsize"), t("set.size")], t("set.size.d"), size),
            modes ? row([iconEl("screen"), t("set.res")], t("set.res.d"), modes) : null,
            // T53: the home background picker; only the default until the curated pictures ship in the image.
            row([iconEl("gallery"), t("set.homebg")], t("set.homebg.d"), h("div", { class: "bgpick", role: "radiogroup", "aria-label": t("set.homebg") },
              h("button", { type: "button", role: "radio", "aria-checked": "true", class: "bgthumb on", title: t("bg.blue") }, h("span", { class: "bgsw blue" }), h("span", {}, t("bg.blue"))))))];
        },
        power() {
          const b = S.sys.battery;
          return [group(row(t("status.off"), t("set.off.d"), btn(t("status.off"), askPowerOff)),
            b ? row(t("pc.battery.now"), t(b.charging ? "tray.battery.charging" : "tray.battery", { n: b.percent })) : null)];
        },
        about() {
          const prot = S.lock.protection ? t("about.protection." + S.lock.protection) : null;
          return [group(row(t("about.version"), [S.sys.release || "ALP-OS", prot].filter(Boolean).join(" · ")), S.sys.build ? row(t("about.build"), h("span", { dir: "ltr" }, S.sys.build)) : null),
            h("h2", {}, t("about.hw")), group(specList()),
            h("h2", {}, t("about.credits")), group(h("p", { class: "d" }, t("about.credits.d")), h("ul", { class: "credits" }, CREDITS.map(([n, l, what]) => h("li", {}, h("b", {}, n), " · " + l + " · " + t(what)))))];
        },
        advanced() {
          const startOver = async () => { const pw = await confirmPassword("reset.pw.title"); if (pw) resetDanger(pw, 1); };
          return [group(row(t("set.disks"), t("set.disks.d"), btn(t("set.open"), () => navigate("disks"))),
            has("media_files") ? row(t("set.files"), t("set.files.d"), btn(t("set.open"), () => navigate("files"))) : null,
            has("terminal") ? row(t("set.term"), t("set.term.d"), btn(t("set.open"), openTerminal)) : null,
            row(t("admin.reset"), t("admin.reset.d"), btn(t("admin.reset.btn"), startOver, "secondary btn-danger-s")))];
        },
      };
      pane.replaceChildren(...[].concat(P[L.setCat]()).filter(Boolean));
    }, { foot: "" });
  }
  // About > Credits: the only place that names the photo service (UX spec 4). Versions come from images.lock in a later build.
  const CREDITS = [["Immich", "AGPL-3.0", "credit.immich"], ["Debian", "various", "credit.debian"], ["systemd", "LGPL-2.1", "credit.systemd"], ["Chromium", "BSD-3-Clause", "credit.chromium"],
    ["cage", "MIT", "credit.cage"], ["gocryptfs", "MIT", "credit.gocryptfs"], ["cryptsetup", "GPL-2.0", "credit.cryptsetup"], ["nginx", "BSD-2-Clause", "credit.nginx"], ["PostgreSQL", "PostgreSQL", "credit.postgres"], ["Noto Sans", "OFL-1.1", "credit.noto"]];
  // Text size (Display & Input): a per-screen preference.
  // T56 size of text and controls. The page is zoomed as a whole; viewport units in style.css go through --vh/--vw,
  // which are set here to the zoomed viewport, so full-height screens still fit exactly.
  const SCALES = [1, 1.25, 1.5, 1.75, 2];
  // Automatic: from the screen's pixel density when alpd knows its physical width, else from its pixel width; the aim
  // is body text of about 15 px at a normal distance (1.25x on a 14" 1080p laptop, 1x on a 24" 1080p monitor, 2x on 4K).
  function autoScale() {
    if (S && S.display && SCALES.includes(S.display.auto_scale)) return S.display.auto_scale;   // alpd's own choice (feature prefs)
    const dpr = window.devicePixelRatio || 1, px = (screen.width || innerWidth) * dpr, mm = S && S.display && S.display.width_mm;
    const snap = (x) => SCALES.reduce((b, v) => (Math.abs(v - x) < Math.abs(b - x) ? v : b), 1);
    if (mm > 100) return snap(Math.min(2, Math.max(1, px / (mm / 25.4) / 125)));
    return px >= 3200 ? 2 : px >= 2400 ? 1.5 : px >= 1800 ? 1.25 : 1;
  }
  const curScale = () => prefs().scale || autoScale();
  function applyPrefs() {
    const root = document.documentElement, dpr = window.devicePixelRatio || 1, z = Math.max(0.5, curScale() / dpr);
    root.style.zoom = z === 1 ? "" : String(z);
    root.style.setProperty("--vh", innerHeight / z / 100 + "px"); root.style.setProperty("--vw", innerWidth / z / 100 + "px");
    root.dataset.scale = String(curScale());
    syncChrome();
  }
  window.addEventListener("resize", () => applyPrefs());
  // T59: full-screen frames (gallery, terminal) start exactly under the top bar (and the critical bar, if shown),
  // at every scale. --chrome is in the zoomed page's own pixels, so the bars' screen height is divided by the zoom.
  function syncChrome() {
    const root = document.documentElement, z = parseFloat(root.style.zoom) || 1;
    const last = document.getElementById("critbar") || document.querySelector(".top");
    if (last) root.style.setProperty("--chrome", Math.max(0, last.getBoundingClientRect().bottom) / z + "px");
  }
  if (window.ResizeObserver && document.querySelector(".top")) new ResizeObserver(() => syncChrome()).observe(document.querySelector(".top"));
  async function setScale(v) {
    savePrefs({ ...prefs(), scale: v });
    if (has("prefs")) { try { await alp.setPrefs({ ui_scale: v }); await refresh(); } catch (e) { banner("bad", e); } }
    applyPrefs();
  }
  // Resolution (feature display_modes): the switch reverts by itself after 15 s unless Keep is pressed.
  // Our own drop-down list (spec 2: no native <select>): a button and a list popover; arrows, Enter, Esc work.
  // The time zones the browser knows, each with its offset now; first "UTC" (no zone chosen).
  function zoneOptions() {
    let names = [];
    try { names = Intl.supportedValuesOf("timeZone"); } catch { names = []; }
    const off = (z) => { try { return new Intl.DateTimeFormat("en", { timeZone: z, timeZoneName: "shortOffset" }).formatToParts(new Date(S.clock.now * 1000)).find((x) => x.type === "timeZoneName").value.replace("GMT", "UTC"); } catch { return ""; } };
    return [{ value: null, text: "UTC" }].concat(names.map((z) => ({ value: z, text: z.replace(/_/g, " "), hint: off(z) })));
  }
  function choiceMenu(label, options, value, onPick) {
    const cur = () => options.find((o) => o.value === value) || options[0] || { text: "" };
    const txt = h("span", {}, cur().text);
    const btnEl = h("button", { class: "secondary choicebtn", type: "button", "aria-haspopup": "listbox", "aria-expanded": "false", "aria-label": label }, txt, iconEl("chevron"));
    const wrap = h("div", { class: "choicemenu" }, btnEl);
    let list = null;
    const close = (focus) => { if (!list) return; list.remove(); list = null; btnEl.setAttribute("aria-expanded", "false"); document.removeEventListener("pointerdown", outside, true); if (focus) btnEl.focus(); };
    const outside = (ev) => { if (!wrap.contains(ev.target)) close(false); };
    const pick = (o) => { close(true); if (o.value === value) return; value = o.value; txt.textContent = o.text; onPick(o.value); };
    btnEl.addEventListener("click", () => {
      if (list) return close(true);
      list = h("ul", { class: "choicelist", role: "listbox", "aria-label": label }, options.map((o) => h("li", { role: "option", tabindex: "-1", "aria-selected": String(o.value === value), onclick: () => pick(o) },
        h("span", {}, o.text), o.hint ? h("span", { class: "hint" }, o.hint) : null, o.value === value ? iconEl("check") : null)));
      list.addEventListener("keydown", (ev) => {
        const items = [...list.children], i = items.indexOf(document.activeElement);
        if (ev.key === "ArrowDown" || ev.key === "ArrowUp") { ev.preventDefault(); items[(i + (ev.key === "ArrowDown" ? 1 : -1) + items.length) % items.length].focus(); }
        else if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); if (i >= 0) pick(options[i]); }
        else if (ev.key === "Escape" || ev.key === "Tab") { ev.preventDefault(); ev.stopPropagation(); close(true); }
      });
      wrap.append(list); btnEl.setAttribute("aria-expanded", "true"); document.addEventListener("pointerdown", outside, true);
      (list.querySelector("[aria-selected=true]") || list.firstChild).focus();
    });
    return wrap;
  }
  async function changeMode(id, back) {
    let r = null;
    try { r = await alp.displayMode(id); } catch (e) { Errs.add({ level: "warn", text: banner("bad", e).textContent }); return; }
    let n = Math.max(5, Math.min(30, (r && r.revert_in) || 15)), timer = null;
    const left = h("p", { class: "sub" }, t("disp.keep.left", { n }));
    // Keep confirms; Revert (or no answer) leaves it to alpd, which goes back by itself when the time is up.
    const done = async (keep) => {
      clearInterval(timer); closeModal();
      try { await (keep ? alp.displayKeep() : alp.displayRevert()); }
      catch (e) { if (e.key !== "display.no_pending") Errs.add({ level: "warn", text: banner("bad", e).textContent }); }   // no_pending: alpd already went back
      setTimeout(() => openSettings("display"), 300);
    };
    sheet(iconEl("screen"), h("h2", {}, t("disp.keep.title")), left,
      h("div", { class: "actions" }, h("button", { class: "primary", type: "button", onclick: () => done(true) }, t("disp.keep")), h("button", { class: "secondary", type: "button", onclick: () => done(false) }, t("disp.revert"))));
    timer = setInterval(() => { n--; left.textContent = t("disp.keep.left", { n }); if (n <= 0) done(false); }, 1000);
  }

  /* ---------- Advanced > Disks (T49 read side, T52 quick and secure format) ---------- */
  // Every disk with a role. The archive disk and the stick carry a lock (only Start over erases them); the computer's
  // own disk is shown read-only. External disks can be formatted: password, warning (what is on it), slide.
  // T58: GET live/disks (feature disks) lists every disk with its lock; an older alpd: built from the hardware report
  // and the archive candidates. Either way the server refuses a locked disk again, whatever the screen sends (review U4).
  async function loadDisks() {
    let ds = null;
    if (has("disks")) ds = (await alp.disks()).map((d) => Object.assign({}, d, { parts: Array.isArray(d.parts) ? d.parts : [], disk_id: d.id }));
    let cands = []; try { cands = await alp.candidates(); } catch (e) { /* none */ }
    const place = S.arch.place, archDisk = place && (cands.find((c) => c.partuuid === place.partuuid) || {}).disk;
    if (!ds) {
      const byDisk = {};
      cands.forEach((c) => { (byDisk[c.disk] = byDisk[c.disk] || []).push(c); });
      const specs = (S.hw.specs && S.hw.specs.disks) || [];   // kernel names, as the candidates' "disk"
      const names = specs.map((d) => d.name).concat(Object.keys(byDisk).filter((k) => !specs.some((d) => d.name === k)));
      ds = names.map((k) => {
        const sp = specs.find((d) => d.name === k) || {}, parts = byDisk[k] || [], c = parts[0] || {};
        const kind = sp.system ? "stick" : sp.internal ? "internal" : "external", role = sp.system ? "system" : k === archDisk ? "backup" : "other";
        return { id: k, name: k, model: sp.model || c.disk_model || c.label || k, size_bytes: sp.size_bytes || parts.reduce((n, x) => Math.max(n, x.size_bytes || 0), 0),
          kind, role, parts, disk_id: c.disk_id || null, health: null, temperature_c: null,
          lock: kind === "internal" ? "internal_live" : kind === "stick" ? "stick" : role === "backup" ? "backup" : null };
      });
    }
    return ds.map((d) => {
      const role = d.role === "backup" || (archDisk && d.name === archDisk) ? "backup" : d.role === "system" || d.kind === "stick" ? "system" : "other";
      const lock = d.kind === "internal" ? "internal_live" : d.lock || (role === "system" ? "stick" : role === "backup" ? "backup" : null);
      return Object.assign({}, d, { role, lock, locked: !!lock, offline: d.kind === "internal" });
    });
  }
  const DISK_LOCK_HINT = { stick: "disks.lock.startover", backup: "disks.lock.startover", write_protected: "disks.lock.wp", open: "disks.lock.open" };
  const diskTitle = (d) => d.model && d.model !== d.name ? d.model : d.name || d.id;
  // Partition bar: width by size, colour by role (Archive, ALPArchive stick, Other, Free).
  function partBar(d) {
    const used = d.parts.reduce((n, p) => n + (p.size_bytes || 0), 0), size = Math.max(d.size_bytes || 0, used) || 1;
    const segs = d.parts.map((p) => ({ cls: d.role === "backup" ? "backup" : d.role === "system" ? "system" : "other", n: p.size_bytes || size / Math.max(1, d.parts.length), p }));
    if (size - used > size * 0.01 && used) segs.push({ cls: "free", n: size - used });
    if (!segs.length) segs.push({ cls: "free", n: size });
    const bar = h("div", { class: "pbar", "aria-hidden": "true" }, segs.map((x) => h("i", { class: "pt-" + x.cls, style: "flex-grow:" + Math.max(1, Math.round(x.n / size * 1000)) })));
    const keys = h("ul", { class: "pkeys" }, segs.map((x) => h("li", {}, h("i", { class: "sw pt-" + x.cls }),
      x.p ? [x.p.label || x.p.name || t("disks.part"), x.p.fs ? String(x.p.fs).toUpperCase() : t("where.prepare.nofs"), x.p.size_bytes ? fmtGB(x.p.size_bytes) : null].filter(Boolean).join(" · ") : t("disks.unused") + " · " + fmtGB(x.n))));
    return h("div", { class: "pwrap" }, bar, keys);
  }
  function disks(selId) {
    show("disks", (card, cur) => {
      const listEl = h("div", { class: "dm-list" }, waitLine(t("disks.loading"), "wait.generic.slow", 20)), pane = h("div", { class: "dm-pane" }), err = h("div");
      card.append(h("div", { class: "dm-head" }, h("div", {}, h("h1", {}, t("disks.title")), h("p", { class: "sub" }, t("disks.sub"))),
        h("button", { class: "secondary", type: "button", onclick: () => disks(sel && sel.id) }, t("disks.reload"))), err, h("div", { class: "dm" }, listEl, pane));
      let all = [], sel = null;
      const select = (d) => { sel = d; listEl.querySelectorAll(".dm-item").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.id === d.id))); drawPane(d); };
      function item(d) {
        const chip = d.offline ? t("disks.offline") : t("disks.role." + d.role);
        const inner = [iconEl(d.offline ? "lock" : d.kind === "stick" ? "usb" : "disk"), h("span", { class: "main" }, h("span", { class: "t", title: diskTitle(d) }, diskTitle(d)),
          h("span", { class: "s" }, d.size_bytes ? fmtGB(d.size_bytes) : "")), !d.offline && LEVELS.includes(d.level) && d.level !== "ok" ? legendIcon(d.level, { word: false }) : null,
          h("span", { class: "chip r-" + (d.offline ? "off" : d.role) }, chip)];
        if (d.offline) {   // the computer's own disk: listed, never selectable
          return h("div", { class: "dm-off" }, h("div", { class: "dm-item off", "aria-disabled": "true" }, ...inner),
            h("p", { class: "fine" }, t("disks.internal.line")), techDetails(h("p", {}, t("disks.internal.tech"))));
        }
        return h("button", { class: "dm-item", type: "button", "data-id": d.id, "aria-pressed": "false", title: diskTitle(d), onclick: () => select(d) }, ...inner);
      }
      function drawPane(d) {
        const rows = h("dl", { class: "dm-rows" });
        const row = (ic, k, v) => { if (v != null && v !== "") rows.append(h("dt", {}, iconEl(ic), t(k)), h("dd", {}, v)); };
        row("file", "disks.fs", [...new Set(d.parts.map((p) => p.fs ? String(p.fs).toUpperCase() : null).filter(Boolean))].join(", ") || t("where.prepare.nofs"));
        const free = d.parts.reduce((n, p) => (p.free_bytes != null ? (n || 0) + p.free_bytes : n), null);
        row("disk", "disks.freespace", free != null ? fmtGB(free) : null);
        row("heart", "disks.health", d.health ? [LEVELS.includes(d.level) && d.level !== "ok" ? legendIcon(d.level) : null, " ", t("disks.health." + d.health)] : null);   // level: alpd's legend level for this disk
        row("chip", "disks.temp", d.temperature_c != null ? t("disks.temp.v", { n: Math.round(d.temperature_c) }) : null);
        const acts = h("div", { class: "dm-acts" }), res = h("div");
        const canFormat = !d.locked && d.kind === "external" && d.disk_id;
        if (canFormat) acts.append(h("button", { class: "secondary", type: "button", "data-ico": "", onclick: () => formatDisk(d) }, iconEl("erase"), t("fmt.go")));
        else acts.append(h("div", { class: "dm-locked" }, iconEl("lock"), h("span", {}, t("fmt.go.locked")), h("span", { class: "fine" }, t(DISK_LOCK_HINT[d.lock] || (d.disk_id ? "disks.lock.other" : "disks.refused.empty")))));
        if (has("disk_eject") && d.kind === "external" && d.lock !== "backup" && d.lock !== "open")
          acts.append(h("button", { class: "secondary", type: "button", onclick: () => ejectDisk(d, res) }, iconEl("eject"), t("disks.eject")));
        if (has("disk_check") && d.kind === "external" && !d.locked)
          acts.append(h("button", { class: "secondary", type: "button", onclick: () => checkDisk(d, res) }, iconEl("wrench"), t("disks.check")));
        pane.replaceChildren(h("div", { class: "dm-title" }, iconEl(d.kind === "stick" ? "usb" : "disk"), h("div", {}, h("h2", {}, diskTitle(d)),
          h("p", { class: "fine" }, [t("disks.kind." + d.kind), d.size_bytes ? fmtGB(d.size_bytes) : null, d.safe_to_remove ? t("disks.safe") : null].filter(Boolean).join(" · ")))),
          partBar(d), rows, acts, res);
      }
      loadDisks().then((ds) => {
        all = ds;
        if (!ds.length) { listEl.replaceChildren(h("p", { class: "fine" }, t("disks.none"))); return; }
        const open = ds.filter((d) => !d.offline);
        listEl.replaceChildren(...open.map(item), ...ds.filter((d) => d.offline).map(item));
        const first = open.find((d) => d.id === selId) || open.find((d) => !d.locked) || open[0];
        if (first) select(first); else pane.replaceChildren(h("p", { class: "fine" }, t("disks.none.open")));
      }).catch((e) => { listEl.replaceChildren(); err.replaceChildren(banner("bad", e)); });
      return { update(s) { if (sel) { const j = s.arch.job; if (j && j.id === "disk.check" && pane.querySelector(".dm-check")) pane.querySelector(".dm-check").textContent = j.message || ""; } } };
    }, { foot: "", wide: true });
  }
  async function ejectDisk(d, res) {
    res.replaceChildren(waitLine(t("disks.ejecting"), "wait.generic.slow", 30));
    try { await alp.diskEject(d.disk_id); res.replaceChildren(h("div", { class: "banner ok", role: "status" }, h("div", {}, t("disks.ejected", { name: diskTitle(d) })))); }
    catch (e) { res.replaceChildren(banner("bad", e)); }
  }
  async function checkDisk(d, res) {
    try { const r = await alp.diskCheck(d.disk_id), j = r && r.job; res.replaceChildren(h("p", { class: "dm-check", role: "status" }, j && j.state === "done" && j.message ? j.message : t("disks.checking"))); }
    catch (e) { res.replaceChildren(banner("bad", e)); }
  }
  // T58 / T52: 1. name and type (exFAT; ext4 and the secure erase under Technical details), 2. password,
  // 3. the slide with "What is removed" / "What stays", 4. progress, then "{name} is ready." with Eject.
  async function formatDisk(d) {   // allowed at critical: preparing a new disk is the way out (security review R6/U1)
    const parts = d.parts.map((p) => p.partuuid);
    let est;
    try { est = await alp.formatEstimate(d.disk_id, parts); } catch (e) { Errs.add({ level: "bad", text: banner("bad", e).textContent }); return dangerDone(t("fmt.refused"), () => disks(d.id), t("common.back")); }
    show("format", (card) => {
      const name = diskTitle(d);
      const exfat = h("input", { type: "radio", name: "fs", value: "exfat", checked: true }), ext4 = h("input", { type: "radio", name: "fs", value: "ext4" });
      const secure = h("input", { type: "checkbox", name: "secure" });
      const mins = Math.max(1, Math.round(((est.secure || {}).seconds || 60) / 60));
      const opt = (inp, title, line) => h("label", { class: "choice" }, inp, h("span", { class: "main" }, h("span", { class: "t" }, title), line ? h("span", { class: "s" }, line) : null));
      const label = h("input", { type: "text", id: "flabel", maxlength: 11, autocomplete: "off", spellcheck: "false", placeholder: "USB", dir: "ltr", value: (d.parts[0] || {}).label && /^[A-Za-z0-9 _-]{1,11}$/.test(d.parts[0].label) ? d.parts[0].label : "" });
      const res = h("div");
      const more = [has("disks") ? opt(ext4, t("fmt.fs.ext4"), t("fmt.fs.ext4.d")) : null,
        opt(secure, t("fmt.secure"), [(est.secure || {}).text || t("fmt.secure.d", { n: mins }), " ", t((est.secure || {}).about ? "fmt.time.about" : "fmt.time", { n: mins })])].filter(Boolean);
      card.append(h("div", { class: "fmtcol" }, hero("disk"), h("h1", {}, t("fmt.title.name", { name })), h("p", { class: "sub" }, fmtGB(est.size_bytes || d.size_bytes || 0)),
        isCritical() ? h("p", { class: "fmt-rescue" }, iconEl("info"), " ", t("fmt.rescue")) : "",
        h("form", { class: "fmtform", onsubmit: async (ev) => { ev.preventDefault();
          if (label.value && !/^[A-Za-z0-9 _-]{1,11}$/.test(label.value)) { res.replaceChildren(banner("warn", null, t("fmt.label.bad"))); return; }
          const mode = secure.checked ? "secure" : "quick", fs = ext4.checked ? "ext4" : "exfat", pw = await confirmPassword("confirm.format");
          if (pw) formatDanger(d, parts, est, mode, pw, label.value.trim(), fs); } },
          h("div", { class: "field" }, h("label", { for: "flabel" }, t("fmt.label")), label),
          h("div", { class: "choices" }, opt(exfat, t("fmt.fs.exfat"), t("fmt.fs.exfat.d"))), techDetails(h("div", { class: "choices" }, ...more)), res,
          h("div", { class: "actions" }, h("button", { class: "primary", type: "submit" }, t("common.continue")), h("button", { class: "link", type: "button", "data-back": "", onclick: () => disks(d.id) }, t("common.cancel"))))));
    }, { foot: "" });
  }
  function formatDanger(d, parts, est, mode, pw, label, fs) {
    const name = diskTitle(d);
    const what = est.files != null ? t("fmt.lose.counted", { name, n: num(est.files), size: fmtGB(est.bytes_used || 0) }) : t("fmt.lose.disk", { name });
    const lose = [["disk", what]];
    if (est.photos_not_in_archive > 0) lose.push(["gallery", t("fmt.lose.photos", { n: num(est.photos_not_in_archive) }), " ",
      h("button", { class: "link", type: "button", onclick: () => { pw = ""; navigate("import"); } }, t("fmt.lose.import"))]);
    (est.warnings || []).forEach((w) => lose.push(["warn", T["fmt.warn." + w] ? t("fmt.warn." + w) : String(w)]));
    const keep = [["lock", t("fmt.keep.archive")], ["phone", t("fmt.keep.phones")], ["screen", t("fmt.keep.computer")]];
    dangerScreen({ title: t("fmt.danger.title", { name }), lead: (est[mode] || {}).text || t("fmt." + mode + ".d", { n: 1 }), lose, keep, warn: t("danger.cannot"),
      slide: t("fmt.slide"), bar: "format", working: t("fmt.working"), onCancel: () => { pw = ""; disks(d.id); },
      onConfirm: async () => {
        const extra = has("disks") ? { fs, confirm: est.confirm } : {};
        try { await alp.format(d.disk_id, parts, mode, pw, label || undefined, extra); }
        catch (e) { if (e.key === "format.confirm_stale") { pw = ""; Errs.add({ level: "warn", text: t("fmt.stale") }); return formatDisk(d); } throw e; }
        pw = "";
        formatProgress(label || name, mode, d);
      } });
  }
  function formatProgress(name, mode, d) {
    show("format", (card, cur) => {
      const bar = h("i"), wrap = h("div", { class: "progress", role: "progressbar", "aria-valuemin": 0, "aria-valuemax": 100 }, bar);
      const line = waitLine(t("fmt.working"), "wait.generic.slow", 120), end = h("div"), left = h("p", { class: "fine fmt-left" });
      const stop = h("button", { class: "secondary hidden", type: "button", onclick: async () => {
        if (!(await confirmPassword("fmt.stop.confirm"))) return;
        stop.disabled = true; try { await alp.cancelArchive(); } catch (e) { stop.disabled = false; end.replaceChildren(banner("bad", e)); } } }, t("fmt.stop"));
      card.append(hero("disk"), h("h1", {}, t("fmt.run.title", { name })), wrap, line, left, mode === "secure" ? h("p", { class: "fine" }, t("fmt.stop.note")) : null, h("div", { class: "actions" }, stop), end);
      let fin = false;
      return { update(s) {
        const j = s.arch.job; if (fin || !j || j.id !== "disk.format") return;
        const p = Math.max(2, j.percent || 0); bar.style.inlineSize = p + "%"; wrap.setAttribute("aria-valuenow", p);
        line.now(j.message || t("fmt.working"));
        left.textContent = j.state === "running" && j.seconds_left != null ? t("fmt.left", { n: Math.max(1, Math.ceil(j.seconds_left / 60)) }) : "";
        stop.classList.toggle("hidden", !(j.can_cancel && j.step === "erasing"));
        if (["done", "failed", "cancelled"].includes(j.state)) {
          fin = true; line.remove(); stop.remove(); left.remove();
          const ok = j.state === "done", res = h("div");
          if (ok) card.querySelector("h1").textContent = t("fmt.ready", { name });
          end.replaceChildren(ok ? h("div", { class: "banner ok", role: "status" }, h("div", {}, t("fmt.ready", { name }))) : banner(j.state === "failed" ? "bad" : "warn", j.error || { message: j.message }),
            h("div", { class: "actions" }, ok && d && has("disk_eject") ? h("button", { class: "primary", type: "button", onclick: () => ejectDisk(d, res) }, iconEl("eject"), t("disks.eject")) : null,
              h("button", { class: ok && d && has("disk_eject") ? "secondary" : "primary", type: "button", onclick: () => disks(d && d.id) }, t("common.done"))), res);
        }
      } };
    }, { foot: "" });
  }
  /* ---------- Advanced > Files (stage F): a read-only look at external disks (media.rs) ---------- */
  // Names come from a disk anyone could have written: text nodes only (h()). The media id is opaque, the path sent
  // back is a list of names the server listed. Nothing is ever written: no rename, delete, copy, drag or "open with".
  // Photos and videos show by URL on the console's origin (img/video only), text files as plain text.
  const FILE_KIND = [["image", /\.(jpe?g|png|gif|webp|bmp)$/i], ["video", /\.(mp4|m4v|mov|webm)$/i], ["text", /\.(txt|log|md|csv|json|xml|ini|cfg|conf|nfo)$/i]];
  const TEXT_MAX = 256 * 1024;
  const kindOf = (name) => (FILE_KIND.find(([, re]) => re.test(name)) || [null])[0];
  const fmtSize = (b) => b == null ? "" : b < 1e3 ? t("files.bytes", { n: b }) : b < 1e6 ? (b / 1e3).toFixed(0) + " KB" : b < 1e9 ? (b / 1e6).toFixed(1) + " MB" : fmtGB(b);
  function files() {
    if (!has("media_files")) return openSettings("advanced");
    show("files", (card, cur) => {
      const left = h("nav", { class: "imp-dev", "aria-label": t("files.devices") }), right = h("div", { class: "imp-main" }), err = h("div");
      card.append(h("div", { class: "importer files" }, left, h("div", { class: "imp-right" }, h("p", { class: "hint ro" }, iconEl("lock"), " ", t("files.ro")), err, right)));
      let media = null, trail = [], entries = [], more = false, skipped = 0, busy = false, gone = null, seq = 0;
      const opened = new Set();
      const fail = (e) => err.replaceChildren(banner("bad", e));
      const label = (m) => (m && m.label) || t("import.device");
      const cur_ = () => (S.media || []).find((m) => m.id === media);
      cur.stop = () => { seq++; right.replaceChildren(); };   // drop a late answer, stop a playing video
      function drawDevices(s) {
        const list = s.media || [];
        if (media && !list.some((m) => m.id === media)) { media = null; trail = []; entries = []; }
        if (!media && !busy) { const first = list.find((m) => !m.refused); if (first) pick(first); }
        fill(left, h("h2", {}, t("files.devices")), list.length ? list.map((m) => h("button", { type: "button", class: "imp-d" + (m.id === media ? " on" : "") + (m.refused ? " refused" : ""), onclick: () => pick(m) },
          iconEl(m.refused ? "lock" : "disk"), h("span", {}, h("b", {}, label(m)), h("span", { class: "d" }, m.refused ? t("files.refused.short")
            : [m.fs_type ? String(m.fs_type).toUpperCase() : null, m.size_bytes ? fmtGB(m.size_bytes) : null].filter(Boolean).join(" · "))))) : h("p", { class: "d" }, t("import.plug")));
        if (!list.length) right.replaceChildren(h("div", { class: "plug" }, h("p", { class: "t" }, gone ? t("files.ejected", { name: gone }) : t("files.none")), h("p", { class: "fine" }, t("import.plug"))));
      }
      function pick(m) {
        err.replaceChildren();
        media = m.id; trail = []; entries = [];
        if (m.refused) {   // the server's reason, as text (e.g. "ALPArchive cannot read this kind of disk (btrfs).")
          right.replaceChildren(h("div", { class: "plug" }, iconEl("lock"), h("p", { class: "t" }, t("files.refused", { name: label(m) })), h("p", { class: "fine" }, String(m.refused))));
          drawDevices(S); return;
        }
        drawDevices(S); load(true);
      }
      async function load(reset) {
        const my = ++seq; busy = true;
        if (reset) { entries = []; more = false; skipped = 0; right.replaceChildren(waitLine(t("files.reading"), "wait.generic.slow", 30)); }
        try {
          if (!opened.has(media)) { await alp.mediaOpen(media); opened.add(media); }
          let r;
          try { r = await alp.mediaList(media, trail, entries.length); }
          catch (e) { if (e.key !== "media.not_open") throw e; await alp.mediaOpen(media); r = await alp.mediaList(media, trail, entries.length); }   // alpd restarted
          if (my !== seq) return;
          entries = entries.concat(r.entries || []); more = !!r.truncated; skipped += r.skipped || 0;
          draw();
        } catch (e) { if (my === seq) { right.replaceChildren(); fail(e); } }
        finally { if (my === seq) busy = false; }
      }
      function crumbs() {
        return h("div", { class: "crumbs" }, h("button", { class: "link", type: "button", onclick: () => { trail = []; load(true); } }, label(cur_())),
          trail.map((n, i) => [" / ", h("button", { class: "link", type: "button", onclick: () => { trail = trail.slice(0, i + 1); load(true); } }, n)]));
      }
      function draw() {
        const eject = h("button", { class: "secondary", type: "button", onclick: doEject }, iconEl("eject"), t("import.eject"));
        const rows = entries.map((e) => h("li", {}, h("button", { type: "button", class: "frow" + (e.dir ? " dir" : ""), onclick: () => (e.dir ? (trail.push(e.name), load(true)) : preview(e)) },
          iconEl(e.dir ? "folder" : kindOf(e.name) === "video" ? "video" : kindOf(e.name) === "image" ? "gallery" : "file"), h("span", { class: "nm" }, e.name), h("span", { class: "d" }, e.dir ? "" : fmtSize(e.size_bytes)))));
        fill(right, h("div", { class: "imp-bar" }, crumbs(), eject),
          entries.length ? h("ul", { class: "flist" }, rows) : h("p", { class: "fine" }, t("files.empty")),
          more ? h("div", { class: "actions" }, h("button", { class: "secondary", type: "button", onclick: () => load(false) }, t("files.more"))) : null,
          skipped ? h("p", { class: "fine" }, t("files.skipped", { n: num(skipped) })) : null);
      }
      async function preview(e) {
        const my = ++seq, kind = kindOf(e.name), path = trail.concat([e.name]);
        const back = h("button", { class: "link", type: "button", "data-back": "", onclick: () => { seq++; draw(); } }, t("files.back"));
        const box = h("div", { class: "fprev" });
        fill(right, h("div", { class: "imp-bar" }, crumbs(), back), h("h2", { class: "nm" }, e.name), h("p", { class: "fine" }, fmtSize(e.size_bytes)), box);
        if (!kind) return box.replaceChildren(h("p", { class: "sub" }, t("files.noopen")));
        if (kind === "image") {
          const img = h("img", { alt: e.name, src: alp.mediaFileUrl(media, path), referrerpolicy: "no-referrer" });
          img.addEventListener("error", () => { if (my === seq) box.replaceChildren(h("p", { class: "sub" }, t("files.unreadable"))); });
          return box.replaceChildren(img);
        }
        if (kind === "video") {
          const v = h("video", { controls: "", preload: "metadata", src: alp.mediaFileUrl(media, path), disablepictureinpicture: "", controlslist: "nodownload noremoteplayback" });
          v.addEventListener("error", () => { if (my === seq) box.replaceChildren(h("p", { class: "sub" }, t("files.unreadable"))); });
          return box.replaceChildren(v);
        }
        box.replaceChildren(waitLine(t("files.reading"), "wait.generic.slow", 20));
        try {
          const txt = await alp.mediaText(media, path, TEXT_MAX);
          if (my !== seq) return;
          box.replaceChildren(h("pre", { class: "ftext" }, txt), (e.size_bytes || 0) > TEXT_MAX ? h("p", { class: "fine" }, t("files.text.cut")) : null);
        } catch (er) { if (my === seq) box.replaceChildren(h("p", { class: "sub" }, t("files.unreadable"))); }
      }
      async function doEject() {
        const m = cur_(), name = label(m);
        seq++; right.replaceChildren(waitLine(t("files.ejecting"), "wait.generic.slow", 30));
        try { await alp.mediaEject(media); opened.delete(media); gone = name; media = null; trail = []; entries = []; await refresh();
          right.replaceChildren(h("div", { class: "plug" }, h("p", { class: "t" }, t("files.ejected", { name })), h("p", { class: "fine" }, t("import.ejected")))); }
        catch (e) { right.replaceChildren(); fail(e); }
      }
      drawDevices(S);
      return { update(s) { const before = media; drawDevices(s); if (before && !media) right.replaceChildren(h("div", { class: "plug" }, h("p", { class: "t" }, t("files.unplugged")))); } };
    }, { foot: "" });
  }
  /* ---------- Advanced > Terminal (stage F, docs/plans/live-terminal.md) ---------- */
  // The server checks the password every time (same lockout as sign-in) and answers a one-time address on its own
  // origin (127.0.0.1:8413/<random>/). The address is the session's only secret: kept in this function only, never
  // in L, the state, (i) or a log. Leaving the screen ends the session; the server also ends it on lock and idle.
  async function openTerminal() {
    if (!has("terminal")) return;
    let pw = await confirmPassword("term.pw");
    if (!pw) return;
    let r;
    try { r = await alp.terminalStart(pw); }
    catch (e) {   // 401 console.locked goes to the lock screen by itself (api.js)
      pw = "";
      if (e.key === "terminal.no_password") return dangerDone(t("err.terminal.no_password"), () => openSettings("password"), t("term.set_password"));
      return dangerDone(banner("bad", e).textContent, () => openSettings("advanced"), t("common.back"));
    }
    pw = "";
    let url = r && localFrameOk(r.url, "terminal");
    r = null;
    if (!url) { alp.terminalStop().catch(() => {}); Errs.add({ id: "term.url", level: "warn", text: t("term.url.bad") }); return dangerDone(t("term.url.bad"), () => openSettings("advanced"), t("common.back")); }
    show("terminal", (card, cur) => {
      const frame = h("iframe", { class: "galframe termframe", src: url, title: t("bar.terminal"), referrerpolicy: "no-referrer", sandbox: "allow-scripts allow-same-origin" });
      url = null;
      card.append(h("p", { class: "termnote" }, iconEl("info"), " ", t("term.logged")), frame);
      cur.stop = () => { frame.src = "about:blank"; frame.remove(); alp.terminalStop().catch(() => {}); };
      return { update(s) { if (s.terminal && s.terminal.state === "off" && frame.isConnected) { cur.stop(); card.replaceChildren(h("div", { class: "galmsg" }, h("h1", {}, t("term.ended")), h("p", { class: "sub" }, t("term.ended.d")),
        h("div", { class: "actions" }, h("button", { class: "primary", type: "button", onclick: () => openSettings("advanced") }, t("common.back"))))); } } };
    }, { cls: "gallerycard", foot: "", instant: true });
  }
  function admin() { L.setCat = "advanced"; settings(); }   // older links: Advanced lives in Settings now
  function computer() { L.setCat = "about"; settings(); }

  /* ---------- the dangerous actions: password -> warning -> slide ---------- */
  // Each takes the password the person just typed (already checked) and shows its warning screen.
  function chainLeaveDanger() {
    dangerScreen({ title: t("danger.chain.title"), lead: t("danger.chain.lead"), lose: [t("danger.chain.lose1"), t("danger.chain.lose2")], keep: [t("danger.chain.keep1"), t("danger.chain.keep2")],
      slide: t("danger.chain.slide"), onCancel: () => route(true),
      onConfirm: async () => { await alp.chainLeave(); await refresh(); dangerDone(t("danger.chain.ok"), () => route(true)); } });
  }
  function forgetDanger() {
    dangerScreen({ title: t("danger.forget.title"), lead: t("danger.forget.lead"), lose: [t("danger.forget.lose1")], keep: [t("danger.forget.keep1"), t("danger.forget.keep2")],
      slide: t("danger.forget.slide"), onCancel: () => route(true),
      onConfirm: async () => { await alp.forgetArchive(); await refresh(); dangerDone(t("danger.forget.ok"), () => { detectMode(); L.welcomed = true; L.view = null; route(true); }); } });
  }
  // The gallery's delete guard: reason = "trash" | "delete"
  function galleryDanger(reason, pw, ids) {
    if (refusedWhileCritical("delete")) return;
    const back = h("button", { class: "primary", type: "button", onclick: backToGallery }, t("confirm.back"));
    dangerScreen({ title: t("danger." + reason + ".title"), lead: t("danger." + reason + ".lead"), lose: [t("danger." + reason + ".lose1")], keep: [t("danger." + reason + ".keep1")],
      slide: t("danger." + reason + ".slide"), warn: t("danger.cannot"), onCancel: () => { pw = ""; route(true); },
      onConfirm: async () => {
        if (reason === "trash") await alp.trashEmpty(pw); else await alp.galleryDelete(ids.length ? ids : ["demo"], pw);
        pw = "";
        show("danger-done", (card) => { card.append(h("div", { class: "big-dot", "aria-hidden": "true" }, "✓"), h("h1", {}, t("danger." + reason + ".ok")), h("p", { class: "sub" }, t("confirm.ok")), h("div", { class: "actions" }, back)); }, { foot: "" });
      } });
  }
  // Delete everything and start over: three screens (button, button, slide).
  function resetDanger(pw, step) {
    const place = S.arch.place ? " (" + (S.arch.place.label || S.arch.place.name) + ")" : "", cancel = () => { pw = ""; openSettings("advanced"); };
    const lose = [t("reset.lose.archive", { name: place }), t("reset.lose.index")], keep = [t("reset.keep.disk"), t("reset.keep.computer"), t("reset.keep.phones")];
    if (step === 1) return dangerScreen({ step: { n: 1, total: 3 }, title: t("reset.1.title"), lead: t("reset.1.lead"), lose, button: t("danger.continue"), onCancel: cancel, onConfirm: () => resetDanger(pw, 2) });
    if (step === 2) return dangerScreen({ step: { n: 2, total: 3 }, title: t("reset.2.title"), lead: t("reset.2.lead"), keep, warn: t("reset.2.warn"), button: t("danger.continue"), onCancel: cancel, onConfirm: () => resetDanger(pw, 3) });
    dangerScreen({ step: { n: 3, total: 3 }, title: t("reset.3.title"), lead: t("reset.3.lead"), lose, warn: t("danger.cannot"), slide: t("reset.slide"), working: t("reset.working"), onCancel: cancel,
      onConfirm: async () => {
        const r = await alp.reset(pw); pw = "";
        Object.assign(L, { welcomed: false, phoneDone: false, recoveryDone: false, finished: false, mode: "setup", signed: false, view: null, pair: null, seed: null, skip: {}, exited: false,
          net: { chosen: null, pw: "", nets: null }, disk: { partuuid: null, size: null }, pwd: { a: "", b: "" }, netFrom: false, netBack: null, setCat: null });
        advancedUntil = 0; hist.length = 0;
        if (r && r.job) return resetProgress(r.job);   // 202: job "reset" (an unprotected stick is erased for real)
        await refresh(); detectMode();
        dangerDone(t("reset.ok"), () => { L.view = null; L.welcomed = false; route(true); }, t("reset.go"));
      } });
  }
  // The reset job: erasing can take many minutes on a big stick. It is done when lock.state is setup again; the
  // job's last message says what happened on the backup disk.
  function resetProgress(job) {
    show("reset", (card) => {
      const bar = h("i"), wrap = h("div", { class: "progress", role: "progressbar", "aria-valuemin": 0, "aria-valuemax": 100 }, bar);
      const line = waitLine(t("reset.working"), "wait.generic.slow", 120), end = h("div");
      card.append(hero("refresh"), h("h1", {}, t("reset.run.title")), h("p", { class: "sub" }, t("reset.run.sub")), wrap, line, end);
      let last = job, fin = false;
      return { update(s) {
        if (fin) return;
        const j = s.arch.job && s.arch.job.id === "reset" ? s.arch.job : null;
        if (j) last = j;
        const p = Math.max(1, (last && last.percent) || 0); bar.style.inlineSize = p + "%"; wrap.setAttribute("aria-valuenow", p);
        if (last && last.message) line.now(last.message);
        if (last && last.state === "failed") { fin = true; line.remove(); end.replaceChildren(banner("bad", last.error || { key: "reset.failed", message: last.message })); return; }
        if (s.lock.state === "setup" && (!j || j.state === "done")) {
          fin = true; detectMode();
          dangerDone(t("reset.ok"), () => { L.view = null; L.welcomed = false; route(true); }, t("reset.go"), last && last.state === "done" ? last.message : null);
        }
      } };
    }, { foot: "" });
  }
  // UX spec 7: turning off "Only paired phones" weakens protection: password (asked before), warning, slide.
  function strictOffDanger() {
    dangerScreen({ title: t("danger.strict.title"), lead: t("danger.strict.lead"), lose: [t("danger.strict.lose")], keep: [t("danger.strict.keep")],
      slide: t("danger.strict.slide"), onCancel: () => openSettings("phones"),
      onConfirm: async () => { await alp.pairStrict(false); await refresh(); dangerDone(t("danger.strict.ok"), () => openSettings("phones")); } });
  }
  // UX spec 7 (feature password_change): current password, the new one twice, a warning (every phone signs in
  // again), a slide. The server changes everything in one step (security review 5).
  async function changePassword() {
    let cur = await confirmPassword("confirm.pwchange");
    if (!cur) return;
    show("pwchange", (card) => {
      const p1 = h("input", { type: "password", id: "np1", autocomplete: "new-password", maxlength: 128, "data-focus": "" }), p2 = h("input", { type: "password", id: "np2", autocomplete: "new-password", maxlength: 128 });
      const res = h("div");
      card.append(codeHero("alp"), h("h1", {}, t("pwc.title")), h("p", { class: "sub" }, t("pwc.sub")),
        h("form", { onsubmit: (ev) => { ev.preventDefault();
          if (p1.value.length < 8) return res.replaceChildren(banner("warn", { key: "password.too_short" }));
          if (p1.value !== p2.value) return res.replaceChildren(banner("warn", null, t("pw.mismatch")));
          let nw = p1.value; p1.value = p2.value = "";
          dangerScreen({ title: t("danger.pwc.title"), lead: t("danger.pwc.lead"), lose: [t("danger.pwc.lose")], keep: [t("danger.pwc.keep1"), t("danger.pwc.keep2")],
            slide: t("danger.pwc.slide"), onCancel: () => { cur = nw = null; openSettings("password"); },
            onConfirm: async () => { await alp.passwordChange(cur, nw); cur = nw = null; await refresh(); dangerDone(t("danger.pwc.ok"), () => openSettings("password")); } });
        } },
          h("div", { class: "field c-alp" }, codeLabel("alp", "np1", t("pwc.new")), h("div", { class: "input-row" }, p1, eyeBtn([p1, p2]))),
          h("div", { class: "field c-alp" }, codeLabel("alp", "np2", t("pw.again")), p2), res,
          h("div", { class: "actions" }, h("button", { class: "primary", type: "submit" }, t("common.continue")),
            h("button", { class: "link", type: "button", "data-back": "", onclick: () => { cur = null; openSettings("password"); } }, t("common.cancel")))));
    }, { foot: "" });
  }

  /* ---------- password again before deleting (the gallery's delete guard sends people here) ---------- */
  function confirmPage() {
    show("confirm", (card) => {
      const reason = params.get("reason") === "trash" ? "trash" : "delete";
      const res = h("div"), pw = h("input", { type: "password", id: "cpw", autocomplete: "current-password", maxlength: 128, "data-focus": "" });
      const go = h("button", { class: "primary", type: "submit" }, t("confirm.go"));
      const f = h("form", { onsubmit: async (ev) => { ev.preventDefault(); if (!pw.value || go.disabled) return; go.disabled = true;
        try { const typed = pw.value; await alp.verify(typed); pw.value = ""; galleryDanger(reason, typed, (params.get("ids") || "").split(",").filter(Boolean)); }   // password first, then the warning
        catch (e) { pw.value = ""; go.disabled = false; res.replaceChildren(banner("bad", e)); pw.focus(); } } },
        h("div", { class: "field c-alp" }, codeLabel("alp", "cpw", t("pw.label")), h("div", { class: "input-row" }, pw, eyeBtn([pw]))), res, h("div", { class: "actions" }, go,
          h("button", { class: "link", type: "button", onclick: backToGallery }, t("confirm.back"))));
      card.append(codeHero("alp"), h("h1", {}, t("confirm." + reason)), h("p", { class: "sub" }, t("confirm.sub")), f);
    }, { foot: "" });
  }

  /* ---------- state polling ---------- */
  async function refresh() {
    try { S = await alp.state(); offline = 0; setConn(false); if (L.mode === "everyday") L.signed = S.console.state === "open" || S.lock.state === "setup"; return true; }
    catch (e) { offline++; if (offline > 1) setConn(true); return false; }
  }
  function setConn(on) {
    const c = document.getElementById("conn");
    if (on && !c) { document.body.append(h("div", { id: "conn", class: "conn", role: "status" }, t("err.api.unreachable"))); Errs.add({ id: "api.unreachable", level: "bad", text: t("err.api.unreachable") }); }
    if (!on && c) { c.remove(); Errs.resolve("api.unreachable"); }
  }
  function stopPolling() { clearTimeout(pollTimer); pollTimer = null; }
  async function loop() {
    if (powerOffDone) return;   // the shutdown screen polls by itself
    if (await refresh() && current) {
      if (current.name === "progress" && S.arch.state !== "creating") route(true);
      else if (L.mode === "everyday" && ((!L.signed && current.name !== "lock" && current.name !== "unlock" && current.name !== "confirm") || (L.signed && current.name === "lock"))) route(true);
      else current.update(S);
      renderTray();
      Errs.sync(S); offerImport(S);
      // The approval replaces the pairing QR (the phone has scanned it), never another dialog.
      if (L.signed && S.pairing.pending.length && (!modalEl || modalEl.dataset.pairqr)) approvePhone(S.pairing.pending[0]);
    }
    pollTimer = setTimeout(loop, 2000);
  }

  window.addEventListener("alp-locked", async () => { await refresh(); if (L.mode === "everyday") { L.signed = false; hist.length = 0; closeModal(); route(true); } });

  /* ---------- keyboard ---------- */
  document.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape") { const b = document.querySelector(".modal [data-back]") || document.querySelector("[data-back]"); if (b) { ev.preventDefault(); b.click(); } return; }
    const el = document.activeElement;
    if (/^[1-9]$/.test(ev.key) && !(el.tagName === "INPUT" && el.type !== "radio")) {
      const r = document.querySelectorAll(".card input[type=radio]:not([disabled])")[Number(ev.key) - 1];
      if (r) { r.checked = true; r.focus(); ev.preventDefault(); }
    }
  });

  /* ---------- boot ---------- */
  async function boot() {
    if (params.get("view") === "confirm") L.view = "confirm";
    const theme = params.get("theme"); if (theme) document.documentElement.dataset.theme = theme;
    if (params.get("dir")) document.documentElement.dir = params.get("dir");
    try { T = await (await fetch("i18n/en.json")).json(); } catch (e) { /* keys show instead of text */ }
    document.title = t("app.name"); document.getElementById("brand").textContent = t("app.name");
    applyPrefs();
    // UX spec 1: the start screen is never silent. After 90 seconds a quiet line with "Keep waiting"; the technical
    // side goes to (i).
    show("starting", (card) => {
      const slow = h("div", { class: "hidden" }, h("p", { class: "fine" }, t("start.slow")),
        h("div", { class: "row" }, h("button", { class: "secondary", type: "button", onclick: () => { slow.classList.add("hidden"); t0 = Date.now(); } }, t("start.wait"))));
      let t0 = Date.now();
      const timer = setInterval(() => { if (!slow.isConnected) return clearInterval(timer); if (Date.now() - t0 > 90000 && slow.classList.contains("hidden")) { slow.classList.remove("hidden"); Errs.add({ id: "boot.slow", level: "warn", text: t("start.slow") }); } }, 1000);
      card.append(hero("logo"), h("h1", {}, t("app.name")), waitLine(t("app.starting")), slow);
    }, { instant: true, foot: "" });
    while (!(await refresh())) await sleep(1500);
    Errs.resolve("boot.slow");
    const ms = params.get("screen");
    detectMode();
    if (!ms) { if (L.mode === "setup") { L.welcomed = S.lock.state !== "setup" && S.arch.state !== "none"; if (S.arch.state === "open" || S.arch.state === "creating") L.phoneDone = S.phone.paired; } }
    else {   // demo/screenshot presets (mock only)
      const setupPresets = ["welcome", "welcome-fail", "password", "network", "network-up", "where", "where-empty", "where-blocked", "progress", "phone", "recovery", "done"];
      L.mode = setupPresets.includes(ms) ? "setup" : "everyday";
      L.welcomed = !["welcome", "welcome-fail"].includes(ms);
      if (ms === "network-up") L.view = "network";
      if (ms === "home-empty") L.exited = true;
      L.phoneDone = ms !== "phone"; L.recoveryDone = ms !== "recovery"; L.finished = !setupPresets.includes(ms);
      L.signed = ["home-empty", "disks", "admin", "home", "home-warn", "home-full", "home-starting", "home-missing", "settings", "settings-chain", "confirm", "chain-seed", "chain-join", "saver", "files"].includes(ms);
      if (ms.startsWith("settings")) L.view = "settings";
      if (ms === "disks" || ms === "admin" || ms === "files") L.view = ms;
      if (ms === "confirm" || ms === "chain-seed" || ms === "chain-join") L.view = ms;
      if (ms === "chain-seed") L.seed = await alp.chainCreate();
      if (ms === "saver") startSaver();
    }
    if (window.useMock && params.get("view")) L.view = params.get("view");
    route(true);
    if (window.useMock && params.get("danger")) demoDanger(params.get("danger"));   // QA screenshots only
    if (window.useMock && params.get("off")) powerOff();   // QA screenshots only: the shutdown screen
    pollTimer = setTimeout(loop, 2000);
  }
  function demoDanger(k) {
    const pw = "demo", step = (n) => resetDanger(pw, n);
    ({ trash: () => galleryDanger("trash", pw, []), delete: () => galleryDanger("delete", pw, ["a", "b"]), chain: chainLeaveDanger, forget: forgetDanger, reset1: () => step(1), reset2: () => step(2), reset3: () => step(3) })[k]();
    setTimeout(() => { const sp = params.get("slide"), tr = document.querySelector(".slide-confirm"); if (sp && tr) tr.style.setProperty("--p", sp); }, 500);
  }
  boot();
})();
