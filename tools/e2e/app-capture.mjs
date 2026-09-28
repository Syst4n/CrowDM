// End-to-end test of the real CrowDM app: a fake browser talks to the real
// native messaging host, the app shows its capture dialog, and the dialog is
// driven through WebView2's DevTools protocol (no screen control needed).
//
// Prerequisites: release build (pnpm tauri build --no-bundle) and the test
// server running on 127.0.0.1:8765 (target/release/crowdm-testserver).
//
// Usage: node app-capture.mjs [--shots <dir>]

import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, rmSync, existsSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { chromium } from "playwright-core";

const root = resolve(import.meta.dirname, "..", "..");
const bin = join(root, "target", "release");
const shots = process.argv.includes("--shots") ? process.argv[process.argv.indexOf("--shots") + 1] : null;
const dataDir = join(root, "target", "e2e-data");
const dlDir = join(root, "target", "e2e-downloads");
const PORT = 9333;
const SERVER = "http://127.0.0.1:8765";

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

const env = {
  ...process.env,
  CROWDM_DATA_DIR: dataDir,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`,
};

/** A fake browser: spawns the native host and speaks its framed protocol. */
class FakeBrowser {
  constructor() {
    this.proc = spawn(join(bin, "crowdm-host.exe"), ["chrome-extension://pomhhnfijfeecghaflldjibhaofbiabm/"], { env, stdio: ["pipe", "pipe", "inherit"] });
    this.buf = Buffer.alloc(0);
    this.messages = [];
    this.waiters = [];
    this.proc.stdout.on("data", (d) => {
      this.buf = Buffer.concat([this.buf, d]);
      while (this.buf.length >= 4) {
        const n = this.buf.readUInt32LE(0);
        if (this.buf.length < 4 + n) break;
        const msg = JSON.parse(this.buf.subarray(4, 4 + n).toString("utf8"));
        this.buf = this.buf.subarray(4 + n);
        msg.at = performance.now();
        this.messages.push(msg);
        this.waiters = this.waiters.filter((w) => !w(msg));
      }
    });
  }
  send(msg) {
    const body = Buffer.from(JSON.stringify(msg), "utf8");
    const len = Buffer.alloc(4);
    len.writeUInt32LE(body.length);
    this.proc.stdin.write(Buffer.concat([len, body]));
  }
  waitFor(pred, timeout = 20000) {
    const found = this.messages.find(pred);
    if (found) return Promise.resolve(found);
    return new Promise((res, rej) => {
      const t = setTimeout(() => rej(new Error("timeout waiting for message")), timeout);
      this.waiters.push((m) => (pred(m) ? (clearTimeout(t), res(m), true) : false));
    });
  }
  close() {
    this.proc.kill();
  }
}

function capture(id, url, extra = {}) {
  return {
    type: "capture",
    id,
    browser: "chrome",
    capture: {
      kind: "intercepted",
      browserDownloadId: Number(id.replace(/\D/g, "")) || 1,
      url,
      finalUrl: null,
      referrer: `${SERVER}/page`,
      pageUrl: `${SERVER}/page`,
      pageTitle: "CrowDM test page",
      fileName: null,
      mime: "application/octet-stream",
      size: null,
      incognito: false,
      headers: [{ name: "User-Agent", value: "Mozilla/5.0 (e2e)" }],
      ...extra,
    },
  };
}

async function pageFor(browser, pred, timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    for (const ctx of browser.contexts()) for (const p of ctx.pages()) if (pred(p.url())) return p;
    await sleep(100);
  }
  throw new Error("page not found");
}

async function connectCdp(timeout = 20000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try {
      return await chromium.connectOverCDP(`http://127.0.0.1:${PORT}`);
    } catch {
      await sleep(200);
    }
  }
  throw new Error("could not attach to WebView2");
}

