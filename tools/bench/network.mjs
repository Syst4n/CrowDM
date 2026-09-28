// Compares download time for the same public files with:
//   curl (one connection, like a browser's own download) and CrowDM's engine
//   with 1 and 8 connections. (timeChromium is kept for manual use; headless
//   Chromium does not reliably start downloads from a direct navigation.)
//   Internet Download Manager is compared by hand (see docs/PERFORMANCE.md):
//   its command line hands the job to the running IDM window, which does not
//   report completion reliably enough to time.
// Runs are interleaved so that network conditions affect every tool alike.
//
// Usage: node tools/bench/network.mjs [rounds] > results.json
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { chromium } from "playwright-core";

const root = resolve(import.meta.dirname, "..", "..");
const rounds = +(process.argv[2] ?? 2);
const out = join(root, "target", "bench-net");
const files = [
  { name: "hetzner-fsn1-100MB", url: "https://fsn1-speed.hetzner.com/100MB.bin", size: 104857600 },
  { name: "ovh-100Mb", url: "https://proof.ovh.net/files/100Mb.dat", size: 104857600 },
];
const IDM = "C:\\Program Files (x86)\\Internet Download Manager\\IDMan.exe";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function fresh() {
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
}

function timeCurl(f) {
  fresh();
  const t = performance.now();
  const r = spawnSync("curl", ["-s", "-L", "-o", join(out, "curl.bin"), "--max-time", "600", f.url]);
  const secs = (performance.now() - t) / 1000;
  return { ok: r.status === 0, secs, bytes: statSync(join(out, "curl.bin")).size };
}

function timeCrowdm(f, conns) {
  fresh();
  const r = execFileSync(join(root, "target", "release", "examples", "get.exe"), [f.url, "--connections", String(conns), "--out", out], { encoding: "utf8" });
  const j = JSON.parse(r.replace(/None/g, "null"));
  return { ok: j.state === "Completed", secs: j.seconds, bytes: j.bytes, peakWorkingSetMb: j.peakWorkingSetMb };
}

let browser;
async function timeChromium(f) {
  fresh();
  browser ??= await chromium.launch({
    executablePath: process.env.CROWDM_CHROMIUM ?? join(process.env.LOCALAPPDATA, "ms-playwright", "chromium-1223", "chrome-win64", "chrome.exe"),
    headless: true,
    downloadsPath: out,
  });
  const ctx = await browser.newContext({ acceptDownloads: true });
  const page = await ctx.newPage();
  const t = performance.now();
  const [dl] = await Promise.all([page.waitForEvent("download", { timeout: 60000 }), page.goto(f.url).catch(() => {})]);
  const path = await dl.path();
  const secs = (performance.now() - t) / 1000;
  const bytes = statSync(path).size;
  await ctx.close();
  return { ok: !(await dl.failure()), secs, bytes };
}

const results = [];
for (const f of files) {
  for (let r = 0; r < rounds; r++) {
    const tools = [
      ["curl (1 connection)", () => timeCurl(f)],
      ["CrowDM (1 connection)", () => timeCrowdm(f, 1)],
      ["CrowDM (8 connections)", () => timeCrowdm(f, 8)],
    ];
    for (const [tool, fn] of tools) {
      try {
        const res = await fn();
        if (!res) continue;
        const mbps = res.bytes / res.secs / 1048576;
        results.push({ file: f.name, round: r + 1, tool, ...res, mbPerSec: +mbps.toFixed(2) });
        console.error(`${f.name} #${r + 1} ${tool}: ${res.secs.toFixed(2)} s, ${mbps.toFixed(2)} MB/s${res.ok ? "" : " (FAILED)"}`);
      } catch (e) {
        console.error(`${f.name} #${r + 1} ${tool}: error ${e.message}`);
      }
    }
  }
}
await browser?.close();
rmSync(out, { recursive: true, force: true });
console.log(JSON.stringify(results, null, 2));
