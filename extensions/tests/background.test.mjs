import { readFileSync } from 'node:fs';
import { createContext, runInContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';
const event = () => ({ listeners: [], addListener(fn) { this.listeners.push(fn); } });
async function background() {
  const chrome = {
    runtime: { getManifest: () => ({ version: '0.1.0' }), id: 'test', onInstalled: event(), onStartup: event(), onMessage: event(), sendMessage: async () => {} },
    storage: { local: { get: async () => ({}), set: async () => {} } },
    downloads: { onCreated: event(), onChanged: event(), onDeterminingFilename: event() },
    contextMenus: { onClicked: event() },
    cookies: { getAll: async () => [] },
    webRequest: { onHeadersReceived: event() }, webNavigation: { onCommitted: event(), onHistoryStateUpdated: event() },
    tabs: { onRemoved: event() },
  };
  const context = createContext({ chrome, navigator: { userAgent: 'Chrome' }, console, URL, setTimeout, clearTimeout, setInterval: () => {} });
  runInContext(readFileSync(new URL('../src/background.js', import.meta.url), 'utf8'), context);
  await runInContext('ready', context);
  return { chrome, eval: (code) => runInContext(code, context) };
}
test('HTML interception is blocked by MIME or extension, and is configurable', async () => {
  const bg = await background();
  assert.equal(bg.eval('shouldCatch({url: "https://site.test/file", mime: "text/html; charset=utf-8"})'), false);
  assert.equal(bg.eval('shouldCatch({url: "https://site.test/page.html"})'), false);
  assert.equal(bg.eval('shouldCatch({url: "https://site.test/file.mp4", mime: "video/mp4"})'), true);
  bg.eval('settings.skipWebPages = false');
  assert.equal(bg.eval('shouldCatch({url: "https://site.test/page.html"})'), true);
});
test('YouTube uses the watch page, not a single CDN track', async () => {
  const bg = await background();
  assert.equal(bg.eval('mediaUrl("https://cdn.test/videoplayback", "https://www.youtube.com/watch?v=abc", 1, 0)'), 'https://www.youtube.com/watch?v=abc');
  assert.equal(bg.eval('mediaUrl("https://cdn.test/movie.mp4", "https://youtube.com.evil.test/watch", 1, 0)'), 'https://cdn.test/movie.mp4');
});
test('blob videos use only an unambiguous manifest in the matching frame, and navigation clears it', async () => {
  const bg = await background();
  const capture = bg.chrome.webRequest.onHeadersReceived.listeners[0];
  capture({ tabId: 1, frameId: 3, url: 'https://cdn.test/movie.m3u8', responseHeaders: [] });
  assert.equal(bg.eval('mediaUrl("blob:test", "https://site.test/player", 1, 3)'), 'https://cdn.test/movie.m3u8');
  assert.equal(bg.eval('mediaUrl("blob:test", "https://site.test/player", 1, 0)'), 'https://site.test/player');
  capture({ tabId: 1, frameId: 3, url: 'https://cdn.test/advert.m3u8', responseHeaders: [] });
  assert.equal(bg.eval('mediaUrl("blob:test", "https://site.test/player", 1, 3)'), 'https://site.test/player');
  bg.chrome.webNavigation.onCommitted.listeners[0]({ tabId: 1, frameId: 0 });
  assert.equal(bg.eval('manifests.size'), 0);
});
test('paused, excluded and private sessions do not send video captures', async () => {
  const bg = await background();
  bg.eval('pausedHere = true');
  assert.equal(await bg.eval('sendUrl("video", "https://site.test/video", "https://site.test/")'), false);
  bg.eval('pausedHere = false; settings.excludedSites = ["site.test"]');
  assert.equal(await bg.eval('sendUrl("video", "https://cdn.test/video", "https://site.test/")'), false);
  bg.eval('settings.excludedSites = []');
  assert.equal(await bg.eval('sendUrl("video", "https://site.test/video", null, null, null, true)'), false);
});
test('format lookup returns native choices and selected quality reaches the capture', async () => {
  const bg = await background();
  bg.eval('port = { postMessage(message) { globalThis.lastSent = message; } }; appReady = true');
  const query = bg.eval('queryVideoOptions("https://site.test/video", "https://site.test/player", null, false)');
  await new Promise(setImmediate);
  const requestId = bg.eval('lastSent.id');
  bg.eval(`onAppMessage(${JSON.stringify({ type: 'videoOptions', replyTo: requestId, title: 'Video', formats: [{ label: '720p', format: 'bv*[height=720]+ba/b[height=720]', height: 720 }] })})`);
  assert.equal((await query).formats[0].label, '720p');
  const sent = bg.eval('sendUrl("video", "https://site.test/video", "https://site.test/player", "Video", null, false, "bv*[height=720]+ba/b[height=720]")');
  await new Promise(setImmediate);
  assert.equal(bg.eval('lastSent.capture.mediaFormat'), 'bv*[height=720]+ba/b[height=720]');
  bg.eval('onAppMessage({type: "received", replyTo: lastSent.id})');
  assert.equal(await sent, true);
});

test('playlist navigation uses the current watch identity, independent of the original document URL', async () => {
  const bg = await background();
  bg.eval('port = { postMessage(message) { globalThis.lastSent = message; if(message.type === "capture") onAppMessage({type:"received",replyTo:message.id}); } }; appReady = true');
  const listener = bg.chrome.runtime.onMessage.listeners[0];
  const sender = { url: 'https://www.youtube.com/watch?v=first&list=playlist', frameId: 0, tab: { id: 1, url: 'https://www.youtube.com/watch?v=first&list=playlist' } };
  for (const pageUrl of ['https://www.youtube.com/watch?v=second&list=playlist&index=2', 'https://www.youtube.com/watch?v=outside']) {
    const response = await new Promise((resolve) => listener({ type: 'video', url: 'https://cdn.test/old-track', pageUrl }, sender, resolve));
    assert.equal(response.ok, true);
    assert.equal(bg.eval('lastSent.capture.url'), `https://www.youtube.com/watch?v=${new URL(pageUrl).searchParams.get('v')}`);
  }
  bg.chrome.webRequest.onHeadersReceived.listeners[0]({ tabId: 1, frameId: 0, url: 'https://cdn.test/old.m3u8' });
  bg.chrome.webNavigation.onHistoryStateUpdated.listeners[0]({ tabId: 1, frameId: 0 });
  assert.equal(bg.eval('manifests.size'), 0);
});

test('a stale menu selection is rejected when the tab has moved to another video', async () => {
  const bg = await background();
  bg.chrome.tabs.get = async () => ({ url: 'https://www.youtube.com/watch?v=new' });
  const response = await new Promise((resolve) => bg.chrome.runtime.onMessage.listeners[0](
    { type: 'video', pageUrl: 'https://www.youtube.com/watch?v=old' },
    { url: 'https://www.youtube.com/watch?v=old', frameId: 0, tab: { id: 1 } }, resolve));
  assert.equal(response.ok, false);
  assert.match(response.error, /video changed/);
});

test('selected link batches are deduplicated, acknowledged together and respect privacy exclusions', async()=>{
  const bg=await background();
  bg.eval('port={postMessage(message){globalThis.lastSent=message;onAppMessage({type:"received",replyTo:message.id});}};appReady=true');
  const result=await bg.eval('sendBatchLinks("https://files.test/a#part01.rar https://files.test/a#same.rar https://files.test/b", "https://page.test/",false)');
  assert.equal(result.ok,true);assert.equal(bg.eval('lastSent.type'),'batchLinks');assert.equal(bg.eval('lastSent.text.split("\\n").length'),2);
  bg.eval('settings.includePrivate=false');assert.equal((await bg.eval('sendBatchLinks("https://files.test/a","https://page.test/",true)')).ok,false);
  bg.eval('settings.excludedSites=["page.test"]');assert.equal((await bg.eval('sendBatchLinks("https://files.test/a","https://page.test/",false)')).ok,false);
  bg.eval('settings.excludedSites=[];pausedHere=true');assert.equal((await bg.eval('sendBatchLinks("https://files.test/a","https://page.test/",false)')).ok,false);
  assert.equal(bg.eval('selectedUrls("https://user:password@files.test/a").length'),0);
});
