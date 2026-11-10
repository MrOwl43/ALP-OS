// SPDX-License-Identifier: AGPL-3.0-or-later
// Client for alpd (http://127.0.0.1:8411/api/v1/live/...), contract: docs/plans/live-alpha-api.md.
// This is the only file that knows the wire shapes. The rest of the UI uses the
// normalised state built by normalize() below.
(function () {
  const BASE = "/api/v1/";
  class ApiError extends Error {
    constructor(key, message, code, status, extra) {
      super(key); this.key = key || "api.unreachable"; this.serverMessage = message || ""; this.code = code || null;
      this.status = status || 0; this.index = extra && extra.index; this.retryAfter = (extra && extra.retry_after_seconds) || 0;
    }
  }
  const params = new URLSearchParams(location.search);
  const useMock = params.get("mock") === "1";

  async function real(method, path, body) {
    let r;
    // Never wait forever for alpd (lead fix 2026-10-21): a request that hangs would stop the state polling.
    const ctl = new AbortController(), timer = setTimeout(() => ctl.abort(), 20000);
    try {
      r = await fetch(BASE + path, {
        method, cache: "no-store", signal: ctl.signal,
        headers: method === "GET" ? {} : { "Content-Type": "application/json" },   // Origin is set by the browser
        body: method === "GET" ? undefined : JSON.stringify(body === undefined ? {} : body),
      });
    } catch (e) { throw new ApiError("api.unreachable", "", null, 0); }
    finally { clearTimeout(timer); }
    let data = null;
    try { data = await r.json(); } catch (e) { /* empty body */ }
    if (!r.ok) {
      const er = (data && data.error) || {};
      if (er.key === "console.locked") window.dispatchEvent(new Event("alp-locked"));
      throw new ApiError(er.key || "internal", er.message, er.code, r.status, er);
    }
    return data;
  }
  const raw = (m, p, b) => (useMock && window.MockApi ? window.MockApi.call(m, p, b) : real(m, p, b));

  // Wire state -> what the screens use.
  function normalize(w) {
    const hw = w.hardware || {}, nw = w.network || {}, ar = w.archive || {}, ph = w.phone || {}, ch = w.chain || {};
    const ports = nw.ports || [];
    const wired = ports.find((p) => p.kind === "wired" && p.state === "connected");
    const wifi = ports.find((p) => p.kind === "wifi" && p.state === "connected");
    // 2026-10-26 fields (addresses, battery, power job, gallery URL, Immich detail, PC specs) may be missing on an older alpd.
    const sy = w.system || {}, im = w.immich || {}, pw = w.power || {};
    const addrs = (Array.isArray(nw.addresses) ? nw.addresses : []).map((a) => (typeof a === "string" ? { ip: a, url: null, kind: null } : a && { ip: a.address, url: a.url || null, kind: a.kind || null })).filter((a) => a && a.ip);
    if (!addrs.length && nw.address) addrs.push({ ip: nw.address, url: null, kind: null });
    const bat = sy.battery;
    return {
      sys: { battery: bat && typeof bat.percent === "number" ? { percent: Math.round(bat.percent), charging: !!bat.charging } : null, release: sy.release || null, build: sy.build || null },
      power: { state: pw.state || "on", job: pw.job || null },
      hw: { canRun: hw.verdict !== "fail", verdict: hw.verdict, machine: hw.machine, reportSaved: !!hw.report_file, pending: !!hw.pending, checks: hw.checks || [], specs: hw.specs || null },
      net: { online: !!nw.online, address: nw.address, wifiPresent: !!nw.wifi_present, kind: wired ? "wired" : wifi ? "wifi" : null,
        ssid: wifi && wifi.ssid, signal: wifi && wifi.signal, waiting: ports.some((p) => p.state === "waiting"), job: nw.job, addresses: addrs },
      lock: { state: (w.lock || {}).state, wait: (w.lock || {}).retry_after_seconds || 0, tpm: (w.lock || {}).tpm || "none", protection: (w.lock || {}).protection || null },
      console: { state: (w.console || {}).state || "open", idle: (w.console || {}).idle_lock_seconds || 300 },
      index: { size: (w.index || {}).size_bytes || 0, free: (w.index || {}).free_bytes || 0, space: (w.index || {}).space || "ok" },
      arch: { state: ar.state, space: ar.space || "ok", health: ar.health || null, recovery: ar.recovery || "none", passwordReopen: ar.password_reopen !== false, serious: !!ar.serious, job: ar.job, place: ar.place, size: ar.size_bytes || 0, free: ar.free_bytes || 0, error: ar.error },
      immich: { state: im.state, detail: im.detail || null, consoleUrl: im.console_url || null, galleryUrl: im.gallery_url || null, error: im.error || null, startingFor: im.seconds_starting == null ? null : im.seconds_starting },
      pairing: { window: (w.pairing || {}).window_seconds || null, pending: (w.pairing || {}).pending || [], devices: ((w.pairing || {}).devices || []).filter((d) => !d.revoked), strict: !!(w.pairing || {}).strict },
      phone: { url: ph.server_url, account: ph.account, paired: !!ph.paired, phones: ph.phones || 0, photos: ph.photos || 0, last: ph.last_upload, devices: ph.devices || [], now: ph.connected_now || [] },
      chain: { state: ch.state, role: ch.role, id: ch.chain_id },
      // 0.2.4 UX spec: every new server capability is announced by name (docs/plans/live-ui-0.2.4.md);
      // without it the screens keep the older behaviour, so this UI also runs on an older alpd.
      features: new Set(Array.isArray(w.features) ? w.features : []),
      // One key: a second "setup" here once hid finished (T09; QA review 0.2.5).
      setup: { finished: !!(w.setup || {}).finished, complete: !!(w.setup || {}).complete, skipped: Array.isArray((w.setup || {}).skipped) ? w.setup.skipped : [] },
      recoveryMade: ar.recovery_made || null,
      events: Array.isArray(w.events) ? w.events : [],
      media: Array.isArray(w.media) ? w.media : [],
      importJob: w.import_job || null,
      terminal: w.terminal || null,
      prefs: { naturalScrolling: (w.prefs || {}).natural_scrolling, uiScale: (w.prefs || {}).ui_scale, homeBackground: (w.prefs || {}).home_background || "blue" },
      // T56: the screen's size and modes when alpd knows them (feature display_modes); width_mm from the screen's EDID.
      display: w.display || null,
      // T54 (feature status_levels): the worst alert and the list; level ok|note|action|now|critical.
      status: w.status || null, alerts: Array.isArray(w.alerts) ? w.alerts : [],
    };
  }

  window.ApiError = ApiError;
  window.useMock = useMock;
  window.alp = {
    state: async () => normalize(await raw("GET", "live/state")),
    wifiNetworks: async () => (await raw("GET", "live/wifi/networks")).networks,
    wifiConnect: (ssid, password) => raw("POST", "live/wifi/connect", { ssid, password }),
    disks: async () => (await raw("GET", "live/disks")).disks,   // NOT in the real API yet: see docs/plans/live-ux.md
    candidates: async () => (await raw("GET", "live/archive/candidates")).candidates,
    // Storage v2 (ADR 0014): no size; the archive grows into the disk.
    createArchive: (partuuid, serious, password, alpPassword) => raw("POST", "live/archive/create", serious ? { partuuid, serious: true, password } : alpPassword ? { partuuid, alp_password: alpPassword } : { partuuid }),
    prepareDisk: (disk_id, partuuids, password) => raw("POST", "live/disks/prepare", { disk_id, partuuids, password }),
    setupPassword: (password, protection) => raw("POST", "live/password/setup", protection ? { password, protection } : { password }),
    unlock: (password) => raw("POST", "live/unlock", { password }),
    forgetArchive: () => raw("POST", "live/archive/forget", {}),
    // 2026-10-26: always 200 with the addresses; ready / needs_password / immich.reason say whether a phone can sign in yet.
    pair: (password) => raw("POST", "live/phone/pair", password ? { password } : {}),
    removeDevice: (id) => raw("POST", "live/phone/devices/remove", { id }),
    // ADR 0017: pairing a phone with the QR and an approval on this screen.
    pairOpen: () => raw("POST", "live/pair/open", {}),
    pairClose: () => raw("POST", "live/pair/close", {}),
    pairApprove: (id) => raw("POST", "live/pair/approve", { id }),
    pairReject: (id) => raw("POST", "live/pair/reject", { id }),
    pairRevoke: (serial) => raw("POST", "live/pair/revoke", { serial }),
    pairStrict: (on) => raw("POST", "live/pair/strict", { on }),
    chainCreate: () => raw("POST", "live/chain/create", {}),
    chainJoin: (words) => raw("POST", "live/chain/join", { words }),
    verify: (password) => raw("POST", "live/console/signin", { password }),   // sign-in; also used to ask for the password again
    lock: () => raw("POST", "live/console/lock", {}),
    activity: () => raw("POST", "live/console/activity", {}),
    // T43 (two phases, feature recovery_commit): create writes recovery.conf.new and answers the words once with
    // confirm: [i, j, k]; commit with those words swaps it in; discard (or a timeout) keeps the old code.
    // alpd answers the positions as check: [i, j, k]; the screens read confirm.
    recoveryCreate: async (opts) => {
      const r = await raw("POST", "live/recovery/create", Object.fromEntries(Object.entries(opts || {}).filter(([, v]) => v !== undefined && v !== null && v !== "")));
      if (r && !r.confirm && Array.isArray(r.check)) r.confirm = r.check;
      return r;
    },
    recoveryCommit: (words) => raw("POST", "live/recovery/commit", { words }),
    recoveryDiscard: () => raw("POST", "live/recovery/discard", {}),
    setupProgress: (body) => raw("POST", "live/setup/progress", body),   // T09 (feature setup_progress)
    setupFinish: () => raw("POST", "live/setup/finish", {}),   // T09 without setup_progress: a restart does not start setup again
    passwordChange: (password, new_password) => raw("POST", "live/password/change", { password, new_password }),
    reportSave: () => raw("POST", "live/report/save", {}),
    setPrefs: (p) => raw("POST", "live/prefs", p),
    displayModes: async () => (await raw("GET", "live/display/modes")).modes || [],
    displayMode: (id) => raw("POST", "live/display/mode", { id }),   // reverts by itself after 15 s unless kept
    displayKeep: () => raw("POST", "live/display/mode/keep", {}),
    // T52: quick or secure format of an external disk (job disk.format, cancel through archive/cancel while erasing).
    formatEstimate: (disk_id, partuuids) => raw("POST", "live/disks/format/estimate", { disk_id, partuuids }),
    // T58 (feature disks): extra fields only when set, so an older alpd gets exactly what it knew.
    format: (disk_id, partuuids, mode, password, label, extra) => raw("POST", "live/disks/format", Object.assign({ disk_id, partuuids, mode, password }, label ? { label } : {}, extra || {})),
    displayRevert: () => raw("POST", "live/display/mode/revert", {}),
    diskEject: (disk_id) => raw("POST", "live/disks/eject", { disk_id }),
    diskCheck: (disk_id) => raw("POST", "live/disks/check", { disk_id }),
    // T47 Import (feature media_import): opaque ids only, never paths.
    mediaBrowse: (media, folder, offset) => raw("POST", "live/media/browse", folder ? { media, folder, offset: offset || 0 } : { media, offset: offset || 0 }),
    importStart: (media, items) => raw("POST", "live/import", { media, items }),
    mediaEject: (media) => raw("POST", "live/media/eject", { media }),
    // Stage F Files (feature media_files): read-only (media.rs). The media id is opaque; the path is made only of
    // names the server listed, one folder per element, checked again by the server (no "..", no links).
    // Terminal (feature terminal): the password in the body every time; the answer's url is a one-time secret.
    terminalStart: (password) => raw("POST", "live/terminal/start", { password }),
    terminalStop: () => raw("POST", "live/terminal/stop", {}),
    mediaOpen: (media) => raw("POST", "live/media/open", { media }),
    mediaList: (media, path, offset) => raw("POST", "live/media/list", { media, path: path || [], offset: offset || 0 }),
    // A file is shown only by URL on the console's own origin (img/video, CSP 'self'), never as HTML: the server
    // sends an allow-listed type with nosniff and a sandbox CSP. Text is read as text into a text node.
    mediaFileUrl: (media, path) => useMock && window.MockApi && window.MockApi.fileUrl ? window.MockApi.fileUrl(media, path)
      : BASE + "live/media/file?media=" + encodeURIComponent(media) + "&path=" + encodeURIComponent(JSON.stringify(path)),
    mediaText: async (media, path, max) => {
      if (useMock && window.MockApi && window.MockApi.fileText) return window.MockApi.fileText(media, path);
      const ctl = new AbortController(), timer = setTimeout(() => ctl.abort(), 20000);
      try {
        const r = await fetch(BASE + "live/media/file?media=" + encodeURIComponent(media) + "&path=" + encodeURIComponent(JSON.stringify(path)),
          { cache: "no-store", signal: ctl.signal, headers: { Range: "bytes=0-" + (max - 1) } });
        if (!r.ok) throw new ApiError("media.unreadable", "", null, r.status);
        if (!/^text\/plain\b/i.test(r.headers.get("Content-Type") || "")) throw new ApiError("media.not_text", "", null, r.status);
        // Never more than max bytes in memory, even if the server ignores Range (review F1): read in chunks, stop at max.
        const rd = r.body.getReader(), parts = []; let got = 0;
        while (got < max) {
          const { done, value } = await rd.read();
          if (done) break;
          const take = value.subarray(0, max - got); parts.push(take); got += take.length;
        }
        rd.cancel().catch(() => {});
        const all = new Uint8Array(got); let at = 0; parts.forEach((x) => { all.set(x, at); at += x.length; });
        return new TextDecoder("utf-8", { fatal: false }).decode(all);
      } catch (e) { if (e instanceof ApiError) throw e; throw new ApiError("api.unreachable", "", null, 0); }
      finally { clearTimeout(timer); }
    },
    encryptArchive: (password) => raw("POST", "live/archive/encrypt", { password }),
    recoveryCheck: (words, passphrase) => raw("POST", "live/recovery/check", passphrase ? { recovery_key: words, passphrase } : { recovery_key: words }),
    reopenWithPassword: (partuuid, alp_password) => raw("POST", "live/archive/recover", { partuuid, alp_password }),
    recoverArchive: (partuuid, recovery_key, passphrase) => raw("POST", "live/archive/recover", passphrase ? { partuuid, recovery_key, passphrase } : { partuuid, recovery_key }),
    galleryDelete: (ids, password) => raw("POST", "live/gallery/delete", { ids, password }),
    trashEmpty: (password) => raw("POST", "live/gallery/trash/empty", { password }),
    chainCode: () => raw("GET", "live/chain/code"),
    chainLeave: () => raw("POST", "live/chain/leave", {}),
    cancelArchive: () => raw("POST", "live/archive/cancel", {}),
    reset: (password) => raw("POST", "live/reset", { password }),     // 202 { job } (job "reset"); an older alpd answers 200
    powerOff: () => raw("POST", "live/power/off", {}),   // 202 { job } (power.off); an older alpd answers {}
  };
})();