async function main() {
  killApp();
  rmSync(dataDir, { recursive: true, force: true });
  rmSync(dlDir, { recursive: true, force: true });
  mkdirSync(dataDir, { recursive: true });
  // Pre-seed settings: downloads go to a scratch folder.
  const seed = spawn(join(bin, "CrowDM.exe"), [], { env, detached: true, stdio: "ignore" });
  seed.unref();

  // ---- cold start through the host is measured later; attach first ----
  const cdp = await connectCdp();
  const main = await pageFor(cdp, (u) => u.includes("tauri.localhost") && !u.includes("#capture"));
  await main.waitForSelector(".sidebar", { timeout: 15000 });
  await main.evaluate(async (dir) => {
    const { invoke } = window.__TAURI_INTERNALS__;
    const s = await invoke("get_settings");
    s.downloadRoot = dir;
    await invoke("save_settings", { settings: s });
  }, dlDir);
  check("main window loads", true);
  if (shots) await main.screenshot({ path: join(shots, "main-empty.png") });

  // ---- warm capture: app already running ----
  const fb = new FakeBrowser();
  fb.send({ type: "hello", id: "h1", browser: "chrome", extensionVersion: "0.1.0", protocol: 1 });
  const hello = await fb.waitFor((m) => m.type === "hello");
  check("host relays hello", hello.protocol === 1, `app ${hello.appVersion}`);

  const t0 = performance.now();
  fb.send(capture("r1", `${SERVER}/file?size=67108864&etag=e2e&name=e2e-video.mp4&mime=video%2Fmp4&rate=1500000&id=e2e1`));
  const received = await fb.waitFor((m) => m.type === "received" && m.replyTo === "r1");
  check("app acknowledges a capture", true, `${(received.at - t0).toFixed(0)} ms`);
  // The dialog window is created on demand for the first capture.
  const dialog = await pageFor(cdp, (u) => u.includes("#capture"));
  await dialog.waitForSelector(".capture-head .title", { timeout: 5000 });
  const visibleAt = performance.now();
  const visible = await dialog.evaluate(() => document.visibilityState);
  check("capture dialog shows the download", visible === "visible", `${(visibleAt - t0).toFixed(0)} ms, ${visible}`);
  await dialog.waitForFunction(() => document.querySelector(".capture-head .title")?.textContent?.includes("e2e-video.mp4"), null, { timeout: 5000 });
  const facts = await dialog.textContent(".capture-head .facts");
  check("dialog shows name and size from the server", facts.includes("64.0 MB"), facts.trim());
  if (shots) await dialog.screenshot({ path: join(shots, "capture-dialog.png") });

  await dialog.click("text=Start download");
  await sleep(800);
  const err = await dialog.$(".error-box");
  if (err) console.log("dialog error:", await err.textContent());
  const decision = await fb.waitFor((m) => m.type === "decision" && m.replyTo === "r1");
  check("Start download tells the browser to hand over", decision.decision === "takeOver");

  await main.waitForSelector(".row", { timeout: 5000 });
  await main.click(".row");
  await sleep(1500);
  if (shots) await main.screenshot({ path: join(shots, "main-downloading.png") });
  const done = await main.waitForFunction(
    () => document.querySelector(".row .status")?.classList.contains("completed"),
    null,
    { timeout: 60000 },
  );
  check("download completes", !!done);
  const file = join(dlDir, "Videos", "e2e-video.mp4");
  check("file saved in the Videos category folder", existsSync(file) && statSync(file).size === 67108864, file);
  if (shots) await main.screenshot({ path: join(shots, "main-completed.png") });

  // ---- "Download in browser" hands the paused download back ----
  fb.send(capture("r2", `${SERVER}/file?size=1000&name=keep-in-browser.bin`));
  await fb.waitFor((m) => m.type === "received" && m.replyTo === "r2");
  await dialog.waitForSelector("text=Download in Chrome", { timeout: 5000 });
  await dialog.click("text=Download in Chrome");
  const d2 = await fb.waitFor((m) => m.type === "decision" && m.replyTo === "r2");
  check("Download in Chrome resumes the browser download", d2.decision === "resumeInBrowser");

  // ---- an HTML answer recommends the browser ----
  fb.send(capture("r3", `${SERVER}/file?size=5000&mime=text%2Fhtml&name=login`, { mime: "application/zip" }));
  await fb.waitFor((m) => m.type === "received" && m.replyTo === "r3");
  await dialog.waitForSelector(".warn-box", { timeout: 5000 });
  const warn = await dialog.textContent(".warn-box");
  check("web-page answers are flagged", /web page/i.test(warn), warn.trim());
  if (shots) await dialog.screenshot({ path: join(shots, "capture-warning.png") });
  await dialog.click("text=Cancel");
  const d3 = await fb.waitFor((m) => m.type === "decision" && m.replyTo === "r3");
  check("Cancel cancels the browser download", d3.decision === "cancel");

  // ---- duplicate click within seconds ----
  fb.send(capture("r4", `${SERVER}/file?size=2000&name=dup.bin`));
  fb.send(capture("r5", `${SERVER}/file?size=2000&name=dup.bin`));
  const d5 = await fb.waitFor((m) => m.type === "decision" && m.replyTo === "r5");
  check("a duplicate capture is cancelled, not downloaded twice", d5.decision === "cancel");
  await dialog.waitForSelector("text=Download in Chrome");
  await dialog.click("text=Download in Chrome");
  await fb.waitFor((m) => m.type === "decision" && m.replyTo === "r4");

  // ---- disconnect while the dialog is open: dialog clears itself ----
  fb.send(capture("r6", `${SERVER}/file?size=2000&name=gone.bin`));
  await fb.waitFor((m) => m.type === "received" && m.replyTo === "r6");
  fb.close();
  await sleep(800);
  const left = await main.evaluate(() => window.__TAURI_INTERNALS__.invoke("capture_list"));
  check("captures from a closed browser connection are dropped", left.length === 0);

  if (shots) {
    await main.click("text=Settings");
    await main.waitForSelector(".settings-body h2");
    await main.screenshot({ path: join(shots, "settings-general.png") });
    await main.click(".settings-nav >> text=Browsers");
    await sleep(300);
    await main.screenshot({ path: join(shots, "settings-browsers.png") });
    await main.evaluate(async () => {
      const { invoke } = window.__TAURI_INTERNALS__;
      const s = await invoke("get_settings");
      s.theme = "dark";
      await invoke("save_settings", { settings: s });
    });
    await main.reload();
    await main.waitForSelector(".row");
    await main.click(".row");
    await sleep(300);
    await main.screenshot({ path: join(shots, "main-dark.png") });
  }

  // ---- memory with the app idle ----
  const mem = await main.evaluate(() => window.__TAURI_INTERNALS__.invoke("memory_usage"));
  const total = mem.reduce((a, m) => a + m.privateBytes, 0);
  check("memory report", mem.length > 0, `${(total / 1048576).toFixed(0)} MB private across ${mem.length} processes: ${mem.map((m) => `${m.name} ${(m.privateBytes / 1048576).toFixed(0)}`).join(", ")}`);
  await cdp.close();

  // ---- cold start: host launches the app ----
  killApp();
  await sleep(500);
  const cold = new FakeBrowser();
  const c0 = performance.now();
  cold.send({ type: "hello", id: "h2", browser: "chrome", extensionVersion: "0.1.0", protocol: 1 });
  const helloCold = await cold.waitFor((m) => m.type === "hello", 30000);
  check("host starts CrowDM when it is not running", true, `hello after ${(helloCold.at - c0).toFixed(0)} ms`);
  cold.send(capture("c1", `${SERVER}/file?size=3000&name=cold.bin`));
  const rc = await cold.waitFor((m) => m.type === "received" && m.replyTo === "c1");
  check("cold-start capture acknowledged", true, `${(rc.at - c0).toFixed(0)} ms after the browser connected`);
  const cdp2 = await connectCdp();
  const dlg2 = await pageFor(cdp2, (u) => u.includes("#capture"));
  await dlg2.waitForSelector(".capture-head .title", { timeout: 10000 });
  check("cold-start dialog rendered", true, `${(performance.now() - c0).toFixed(0)} ms after the browser connected`);
  await dlg2.click("text=Download in Chrome");
  await cold.waitFor((m) => m.type === "decision" && m.replyTo === "c1");
  await cdp2.close();
  cold.close();
  killApp();

  writeFileSync(join(root, "target", "e2e-results.json"), JSON.stringify(results, null, 2));
  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
  process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  killApp();
  process.exit(1);
});
