#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// T-UI-SMOKE: the Live console over its mock API (ui/live/mock.js) in headless Chromium.
// docs/plans/live-ui-0.2.4.md. Checks, for every screen preset and the 0.2.4 flows:
//   - the screen draws (the card has content) and no script error happens;
//   - no browser dialog opens (alert, confirm, prompt, print, file picker);
//   - names from external disks are text only (an <img onerror> label makes no element);
//   - the recovery code: never made on load; Replace = password, warning, slide, 3 words;
//   - settings: all nine categories; format, import, reset and the (i) panel.
// Screenshots go to $ALP_QA_OUT (default out/qa/ui). Exit 1 on the first failure list.
// Usage: node tests/ui/smoke.mjs   (needs the playwright package and its Chromium)
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
let chromium;
try { ({ chromium } = require("playwright")); } catch { ({ chromium } = await import("playwright")); }

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../../ui/live");
const out = process.env.ALP_QA_OUT || path.resolve(here, "../../out/qa/ui");
fs.mkdirSync(out, { recursive: true });
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml" };
const server = http.createServer((req, res) => {
  const u = new URL(req.url, "http://x");
  const f = path.join(root, path.normalize(decodeURIComponent(u.pathname)).replace(/^([/\\])+/, "") || "index.html");
  if (!f.startsWith(root) || !fs.existsSync(f) || fs.statSync(f).isDirectory()) { res.writeHead(404); return res.end(); }
  res.writeHead(200, { "Content-Type": TYPES[path.extname(f)] || "application/octet-stream" }); fs.createReadStream(f).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/index.html?mock=1&fast=1`;

const browser = await chromium.launch();
const failures = [];
const fail = (what) => { failures.push(what); console.log("FAIL: " + what); };
const ok = (what) => console.log("ok:   " + what);

async function open(query, size) {
  const page = await browser.newPage({ viewport: size || { width: 1366, height: 768 }, reducedMotion: "reduce" });
  page.errors = [];
  page.on("pageerror", (e) => page.errors.push(String(e)));
  page.on("console", (m) => { if (m.type() === "error" && !/404|Failed to load resource/.test(m.text())) page.errors.push(m.text()); });
  page.on("dialog", async (d) => { page.errors.push("browser dialog: " + d.type()); await d.dismiss(); });
  page.on("filechooser", () => page.errors.push("browser file picker opened"));
  await page.goto(base + query);
  await page.waitForFunction(() => document.body.dataset.view && document.body.dataset.view !== "starting", null, { timeout: 15000 });
  await page.waitForTimeout(400);
  return page;
}
async function done(page, name) {
  await page.screenshot({ path: path.join(out, name + ".png") });
  if (page.errors.length) fail(name + ": " + page.errors.join(" | ")); else ok(name);
  await page.close();
}
const view = (page) => page.evaluate(() => document.body.dataset.view);
const text = (page) => page.evaluate(() => document.getElementById("card").innerText);
async function slide(page) {   // the keyboard path of the slider: End confirms
  await page.locator(".sl-knob").focus(); await page.keyboard.press("End");
}
async function password(page, pw) {
  await page.waitForSelector("#mpw"); await page.fill("#mpw", pw || "demo-password"); await page.press("#mpw", "Enter");
}

// 1. every preset draws
const presets = ["welcome", "welcome-fail", "password", "network", "network-up", "where", "where-empty", "where-blocked", "progress", "phone", "recovery", "done",
  "unlock", "unlock-wait", "lock", "lock-warn", "home", "home-warn", "home-full", "home-starting", "home-missing", "home-empty", "settings", "settings-chain", "chain-seed", "chain-join", "confirm", "disks", "admin", "files"];
for (const p of presets) {
  const page = await open("&screen=" + p);
  const n = await page.evaluate(() => document.getElementById("card").childElementCount);
  if (!n) page.errors.push("empty screen");
  await done(page, "preset-" + p);
}
// a first start with no preset begins setup (T09: the server's setup state decides)
{ const page = await open(""); if ((await view(page)) !== "welcome") page.errors.push("first start shows " + (await view(page))); await done(page, "first-start"); }
// the same screens on an older alpd (no 0.2.4 features)
for (const p of ["home", "settings", "recovery"]) { const page = await open("&features=none&screen=" + p); await done(page, "old-alpd-" + p); }

// 2. recovery code: the screen never makes a code by itself (T42)
{
  const page = await open("&screen=recovery");
  const t = await text(page);
  if (/island|jungle|kettle/.test(t)) page.errors.push("words shown without a tap");
  await page.click("text=Make it");
  await password(page);
  await page.waitForSelector(".words");
  if (await page.locator("text=Print this page").count()) page.errors.push("Print button still there");
  await page.check("#saved"); await page.click("button:has-text('Continue')");
  await page.waitForSelector("#cw2");
  await page.fill("#cw2", "jungle"); await page.fill("#cw7", "orchard"); await page.fill("#cw13", "umbrella");
  await page.click("button:has-text('Check')");
  await page.waitForSelector("text=Your recovery code is ready");
  if (await page.locator(".words").count()) page.errors.push("words still on screen after commit");
  await done(page, "flow-recovery-make");
}
// 3. Replace: password, warning, slide, words, 3 words (T43)
{
  const page = await open("&screen=settings");
  await page.click(".setcat:has-text('Password & Recovery')");
  await page.click("button:has-text('Replace')");
  await password(page);
  await page.waitForSelector("text=Replace your recovery code?");
  if (!(await page.locator(".dbox.lose").count())) page.errors.push("no 'what is lost' list");
  await page.screenshot({ path: path.join(out, "flow-recovery-replace-warning.png") });
  await slide(page);
  await page.waitForSelector(".words");
  await done(page, "flow-recovery-replace");
}
// 4. settings: all nine categories, Advanced asks for the password
{
  const page = await open("&screen=settings");
  for (const c of ["Phones", "Password & Recovery", "Backup Disk", "Family Chain", "Network", "Display & Input", "Power", "About"]) {
    await page.click(`.setcat:has-text('${c}')`); await page.waitForTimeout(250);
    await page.screenshot({ path: path.join(out, "settings-" + c.replace(/\W+/g, "-").toLowerCase() + ".png") });
  }
  if (!/Immich/.test(await text(page))) page.errors.push("About > Credits does not name the photo service");
  await page.click(".setcat:has-text('Advanced')"); await password(page); await page.waitForSelector("text=Delete everything and start over");
  await page.screenshot({ path: path.join(out, "settings-advanced.png") });
  if (/Read the specification/.test(await text(page))) page.errors.push("Read the specification is still there");
  await done(page, "settings-categories");
}
// 5. Immich is not named outside the phone app step and Credits
{
  const page = await open("&screen=home");
  if (/Immich/.test(await page.evaluate(() => document.body.innerText))) page.errors.push("home names Immich");
  await page.click(".errb"); await page.waitForSelector(".errpanel");
  await page.screenshot({ path: path.join(out, "errors-panel.png") });
  await done(page, "home-and-errors");
}
// 6. format (T58): the disk manager, name and type, secure erase under Technical details, password,
//    "What is removed" / "What stays" side by side, slide (with the confirm token), progress, "is ready" + Eject
{
  const page = await open("&screen=disks");
  await page.waitForSelector(".dm-item");
  // the computer's own disk: greyed, Offline, never selectable
  if (!(await page.locator(".dm-off .dm-item.off:has-text('Samsung SSD 980')").count())) page.errors.push("internal disk not shown as offline");
  if (await page.locator("button.dm-item:has-text('Samsung SSD 980')").count()) page.errors.push("internal disk is selectable");
  if (!(await page.locator(".dm-off:has-text('never touches it')").count())) page.errors.push("no explanation under the internal disk");
  if (await page.locator(".dm-item b").count()) page.errors.push("a disk model became HTML");
  // the archive disk: viewable, Format locked
  await page.click("button.dm-item:has-text('WD Elements')");
  await page.waitForSelector(".dm-locked:has-text('Start over')");
  if (!(await page.locator(".dm-rows:has-text('Healthy')").count())) page.errors.push("health row missing");
  await page.screenshot({ path: path.join(out, "disks-archive-locked.png") });
  await page.click("button.dm-item:has-text('Ultra')");
  await page.waitForSelector(".pbar .pt-free");
  await page.screenshot({ path: path.join(out, "disks-manager.png") });
  await page.click(".dm-acts button:has-text('Format')");
  await page.waitForSelector("text=Works everywhere (exFAT)");
  if (await page.locator("text=Secure format").isVisible()) page.errors.push("secure format not under Technical details");
  await page.click(".techd summary");
  if (!(await page.locator("text=Takes about 28 minutes on this disk").count())) page.errors.push("secure format time from the estimate missing");
  if (!(await page.locator("text=For an archive on Linux (ext4)").count())) page.errors.push("ext4 missing under Technical details");
  await page.screenshot({ path: path.join(out, "flow-format-choice.png") });
  await page.click("label.choice:has-text('Secure format')"); await page.click("button:has-text('Continue')");
  await password(page);
  await page.waitForSelector("text=A system that can start a computer");
  if (!(await page.locator(".dcols .dbox.lose:has-text('1,276 files')").count())) page.errors.push("files count missing in What is removed");
  if (!(await page.locator(".dcols .dbox.keep:has-text('This computer')").count())) page.errors.push("What stays column missing");
  await page.screenshot({ path: path.join(out, "flow-format-slide.png") });
  await slide(page);
  await page.waitForSelector("text=Formatting");
  await page.waitForSelector("text=Stop", { timeout: 5000 });
  await page.waitForSelector(".fmt-left:has-text('min left')", { timeout: 5000 });
  await page.screenshot({ path: path.join(out, "flow-format-progress.png") });
  await done(page, "flow-format");
}
// 6b. an older alpd (no disks feature): the list is built from the hardware report, Format still works
{
  const page = await open("&features=recovery_commit,events&screen=disks");
  await page.waitForSelector(".dm-item");
  if (!(await page.locator(".dm-off").count())) page.errors.push("old alpd: internal disk not offline");
  await done(page, "old-alpd-disks");
}
// 6c. T54 legend: each level has its own shape and word; critical shows the stop bar and pauses archive actions
for (const lv of ["note", "action", "now", "critical"]) {
  const page = await open("&screen=home&level=" + lv);
  const cls = await page.evaluate(() => document.querySelector(".homelv .lv").className);
  if (!cls.includes("lv-" + lv)) page.errors.push("home legend shows " + cls);
  if (lv === "critical") {
    if (!(await page.locator("#critbar:has-text('Stop')").count())) page.errors.push("no critical bar");
    if (await page.locator(".modal").count()) page.errors.push("a sheet opened by itself at critical");
    await page.screenshot({ path: path.join(out, "legend-critical.png") });
    await page.click("button:has-text('Import')");
    await page.waitForSelector("text=Importing is paused", { timeout: 3000 }).catch(() => page.errors.push("import not paused while critical"));
    // formatting another USB disk stays allowed at critical: it is the way to prepare a new archive disk (review U1)
    await page.goto(base + "&level=critical&screen=disks"); await page.waitForSelector("button.dm-item:has-text('Ultra')");
    await page.click("button.dm-item:has-text('Ultra')"); await page.click(".dm-acts button:has-text('Format')");
    await page.waitForSelector(".fmt-rescue", { timeout: 4000 }).catch(() => page.errors.push("format refused while critical"));
  }
  if (lv === "note" && (await page.evaluate(() => document.body.dataset.health)) !== "ok") page.errors.push("note turned home red");
  if (lv === "now" && (await page.evaluate(() => document.body.dataset.health)) !== "bad") page.errors.push("now did not turn home red");
  await done(page, lv === "critical" ? "legend-critical-format" : "legend-" + lv);
}
// A stand-in gallery: its own top bar and a scrolling grid, so a screenshot shows whether the frame fits.
const FAKE_GALLERY = "<html><meta charset='utf-8'><body style='margin:0;font:16px sans-serif;background:#fff'><div style='position:sticky;top:0;height:56px;display:flex;align-items:center;padding:0 16px;background:#f4f4f6;border-bottom:1px solid #ddd'>Gallery top bar · Search · Albums · People</div>" +
  "<div style='display:grid;grid-template-columns:repeat(auto-fill,minmax(160px,1fr));gap:4px;padding:4px'>" + Array.from({ length: 60 }, (_, i) => `<div style='aspect-ratio:1;background:hsl(${(i * 37) % 360} 45% 70%)'></div>`).join("") + "</div></body></html>";
// 6e. the tile behind a problem carries the legend mark (design review 1)
{
  const page = await open("&screen=home-warn");
  if (!(await page.locator(".tile.tile-lv:has-text('Room left') .lv-action").count())) page.errors.push("the problem tile has no legend mark");
  await done(page, "home-warn-tile");
}
// 6d. T56/T59: at every size step the gallery fills exactly the area under the top bar, with no page scroll
for (const sc of [1, 1.25, 1.5, 1.75, 2]) {   // without the feature prefs, so the size comes from this browser's own setting
  const page = await open("&features=setup_progress,recovery_commit,events,gallery_session&screen=home");
  await page.evaluate((v) => localStorage.setItem("alp.ui.prefs", JSON.stringify({ scale: v })), sc);
  await page.route("http://127.0.0.1:8412/**", (r) => r.fulfill({ body: FAKE_GALLERY, contentType: "text/html" }));
  await page.reload(); await page.waitForFunction(() => document.body.dataset.view === "home");
  await page.click("button:has-text('Photos')"); await page.waitForSelector("iframe.galframe"); await page.waitForTimeout(200);
  if ((await page.evaluate(() => document.documentElement.dataset.scale)) !== String(sc)) page.errors.push("size step " + sc + " not applied");
  const m = await page.evaluate(() => { const f = document.querySelector("iframe.galframe").getBoundingClientRect(), b = document.querySelector(".top").getBoundingClientRect(), e = document.documentElement;
    return { gap: Math.round(f.top - b.bottom), bottom: Math.round(f.bottom - innerHeight), w: Math.round(f.width - innerWidth), scroll: e.scrollHeight - e.clientHeight }; });
  if (Math.abs(m.gap) > 1 || Math.abs(m.bottom) > 1 || Math.abs(m.w) > 1 || m.scroll > 0) page.errors.push("gallery at " + sc + ": " + JSON.stringify(m));
  await done(page, "gallery-fit-" + sc);
}
// 7. import: the offer, the picker, names as text only, progress
{
  const page = await open("&screen=home&media=1");
  await page.waitForSelector("text=Photos found on Camera card", { timeout: 6000 });
  await page.click("button:has-text('Import them')");
  await page.waitForSelector(".imp-grid");
  if (await page.locator(".importer img").count()) page.errors.push("a folder name became HTML");
  await page.screenshot({ path: path.join(out, "flow-import-picker.png") });
  await page.click("button:has-text('Import everything here')");
  await page.waitForSelector("text=imported", { timeout: 8000 });
  await done(page, "flow-import");
}
// 8. reset: a job with progress, then setup again
{
  const page = await open("&screen=settings");
  await page.click(".setcat:has-text('Advanced')"); await password(page);
  await page.click("button:has-text('Delete everything')"); await password(page);
  for (let i = 0; i < 2; i++) { await page.waitForTimeout(400); await page.click("button:has-text('continue')"); }
  await page.waitForSelector(".sl-knob"); await slide(page);
  await page.waitForSelector("text=Your archive was erased", { timeout: 15000 });
  await done(page, "flow-reset");
}
// 9. Photos full screen; and the phone-sized layout
{
  const page = await open("&screen=home");
  await page.click("button:has-text('Photos')");
  await page.waitForSelector("iframe.galframe");
  if (await page.locator("text=Sign in with").count()) page.errors.push("old sign-in line shown");
  await done(page, "photos");
  const p2 = await open("&features=none&screen=home");
  await p2.click("button:has-text('Photos')");
  await p2.waitForSelector("text=Photos are coming to this screen");
  await done(p2, "photos-not-ready");
}
// 9b. security review U2: a gallery address on the console's own origin (or not http://127.0.0.1) is never framed
{
  for (const bad of [new URL(base).origin + "/x", "https://127.0.0.1:8412/", "http://example.com:8412/", "http://127.0.0.1:8413/", "javascript:alert(1)"]) {
    const page = await open("&screen=home&gallery=" + encodeURIComponent(bad));
    await page.click("button:has-text('Photos')");
    await page.waitForSelector("text=Photos are coming to this screen");
    if (await page.locator("iframe").count()) page.errors.push("framed a bad gallery address: " + bad);
    await done(page, "photos-bad-url-" + bad.replace(/\W+/g, "-").slice(0, 30));
  }
}
// 9c. security review U1: no Replace without the two-phase recovery server (an older alpd rotates at once)
{
  const page = await open("&features=setup_progress,events&screen=settings");
  await page.click(".setcat:has-text('Password & Recovery')"); await page.waitForTimeout(250);
  if (await page.locator("button:has-text('Replace')").count()) page.errors.push("Replace shown without recovery_commit");
  if (!(await page.locator("button:has-text('Check')").count())) page.errors.push("Check my code missing");
  await done(page, "recovery-no-replace-on-old-alpd");
}
// 11. Disks (T58): every disk of the computer; the stick and the archive are viewable with Format locked; the computer's
//     own disk is offline; only an unlocked external disk offers Format, Eject and Check for errors
for (const q of ["", "&features=recovery_commit,events"]) {
  const page = await open(q + "&screen=disks");
  await page.waitForSelector(".dm-item");
  for (const name of q ? ["Cruzer Blade"] : ["Cruzer Blade", "WD Elements"]) {   // an older alpd: the archive is known from the candidates only
    const row = page.locator("button.dm-item:has-text('" + name + "')");
    if (!(await row.count())) { page.errors.push("not viewable: " + name); continue; }
    await row.click();
    if (await page.locator(".dm-acts button:has-text('Format')").count()) page.errors.push("Format offered on " + name);
    if (!(await page.locator(".dm-locked:has-text('Start over')").count())) page.errors.push("no lock reason on " + name);
  }
  if (q && !(await page.locator(".dm-off .dm-item.off").count())) page.errors.push("old alpd: internal disk not offline");
  if (!q) await page.click("button.dm-item:has-text('Ultra')");
  if (!q && !(await page.locator(".dm-acts button:has-text('Format')").count())) page.errors.push("the external disk has no Format");
  if (!q) for (const b of ["Eject", "Check for errors"]) if (!(await page.locator(`.dm-acts button:has-text('${b}')`).count())) page.errors.push("no " + b);
  if (!q) { await page.click(".dm-acts button:has-text('Check for errors')"); await page.waitForSelector(".dm-check"); }
  await done(page, "disks" + (q ? "-old-alpd" : ""));
}
// 12. stage F Files: read only; names as text; paging; previews; refused disk; eject
{
  const page = await open("&screen=files");
  await page.waitForSelector(".flist");
  if (await page.locator(".importer img, .importer script").count()) page.errors.push("a name from the disk became HTML");
  if (!(await page.locator("text=<b>Old disk</b>").count())) page.errors.push("the disk label is not shown as plain text");
  if (!(await page.locator("text=1 names could not be shown").count())) page.errors.push("skipped names not reported");
  for (const w of ["Delete", "Rename", "Copy", "Move"]) if (await page.locator(`.importer button:has-text('${w}')`).count()) page.errors.push("a write action exists: " + w);
  await page.screenshot({ path: path.join(out, "stagef-files.png") });
  await page.click(".frow:has-text('DCIM')"); await page.waitForSelector(".frow:has-text('IMG_1019')");
  if ((await page.locator(".frow").count()) !== 20) page.errors.push("first page is not 20 entries");
  await page.click("button:has-text('Show more')"); await page.waitForSelector(".frow:has-text('IMG_1029')");
  if ((await page.locator(".frow").count()) !== 30) page.errors.push("paging did not add the rest");
  await page.click(".frow:has-text('IMG_1001.JPG')"); await page.waitForSelector(".fprev img");
  if (!/^data:image\//.test(await page.locator(".fprev img").getAttribute("src"))) page.errors.push("image preview src unexpected");
  await page.screenshot({ path: path.join(out, "stagef-files-photo.png") });
  await page.click("button:has-text('Back to the folder')"); await page.click(".crumbs button >> nth=0"); await page.waitForSelector(".frow:has-text('readme.txt')");
  await page.click(".frow:has-text('readme.txt')"); await page.waitForSelector("pre.ftext");
  if (!/<script>/.test(await page.locator("pre.ftext").innerText())) page.errors.push("text preview is not plain text");
  if (await page.locator(".fprev script, .fprev b").count()) page.errors.push("text preview made elements");
  await page.click("button:has-text('Back to the folder')"); await page.click(".frow:has-text('setup.exe')");
  await page.waitForSelector("text=This kind of file cannot be opened here");
  await page.click(".imp-d:has-text('Old disk')"); await page.waitForSelector("text=ALPArchive cannot read this kind of disk (btrfs).");
  await page.click(".imp-d:has-text('Camera card')"); await page.waitForSelector(".flist");
  await page.click("button:has-text('Eject')"); await page.waitForSelector("text=You can unplug Camera card now.");
  await done(page, "stagef-files-flow");
  const old = await open("&features=setup_progress&screen=settings");
  await old.click(".setcat:has-text('Advanced')"); await password(old); await old.waitForSelector("text=Delete everything and start over");
  if (await old.locator("text=Files on other disks").count()) old.errors.push("Files offered without media_files");
  await done(old, "stagef-files-hidden-on-old-alpd");
}
// 13. Terminal: password first, a frame only for a good one-time address, stop on leave, hidden without the feature
{
  const page = await open("&screen=settings");
  await page.click(".setcat:has-text('Advanced')"); await password(page); await page.waitForSelector("text=Delete everything and start over");
  await page.click(".setrow:has-text('Terminal') button"); await password(page);
  await page.waitForSelector("iframe.termframe");
  const src = await page.locator("iframe.termframe").getAttribute("src"), sb = await page.locator("iframe.termframe").getAttribute("sandbox");
  if (!/^http:\/\/127\.0\.0\.1:8413\//.test(src)) page.errors.push("terminal frame address: " + src);
  if (/allow-(popups|top-navigation|downloads|modals)/.test(sb)) page.errors.push("terminal sandbox too open: " + sb);
  if (!(await page.locator("text=Commands you run here are kept in the system log. The Terminal closes when the screen locks").count())) page.errors.push("no command log line");
  if (await page.evaluate(() => JSON.stringify(localStorage) + JSON.stringify(sessionStorage)).then((x) => /8413/.test(x))) page.errors.push("terminal address stored");
  await page.screenshot({ path: path.join(out, "stagef-terminal.png") });
  await page.click("[data-back], .bar .back >> nth=0").catch(() => {});
  await done(page, "stagef-terminal");
  for (const bad of [new URL(base).origin + "/x", "https://127.0.0.1:8413/abc/", "http://example.com:8413/", "http://127.0.0.1:8412/abc/"]) {
    const p2 = await open("&screen=settings&term=" + encodeURIComponent(bad));
    await p2.click(".setcat:has-text('Advanced')"); await password(p2); await p2.waitForSelector("text=Delete everything and start over");
    await p2.click(".setrow:has-text('Terminal') button"); await password(p2);
    await p2.waitForSelector("text=The Terminal could not open");
    if (await p2.locator("iframe").count()) p2.errors.push("framed a bad terminal address: " + bad);
    await done(p2, "stagef-terminal-bad-" + bad.replace(/\W+/g, "-").slice(0, 24));
  }
  const p4 = await open("&screen=settings&term=nopw");
  await p4.click(".setcat:has-text('Advanced')"); await password(p4); await p4.waitForSelector("text=Delete everything and start over");
  await p4.click(".setrow:has-text('Terminal') button"); await password(p4);
  await p4.waitForSelector("text=The Terminal needs an ALPArchive password");
  if (await p4.locator("iframe").count()) p4.errors.push("framed without a password");
  await done(p4, "stagef-terminal-no-password");
  const p3 = await open("&features=setup_progress&screen=settings");
  await p3.click(".setcat:has-text('Advanced')"); await password(p3); await p3.waitForSelector("text=Delete everything and start over");
  if (await p3.locator(".setrow:has-text('Terminal')").count()) p3.errors.push("Terminal offered without the feature");
  await done(p3, "stagef-terminal-hidden");
}
// 10. anti-browser: no context menu text selection outside fields
{
  const page = await open("&screen=home");
  const sel = await page.evaluate(() => getComputedStyle(document.body).userSelect);
  if (sel !== "none") page.errors.push("text can be selected: " + sel);
  await done(page, "no-browser-feel");
}

await browser.close(); server.close();
if (failures.length) { console.log(`FAIL: T-UI-SMOKE (${failures.length})`); process.exit(1); }
console.log("PASS: T-UI-SMOKE");
