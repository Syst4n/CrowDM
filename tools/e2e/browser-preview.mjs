// Real extension/native-host regression in an isolated Chrome for Testing
// profile. A unique native-host registry key avoids changing the user's host.
const { chromium } = await import(process.env.CROWDM_PLAYWRIGHT || 'playwright-core');
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { cpSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
const root = resolve(import.meta.dirname, '../..');
const out = join(root, 'target', `browser-preview-${Date.now()}`);
mkdirSync(out, { recursive: true });
const downloads = join(out, 'downloads'); mkdirSync(downloads);
const nativeName = `com.crowdm.preview${process.pid}`;
const registryKey = `HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts\\${nativeName}`;
const extension = join(out, 'extension'); cpSync(join(root, 'extensions/build/chromium'), extension, { recursive: true });
writeFileSync(join(extension, 'background.js'), readFileSync(join(extension, 'background.js'), 'utf8').replace('"com.crowdm.host"', JSON.stringify(nativeName)));
const manifest = join(out, 'host.json');
writeFileSync(manifest, JSON.stringify({ name: nativeName, description: 'CrowDM isolated preview test', path: join(root, 'target/release/crowdm-host.exe'), type: 'stdio', allowed_origins: ['chrome-extension://pomhhnfijfeecghaflldjibhaofbiabm/'] }));
execFileSync(join(root, 'tools/media/ffmpeg.exe'), ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=320x180:rate=10', '-f', 'lavfi', '-i', 'sine=frequency=440', '-t', '2', '-c:v', 'libx264', '-c:a', 'aac', '-y', join(out, 'video.mp4')]);
const video = readFileSync(join(out, 'video.mp4'));
const file = Buffer.alloc(16384, 42);
const server = createServer((req, res) => {
  if (req.url === '/file.zip') { res.writeHead(200, { 'Content-Type': 'application/zip', 'Content-Disposition': 'attachment; filename="Browser test.zip"', 'Content-Length': file.length }); return res.end(file); }
  if (req.url === '/video.mp4') { res.writeHead(200, { 'Content-Type': 'video/mp4', 'Content-Length': video.length }); return res.end(video); }
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end('<html><head><title>CrowDM browser preview</title></head><body style="margin:40px;background:#0d131c;color:#e3eaf3;font:16px Segoe UI"><h2>CrowDM · video downloads</h2><p><a href="/file.zip" download>Download test archive</a></p><video controls muted autoplay loop src="/video.mp4" style="width:640px;height:360px"></video></body></html>');
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const url = `http://127.0.0.1:${server.address().port}`;
const env = { ...process.env, CROWDM_DATA_DIR: out, CROWDM_TEST_INSTANCE: `preview-${process.pid}`, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=19338' };
const app = spawn(join(root, 'target/release/CrowDM.exe'), [], { env, stdio: 'ignore' });
const delay = (ms) => new Promise((r) => setTimeout(r, ms));
const results = [];
const check = (name, ok) => { results.push({ name, ok }); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}`); if (!ok) throw new Error(name); };
let webview, browser;
try {
  execFileSync('reg.exe', ['add', registryKey, '/ve', '/t', 'REG_SZ', '/d', manifest, '/f'], { windowsHide: true, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) { try { if ((await fetch('http://127.0.0.1:19338/json/version')).ok) break; } catch {} await delay(200); }
  webview = await chromium.connectOverCDP('http://127.0.0.1:19338');
  const appContext = webview.contexts()[0];
  const main = appContext.pages().find((p) => !p.url().includes('#capture'));
  const invoke = (command, args = {}) => main.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  await main.getByLabel('Sort downloads').waitFor();
  const settings = await invoke('get_settings');
  await invoke('save_settings', { settings: { ...settings, downloadRoot: downloads, organizeByCategory: false, notifyOnComplete: false, closeToTray: false, resumeOnStartup: false, appearance: {mode:'dark',palette:'nightwing'} } });
  browser = await chromium.launchPersistentContext(join(out, 'chrome-profile'), {
    executablePath: process.env.CROWDM_CHROMIUM || join(process.env.LOCALAPPDATA, 'ms-playwright/chromium-1223/chrome-win64/chrome.exe'),
    headless: false, viewport: { width: 960, height: 640 }, env,
    args: [`--disable-extensions-except=${extension}`, `--load-extension=${extension}`, '--no-first-run'],
    acceptDownloads: true,
  });
  const page = browser.pages()[0]; await page.goto(url);
  await page.getByRole('link', { name: 'Download test archive' }).click();
  let popup;
  for (let i = 0; i < 150; i++) { popup = appContext.pages().find((p) => p.url().includes('#capture')); if (popup && (await invoke('capture_list')).some((c) => c.probe)) break; await delay(100); }
  check('real browser link reaches CrowDM before completion', (await invoke('capture_list')).some((c) => c.kind === 'intercepted' && c.probe.fileName === 'Browser test.zip'));
  await popup.getByRole('button', { name: 'Start download', exact: true }).click();
  for (let i = 0; i < 100; i++) { if ((await invoke('list_jobs')).some((j) => j.state === 'completed')) break; await delay(100); }
  check('browser archive is byte exact', readFileSync(join(downloads, 'Browser test.zip')).equals(file));
  const worker = browser.serviceWorkers()[0] || await browser.waitForEvent('serviceworker');
  const captured = await worker.evaluate(() => chrome.downloads.search({}));
  check('browser copy is cancelled after takeover', captured.every((j) => j.state !== 'complete'));
  const box = await page.locator('video').boundingBox();
  await page.mouse.move(box.x + 150, box.y + 150);
  await page.locator('crowdm-video-button').waitFor({ state: 'visible' });
  await page.mouse.move(box.x + box.width - 25, box.y + 25);
  await delay(250); await page.mouse.click(box.x + box.width - 25, box.y + 25);
  // Native query resolves real formats; the closed shadow root is left intact.
  await delay(4500);
  await page.screenshot({ path: join(out, 'video-menu.png') });
  await page.mouse.click(box.x + box.width - 120, box.y + 92);
  for (let i = 0; i < 150; i++) { if ((await invoke('capture_list')).some((c) => c.kind === 'video' && c.probe)) break; await delay(100); }
  const capture = (await invoke('capture_list')).find((c) => c.kind === 'video');
  check('real overlay and native host resolve video formats', !!capture?.probe?.mediaOptions.some((f) => f.container === 'mp4') && capture.probe.mediaOptions.some((f) => f.container === 'mp3'));
  await popup.getByLabel('Download format and quality').click();
  await popup.locator('.select-option[data-value="mp3:ba/b"]').click();
  await delay(200);
  await popup.getByRole('button', { name: 'Start download', exact: true }).click();
  for (let i = 0; i < 300; i++) { if ((await invoke('list_jobs')).some((j) => j.category === 'audio' && ['completed', 'failed'].includes(j.state))) break; await delay(100); }
  check('real browser video converts to completed MP3', (await invoke('list_jobs')).some((j) => j.category === 'audio' && j.state === 'completed' && j.fileName.endsWith('.mp3')));
  console.log(`ARTIFACTS ${out}`);
} finally {
  writeFileSync(join(out, 'results.json'), JSON.stringify(results, null, 2));
  if (browser) await browser.close();
  if (webview) await webview.close().catch(() => {});
  app.kill(); server.close();
  execFileSync('reg.exe', ['delete', registryKey, '/f'], { windowsHide: true, stdio: 'ignore' });
}
