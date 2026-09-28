// Measures how quickly CrowDM shows its dialog for a browser download, with
// the dialog pre-loaded and without it. A fake browser drives the real native
// host; the app logs when the dialog has painted each capture.
//
// Usage (after a release build): node tools/bench/popup-latency.mjs
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, readFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";

const root = resolve(import.meta.dirname, "..", "..");
const bin = join(root, "target", "release");
const dataDir = join(root, "target", "bench-data");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const kill = () => {
  for (const im of ["CrowDM.exe", "crowdm-host.exe"]) {
    try {
      execFileSync("taskkill", ["/IM", im, "/F"], { stdio: "ignore" });
    } catch {}
  }
};

function seed(prewarm) {
  rmSync(dataDir, { recursive: true, force: true });
  mkdirSync(dataDir, { recursive: true });
  const db = new DatabaseSync(join(dataDir, "crowdm.sqlite"));
  db.exec("CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
  db.prepare("INSERT INTO settings (key, value) VALUES ('settings', ?)").run(JSON.stringify({ prewarmDialog: prewarm, downloadRoot: join(root, "target", "bench-downloads") }));
  db.close();
}

function host(env) {
  const p = spawn(join(bin, "crowdm-host.exe"), ["chrome-extension://pomhhnfijfeecghaflldjibhaofbiabm/"], { env, stdio: ["pipe", "pipe", "ignore"] });
  let buf = Buffer.alloc(0);
  const msgs = [];
  p.stdout.on("data", (d) => {
    buf = Buffer.concat([buf, d]);
    while (buf.length >= 4 && buf.length >= 4 + buf.readUInt32LE(0)) {
      const n = buf.readUInt32LE(0);
      msgs.push({ ...JSON.parse(buf.subarray(4, 4 + n)), at: performance.now() });
      buf = buf.subarray(4 + n);
    }
  });
  const send = (m) => {
    const b = Buffer.from(JSON.stringify(m));
    const l = Buffer.alloc(4);
    l.writeUInt32LE(b.length);
    p.stdin.write(Buffer.concat([l, b]));
  };
  const wait = async (pred, ms = 30000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      const m = msgs.find(pred);
      if (m) return m;
      await sleep(5);
    }
    throw new Error("timeout");
  };
  return { p, send, wait };
}

const latencies = () =>
  readFileSync(join(dataDir, "logs", "app.log"), "utf8")
    .split("\n")
    .map((l) => l.match(/dialog painted (\d+) ms/))
    .filter(Boolean)
    .map((m) => +m[1]);

async function run(prewarm, rounds = 8) {
  kill();
  seed(prewarm);
  const env = { ...process.env, CROWDM_DATA_DIR: dataDir };
  const h = host(env);
  const t0 = performance.now();
  h.send({ type: "hello", id: "h", browser: "chrome", extensionVersion: "bench", protocol: 1 });
  const hello = await h.wait((m) => m.type === "hello");
  const coldConnect = hello.at - t0;
  await sleep(prewarm ? 2500 : 500); // let the pre-loaded dialog finish loading
  const acks = [];
  for (let i = 0; i < rounds; i++) {
    const id = `b${i}`;
    const s = performance.now();
    h.send({
      type: "capture",
      id,
      browser: "chrome",
      capture: { kind: "intercepted", browserDownloadId: i + 1, url: `http://127.0.0.1:8765/file?size=1000&name=bench${i}.bin`, finalUrl: null, referrer: null, pageUrl: null, pageTitle: null, fileName: null, mime: null, size: 1000, incognito: false, headers: [] },
    });
    const r = await h.wait((m) => m.type === "received" && m.replyTo === id);
    acks.push(r.at - s);
    await sleep(1500);
    h.send({ type: "captureGone", id });
    await sleep(700);
  }
  const painted = latencies();
  h.p.kill();
  kill();
  const med = (a) => [...a].sort((x, y) => x - y)[Math.floor(a.length / 2)];
  return { prewarm, coldConnect: Math.round(coldConnect), ackMedian: +med(acks).toFixed(1), paintFirst: painted[0], paintMedian: med(painted), paintAll: painted };
}

const out = [await run(true), await run(false)];
console.log(JSON.stringify(out, null, 2));
