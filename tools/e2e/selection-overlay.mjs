// Component test for the selected-link control, without touching clipboard or user profiles.
import {createServer} from 'node:http';
import {readFileSync,mkdirSync,writeFileSync} from 'node:fs';
import {join,resolve} from 'node:path';
const {chromium}=await import(process.env.CROWDM_PLAYWRIGHT||'playwright-core');
const root=resolve(import.meta.dirname,'../..'),out=join(root,'target',`selection-overlay-${Date.now()}`);mkdirSync(out,{recursive:true});
const server=createServer((req,res)=>{res.writeHead(200,{'Content-Type':'text/html'});res.end('<html><body style="margin:40px;font:16px Segoe UI;background:#0d131c;color:#e3eaf3"><h2>Multipart download links</h2><div id="links"><p>https://files.example.test/a#Sample.part01.rar</p><p>https://files.example.test/b#Sample.part02.rar</p><p>https://files.example.test/a#duplicate.rar</p></div><div id="labels"><a href="/first.zip">First part</a> <a href="/second.zip">Second part</a></div><textarea>https://private.example.test/a</textarea></body></html>');});await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await chromium.launch({headless:true,executablePath:process.env.CROWDM_CHROMIUM});const results=[];
function check(name,ok){results.push({name,ok});console.log(`${ok?'PASS':'FAIL'} ${name}`);if(!ok)throw Error(name);}
try{
  const page=await browser.newPage({viewport:{width:960,height:640}});await page.goto(`http://127.0.0.1:${server.address().port}`);const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.evaluate(()=>{const attach=Element.prototype.attachShadow;Element.prototype.attachShadow=function(o){return attach.call(this,this.tagName==='CROWDM-SELECTION-BUTTON'?{...o,mode:'open'}:o);};window.__messages=[];window.chrome={extension:{inIncognitoContext:false},storage:{local:{get:async()=>({settings:{enabled:true,theme:'nightwing'}})},onChanged:{addListener(fn){window.__settings=fn;}}},runtime:{sendMessage:async m=>{window.__messages.push(m);return {ok:true};}}};});
  await page.addScriptTag({content:readFileSync(join(root,'extensions/build/chromium/themes.js'),'utf8')});await page.addScriptTag({content:readFileSync(join(root,'extensions/src/selection.js'),'utf8')});
  const select=async id=>{await page.evaluate(id=>{document.activeElement?.blur();const r=document.createRange();r.selectNodeContents(document.getElementById(id));const s=getSelection();s.removeAllRanges();s.addRange(r);},id);await page.waitForTimeout(220);};
  await select('links');const button=page.getByRole('button',{name:'Collect selected links with CrowDM',exact:true});await button.waitFor();
  check('selection alone sends nothing',await page.evaluate(()=>window.__messages.length===0));
  check('selected-link icon is faint and compact',await button.evaluate(e=>+getComputedStyle(e).opacity<.4&&e.getBoundingClientRect().width<40));
  check('selected-link control stays inside viewport',await button.evaluate(e=>{const r=e.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&r.top>=0&&r.bottom<=innerHeight;}));
  check('icon sits beside selected text rather than the full-width block',await button.evaluate(e=>e.getBoundingClientRect().left<500));
  await page.screenshot({path:join(out,'selected-links.png')});await button.hover();await page.waitForTimeout(250);
  check('hover expands the count at full opacity',await button.evaluate(e=>+getComputedStyle(e).opacity===1&&e.textContent.includes('Collect 2 links')));
  const selection=await page.evaluate(()=>getSelection().toString());await button.click();
  check('one click sends the deduplicated batch',await page.evaluate(()=>window.__messages.length===1&&window.__messages[0].type==='batchLinks'&&window.__messages[0].text.split('\n').length===2));
  check('click preserves the highlighted page text',await page.evaluate(()=>getSelection().toString())===selection);
  await page.waitForTimeout(1500);await select('labels');await button.waitFor();await button.click();
  check('hyperlinked labels send their target addresses',await page.evaluate(()=>window.__messages.at(-1).text.includes('/first.zip')&&window.__messages.at(-1).text.includes('/second.zip')));
  await page.waitForTimeout(1500);await select('links');await page.evaluate(()=>window.__settings({pausedHere:{newValue:true}}));
  check('paused integration hides the control',await button.isHidden());await page.evaluate(()=>window.__settings({pausedHere:{newValue:false},settings:{newValue:{enabled:true,appearance:{mode:'light',palette:'rose'}}}}));await select('labels');
  check('selection logo follows app mode and palette',await page.locator('crowdm-selection-button').evaluate(e=>e.dataset.theme==='light'&&e.dataset.palette==='rose'));
  await page.locator('textarea').focus();await page.locator('textarea').evaluate(e=>e.select());await page.waitForTimeout(250);check('editable input text never gets a selection button',await button.isHidden());
  await select('links');await page.evaluate(()=>history.pushState({},'','/next'));await page.waitForTimeout(450);check('navigation hides stale selection controls',await button.isHidden());
  await page.evaluate(()=>window.chrome.extension.inIncognitoContext=true);await select('links');check('private selections are excluded by default',await button.isHidden());
  check('no renderer errors',errors.length===0);console.log(`ARTIFACTS ${out}`);
}finally{writeFileSync(join(out,'results.json'),JSON.stringify(results,null,2));await browser.close();server.close();}
