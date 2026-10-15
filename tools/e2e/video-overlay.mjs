// Browser component regression for the overlay. Native format extraction is
// covered separately by updates.mjs and the Rust media integration test.
import { createServer } from 'node:http';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join, resolve } from 'node:path';
const { chromium } = await import(process.env.CROWDM_PLAYWRIGHT || 'playwright-core');
const root = resolve(import.meta.dirname, '../..');
const out = join(root, 'target', `video-overlay-${Date.now()}`);
mkdirSync(out, { recursive: true });
execFileSync(join(root, 'tools/media/ffmpeg.exe'), ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=320x180:rate=10', '-t', '1', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-y', join(out, 'clip.mp4')]);
const clip = readFileSync(join(out, 'clip.mp4'));
const server = createServer((req, res) => {
  if (req.url === '/clip.mp4') { res.writeHead(200, { 'Content-Type': 'video/mp4', 'Content-Length': clip.length }); return res.end(clip); }
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end('<html><head><title>Overlay verification</title></head><body style="margin:40px;background:#101216;color:#eee;font:16px Segoe UI"><h2>CrowDM video controls</h2><video controls muted loop autoplay src="/clip.mp4" style="width:640px;height:360px"></video></body></html>');
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const browser = await chromium.launch({ headless: true, executablePath: process.env.CROWDM_CHROMIUM || join(process.env.LOCALAPPDATA, 'ms-playwright/chromium-1223/chrome-win64/chrome.exe') });
const results = [];
const check = (name, ok) => { results.push({ name, ok }); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}`); if (!ok) throw new Error(name); };
try {
  const page = await browser.newPage({ viewport: { width: 960, height: 600 } });
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  const errors = []; page.on('pageerror', (error) => errors.push(error.message));
  await page.evaluate(() => {
    const attach = Element.prototype.attachShadow;
    // Open only this component's shadow root in the test for DOM assertions.
    Element.prototype.attachShadow = function(options) { return attach.call(this, this.tagName === 'CROWDM-VIDEO-BUTTON' ? { ...options, mode: 'open' } : options); };
    window.__videoMessages = [];
    window.chrome = {
      storage: { local: { get: async () => ({ settings: { enabled: true, videoButton: true, theme: 'nightwing' } }) }, onChanged: { addListener(fn) { window.__storageChanged = fn; } } },
      runtime: { onMessage: { addListener() {} }, sendMessage: async (message) => { window.__videoMessages.push(message); return message.type === 'videoOptions' ? { ok: true, formats: [
        { format: 'bv*+ba/b', label: 'Best available' }, { format: 'bv*[height=180]+ba/b[height=180]', label: '180p' }, { format: 'bv*[height=90]+ba/b[height=90]', label: '90p' }
      ] } : { ok: true }; } }
    };
  });
  await page.addScriptTag({ content: readFileSync(join(root, 'extensions/build/chromium/themes.js'), 'utf8') });
  await page.addScriptTag({ content: readFileSync(join(root, 'extensions/src/video.js'), 'utf8') });
  await page.mouse.move(300, 240);
  const trigger = page.locator('crowdm-video-button .trigger');
  await trigger.waitFor({ state: 'visible' });
  await page.waitForTimeout(250);
  check('collapsed control is a faint icon', await trigger.evaluate((el) => +getComputedStyle(el).opacity < 0.4 && el.getBoundingClientRect().width < 40));
  const videoBox = await page.locator('video').boundingBox();
  const iconBox = await trigger.boundingBox();
  check('icon is at the top right of the video', Math.abs(iconBox.x + iconBox.width - videoBox.x - videoBox.width + 10) < 2 && Math.abs(iconBox.y - videoBox.y - 10) < 2);
  await page.screenshot({ path: join(out, 'icon.png') });
  await trigger.hover(); await page.waitForTimeout(250);
  check('hover expands Download video at full opacity', await trigger.evaluate((el) => +getComputedStyle(el).opacity === 1 && el.getBoundingClientRect().width > 120));
  await page.screenshot({ path: join(out, 'hover.png') });
  await trigger.click();
  await page.getByRole('menuitem', { name: '90p MKV' }).waitFor();
  check('click lists available quality choices', await page.getByRole('menuitem').count() === 3);
  await page.screenshot({ path: join(out, 'quality-menu.png') });
  await page.getByRole('menuitem', { name: '90p MKV' }).click();
  check('selected quality is sent to CrowDM', await page.evaluate(() => window.__videoMessages.some((m) => m.type === 'video' && m.mediaFormat === 'bv*[height=90]+ba/b[height=90]')));
  check('quality menu closes after acknowledgement', await page.getByRole('menu', { name: 'Available video downloads' }).isHidden());
  await trigger.click();
  await page.getByRole('menuitem').first().waitFor();
  await page.evaluate(() => history.pushState({}, '', '/watch?v=next&list=mix'));
  await page.waitForTimeout(350);
  check('SPA navigation invalidates an open quality menu', await page.getByRole('menu').isHidden());
  await page.mouse.move(310, 245);
  await trigger.click();
  await page.getByRole('menuitem').first().waitFor();
  await page.getByRole('menuitem').first().click();
  check('next video sends the current SPA address', await page.evaluate(() => window.__videoMessages.filter((m) => m.type === 'video').at(-1).pageUrl.endsWith('/watch?v=next&list=mix')));
  await page.evaluate(() => history.pushState({}, '', '/watch?v=outside'));
  await page.waitForTimeout(350);
  await page.mouse.move(320, 250);
  await trigger.click();
  await page.getByRole('menuitem').first().waitFor();
  await page.getByRole('menuitem').first().click();
  check('leaving the playlist sends the new video address', await page.evaluate(() => window.__videoMessages.filter((m) => m.type === 'video').at(-1).pageUrl.endsWith('/watch?v=outside')));
  await page.evaluate(() => window.__storageChanged({ settings: { newValue: { enabled: true, videoButton: true, theme: 'sand' } } }));
  check('overlay logo follows app palette changes', await page.locator('crowdm-video-button').evaluate((el) => el.dataset.palette === 'sand' && getComputedStyle(el.shadowRoot.querySelector('path[fill^="var"]')).fill === 'rgb(149, 97, 37)'));
  await page.evaluate(() => window.__storageChanged({ settings: { newValue: { enabled:true,videoButton:true,appearance:{mode:'light',palette:'violet'} } } }));
  check('overlay mode and palette are independent', await page.locator('crowdm-video-button').evaluate((el) => el.dataset.theme === 'light' && el.dataset.palette === 'violet' && getComputedStyle(el.shadowRoot.querySelector('path[fill^="var"]')).fill === 'rgb(118, 80, 161)'));
  check('no renderer errors', errors.length === 0);
  console.log(`ARTIFACTS ${out}`);
} finally {
  writeFileSync(join(out, 'results.json'), JSON.stringify(results, null, 2));
  await browser.close(); server.close();
}
