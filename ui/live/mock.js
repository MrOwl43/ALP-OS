// SPDX-License-Identifier: AGPL-3.0-or-later
// Mock of the Live console API (shapes from docs/plans/live-alpha-api.md) for ?mock=1.
//   &screen=home-empty|where-blocked|disks|admin|network-up|welcome|welcome-fail|password|network|where|where-empty|progress|phone|recovery|done|unlock|unlock-wait|lock|lock-warn|home|home-warn|home-full|home-starting|home-missing|settings|settings-chain|chain-seed|chain-join|confirm|saver
//   &fail=wifi|archive|phone|chain|needpw|off   one failure with the contract's error key (needpw: pair asks for the ALP password once)
//   &battery=none|low|charging (default: 76 %, not plugged in)   &immich=slow (the photo service takes long to start)
//   &theme=dark|light   &dir=rtl   &fast=1 (short delays)   &showerr=<key> (demo banner)
//   &features=none|<comma list> (0.2.4 server features; default: all of them, docs/plans/live-ui-0.2.4.md)
//   &media=1 (a camera card plugged in, for Import)
(function () {
  const p = new URLSearchParams(location.search);
  const fast = p.get("fast") === "1";
  if (p.get("shot")) document.write('<img src="/__slow" alt="" hidden>');   // QA only: delays the load event so a headless screenshot waits for the screen
  const sl = (ms) => new Promise((r) => setTimeout(r, fast ? 30 : ms));
  const GB = 1e9;
  const ALL_FEATURES = ["setup_progress", "recovery_commit", "password_change", "events", "gallery_session", "media_import", "natural_scrolling", "media_files", "terminal", "disks", "disk_check", "disk_eject", "status_levels", "prefs", "display_modes", "idle_lock_pref"];
  const FEATURES = p.get("features") === "none" ? [] : p.get("features") ? p.get("features").split(",") : ALL_FEATURES;
  let failOnce = p.get("fail");
  const E = (key, message, status, extra) => new ApiError(key, message, null, status || 400, extra);

  const base = () => ({
    api: 1,
    hardware: { machine: "ASUSTeK COMPUTER INC. VivoBook K3402ZA", verdict: "ok", report_file: "hw-ASUS-K3402ZA-20261019-0941.txt", checks: [
      { id: "memory", item: "Memory", status: "ok", value: "15.6 GB", advice: "" }, { id: "storage", item: "Storage", status: "ok", value: "1 TB", advice: "" },
      { id: "tpm", item: "Security chip", status: "note", value: "none", advice: "No security chip was found. Your archive still works and is still locked." },
      { id: "wired_network", item: "Network", status: "note", value: "none", advice: "No network yet." }],
      specs: { cpu: { model: "12th Gen Intel(R) Core(TM) i5-12500H", threads: 16 }, memory_bytes: 16777216000,
        disks: [{ name: "nvme0n1", model: "Samsung SSD 980", size_bytes: 512110190592, kind: "nvme", internal: true, system: false },
          { name: "sda", model: "SanDisk Cruzer Blade", size_bytes: 32010928128, kind: "usb", internal: false, system: true },
          { name: "sdb", model: "WD Elements 2TB", size_bytes: 2000e9, kind: "usb", internal: false, system: false },
          { name: "sdc", model: "SanDisk Ultra", size_bytes: 128e9, kind: "usb", internal: false, system: false }],
        graphics: [{ vendor: "Intel", pci_id: "8086:46a3" }],
        network: [{ name: "wlan0", kind: "wifi", driver: "iwlwifi", usb: false, working: true }, { name: "enx00e04c680001", kind: "wired", driver: "r8152", usb: true, working: true }],
        tpm: "2.0", uefi: true, secure_boot: false, battery: { model: "L20M4PC1", health_percent: 91 } } },
    system: { release: "ALP-OS Live alpha 0.2.5", build: "0650152a1b2c 2026-10-30", battery: p.get("battery") === "none" ? null : p.get("battery") === "low" ? { percent: 12, charging: false } : { percent: 76, charging: p.get("battery") === "charging" } },
    power: { state: "on", job: null },
    lock: { state: "open", retry_after_seconds: 0, tpm: "on", protection: p.get("protection") || "password" },
    console: { state: "open", idle_lock_seconds: p.get("idle") ? +p.get("idle") : 300 },
    index: { size_bytes: 28 * GB, free_bytes: 27 * GB, space: "ok" },
    network: { online: false, address: null, addresses: [], ports: [{ name: "wlan0", kind: "wifi", state: "off", address: null }], wifi_present: true, job: null },
    archive: { state: "none", space: "ok", health: "ok", recovery: "none", place: null, size_bytes: 0, free_bytes: 0, job: null, error: null },
    immich: { state: "off", error: null, detail: null, seconds_starting: null, console_url: null },
    phone: { server_url: null, account: null, paired: false, phones: 0, photos: 0, last_upload: null, devices: [] },
    chain: { state: "none", chain_id: null, role: null, since: null },
    features: FEATURES, setup: { complete: false, skipped: [] }, events: [], media: [], import_job: null, prefs: { natural_scrolling: true, ui_scale: null, home_background: "blue" }, display: { width_mm: 309, height_mm: 174, auto_scale: 1.25 }, alerts: [], status: null,
  });
  const W = base();
  const online = (kind) => { W.network.online = true; W.network.address = "192.168.1.23";
    W.network.addresses = [{ address: "192.168.1.23", url: "https://192.168.1.23", port: kind === "wired" ? "enp0s3" : "wlan0", kind }].concat(p.get("addrs") === "2" ? [{ address: "10.0.0.5", url: "https://10.0.0.5", port: "wlan0", kind: "wifi" }] : []);
    W.network.ports = [kind === "wired"
    ? { name: "enp0s3", kind: "wired", state: "connected", address: "192.168.1.23" } : { name: "wlan0", kind: "wifi", state: "connected", address: "192.168.1.23", ssid: "Home-Fiber-5G", signal: 3 }]; W.hardware.checks[3] = { id: "wired_network", item: "Network", status: "ok", value: "online", advice: "" }; };
  const ready = () => Object.assign(W.immich, { state: "ready", detail: null, seconds_starting: null, console_url: "http://127.0.0.1:2283/" });
  const open = () => { Object.assign(W.archive, { state: "open", place: { partuuid: "p1", name: "sdb1", label: "Backup disk" }, size_bytes: 100 * GB, free_bytes: 90 * GB }); ready(); };
  // The photo service starting, with the plain sentences alpd gives (immich.detail, 2026-10-26).
  let immStart = 0;
  const startImmich = () => { immStart = Date.now(); Object.assign(W.immich, { state: "starting", console_url: null }); };
  const IMM_DETAIL = ["Unpacking the photo service for the first time (about 2.5 GB): part 1 of 3, 0.4 GB of 2.5 GB…", "Unpacking the photo service for the first time (about 2.5 GB): part 2 of 3, 1.1 GB of 2.5 GB…",
    "Unpacking the photo service for the first time (about 2.5 GB): part 3 of 3, 2.1 GB of 2.5 GB…", "Starting the photo service…", "Still starting: the first start sets up the photo database. This can take a few minutes more."];
  let offStart = 0;   // power/off: the job's steps, then alpd stops answering
  let needPw = false;
  let jobStart = 0, pairStart = 0, pw = null, prepStart = 0, prepared = false;   // pw: the password chosen in setup (null = accept anything but "wrong")
  const pwOk = (x) => !!x && x !== "wrong" && (pw === null || x === pw);

  const preset = p.get("screen");
  if (["welcome", "welcome-fail", "password", "network", "network-up"].includes(preset) || !preset) W.lock = { state: "setup", retry_after_seconds: 0 };
  if (preset === "network") { W.network.ports[0].state = "off"; }
  if (preset === "unlock" || preset === "unlock-wait") { W.lock = { state: "locked", retry_after_seconds: preset === "unlock-wait" ? 120 : 0 }; online("wired"); Object.assign(W.archive, { state: "locked" }); }
  let plugAt = preset === "where-empty" ? Infinity : Date.now() + (fast ? 300 : 6000);
  if (preset === "where" || preset === "where-blocked") plugAt = 0;
  if (preset === "network-up") online("wifi");
  if (preset === "welcome-fail") { W.hardware.verdict = "fail"; W.hardware.checks[0] = { id: "memory", item: "Memory", status: "fail", value: "2 GB", advice: "" }; }
  if (["password", "where", "where-empty", "where-blocked", "progress", "phone", "recovery", "done", "home-empty", "disks", "admin", "lock", "lock-warn", "home", "home-warn", "home-full", "home-starting", "home-missing", "settings", "settings-chain", "confirm", "saver", "files"].includes(preset)) { online(preset === "home-empty" ? "wifi" : "wired"); W.lock = { state: "open", retry_after_seconds: 0, protection: "password" }; }
  if (["lock", "lock-warn", "saver"].includes(preset)) W.console.state = "locked";
  if (["home", "home-warn", "home-full", "home-starting", "settings", "settings-chain", "confirm", "disks", "admin", "lock", "lock-warn", "saver", "files"].includes(preset)) W.archive.recovery = "set";
  if (preset === "progress") { W.archive.state = "creating"; jobStart = Date.now(); }
  if (["phone", "recovery", "done", "lock", "lock-warn", "home", "home-warn", "home-full", "home-starting", "home-missing", "settings", "settings-chain", "confirm", "saver", "disks", "admin", "files"].includes(preset)) open();
  if (preset === "phone") pairStart = Date.now();
  const EVERY = ["disks", "admin", "files", "lock", "lock-warn", "home", "home-warn", "home-full", "home-starting", "settings", "settings-chain", "confirm", "saver"];
  if (EVERY.includes(preset) || preset === "home-missing") {
    W.phone = { server_url: "https://192.168.1.23", account: "family@alparchive.local", paired: true, phones: 3, photos: 1284, last_upload: null,
      devices: [{ id: "s1", kind: "phone", name: "iPhone", os: "iOS", last_seen: "2026-10-28T09:12:00.000Z" }, { id: "s2", kind: "phone", name: "Pixel 8", os: "Android", last_seen: "2026-10-28T08:40:00.000Z" }, { id: "s3", kind: "tablet", name: "iPad", os: "iOS", last_seen: "2026-10-27T19:05:00.000Z" }],
      connected_now: ["192.168.1.50"] };
    W.chain = { state: "member", chain_id: "3f9c2a1be04d7781", role: "created" };
  }
  if (preset === "settings-chain") W.chain = { state: "none", chain_id: null, role: null };
  if (W.lock.state !== "setup" && !W.lock.protection) W.lock.protection = "password";
  if (p.get("protection")) W.lock.protection = p.get("protection");   // &protection=none|console|password|strong
  const lvp = p.get("level");   // &level=note|action|now|critical: T54 status from the server
  if (lvp && FEATURES.includes("status_levels")) { W.status = { level: lvp, key: "mock." + lvp, message: "Mock status: " + lvp, code: "ALP-MOCK-1" }; W.alerts = [Object.assign({}, W.status)]; }
  if (preset === "lock-warn" || preset === "home-warn") { W.archive.free_bytes = 6 * GB; W.archive.space = "low"; W.archive.health = "warn";
    if (!lvp) { W.status = { level: "action", key: "space.low", message: "The backup disk is getting full.", code: "ALP-STO-110" }; W.alerts = [Object.assign({}, W.status)]; } }
  if (preset === "home-full") { W.archive.free_bytes = 0; W.archive.space = "full"; W.archive.health = "warn"; W.immich.state = "paused"; }
  if (preset === "home-starting" || (p.get("immich") === "slow" && W.immich.state === "ready")) startImmich();
  if (EVERY.includes(preset) || preset === "home-missing") W.setup.complete = true;
  if (["home", "settings"].includes(preset)) W.archive.recovery_made = "2026-11-01T10:00:00Z";
  if (p.get("media") === "1") W.media = [{ id: "m1", label: "Camera card", fs_type: "exfat", size_bytes: 64 * GB, photos: true }];
  if (preset === "files") W.media = [{ id: "m1", label: "Camera card", fs_type: "exfat", size_bytes: 64 * GB, mounted: false, refused: null },
    { id: "m2", label: "<b>Old disk</b>", fs_type: "btrfs", size_bytes: 500 * GB, mounted: false, refused: "ALPArchive cannot read this kind of disk (btrfs)." }];
  if (preset === "home-missing") { W.archive.state = "missing"; W.archive.place = null; W.immich.state = "off"; W.archive.error = { key: "archive.missing", message: "We cannot find your archive." }; }

  let pendingRec = null, fmtJob = null, resetAt = 0, impAt = 0;
  const MODES = [{ id: "1920x1080@60", width: 1920, height: 1080, refresh: 60, native: true, current: true }, { id: "1600x900@60", width: 1600, height: 900, refresh: 60, native: false, current: false },
    { id: "1366x768@60", width: 1366, height: 768, refresh: 60, native: false, current: false }, { id: "1280x720@60", width: 1280, height: 720, refresh: 60, native: false, current: false }];
  const ejected = new Set();
  // T58 GET live/disks, in the shape agreed with the Storage Engineer.
  const mockDisks = () => [
    { id: "nvme0n1-3", name: "nvme0n1", model: "Samsung SSD 980", size_bytes: 512110190592, kind: "internal", role: "other", locked: true, lock: "internal_live", safe_to_remove: false, health: "healthy", temperature_c: 41,
      parts: [{ partuuid: "i1", label: "EFI", fs: "vfat", size_bytes: 0.3 * GB }, { partuuid: "i2", label: "Windows", fs: "ntfs", size_bytes: 511 * GB }] },
    { id: "sda-10", name: "sda", model: "SanDisk Cruzer Blade", size_bytes: 32010928128, kind: "stick", role: "system", locked: true, lock: "stick", safe_to_remove: false, health: null, temperature_c: null,
      parts: [{ partuuid: "s1", label: "ALP-EFI", fs: "vfat", size_bytes: 1 * GB }, { partuuid: "s2", label: "ALP-SYSTEM", fs: "erofs", size_bytes: 8 * GB }, { partuuid: "s3", label: "ALP-STATE", fs: "ext4", size_bytes: 22 * GB }] },
    { id: "sdb-11", name: "sdb", model: "WD Elements 2TB", size_bytes: 2000 * GB, kind: "external", role: "backup", locked: true, lock: "backup", safe_to_remove: false, health: "healthy", temperature_c: 34,
      parts: [{ partuuid: "p1", label: "Backup disk", fs: "exfat", size_bytes: 2000 * GB, free_bytes: 1800 * GB }] },
    { id: "sdc-12", name: "sdc", model: "<b>SanDisk</b> Ultra", size_bytes: 128 * GB, kind: "external", role: "other", locked: false, lock: null, safe_to_remove: ejected.has("sdc-12"), health: "wear", level: "action", temperature_c: null,
      parts: [{ partuuid: "p2", label: "Photos USB", fs: "vfat", size_bytes: 100 * GB, free_bytes: 76 * GB }] },
  ];
  function tick() {
    if (W.immich.state === "ready" && FEATURES.includes("gallery_session")) W.immich.gallery_url = p.get("gallery") || "http://127.0.0.1:8412/";
    if (fmtJob) {
      const sec = (Date.now() - fmtJob.t) / 1000 / (fast ? 20 : 1), dur = fmtJob.mode === "secure" ? 30 : 4;
      const pc = Math.min(100, Math.round((sec / dur) * 100));
      if (fmtJob.cancel) W.archive.job = { id: "disk.format", state: "cancelled", step: "erasing", percent: pc, message: "Stopped. The disk is only partly erased and has no file system now; format it again to use it.", can_cancel: false, error: null };
      else W.archive.job = { id: "disk.format", state: pc >= 100 ? "done" : "running", step: pc >= 100 ? "done" : fmtJob.mode === "secure" && pc < 85 ? "erasing" : "formatting", percent: pc,
        can_cancel: fmtJob.mode === "secure" && pc < 85, error: null, seconds_left: pc >= 100 ? null : Math.round((dur - sec) * 56),
        message: pc >= 100 ? "Done: the disk is formatted (" + fmtJob.mode + ")." : fmtJob.mode === "secure" && pc < 85 ? "Overwriting with zeros… " + pc + "% (about " + Math.max(1, Math.round((dur - sec) / 60)) + " min left)" : "Making the file system…" };
      if (pc >= 100 || fmtJob.cancel) fmtJob = null;
    }
    if (resetAt) {
      const pc = Math.min(100, Math.round(((Date.now() - resetAt) / (fast ? 300 : 6000)) * 100));
      W.archive.job = { id: "reset", state: pc >= 100 ? "done" : "running", step: pc >= 100 ? "done" : "erasing", percent: pc, error: null,
        message: pc >= 100 ? "Your archive was erased. Other files on the backup disk stayed." : "Erasing the stick… " + pc + "% (about 1 min left)" };
      if (pc >= 100) { resetAt = 0; const job = W.archive.job; for (const k of Object.keys(W)) delete W[k]; Object.assign(W, base()); W.lock = { state: "setup", retry_after_seconds: 0 }; online("wired"); W.archive.job = job; plugAt = Date.now() + (fast ? 300 : 6000); }
    }
    if (impAt) {
      const pc = Math.min(100, Math.round(((Date.now() - impAt) / (fast ? 300 : 8000)) * 100));
      W.import_job = { id: "import", state: pc >= 100 ? "done" : "running", percent: pc, message: "Importing " + Math.round(pc * 13) + " of 1,276…", photos: 1240, videos: 36, skipped: 112, error: null };
      if (pc >= 100) { impAt = 0; W.phone.photos += 1276; }
    }
    if (W.immich.state === "starting" && immStart) {
      const sec = (Date.now() - immStart) / 1000, dur = fast ? 2 : p.get("immich") === "slow" || preset === "home-starting" ? 60 : 12;
      W.immich.seconds_starting = Math.round(sec);
      W.immich.detail = IMM_DETAIL[Math.min(IMM_DETAIL.length - 1, Math.floor((sec / dur) * IMM_DETAIL.length))];
      if (sec > dur) { immStart = 0; ready(); }
    }
    if (offStart) {
      const sec = (Date.now() - offStart) / 1000 / (fast ? 10 : 1), steps = [[2, "immich", 10, "Stopping the photo service…"], [4, "archive", 40, "Closing the archive safely…"], [6, "index", 70, "Locking the index…"], [8, "saving", 85, "Saving everything to the disks…"], [99, "off", 100, "Turning off now…"]];
      const st = steps.find((x) => sec < x[0]);
      W.power.job = { id: "power.off", state: "running", step: st[1], percent: st[2], message: st[3], can_cancel: false, error: null };
    }
    const e = Date.now() - jobStart;
    if (W.archive.state === "creating") {
      const dur = fast ? 300 : 6000, pc = Math.min(100, Math.round((e / dur) * 100));
      const steps = [[5, "preparing"], [40, "checking"], [60, "encrypting"], [80, "formatting"], [100, "done"]];   // storage v2 (ADR 0014)
      const st = steps.find((s) => pc <= s[0]) || steps[4];
      const mb = Math.min(256, Math.round(((pc - 5) / 35) * 256));
      W.archive.job = { id: "archive.create", state: pc >= 100 ? "done" : "running", step: st[1], percent: pc, error: null,
        message: st[1] === "checking" ? "Checking the disk: " + mb + " MB of 256 MB at 18 MB/s…" : st[1] === "preparing" ? "This disk was not removed safely last time. Repairing it first…" : "" };
      if (pc >= 100) { open(); startImmich(); }
    }
    if (prepStart && Date.now() - prepStart > (fast ? 300 : 4000)) { prepStart = 0; prepared = true; W.archive.job = { id: "disk.prepare", state: "done", step: "done", percent: 100, message: "The disk is ready.", error: null }; }
    if (W.network.job && W.network.job.state === "running" && Date.now() - W.network.job.t > (fast ? 200 : 1800)) {
      if (W.network.job.bad) { W.network.job = { id: "wifi.connect", state: "failed", step: "joining", percent: null, error: { key: "wifi.password_refused", message: "The network did not accept this password. Please try again." } }; }
      else { online("wifi"); W.network.job = { id: "wifi.connect", state: "done", step: "address", percent: 100, error: null }; }
    }
    if (pairStart && !W.phone.paired && Date.now() - pairStart > (fast ? 800 : 9000)) W.phone.paired = true;
    if (W.phone.paired && pairStart) W.phone.photos = Math.max(W.phone.photos, Math.round(Math.max(0, Date.now() - pairStart - (fast ? 900 : 10000)) / 8));
  }

  function qr(seed) { // fake, NOT scannable; deterministic pattern with finder squares
    const n = 29, c = 8; let x = seed, r = "";
    const rnd = () => (x = (x * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff;
    const fin = (ox, oy) => `<rect x="${ox*c}" y="${oy*c}" width="${7*c}" height="${7*c}"/><rect x="${(ox+1)*c}" y="${(oy+1)*c}" width="${5*c}" height="${5*c}" fill="#fff"/><rect x="${(ox+2)*c}" y="${(oy+2)*c}" width="${3*c}" height="${3*c}"/>`;
    for (let i = 0; i < n; i++) for (let j = 0; j < n; j++) {
      const inF = (i < 8 && j < 8) || (i > n - 9 && j < 8) || (i < 8 && j > n - 9);
      if (!inF && rnd() > 0.52) r += `<rect x="${i*c}" y="${j*c}" width="${c}" height="${c}"/>`;
    }
    const s = n * c;
    return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-16 -16 ${s+32} ${s+32}" shape-rendering="crispEdges"><rect x="-16" y="-16" width="${s+32}" height="${s+32}" fill="#fff"/><g fill="#000">${r}${fin(0,0)}${fin(n-7,0)}${fin(0,n-7)}</g></svg>`;
  }
  const WORDS = "acorn,border,candle,dune,ember,fable,garden,harbor".split(",");   // chain code: 8 words
  const RWORDS = "island,jungle,kettle,lantern,meadow,nectar,orchard,pebble,quiver,river,saddle,timber,umbrella,velvet,willow".split(",");   // recovery code: 15 words
  const fail = (what, key, msg, extra) => { if (failOnce === what) { failOnce = null; throw E(key, msg, 400, extra); } };

  window.MockApi = {
    async call(method, path, body) {
      await sl(200); tick();
      switch (method + " " + path) {
        case "GET live/state":
          if (offStart && Date.now() - offStart > (fast ? 900 : 9000)) throw new ApiError("api.unreachable", "", null, 0);
          return JSON.parse(JSON.stringify(W));
        case "GET live/wifi/networks": await sl(900); return { networks: [
          { ssid: "Home-Fiber-5G", security: "psk", signal: 4, connected: false }, { ssid: "Neighbor-2.4G", security: "psk", signal: 3, connected: false },
          { ssid: "Neighbour_2.4", security: "psk", signal: 2, connected: false }, { ssid: "Cafe Guest", security: "open", signal: 1, connected: false },
          { ssid: "Office-Login", security: "8021x", signal: 1, connected: false }] };
        case "POST live/wifi/connect":
          if (body.password && body.password.length < 8) throw E("wifi.password_length", "A Wi-Fi password has 8 to 63 characters.");
          W.network.job = { id: "wifi.connect", state: "running", step: "joining", percent: null, error: null, t: Date.now(), bad: failOnce === "wifi" }; failOnce = failOnce === "wifi" ? null : failOnce;
          return { job: W.network.job };
        case "GET live/archive/candidates": await sl(400);
          if (Date.now() < plugAt) return { candidates: [] };
          return { candidates: (preset === "where-blocked" ? [] : [
          { partuuid: "p1", name: "sdb1", label: "Backup disk", fs: "exfat", disk: "sdb", disk_id: "sdb-11", disk_model: "WD Elements 2TB", size_bytes: 2000 * GB, free_bytes: 1800 * GB, usable: true, reason: null, recommended: true, default_size_bytes: 1780 * GB, min_size_bytes: 1 * GB, max_size_bytes: 1780 * GB, has_archive: false,
            repair: true, note: "This disk was not removed safely last time. ALPArchive will repair it first.", can_prepare: false },
          prepared ? { partuuid: "p2n", name: "sdc1", label: "ALPArchive", fs: "exfat", disk: "sdc", disk_id: "sdc-12", disk_model: "SanDisk Ultra", size_bytes: 128 * GB, free_bytes: 127 * GB, usable: true, reason: null, code: null, recommended: false, default_size_bytes: 126 * GB, min_size_bytes: 1 * GB, max_size_bytes: 126 * GB, has_archive: false, repair: false, note: null, can_prepare: false }
            : { partuuid: "p2", name: "sdc1", label: "Photos USB", fs: "vfat", disk: "sdc", disk_id: "sdc-12", disk_model: "SanDisk Ultra", size_bytes: 128 * GB, free_bytes: 0, usable: false, code: "ALP-STO-101", reason: "FAT32 can't hold the archive. Use a disk formatted as exFAT, NTFS or ext4, or another disk.", recommended: false, default_size_bytes: 0, min_size_bytes: 1 * GB, max_size_bytes: 0, has_archive: false, repair: false, note: null, can_prepare: true },
          { partuuid: "p4", name: "sde1", label: "Old backup disk", fs: "exfat", disk: "sde", disk_id: "sde-14", disk_model: "Seagate Portable", size_bytes: 1000 * GB, free_bytes: 0, usable: false, reason: "This disk already holds an ALPArchive.", code: null, recommended: false, default_size_bytes: 0, min_size_bytes: 0, max_size_bytes: 0, has_archive: true },
          { partuuid: "p3", name: "sdd2", label: "Windows", fs: "ntfs", disk: "sdd", disk_id: "sdd-13", disk_model: "Old laptop disk", size_bytes: 500 * GB, free_bytes: 0, usable: false, reason: "This disk has an operating system on it, so ALPArchive will not use it. Plug in a disk that is only for files.", code: "ALP-STO-102", recommended: false, default_size_bytes: 0, min_size_bytes: 0, max_size_bytes: 0, has_archive: false }]).concat([
          { partuuid: "p5", name: "sdf1", label: "Camera card", fs: "exfat", disk: "sdf", disk_id: "sdf-15", disk_model: "Generic card reader", size_bytes: 64 * GB, free_bytes: 0, usable: false, reason: "There is not enough free room on this disk (0.4 GB free, at least 20 GB needed). Free some space or plug in a bigger disk.", code: "ALP-STO-100", recommended: false, default_size_bytes: 0, min_size_bytes: 0, max_size_bytes: 0, has_archive: false }]) };
        case "POST live/disks/prepare": await sl(300);
          if (!pwOk(body.password)) throw E("password.wrong", "That password is not right. Please try again.", 403);
          if (body.disk_id !== "sdc-12" || prepared) throw E("disk.not_found", "This disk is not plugged in any more. Look at the list of disks again.", 404);
          if (JSON.stringify(body.partuuids) !== JSON.stringify(["p2"])) throw E("disk.changed", "What is on this disk changed. Look at the list of disks again.", 409);
          fail("prepare", "disk.refused", "This disk cannot be prepared here.");
          prepStart = Date.now(); W.archive.job = { id: "disk.prepare", state: "running", step: "erasing", percent: 30, message: "Erasing the disk…", error: null };
          return { job: W.archive.job };
        case "POST live/archive/create":
          fail("archive", "archive.refused", "ALPArchive cannot write there (it is full or locked by Windows). Nothing was changed. Please pick another place.");
          if (body.serious && !body.password) throw E("request.json", "The request misses \"password\".");
          W.archive.serious = !!body.serious;
          W.archive.state = "creating"; jobStart = Date.now(); W.archive.job = { id: "archive.create", state: "running", step: "preparing", percent: 5, error: null };
          return { job: W.archive.job };
        case "POST live/archive/cancel": await sl(300);
          if (fmtJob) { fmtJob.cancel = true; tick(); return { job: W.archive.job }; }
          if (W.archive.state !== "creating") throw E("archive.not_creating", "No archive is being made.", 409);
          W.archive = Object.assign(base().archive, { state: "none" }); jobStart = 0; return {};
        case "POST live/reset": await sl(1500);
          if (!pwOk(body.password)) throw E("password.wrong", "That password is not right. Please try again.", 403);
          if (failOnce === "reset") { failOnce = null; throw E("reset.busy", "ALPArchive is busy right now. Please try again in a minute.", 409); }
          jobStart = 0; pairStart = 0; pw = null; resetAt = Date.now(); W.console.state = "locked"; tick(); return { job: W.archive.job };   // 2026-11-03: 202 job "reset"
        case "POST live/archive/encrypt":
          W.archive.job = { id: "archive.encrypt", state: "running", step: "encrypting", message: "Locking the stick with your password…", percent: 10 };
          setTimeout(() => { W.archive.job = { id: "archive.encrypt", state: "done", step: "done", message: "Everything is encrypted now.", percent: 100 }; W.lock.protection = "password"; }, fast ? 1500 : 8000);
          return { job: W.archive.job };
        case "POST live/recovery/check": await sl(800); return { opens: (body.recovery_key || "").trim().split(/\s+/).length === 15 };
        case "POST live/pair/open":
          W.pairing = W.pairing || { pending: [], devices: [] }; W.pairing.window_seconds = 300;
          setTimeout(() => { if (W.pairing.window_seconds) W.pairing.pending = [{ id: "r1", name: "Pixel 8", model: "Pixel 8", os: "Android", code: "4821" }]; }, fast ? 800 : 6000);
          return { uri: "alparchive://pair?v=1&id=abc&ca=00&t=tok&a=192.168.1.23&p=443&n=ALPArchive&exp=0", qr_svg: qr(9), expires_in: 300 };
        case "POST live/pair/close": if (W.pairing) W.pairing.window_seconds = null; return {};
        case "POST live/pair/approve":
          W.pairing.pending = []; W.pairing.window_seconds = null; W.pairing.devices.push({ serial: "1A2B", name: "Pixel 8", model: "Pixel 8", os: "Android", added: "2026-10-30T06:00:00Z", revoked: false }); return { device: {} };
        case "POST live/pair/reject": W.pairing.pending = []; return {};
        case "POST live/pair/revoke": W.pairing.devices = W.pairing.devices.filter((d) => d.serial !== body.serial); return {};
        case "POST live/pair/strict": W.pairing.strict = !!body.on; return {};
        case "POST live/phone/devices/remove":
          W.phone.devices = (W.phone.devices || []).filter((d) => d.id !== body.id); W.phone.phones = W.phone.devices.length; W.phone.paired = W.phone.phones > 0; return {};
        case "POST live/phone/pair": {   // 2026-10-26: always 200 with the addresses; ready says whether a phone can sign in yet
          if (body.password !== undefined && !pwOk(body.password)) throw E("password.wrong", "That password is not right. Please try again.", 403);
          if (body.password) needPw = false;
          if (failOnce === "needpw" && W.immich.state === "ready") { failOnce = null; needPw = true; }
          const urls = W.network.addresses.map((a) => a.url), has = urls.length > 0, rdy = W.immich.state === "ready" && !needPw;
          if (rdy && !pairStart) pairStart = Date.now();
          if (has) { W.phone.server_url = urls[0]; W.phone.account = "family@alparchive.local"; }
          return { ready: rdy, needs_password: W.immich.state === "ready" && needPw, server_url: has ? urls[0] : null, addresses: urls, email: "family@alparchive.local", password: null, uses_main_password: true,
            immich: { state: W.immich.state, reason: !has ? "This computer is not on your home network. Connect a cable or Wi-Fi." : W.immich.state === "starting" ? "The photo service is still starting. " + (W.immich.detail || "") : null },
            qr_svg: has ? qr(7) : null, qr_text: has ? urls[0] : null, certificate_sha256: "AB:CD:12:34:56:78:9A:BC:DE:F0:11:22:33:44:55:66", certificate_url: has ? "http://192.168.1.23/" : null, certificate_qr_svg: has ? qr(55) : null }; }
        case "POST live/password/setup": await sl(2500);
          if (body.protection !== "none" && body.password.length < 8) throw E("password.too_short", "The password needs at least 8 characters.");
          pw = body.protection === "none" ? "" : body.password; W.lock = { state: "open", retry_after_seconds: 0, tpm: body.protection ? "none" : "on", protection: body.protection || "password" }; return {};
        case "POST live/console/signin": await sl(300);
          if (failOnce === "wait") { failOnce = null; throw E("password.wait", "Too many tries.", 429, { retry_after_seconds: 30 }); }
          if (!pwOk(body.password)) throw E("password.wrong", "That password is not right. Please try again.", 403);
          W.console.state = "open"; return {};
        case "POST live/console/lock": W.console.state = "locked"; return {};
        case "POST live/console/activity": return { idle_lock_seconds: W.console.idle_lock_seconds };
        case "POST live/recovery/create": await sl(300);
          if (FEATURES.includes("recovery_commit")) {   // T43: password in the body, replace when one exists, two phases
            if (!pwOk(body.password)) throw E("password.wrong", "That password is not right. Please try again.", 403);
            if (W.archive.recovery === "set" && body.replace !== true) throw E("recovery.exists", "This archive already has a recovery code.", 409);
          }
          if (W.archive.serious && !(body.passphrase && body.passphrase.length >= 8)) throw E("recovery.passphrase", "The extra passphrase needs 8 to 128 characters.");
          if (!FEATURES.includes("recovery_commit")) { W.archive.recovery = "set"; W.archive.recovery_made = new Date().toISOString(); }
          pendingRec = [2, 7, 13];
          return { words: RWORDS.slice(), recovery_key: RWORDS.join(" "), qr_text: "alparchive-recovery:1:" + RWORDS.join("-"), qr_svg: qr(321), confirm: FEATURES.includes("recovery_commit") ? pendingRec : undefined };
        case "POST live/recovery/commit": await sl(300);
          if (!pendingRec) throw E("recovery.no_pending", "There is no new code to keep. Make one again.", 409);
          for (const n of pendingRec) if ((body.words || {})[String(n)] !== RWORDS[n - 1]) throw E("recovery.confirm_wrong", "Word " + n + " is not right. Look at your words again.", 400);
          pendingRec = null; W.archive.recovery = "set"; W.archive.recovery_made = new Date().toISOString(); return {};
        case "POST live/recovery/discard": pendingRec = null; return {};
        case "POST live/recovery/cancel": pendingRec = null; return {};
        case "POST live/setup/finish": W.setup = Object.assign(W.setup || {}, { finished: true }); return {};
        case "POST live/setup/progress":
          if (body.skipped) W.setup.skipped = body.skipped; if (body.complete) W.setup.complete = true; return { setup: W.setup };
        case "POST live/password/change": await sl(800);
          if (!pwOk(body.password)) throw E("password.wrong", "That password is not right. Please try again.", 403);
          pw = body.new_password; return {};
        case "POST live/prefs":   // a partial update: only the fields sent change
          if ("natural_scrolling" in body) W.prefs.natural_scrolling = !!body.natural_scrolling;
          if ("ui_scale" in body) W.prefs.ui_scale = body.ui_scale;
          if ("idle_lock_seconds" in body) W.console.idle_lock_seconds = body.idle_lock_seconds;
          return { prefs: W.prefs };
        case "GET live/display/modes": await sl(200); return { modes: MODES };
        case "POST live/display/mode": MODES.forEach((m) => (m.current = m.id === body.id)); return { revert_in: 15 };
        case "POST live/display/mode/keep": return {};
        case "POST live/display/mode/revert": return {};
        case "GET live/disks": await sl(300); return { disks: mockDisks() };
        case "POST live/disks/eject": await sl(300); ejected.add(body.disk_id); return {};
        case "POST live/disks/check": await sl(200); W.archive.job = { id: "disk.check", state: "done", percent: 100, message: "No errors found on this disk." }; return { job: W.archive.job };
        case "POST live/report/save": await sl(500); return { file: "alp-report-20261103.txt" };
        case "POST live/disks/format/estimate": await sl(400);
          if (body.disk_id !== "sdc-12") throw E("disk.refused", "This disk is part of this computer or of ALPArchive, so it cannot be formatted here.", 422, {});
          return { size_bytes: 128 * GB, warnings: ["windows"], confirm: "c0ffee", files: 1276, bytes_used: 23.4 * GB, photos_not_in_archive: p.get("notinarchive") ? 37 : 0, quick: { text: "Fast. Old files can still be recovered with special tools.", seconds: 20 },
            secure: { text: "Overwrites the whole disk so old files are very hard to recover. About 28 minutes.", seconds: 1680, about: true, bytes_per_second: 76e6 } };
        case "POST live/disks/format": await sl(300);
          if (!pwOk(body.password)) throw E("password.wrong", "That password is not right. Please try again.", 403);
          if (FEATURES.includes("disks") && body.confirm !== "c0ffee") throw E("format.confirm_stale", "The disk changed. Look at it again.", 409);
          fmtJob = { t: Date.now(), mode: body.mode }; tick(); return { job: W.archive.job };
        case "POST live/media/browse": await sl(300);
          if (!body.folder) return { entries: [{ id: "f1", kind: "folder", name: "DCIM", count: 1276 }, { id: "f2", kind: "folder", name: "<img src=x onerror=alert(1)>", count: 3 }, { id: "a1", kind: "photo", name: "IMG_0001.JPG", size_bytes: 4.2e6 }], known: 112, truncated: false };
          return { entries: Array.from({ length: 12 }, (_, i) => ({ id: "p" + i, kind: i % 5 ? "photo" : "video", name: "IMG_" + String(1000 + i) + (i % 5 ? ".JPG" : ".MP4"), size_bytes: 4.1e6 })), known: 3, truncated: true };
        case "POST live/import": impAt = Date.now(); tick(); return { job: W.import_job };
        case "POST live/media/eject": W.media = W.media.filter((m) => m.id !== body.media); return {};
        case "POST live/terminal/start": await sl(200); if (p.get("term") === "nopw") throw E("terminal.no_password", "Set an ALP password first.", 409); if (!pwOk(body.password)) throw E("password.wrong", "That password is not right. Please try again.", 403);
          W.terminal = { state: "running" }; return { url: (p.get("term") !== "nopw" && p.get("term")) || "http://127.0.0.1:8413/3f9c2a1be04d7781a0b1c2d3e4f50617/" };
        case "POST live/terminal/stop": W.terminal = { state: "off" }; return {};
        case "POST live/media/open": await sl(200); { const m = W.media.find((x) => x.id === body.media); if (!m) throw E("media.not_found", "This disk is not plugged in any more.", 404); if (m.refused) throw E("media.refused", m.refused, 422); m.mounted = true; return {}; }
        case "POST live/media/list": await sl(250); {
          const at = (body.path || []).join("/"), off = body.offset || 0;
          if (at === "") return { entries: [{ name: "DCIM", dir: true, size_bytes: 0 }, { name: "<img src=x onerror=alert(1)>", dir: true, size_bytes: 0 }, { name: "readme.txt", dir: false, size_bytes: 120 },
            { name: "notes <script>.txt", dir: false, size_bytes: 300000 }, { name: "setup.exe", dir: false, size_bytes: 5e6 }], truncated: false, skipped: 1 };
          if (at === "DCIM") { const all = Array.from({ length: 30 }, (_, i) => ({ name: "IMG_" + String(1000 + i) + (i % 7 ? ".JPG" : ".MP4"), dir: false, size_bytes: 4.1e6 }));
            const page = all.slice(off, off + 20); return { entries: page, truncated: off + page.length < all.length, skipped: 0 }; }
          return { entries: [], truncated: false, skipped: 0 }; }
        case "POST live/archive/recover": fail("archive", "recovery.wrong", "This recovery key does not open this archive.");
          { const tw = String(body.recovery_key || "").trim().toLowerCase().split(/[\s,-]+/).filter(Boolean), n = tw.length;
            const old = n === 8 && tw.every((x) => /^[cbdefghijklnrtuv]{8}$/.test(x));   // 64-letter key of an archive made before
            if (n === 8 && !old) throw E("recovery.chain_code", "This is a chain code (8 words).");
            if (n !== 15 && !old) throw E("recovery.word_count", "A recovery code has 15 words."); }
          W.archive.state = "creating"; jobStart = Date.now(); return { job: { id: "archive.recover", state: "running", step: "checking", percent: 10, error: null } };
        case "POST live/gallery/delete": if (!pwOk(body.password)) throw E("password.wrong", "That password is not right. Please try again.", 403); return { deleted: (body.ids || []).length };
        case "POST live/gallery/trash/empty": if (!pwOk(body.password)) throw E("password.wrong", "That password is not right. Please try again.", 403); return {};
        case "GET live/chain/code": if (W.chain.state === "none") throw E("chain.none", "", 404);
          return { chain_id: "3f9c2a1be04d7781", words: WORDS, qr_text: "alparchive-chain:2:" + WORDS.join("-"), qr_svg: qr(99) };
        case "POST live/chain/leave": W.chain = { state: "none", chain_id: null, role: null }; return {};
        case "POST live/unlock": await sl(800);
          if (failOnce === "unlock") { failOnce = null; throw E("password.wrong", "That password did not open your archive. Please try again."); }
          if (failOnce === "wait") { failOnce = null; throw E("password.wait", "Too many tries.", 429, { retry_after_seconds: 30 }); }
          if (!pwOk(body.password)) throw E("password.wrong", "That password did not open your archive. Please try again.", 403);
          W.console.state = "open"; W.lock = { state: "open", retry_after_seconds: 0 }; open(); return {};
        case "POST live/archive/forget": W.archive = { state: "none", space: "ok", place: null, size_bytes: 0, free_bytes: 0, job: null, error: null }; return {};
        case "POST live/phone/reset": throw E("phone.reset_gone", "Phones now use your ALPArchive password.", 410);
        case "POST live/chain/create": fail("chain", "chain.exists", "This computer already has a chain."); W.chain = { state: "member", chain_id: "3f9c2a1be04d7781", role: "created" };
          return { chain_id: "3f9c2a1be04d7781", words: WORDS, qr_text: "alparchive-chain:2:" + WORDS.join("-"), qr_svg: qr(99) };
        case "POST live/chain/join": await sl(1500);
          fail("chain", "chain.word_unknown", "One of the words is not in the list.", { index: 7 });
          { const n = body.words.filter(Boolean).length;
            if (n === 15) throw E("chain.recovery_code", "This is a recovery code (15 words).");
            if (n !== 8 && n !== 24) throw E("chain.word_count", "A chain code has 8 words."); }
          W.chain = { state: "member", chain_id: "3f9c2a1be04d7781", role: "joined" }; return { chain_id: "3f9c2a1be04d7781" };
        case "POST live/power/off":
          if (failOnce === "off") { failOnce = null; throw E("archive.busy", "ALPArchive is busy right now. Please try again in a minute.", 409); }
          if (!offStart) { offStart = Date.now(); W.power.state = "stopping"; tick(); }
          return { job: W.power.job };
        default: throw E("request.not_found", "", 404);
      }
    },
    // Files preview (stage F): a tiny image as a data: URL (img-src allows data:), text as text.
    fileUrl: () => "data:image/svg+xml;charset=utf-8," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="#8ab"/><circle cx="240" cy="60" r="28" fill="#fd6"/></svg>'),
    fileText: async (media, path) => { await sl(150); return path[path.length - 1] === "readme.txt" ? "Hello <b>from</b> the camera card.\n<script>alert(1)</script>" : "x".repeat(1000); },
  };
})();
