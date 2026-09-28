// End-to-end test with a real Chromium and the real CrowDM extension:
// clicking a download link on a web page must open CrowDM's dialog, and the
// browser download must be handed over (or given back) correctly.
//
// Uses Playwright's Chromium in a throwaway profile, so the user's own
// browsers are untouched. Needs the release build and the test server.
//
// Usage: node browser-capture.mjs [--shots <dir>]

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { chromium } from "playwright-core";
import { DatabaseSync } from "node:sqlite";

const root = resolve(import.meta.dirname, "..", "..");
const bin = join(root, "target", "release");
const extDir = join(root, "extensions", "build", "chromium");
const dataDir = join(root, "target", "e2e-data");
const dlDir = join(root, "target", "e2e-downloads");
const shots = process.argv.includes("--shots") ? process.argv[process.argv.indexOf("--shots") + 1] : null;
const SERVER = "http://127.0.0.1:8765";
const APP_CDP = 9333;

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? `: ${detail}` : ""}`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function killApp() {
  for (const im of ["CrowDM.exe", "crowdm-host.exe"]) {
    try {
      execFileSync("taskkill", ["/IM", im, "/F"], { stdio: "ignore" });
    } catch {}
  }
}

async function pageFor(browser, pred, timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    for (const ctx of browser.contexts()) for (const p of ctx.pages()) if (pred(p.url())) return p;
    await sleep(100);
  }
  throw new Error("page not found");
}

async function connectApp(timeout = 25000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try {
      return await chromium.connectOverCDP(`http://127.0.0.1:${APP_CDP}`);
    } catch {
      await sleep(200);
    }
  }
  throw new Error("could not attach to the CrowDM WebView");
}

