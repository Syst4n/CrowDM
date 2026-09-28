// End-to-end test with the installed Firefox and the CrowDM Firefox add-on
// (loaded as a temporary add-on into a throwaway profile via geckodriver).
// Firefox keeps downloading while CrowDM asks, so this checks that the
// hand-off happens and that Firefox's copy is cancelled when CrowDM takes it.
//
// Needs the release build and the test server (see docs/TESTING.md).

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { Builder, By } from "selenium-webdriver";
import firefox from "selenium-webdriver/firefox.js";
import { chromium } from "playwright-core";

const root = resolve(import.meta.dirname, "..", "..");
const bin = join(root, "target", "release");
const dataDir = join(root, "target", "e2e-data");
const dlDir = join(root, "target", "e2e-downloads");
const ffDownloads = join(root, "target", "e2e-firefox-downloads");
const SERVER = "http://127.0.0.1:8765";
const APP_CDP = 9333;

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? `: ${detail}` : ""}`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const kill = () => {
  for (const im of ["CrowDM.exe", "crowdm-host.exe"]) {
    try {
      execFileSync("taskkill", ["/IM", im, "/F"], { stdio: "ignore" });
    } catch {}
  }
};

async function connectApp(timeout = 30000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try {
      return await chromium.connectOverCDP(`http://127.0.0.1:${APP_CDP}`);
    } catch {
      await sleep(250);
    }
  }
  throw new Error("could not attach to the CrowDM WebView");
}

async function dialogPage(app, timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    for (const c of app.contexts()) for (const p of c.pages()) if (p.url().includes("#capture")) return p;
    await sleep(100);
  }
  throw new Error("no dialog");
}

const clicks = [];

async function main() {
  kill();
  for (const d of [dataDir, dlDir, ffDownloads]) rmSync(d, { recursive: true, force: true });
  mkdirSync(dataDir, { recursive: true });
  mkdirSync(ffDownloads, { recursive: true });
  const db = new DatabaseSync(join(dataDir, "crowdm.sqlite"));
  db.exec("CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
  db.prepare("INSERT INTO settings (key, value) VALUES ('settings', ?)").run(JSON.stringify({ downloadRoot: dlDir }));
  db.close();

  // The browser inherits these, so a CrowDM started by the host uses the
  // test data folder and exposes its WebView for the test.
  process.env.CROWDM_DATA_DIR = dataDir;
  process.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = `--remote-debugging-port=${APP_CDP}`;
  spawn(join(bin, "CrowDM.exe"), ["--background"], { env: process.env, detached: true, stdio: "ignore" }).unref();
  await sleep(1500);

  const opts = new firefox.Options()
    .addArguments("-headless")
    .setPreference("browser.download.folderList", 2)
    .setPreference("browser.download.dir", ffDownloads)
    .setPreference("browser.download.useDownloadDir", true)
    .setPreference("browser.download.always_ask_before_handling_new_types", false)
    .setPreference("browser.helperApps.neverAsk.saveToDisk", "application/octet-stream,video/mp4")
    .setPreference("xpinstall.signatures.required", false);
  const driver = await new Builder().forBrowser("firefox").setFirefoxOptions(opts).build();
  try {
    const addonId = await driver.installAddon(join(root, "extensions", "build", "firefox"), true);
    check("add-on installs in Firefox", addonId === "crowdm@syst4n.github.io", addonId);
    await driver.get(`${SERVER}/page`);

    const t0 = Date.now();
    clicks.push(t0);
    // Firefox keeps downloading while CrowDM asks, so use a slow file: a
    // fast local one would finish in Firefox before the dialog is answered.
    await driver.findElement(By.id("slow")).click();
    const app = await connectApp();
    const dialog = await dialogPage(app);
    await dialog.waitForFunction(() => document.querySelector(".capture-head .title")?.textContent?.includes("slow.bin"), null, { timeout: 20000, polling: 100 });
    // Selenium's click() waits several seconds for a navigation that never
    // comes, so the real latency is computed from the app log at the end.
    check("clicking a download link in Firefox opens the CrowDM dialog", true);
    const facts = await dialog.textContent(".capture-head .facts");
    check("dialog notes that Firefox is waiting", /Firefox/.test(facts), facts.trim());

    await dialog.click("text=Start download");
    const target = join(dlDir, "Other", "slow.bin");
    const end = Date.now() + 90000;
    while (Date.now() < end && !(existsSync(target) && statSync(target).size === 33554432)) await sleep(250);
    check("CrowDM downloads the file", existsSync(target) && statSync(target).size === 33554432);
    await sleep(1500);
    const leftovers = existsSync(ffDownloads) ? execFileSync("cmd", ["/c", "dir", "/b", ffDownloads], { encoding: "utf8" }).trim() : "";
    check("Firefox's own copy is cancelled", !/slow\.bin/.test(leftovers), leftovers || "(no files)");

    // Give a download back to Firefox (a second, warm capture).
    await driver.executeScript(`const a = document.createElement("a"); a.id = "back"; a.href = "/file?size=16777216&rate=1048576&name=back.bin&etag=bk"; a.textContent = "back"; document.body.appendChild(a);`);
    const t1 = Date.now();
    clicks.push(t1);
    await driver.findElement(By.id("back")).click();
    await dialog
      .waitForFunction(() => document.querySelector(".capture-head .title")?.textContent?.includes("back.bin"), null, { timeout: 15000, polling: 100 })
      .catch(async (e) => {
        console.log("dialog state:", await dialog.evaluate(() => ({ html: document.body.innerHTML.slice(0, 400), vis: document.visibilityState })).catch((x) => String(x)));
        throw e;
      });
    check("second capture opens the dialog", true);
    await dialog.click("text=Download in Firefox");
    const backPath = join(ffDownloads, "back.bin");
    const end2 = Date.now() + 45000;
    while (Date.now() < end2 && !(existsSync(backPath) && statSync(backPath).size === 16777216)) await sleep(250);
    check('"Download in Firefox" leaves the file to Firefox', existsSync(backPath) && statSync(backPath).size === 16777216);
    const inCrowdm = existsSync(join(dlDir, "Other", "back.bin"));
    check("and CrowDM does not download it", !inCrowdm);

    await Promise.race([app.close().catch(() => {}), sleep(3000)]);
  } finally {
    await driver.quit().catch(() => {});
    kill();
  }
  // Where the time goes: click -> capture arrives at the app -> dialog painted.
  const log = readFileSync(join(dataDir, "logs", "app.log"), "utf8").split("\n");
  const arrivals = log.filter((l) => l.includes("capture from firefox")).map((l) => Date.parse(l.slice(0, 27)));
  console.log("capture arrival times (ms since click):", arrivals.map((t, i) => t - clicks[i]));
  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
  process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  kill();
  process.exit(1);
});
