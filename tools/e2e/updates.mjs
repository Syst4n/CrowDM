// Isolated native-host + WebView2 regression test. Never closes the user's
// app or changes its native-messaging registration. Requires a release build
// and playwright-core (pnpm --dir tools install).
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
const { chromium } = await import(process.env.CROWDM_PLAYWRIGHT || 'playwright-core');
const root = resolve(import.meta.dirname, '../..');
const out = join(root, 'target', `ui-updates-${Date.now()}`);
const downloads = join(out, 'downloads');
mkdirSync(downloads, { recursive: true });
const db = new DatabaseSync(join(out, 'crowdm.sqlite'));
db.exec("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE jobs (id TEXT PRIMARY KEY, data TEXT NOT NULL, segments TEXT NOT NULL DEFAULT '[]', created_at INTEGER NOT NULL)");
db.prepare('INSERT INTO settings VALUES (?,?)').run('settings', JSON.stringify({ downloadRoot: downloads, organizeByCategory: false, theme: 'dark', notifyOnComplete: false, closeToTray: false, resumeOnStartup: false }));
for (const [i, name] of ['Archive.zip', 'Zeta.zip', 'Middle.zip'].entries()) {
  const at = Date.now() - (i + 2) * 86400000;
  const job = { id: randomUUID(), url: `https://example.test/${name}`, finalUrl: null, referrer: null, headers: [], fileName: name, dir: downloads, category: 'archives', size: (i + 1) * 1024, resumable: true, etag: null, lastModified: null, mime: 'application/zip', state: 'completed', error: null, createdAt: at, startedAt: at, completedAt: at + 60000, queued: false, queueOrder: at, maxConnections: null, speedLimit: null, checksum: null, source: { kind: 'manual', pageUrl: null, browser: null }, downloaded: (i + 1) * 1024 };
  writeFileSync(join(downloads, name), Buffer.alloc(job.size));
  db.prepare('INSERT INTO jobs (id,data,created_at) VALUES (?,?,?)').run(job.id, JSON.stringify(job), at);
}
db.close();
execFileSync(join(root, 'tools/media/ffmpeg.exe'), ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=160x90:rate=10', '-f', 'lavfi', '-i', 'sine=frequency=440', '-t', '1', '-c:v', 'libx264', '-c:a', 'aac', '-y', join(out, 'video.mp4')]);
const video = readFileSync(join(out, 'video.mp4'));
execFileSync(join(root, 'tools/media/ffmpeg.exe'), ['-hide_banner','-loglevel','error','-f','lavfi','-i','color=black:size=160x90:rate=10:duration=2','-f','lavfi','-i','testsrc2=size=160x90:rate=10:duration=2','-filter_complex','[0:v][1:v]concat=n=2:v=1:a=0[v]','-map','[v]','-c:v','libx264','-y',join(out,'black-intro.mp4')]);
const cover = join(root, 'apps/desktop/src-tauri/icons/128x128@2x.png');
execFileSync(join(root, 'tools/media/ffmpeg.exe'), ['-hide_banner', '-loglevel', 'error', '-i', join(out, 'video.mp4'), '-i', cover, '-map', '0:a:0', '-map', '1:v:0', '-c:a', 'libmp3lame', '-c:v', 'copy', '-id3v2_version', '3', '-disposition:v:0', 'attached_pic', '-y', join(out, 'artwork.mp3')]);
const server = createServer((req, res) => {
  const reply = () => {
    if (req.url === '/page') { res.writeHead(200, { 'Content-Type': 'text/html' }); return res.end('<html><body>Watch page</body></html>'); }
    const resource = req.url === '/video.mp4' ? [video, 'video/mp4', 'video.mp4'] : req.url === '/black-intro.mp4' ? [readFileSync(join(out,'black-intro.mp4')), 'video/mp4', 'Black intro.mp4'] : req.url === '/cover.png' ? [readFileSync(cover), 'image/png', 'Nightwing.png'] : req.url === '/artwork.mp3' ? [readFileSync(join(out, 'artwork.mp3')), 'audio/mpeg', 'Studio session.mp3'] : [Buffer.alloc(4096, 42), 'application/zip', 'Captured.zip'];
    const [body, mime, filename] = resource;
    res.writeHead(200, { 'Content-Type': mime, 'Content-Length': body.length, 'Content-Disposition': `attachment; filename="${filename}"` });
    res.end(body);
  };
  if (req.url === '/slow') setTimeout(reply, 900); else reply();
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const url = `http://127.0.0.1:${server.address().port}`;
const env = { ...process.env, CROWDM_DATA_DIR: out, CROWDM_TEST_INSTANCE: `updates-${process.pid}`, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=19337' };
const app = spawn(join(root, 'target/release/CrowDM.exe'), [], { env, stdio: 'ignore' });
const results = [];
const check = (name, ok, detail) => { results.push({ name, ok, detail }); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? `: ${detail}` : ''}`); if (!ok) throw new Error(name); };
const delay = (ms) => new Promise((r) => setTimeout(r, ms));
const selectValue = async (page, label, value) => {
  await page.getByRole('combobox', { name: label, exact: true }).click();
  const options = page.locator('.select-menu').getByRole('option');
  await options.first().waitFor();
  const values = await options.evaluateAll((items) => items.map((i) => i.dataset.value));
  const index = values.indexOf(String(value));
  if (index < 0) throw new Error(`Missing ${label} choice: ${value}`);
  await options.nth(index).click();
};
let browser, host;
try {
  let connected;
  for (let i = 0; i < 100; i++) { try { connected = await fetch('http://127.0.0.1:19337/json/version'); if (connected.ok) break; } catch {} await delay(200); }
  browser = await chromium.connectOverCDP('http://127.0.0.1:19337');
  const context = browser.contexts()[0];
  const errors = [];
  const watch = (page) => page.on('pageerror', (e) => errors.push(e.message));
  context.pages().forEach(watch);
  context.on('page', watch);
  const main = context.pages().find((p) => !p.url().includes('#capture'));
  await main.getByRole('combobox', {name:'Sort downloads', exact:true}).waitFor();
  const invoke = (page, command, args = {}) => page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  check('separate test data directory', (await invoke(main, 'app_info')).dataDir === out);
  await main.getByRole('combobox', {name:'Sort downloads', exact:true}).click();
  await main.locator('.select-menu').getByRole('option').first().waitFor();
  const sortChoices = await main.locator('.select-menu').getByRole('option').allTextContents();
  check('all requested sort choices', sortChoices.length === 9, JSON.stringify(sortChoices));
  check('sort popup has no wrapping or visible scrollbars', await main.locator('.select-menu').evaluate((menu) => menu.getBoundingClientRect().width >= 220 && getComputedStyle(menu).scrollbarWidth === 'none' && menu.scrollHeight <= menu.clientHeight + 2 && [...menu.querySelectorAll('.select-option > span')].every((label) => label.scrollWidth <= label.clientWidth)));
  await main.screenshot({ path: join(out, 'sort-menu.png') });
  await main.getByRole('combobox', {name:'Sort downloads', exact:true}).press('Escape');
  await selectValue(main, 'Sort downloads', 'name');
  check('A–Z sort', (await main.locator('.row .name .title').allTextContents()).join(',') === 'Archive.zip,Middle.zip,Zeta.zip');
  await selectValue(main, 'Sort downloads', 'nameDesc');
  check('Z–A sort', (await main.locator('.row .name .title').first().textContent()) === 'Zeta.zip');
  check('file-location button on every row', await main.getByRole('button', { name: /^Open file location for/ }).count() === 3);
  const older = await main.locator('.status.completed').first().textContent();
  check('older status includes weekday, date and time', /[A-Za-z]{3}/.test(older) && /\d{1,2}:\d{2}/.test(older), older.trim());
  await main.getByText('Archive.zip', { exact: true }).click();
  const panel = main.locator('.details-pane');
  check('compact details default', Math.round((await panel.boundingBox()).height) === 180);
  const handle = await main.getByRole('separator', { name: 'Resize download details' }).boundingBox();
  await main.mouse.move(handle.x + 60, handle.y + 3); await main.mouse.down(); await main.mouse.move(handle.x + 60, handle.y - 70); await main.mouse.up();
  check('details drag resize', (await panel.boundingBox()).height > 230);
  await main.getByRole('separator').dblclick();
  check('details reset to compact height', Math.round((await panel.boundingBox()).height) === 180);
  await main.screenshot({ path: join(out, 'main.png') });
  await main.getByRole('button', { name: 'Settings', exact: true }).click();
  for (const label of ['Launch at startup', 'Do not download web pages', 'Avoid sleep mode if there is an active download', 'Avoid sleep mode if there is a scheduled download', 'Allow sleep mode if downloads can be resumed later']) {
    check(`settings: ${label}`, await main.getByText(label, { exact: true }).isVisible());
  }
  await main.screenshot({ path: join(out, 'settings.png') });
  const originalSettings = await invoke(main, 'get_settings');
  await invoke(main, 'save_settings', { settings: { ...originalSettings, power: { ...originalSettings.power, allowSleepResumable: true } } });
  const settingsDb = new DatabaseSync(join(out, 'crowdm.sqlite'), { readOnly: true });
  const persisted = JSON.parse(settingsDb.prepare("SELECT value FROM settings WHERE key='settings'").get().value);
  settingsDb.close();
  check('power preferences persist to disk', persisted.power.allowSleepResumable === true);
  await invoke(main, 'save_settings', { settings: originalSettings });
  await main.getByRole('button', { name: /All downloads/ }).click();
  host = spawn(join(root, 'target/release/crowdm-host.exe'), ['chrome-extension://pomhhnfijfeecghaflldjibhaofbiabm/'], { env, stdio: ['pipe', 'pipe', 'ignore'] });
  const messages = []; let buffer = Buffer.alloc(0);
  host.stdout.on('data', (data) => { buffer = Buffer.concat([buffer, data]); while (buffer.length >= 4 && buffer.length >= buffer.readUInt32LE(0) + 4) { const n = buffer.readUInt32LE(0); messages.push(JSON.parse(buffer.subarray(4, n + 4))); buffer = buffer.subarray(n + 4); } });
  const send = (message) => { const data = Buffer.from(JSON.stringify(message)); const length = Buffer.alloc(4); length.writeUInt32LE(data.length); host.stdin.write(Buffer.concat([length, data])); };
  const waitMessage = async (predicate) => { for (let i = 0; i < 150; i++) { const found = messages.find(predicate); if (found) return found; await delay(100); } throw new Error('native-message timeout'); };
  send({ type: 'hello', id: 'hello', browser: 'chrome', extensionVersion: 'test', protocol: 1 });
  const hello = await waitMessage((m) => m.type === 'hello');
  check('native settings include HTML exclusion', hello.settings.skipWebPages === true);
  const capture = (id, target, kind = 'intercepted') => send({ type: 'capture', id, browser: 'chrome', capture: { kind, browserDownloadId: kind === 'intercepted' ? Number(id.replace(/\D/g, '')) || 1 : null, url: target, finalUrl: null, referrer: url, pageUrl: url, pageTitle: 'Fixture', fileName: null, mime: null, size: null, incognito: false, headers: [] } });
  capture('capture1', `${url}/file`);
  await waitMessage((m) => m.type === 'received' && m.replyTo === 'capture1');
  let popup;
  for (let i = 0; i < 100; i++) { popup = context.pages().find((p) => p.url().includes('#capture')); if (popup) break; await delay(100); }
  await popup.getByLabel('File name', { exact: true }).waitFor();
  await popup.getByRole('button', { name: 'Start download', exact: true }).waitFor();
  await popup.waitForFunction(() => !document.querySelector('.capture-foot .primary')?.disabled);
  check('capture metadata arrives', await popup.getByLabel('File name', { exact: true }).inputValue() === 'Captured.zip');
  check('compact popup height', (await popup.evaluate(() => innerHeight)) < 420, await popup.evaluate(() => `${innerWidth} × ${innerHeight}`));
  await popup.screenshot({ path: join(out, 'capture.png') });
  await popup.locator('#dir').fill('C:\\Downloads\\CrowDM\\Archives');
  await popup.screenshot({ path: join(out, 'capture-public.png') });
  await popup.locator('#dir').fill(downloads);
  await popup.getByRole('button', { name: 'More options' }).click();
  check('advanced options expand', await popup.getByLabel('Checksum (MD5, SHA-1 or SHA-256)').isVisible());
  await popup.getByRole('button', { name: 'More options' }).click();
  await popup.getByRole('button', { name: 'Start download', exact: true }).click();
  await waitMessage((m) => m.type === 'decision' && m.replyTo === 'capture1' && m.decision === 'takeOver');
  for (let i = 0; i < 100; i++) { if ((await invoke(main, 'list_jobs')).some((j) => j.fileName === 'Captured.zip' && j.state === 'completed')) break; await delay(100); }
  check('native capture downloads actual file', readFileSync(join(downloads, 'Captured.zip')).length === 4096);
  capture('capture2', `${url}/page`);
  await waitMessage((m) => m.type === 'decision' && m.replyTo === 'capture2' && m.decision === 'resumeInBrowser');
  check('HTML handed back to browser', !(await invoke(main, 'list_jobs')).some((j) => j.url.endsWith('/page')));
  for (let i = 0; i < 4; i++) {
    const key = await invoke(main, 'capture_new', { url: `${url}/file?capture=${i}` });
    await popup.getByLabel('File name', { exact: true }).waitFor();
    await popup.waitForFunction(() => document.querySelector('#name')?.value === 'Captured.zip');
    await invoke(main, 'capture_dismiss', { key });
  }
  check('reused popup never loses capture data', true);
  const staleKey = await invoke(main, 'capture_new', { url: `${url}/slow` });
  await invoke(main, 'capture_set_url', { key: staleKey, url: `${url}/video.mp4` });
  await delay(1300);
  check('late probe cannot replace new address metadata', (await invoke(main, 'capture_list'))[0].probe.fileName === 'video.mp4');
  await invoke(main, 'capture_dismiss', { key: staleKey });
  send({ type: 'videoOptions', id: 'quality-query', capture: { kind: 'video', url: `${url}/video.mp4`, pageUrl: url, headers: [] } });
  const qualityOptions = await waitMessage((m) => m.type === 'videoOptions' && m.replyTo === 'quality-query');
  check('native video format query returns available choices', qualityOptions.formats.some((f) => f.format === 'bv*+ba/b'));
  check('quality lookup does not create a capture', (await invoke(main, 'capture_list')).length === 0);
  capture('video3', `${url}/video.mp4`, 'video');
  await popup.waitForFunction(() => document.querySelector('#name')?.value.endsWith('.mkv') && !document.querySelector('.capture-foot .primary')?.disabled, { timeout: 30000 });
  const selectedFormat = qualityOptions.formats.find((f) => f.height != null)?.format || 'bv*+ba/b';
  const mp4 = qualityOptions.formats.find((f) => f.container === 'mp4');
  check('format menu includes MP4 and MP3', !!mp4 && qualityOptions.formats.some((f) => f.container === 'mp3'));
  await selectValue(popup, 'Download format and quality', mp4.format);
  await delay(200);
  check('MP4 selection changes the output extension', (await popup.getByLabel('File name', { exact: true }).inputValue()).endsWith('.mp4'));
  await selectValue(popup, 'Download format and quality', selectedFormat);
  await delay(500);
  check('video popup keeps action buttons on screen', await popup.evaluate(() => {
    const footer = document.querySelector('.capture-foot').getBoundingClientRect();
    return footer.top >= 0 && footer.bottom <= innerHeight + 1;
  }));
  await popup.screenshot({ path: join(out, 'video-quality.png') });
  await popup.locator('#dir').fill('C:\\Downloads\\CrowDM\\Videos');
  await popup.screenshot({ path: join(out, 'video-quality-public.png') });
  await popup.locator('#dir').fill(downloads);
  check('quality selection saved on capture', (await invoke(main, 'capture_list'))[0].mediaFormat === selectedFormat);
  await popup.getByRole('button', { name: 'Start download', exact: true }).click();
  for (let i = 0; i < 300; i++) { if ((await invoke(main, 'list_jobs')).some((j) => j.source.kind === 'video' && j.state === 'completed')) break; await delay(100); }
  check('native video capture resolves and completes', (await invoke(main, 'list_jobs')).some((j) => j.mediaDownload && j.state === 'completed'));
  const completedVideo = (await invoke(main, 'list_jobs')).find((j) => j.mediaDownload && j.state === 'completed');
  check('video frame thumbnail is generated locally', (await invoke(main, 'job_thumbnail', { id: completedVideo.id }))?.startsWith('data:image/jpeg;base64,'));
  check('thumbnail command cannot read arbitrary paths', await invoke(main, 'job_thumbnail', { id: randomUUID() }).then(() => false, () => true));
  await main.getByRole('button', { name: 'Settings', exact: true }).click();
  await main.getByRole('button', { name: 'Appearance', exact: true }).click();
  for (const mode of ['Light', 'Dark']) {
    await main.getByRole('group', { name: 'Color mode', exact: true }).getByRole('button', { name: mode, exact: true }).click();
    for (const theme of ['Classic', 'Nightwing', 'Ocean', 'Forest', 'Sand', 'Rose', 'Violet', 'Amber', 'Slate', 'Cherry']) {
    await main.getByRole('button', { name: `${theme} palette`, exact: true }).click();
    await delay(300);
    check(`${mode} ${theme} applies`, await main.evaluate(({mode,palette}) => document.documentElement.dataset.theme === mode && document.documentElement.dataset.palette === palette, {mode:mode.toLowerCase(),palette:theme.toLowerCase()}));
    const colors = await main.evaluate(() => {
      const style = getComputedStyle(document.documentElement);
      return ['--text', '--text-2', '--surface', '--accent', '--accent-text'].map((p) => style.getPropertyValue(p).trim());
    });
    const luminance = (hex) => { hex = hex.replace('#', ''); if (hex.length === 3) hex = [...hex].map((c) => c+c).join(''); const a = hex.match(/.{2}/g).map((s) => parseInt(s, 16) / 255).map((v) => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4); return .2126 * a[0] + .7152 * a[1] + .0722 * a[2]; };
    const contrast = (a,b) => (Math.max(luminance(a),luminance(b))+.05)/(Math.min(luminance(a),luminance(b))+.05);
    check(`${mode} ${theme} text and button contrast`, contrast(colors[0], colors[2]) >= 4.5 && contrast(colors[1], colors[2]) >= 4.5 && contrast(colors[3], colors[4]) >= 4.5, colors.join(' '));
    if (['Nightwing', 'Sand', 'Rose'].includes(theme)) await main.screenshot({ path: join(out, `appearance-${mode.toLowerCase()}-${theme.toLowerCase()}.png`) });
    }
  }
  await main.getByRole('button', { name: 'Nightwing palette', exact: true }).click();
  await main.getByLabel('Subtle animations', { exact: false }).uncheck();
  await delay(200);
  check('animation preference applies', await main.evaluate(() => document.documentElement.dataset.motion === 'off'));
  await main.getByLabel('Subtle animations', { exact: false }).check();
  await main.emulateMedia({ reducedMotion: 'reduce' });
  await delay(150);
  const reduced = await main.locator('.theme-card').first().evaluate((el) => ({ matches: matchMedia('(prefers-reduced-motion: reduce)').matches, duration: getComputedStyle(el).transitionDuration }));
  check('reduced motion disables visible transitions', reduced.matches && reduced.duration.split(',').every((s) => parseFloat(s) <= .001), JSON.stringify(reduced));
  check('motion preview explains Windows reduced motion', await main.getByRole('button', { name: 'Preview motion', exact: true }).isDisabled());
  await main.getByLabel('Follow Windows reduced-motion preference', { exact: false }).uncheck();
  await main.getByRole('button', { name: 'Preview motion', exact: true }).click();
  check('motion preview animates when explicitly enabled', await main.locator('.motion-sample').evaluate((el) => el.getAnimations({ subtree: true }).some((a) => a.playState === 'running')));
  await main.getByLabel('Follow Windows reduced-motion preference', { exact: false }).check();
  await main.emulateMedia({ reducedMotion: 'no-preference' });
  await main.getByRole('button', { name: /All downloads/ }).click();
  await main.getByRole('combobox', {name:'Sort downloads', exact:true}).click();
  const darkMenu = await main.locator('.select-menu').evaluate((el) => getComputedStyle(el).backgroundColor);
  await main.getByRole('combobox', {name:'Sort downloads', exact:true}).press('Escape');
  const paletteBefore = await invoke(main, 'get_settings');
  await invoke(main, 'save_settings', { settings: { ...paletteBefore, appearance: {mode:'light',palette:'rose'} } });
  // Settings events normally update the store through the Appearance UI.
  await main.getByRole('button', { name: 'Settings', exact: true }).click();
  await main.getByRole('button', { name: 'Appearance', exact: true }).click();
  await main.getByRole('group', { name: 'Color mode', exact: true }).getByRole('button', { name: 'Light', exact: true }).click();
  await main.getByRole('button', { name: 'Rose palette', exact: true }).click();
  await main.getByRole('button', { name: /All downloads/ }).click();
  await main.getByRole('combobox', {name:'Sort downloads', exact:true}).click();
  check('sort popup updates after changing palette and mode', await main.locator('.select-menu').evaluate((el, old) => getComputedStyle(el).backgroundColor !== old && getComputedStyle(el).backgroundColor === getComputedStyle(document.querySelector('.main')).backgroundColor, darkMenu));
  await main.getByRole('combobox', {name:'Sort downloads', exact:true}).press('Escape');
  await main.getByRole('combobox', {name:'Sort downloads', exact:true}).press('ArrowDown');
  await main.getByRole('combobox', {name:'Sort downloads', exact:true}).press('End');
  await main.getByRole('combobox', {name:'Sort downloads', exact:true}).press('Enter');
  check('sort popup supports keyboard selection', await main.getByRole('combobox', {name:'Sort downloads', exact:true}).getAttribute('data-value') === 'progress');
  const beforeAudio = await invoke(main, 'get_settings');
  await invoke(main, 'save_settings', { settings: { ...beforeAudio, organizeByCategory: true } });
  capture('audio4', `${url}/video.mp4`, 'video');
  await popup.waitForFunction(() => document.querySelector('#quality') && !document.querySelector('.capture-foot .primary')?.disabled);
  await selectValue(popup, 'Download format and quality', 'mp3:ba/b');
  await delay(300);
  check('MP3 changes filename and category', (await popup.getByLabel('File name', { exact: true }).inputValue()).endsWith('.mp3') && await popup.getByLabel('Category', { exact: true }).getAttribute('data-value') === 'audio');
  check('MP3 chooses the Audio folder', /[\\/]Audio$/.test(await popup.locator('#dir').inputValue()));
  await popup.screenshot({ path: join(out, 'mp3-download.png') });
  const audioDir = await popup.locator('#dir').inputValue();
  await popup.locator('#dir').fill('C:\\Downloads\\CrowDM\\Audio');
  await popup.screenshot({ path: join(out, 'mp3-download-public.png') });
  await popup.locator('#dir').fill(audioDir);
  await popup.getByRole('button', { name: 'Start download', exact: true }).click();
  let audio;
  for (let i = 0; i < 400; i++) { audio = (await invoke(main, 'list_jobs')).find((j) => j.category === 'audio'); if (audio?.state === 'completed' || audio?.state === 'failed') break; await delay(100); }
  check('MP3 output completes in Audio', audio?.state === 'completed' && /[\\/]Audio$/.test(audio.dir));
  check('MP3 without embedded artwork keeps its icon', await invoke(main, 'job_thumbnail', { id: audio.id }) === null);
  for (const [path, category] of [['cover.png', 'images'], ['artwork.mp3', 'audio'], ['black-intro.mp4','videos']]) {
    const key = await invoke(main, 'capture_new', { url: `${url}/${path}` });
    for (let i = 0; i < 100; i++) { const c = (await invoke(main, 'capture_list')).find((c) => c.key === key); if (c?.probe && !c.probing) break; await delay(100); }
    const id = await invoke(main, 'capture_accept', { key, options: { start: true } });
    let job;
    for (let i = 0; i < 100; i++) { job = (await invoke(main, 'list_jobs')).find((j) => j.id === id); if (job.state === 'completed') break; await delay(100); }
    check(`${path} completes in the correct category`, job.state === 'completed' && job.category === category);
    check(`${path} has its own thumbnail`, (await invoke(main, 'job_thumbnail', { id }))?.startsWith('data:image/jpeg;base64,'));
    if (path === 'black-intro.mp4') {
      const thumbnail = await invoke(main, 'job_thumbnail', { id });
      const nonblack = await main.evaluate(async (src) => { const img = new Image(); img.src = src; await img.decode(); const canvas = document.createElement('canvas'); canvas.width = img.width; canvas.height = img.height; const ctx = canvas.getContext('2d'); ctx.drawImage(img,0,0); const pixels = ctx.getImageData(0,0,img.width,img.height).data; let lit=0; for(let i=0;i<pixels.length;i+=4) if(Math.max(pixels[i],pixels[i+1],pixels[i+2])>32)lit++; return lit/(pixels.length/4); }, thumbnail);
      check('black intro produces a representative non-black preview', nonblack > .2, String(nonblack));
    }
  }
  const newRoot = join(out, 'changed-root');
  const pending = await invoke(main, 'capture_new', {url:`${url}/file?new-root`});
  for(let i=0;i<100;i++){ if((await invoke(main,'capture_list')).find(c=>c.key===pending)?.probe)break; await delay(100); }
  const beforeRoot = await invoke(main,'get_settings');
  await invoke(main,'save_settings',{settings:{...beforeRoot,downloadRoot:newRoot,organizeByCategory:true}});
  await popup.waitForFunction((root)=>document.querySelector('#dir')?.value===root,newRoot+'\\Archives');
  check('pending capture follows the changed root and category setting', await popup.locator('#dir').inputValue()===join(newRoot,'Archives'));
  await popup.getByRole('button',{name:'Start download',exact:true}).click();
  for(let i=0;i<100;i++){ if((await invoke(main,'list_jobs')).some(j=>j.dir===join(newRoot,'Archives')&&j.state==='completed'))break; await delay(100); }
  check('archive is written under the new root Archives folder', readFileSync(join(newRoot,'Archives','Captured.zip')).length===4096);
  const flatKey=await invoke(main,'capture_new',{url:`${url}/file?flat-mode`});
  for(let i=0;i<100;i++){ if((await invoke(main,'capture_list')).find(c=>c.key===flatKey)?.probe)break; await delay(100); }
  await invoke(main,'save_settings',{settings:{...(await invoke(main,'get_settings')),organizeByCategory:false}});
  await popup.waitForFunction((root)=>document.querySelector('#dir')?.value===root,newRoot);
  check('flat-folder mode remains an explicit supported choice', await popup.locator('#dir').inputValue()===newRoot);
  await invoke(main,'capture_dismiss',{key:flatKey});
  await main.getByRole('button',{name:'Settings',exact:true}).click();
  await main.getByRole('button',{name:'Folders and categories',exact:true}).click();
  await main.getByRole('button',{name:'Use category folders',exact:true}).click();
  check('category folders can be enabled clearly in settings', await main.getByLabel('Sort files into category folders',{exact:false}).isChecked());
  await main.getByRole('button',{name:'Appearance',exact:true}).click();
  await main.getByRole('group',{name:'Color mode',exact:true}).getByRole('button',{name:'Dark',exact:true}).click();
  await main.getByRole('button',{name:'Nightwing palette',exact:true}).click();
  await main.getByRole('button',{name:/All downloads/}).click();
  await selectValue(main, 'Sort downloads', 'added');
  const selectedRow = main.locator('.row[aria-selected="true"]');
  if (await selectedRow.count() === 1) await selectedRow.locator('.name').click({ modifiers: ['Control'] });
  await main.locator('.file-preview').first().waitFor();
  await delay(300);
  await main.screenshot({ path: join(out, 'main-nightwing.png') });
  await invoke(main, 'plugin:window|set_size', { label: 'main', value: { Logical: { width: 820, height: 520 } } });
  await delay(300);
  check('small-window list stays inside viewport', await main.evaluate(() => [...document.querySelectorAll('.row')].every((r) => r.getBoundingClientRect().right <= innerWidth)));
  await main.screenshot({ path: join(out, 'small-window.png') });
  check('no unhandled renderer errors', errors.length === 0, errors.join('; '));
  console.log(`ARTIFACTS ${out}`);
} finally {
  writeFileSync(join(out, 'results.json'), JSON.stringify(results, null, 2));
  if (host) host.kill();
  if (browser) await browser.close().catch(() => {});
  app.kill();
  server.close();
}