async function main() {
  killApp();
  rmSync(dataDir, { recursive: true, force: true });
  rmSync(dlDir, { recursive: true, force: true });
  mkdirSync(dataDir, { recursive: true });
  const env = { ...process.env, CROWDM_DATA_DIR: dataDir, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${APP_CDP}` };

  // Start the app ourselves (so it gets the debug port) and point downloads
  // at a scratch folder. It registers the native host at startup.
  const db = new DatabaseSync(join(dataDir, "crowdm.sqlite"));
  db.exec("CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
  db.prepare("INSERT INTO settings (key, value) VALUES ('settings', ?)").run(JSON.stringify({ downloadRoot: dlDir }));
  db.close();
  spawn(join(bin, "CrowDM.exe"), ["--background"], { env, detached: true, stdio: "ignore" }).unref();

  const profile = mkdtempSync(join(tmpdir(), "crowdm-chromium-"));
  const browserDownloads = join(profile, "downloads");
  // Chrome for Testing from Playwright's cache (override with CROWDM_CHROMIUM).
  const exe = process.env.CROWDM_CHROMIUM ?? join(process.env.LOCALAPPDATA, "ms-playwright", "chromium-1223", "chrome-win64", "chrome.exe");
  const ctx = await chromium.launchPersistentContext(profile, {
    executablePath: exe,
    // The browser starts the native host, which may start CrowDM: keep the
    // test's data folder and debug port for that copy too.
    env,
    headless: true,
    acceptDownloads: true,
    downloadsPath: browserDownloads,
    args: [`--disable-extensions-except=${extDir}`, `--load-extension=${extDir}`],
  });
  let [worker] = ctx.serviceWorkers();
  if (!worker) worker = await ctx.waitForEvent("serviceworker", { timeout: 15000 });
  check("extension loads in Chromium", worker.url().includes("pomhhnfijfeecghaflldjibhaofbiabm"), worker.url());

  const page = await ctx.newPage();
  await page.goto(`${SERVER}/page`);

  // ---- click a normal download link ----
  const t0 = Date.now();
  await page.click("#big");
  // No window exists until the first capture; attach once it does.
  const app = await connectApp();
  const dialog = await pageFor(app, (u) => u.includes("#capture"));
  await dialog.waitForFunction(() => document.querySelector(".capture-head .title")?.textContent?.includes("big.bin"), null, { timeout: 20000 });
  check("clicking a download link opens the CrowDM dialog", true, `${Date.now() - t0} ms from click (first click also starts the native host)`);
  const facts = await dialog.textContent(".capture-head .facts");
  check("dialog knows the browser is waiting", /Paused in Chrome/.test(facts), facts.trim());
  const state1 = await worker.evaluate(async () => (await chrome.downloads.search({})).map((d) => ({ id: d.id, state: d.state, paused: d.paused })));
  check("browser download is paused while the user decides", state1.length === 1 && state1[0].paused === true, JSON.stringify(state1));
  if (shots) await dialog.screenshot({ path: join(shots, "capture-from-chromium.png") });

  await dialog.click("text=Start download");
  await sleep(1200);
  const state2 = await worker.evaluate(async () => (await chrome.downloads.search({})).map((d) => ({ id: d.id, state: d.state })));
  check("browser download is cancelled and removed after CrowDM takes it", state2.length === 0, JSON.stringify(state2));
  const target = join(dlDir, "Other", "big.bin");
  const end = Date.now() + 60000;
  while (Date.now() < end && !(existsSync(target) && statSync(target).size === 268435456)) await sleep(250);
  check("CrowDM downloads the file", existsSync(target) && statSync(target).size === 268435456, target);

  // ---- warm click: the second capture is faster ----
  const t1 = Date.now();
  await page.click("#small");
  await dialog.waitForFunction(() => document.querySelector(".capture-head .title")?.textContent?.includes("small.bin"), null, { timeout: 10000 });
  check("second capture opens the dialog", true, `${Date.now() - t1} ms from click`);
  // ---- hand it back to the browser ----
  const dlPromise = page.waitForEvent("download", { timeout: 1000 }).catch(() => null);
  await dialog.click("text=Download in Chrome");
  await dlPromise;
  let back = null;
  const end2 = Date.now() + 15000;
  while (Date.now() < end2) {
    back = await worker.evaluate(async () => (await chrome.downloads.search({ urlRegex: "small" })).map((d) => ({ state: d.state, bytes: d.bytesReceived })));
    if (back.length && back[0].state === "complete") break;
    await sleep(250);
  }
  check('"Download in Chrome" lets the browser finish it', back?.[0]?.state === "complete" && back[0].bytes === 1048576, JSON.stringify(back));

  // ---- exclusions keep downloads in the browser ----
  await dialog.evaluate(async () => {
    const { invoke } = window.__TAURI_INTERNALS__;
    const s = await invoke("get_settings");
    s.interception.excludedExtensions = ["weirdext"];
    await invoke("save_settings", { settings: s });
  });
  await sleep(300);
  await page.click("#odd");
  await sleep(2500);
  const caps = await dialog.evaluate(() => window.__TAURI_INTERNALS__.invoke("capture_list"));
  const odd = await worker.evaluate(async () => (await chrome.downloads.search({ urlRegex: "weirdext" })).map((d) => d.state));
  check("excluded file types go back to the browser", caps.length === 0 && odd[0] === "complete", JSON.stringify(odd));

  // ---- the video button ----
  const box = await page.locator("#v").boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await sleep(400);
  const btnVisible = await page.evaluate(() => {
    const h = document.querySelector("crowdm-video-button");
    return !!h && h.style.display === "block";
  });
  check("Download video button appears over a file-backed video", btnVisible);
  if (shots) await page.screenshot({ path: join(shots, "video-button.png"), clip: { x: box.x - 10, y: box.y - 10, width: box.width + 20, height: box.height + 20 } });
  // The button sits inside the video's top-right corner.
  const bx = await page.evaluate(() => document.querySelector("crowdm-video-button").getBoundingClientRect().toJSON());
  await page.mouse.click(bx.x + bx.width / 2, bx.y + bx.height / 2);
  await dialog.waitForFunction(() => document.querySelector(".capture-head .title")?.textContent?.includes("clip.mp4"), null, { timeout: 10000 });
  check("video button sends the video to CrowDM", true);
  await dialog.click("text=Cancel");

  // ---- app unavailable: the browser keeps the download ----
  killApp();
  await sleep(1000);
  const t2 = Date.now();
  await page.click("#small");
  const app2 = await connectApp(30000).catch(() => null);
  if (app2) {
    const dlg2 = await pageFor(app2, (u) => u.includes("#capture"));
    await dlg2.waitForFunction(() => document.querySelector(".capture-head .title")?.textContent?.length > 0, null, { timeout: 20000 });
    check("a click while CrowDM is closed starts it and shows the dialog", true, `${Date.now() - t2} ms from click`);
    await dlg2.click("text=Cancel");
    await app2.close();
  } else {
    check("a click while CrowDM is closed starts it and shows the dialog", false);
  }

  const within = (p) => Promise.race([p.catch(() => {}), sleep(3000)]);
  await within(ctx.close());
  await within(app.close());
  killApp();
  rmSync(profile, { recursive: true, force: true });
  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
  process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  killApp();
  process.exit(1);
});
