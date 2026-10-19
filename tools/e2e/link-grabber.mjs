// Native WebView2 + local HTTP server. Never touches installed user data.
import {spawn} from 'node:child_process';
import {createServer} from 'node:http';
import {mkdirSync,readFileSync,writeFileSync,existsSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {DatabaseSync} from 'node:sqlite';
const {chromium}=await import(process.env.CROWDM_PLAYWRIGHT||'playwright-core');
const root=resolve(import.meta.dirname,'../..'),out=join(root,'target',`link-grabber-${Date.now()}`);mkdirSync(out,{recursive:true});
const payload=Buffer.from(Array.from({length:512*1024},(_,i)=>i%251));
const server=createServer((req,res)=>{
  const path=new URL(req.url,'http://localhost').pathname;
  if(path==='/links'){res.writeHead(200,{'Content-Type':'text/html'});return res.end('<a href="/files/Sample.part01.rar">1</a><a href="/files/Sample.part02.rar">2</a><a href="/about">About</a>');}
  if(path==='/host/a'){res.writeHead(200,{'Content-Type':'text/html'});return res.end('<button onclick="window.open(\'/files/Sample.part01.rar\')">Download</button>');}
  if(path==='/login'){res.writeHead(200,{'Content-Type':'text/html'});return res.end('<html><body>Sign in</body></html>');}
  if(!path.startsWith('/files/')){res.writeHead(404);return res.end();}
  const match=req.headers.range?.match(/bytes=(\d+)-(\d*)/);const start=match?Number(match[1]):0,end=match?.[2]?Math.min(Number(match[2]),payload.length-1):payload.length-1;
  res.writeHead(match?206:200,{'Content-Type':'application/octet-stream','Content-Disposition':`attachment; filename="${path.split('/').at(-1)}"`,'Content-Length':end-start+1,'Accept-Ranges':'bytes',...(match?{'Content-Range':`bytes ${start}-${end}/${payload.length}`}:{})});res.end(payload.subarray(start,end+1));
});await new Promise(r=>server.listen(0,'127.0.0.1',r));const base=`http://127.0.0.1:${server.address().port}`;
const db=new DatabaseSync(join(out,'crowdm.sqlite'));db.exec('CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)');db.prepare('INSERT INTO settings VALUES (?,?)').run('settings',JSON.stringify({downloadRoot:join(out,'downloads'),notifyOnComplete:false,closeToTray:false,resumeOnStartup:false,theme:'nightwing',updates:{checkAutomatically:false}}));db.close();
const results=[];function check(name,ok){results.push({name,ok});console.log(`${ok?'PASS':'FAIL'} ${name}`);if(!ok)throw Error(name);}
const delay=ms=>new Promise(r=>setTimeout(r,ms));const env={...process.env,CROWDM_DATA_DIR:out,CROWDM_TEST_INSTANCE:`grabber-${process.pid}`,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=19342'};let app,browser,main;
async function launch(){app=spawn(join(root,'target/release/CrowDM.exe'),[],{env,stdio:'ignore'});for(let i=0;i<150;i++){try{if((await fetch('http://127.0.0.1:19342/json/version')).ok)break;}catch{}await delay(200);}browser=await chromium.connectOverCDP('http://127.0.0.1:19342');main=browser.contexts()[0].pages().find(p=>!p.url().includes('#capture'));await main.getByRole('button',{name:/^Link Grabber/}).waitFor();}
const invoke=(cmd,args={})=>main.evaluate(({cmd,args})=>window.__TAURI_INTERNALS__.invoke(cmd,args),{cmd,args});
async function settled(){for(let i=0;i<300;i++){const d=await invoke('batch_list');if(!d.checking)return d;await delay(100);}throw Error('checking timed out');}
try{
  await launch();const errors=[];main.on('pageerror',e=>errors.push(e.message));await invoke('set_queue_running',{running:false});
  await main.getByRole('button',{name:/^Link Grabber/}).click();
  await main.getByLabel('Links or copied link text',{exact:true}).fill(`${base}/host/a#Sample.part01.rar\n${base}/files/Sample.part02.rar\n${base}/files/Sample.part02.rar\n${base}/missing.zip\n${base}/login`);
  await main.getByRole('button',{name:'Analyze links',exact:true}).click();let d=await settled();
  check('duplicate clipboard lines are deduplicated',d.links.length===4);
  check('landing-page file resolver finds the real file',d.links.some(l=>l.status==='ready'&&l.url.endsWith('/files/Sample.part01.rar')));
  check('HTTP failure and HTML login pages remain unselected',d.links.filter(l=>l.status==='error').length===2);
  check('related archive parts form one package',d.links.filter(l=>l.status==='ready').every(l=>l.package==='Sample'));
  check('checking starts no download or capture popup',(await invoke('list_jobs')).length===0&&(await invoke('capture_list')).length===0);
  // Source landing-page duplicates must remain duplicates after URL resolution.
  await invoke('batch_analyze',{text:`${base}/host/a#Sample.part01.rar`});d=await settled();check('resolved landing link is still deduplicated',d.links.length===4);
  const saved=JSON.parse(readFileSync(join(out,'link-grabber.json')));check('inbox is persisted before restarting',saved.links.length===4&&!saved.checking);
  await browser.close();browser=null;app.kill();await new Promise(r=>app.on('exit',r));await delay(1200);await launch();await main.getByRole('button',{name:/^Link Grabber/}).click();d=await invoke('batch_list');check('inbox survives desktop restart',d.links.length===4&&d.links.filter(l=>l.status==='ready').length===2);
  await main.getByRole('button',{name:'Hold queue',exact:true}).count().catch(()=>0);await invoke('set_queue_running',{running:false});
  await main.getByRole('button',{name:'Add selected to queue',exact:true}).click();
  for(let i=0;i<30&&(await invoke('list_jobs')).length!==2;i++)await delay(100);
  let jobs=await invoke('list_jobs');check('only ready files enter the held queue',jobs.length===2&&jobs.every(j=>j.state==='queued'));
  check('all parts share the package folder',jobs.every(j=>j.package==='Sample'&&j.dir===join(out,'downloads','Sample')));
  await main.locator('.package-card summary').first().click();await main.getByRole('button',{name:'Resume package',exact:true}).click();await delay(200);check('package resume respects the held queue',(await invoke('list_jobs')).every(j=>j.state==='queued'));await main.getByRole('button',{name:'Start queue',exact:true}).click();for(let i=0;i<150;i++){jobs=await invoke('list_jobs');if(jobs.every(j=>j.state==='completed'))break;await delay(100);}
  check('all parts complete byte exactly',jobs.every(j=>j.state==='completed'&&readFileSync(join(j.dir,j.fileName)).equals(payload)));
  check('parts are kept together without extracting or combining',existsSync(join(out,'downloads','Sample','Sample.part01.rar'))&&existsSync(join(out,'downloads','Sample','Sample.part02.rar')));
  await main.getByRole('button',{name:'Packages (1)',exact:true}).click();if(!await main.getByRole('button',{name:'Pause package',exact:true}).isVisible())await main.locator('.package-card summary').first().click();check('package controls are available',await main.getByRole('button',{name:'Pause package',exact:true}).isVisible());
  await main.screenshot({path:join(out,'packages.png')});
  await main.getByRole('button',{name:'Collected links (2)',exact:true}).click();await invoke('batch_remove',{ids:(await invoke('batch_list')).links.map(l=>l.id)});
  await main.getByLabel('Or collect links from a page',{exact:true}).fill(`${base}/links`);await main.getByRole('button',{name:'Collect links from page',exact:true}).click();d=await settled();check('page collector collects parts but skips navigation',d.links.length===2&&d.links.every(l=>l.status==='duplicate'));
  await main.getByRole('button',{name:'Settings',exact:true}).click();await main.getByRole('button',{name:'Batch downloads',exact:true}).click();check('quiet batch defaults and opt-in clipboard monitor',!(await invoke('get_settings')).batch.monitorClipboard&&(await invoke('get_settings')).batch.quietNotifications);
  await main.getByLabel('Simultaneous link checks',{exact:true}).fill('6');await main.getByRole('heading',{name:'Batch downloads',exact:true}).click();await delay(300);check('batch preferences persist',(await invoke('get_settings')).batch.checkConcurrency===6);
  await main.screenshot({path:join(out,'batch-settings.png')});
  await main.getByRole('button',{name:'Updates and downloads',exact:true}).click();check('update and extension guidance are present',await main.getByRole('button',{name:'Check for updates',exact:true}).isVisible()&&await main.getByRole('button',{name:'Open extension files',exact:true}).isVisible());
  check('download counter distinguishes downloads from active users',(await main.textContent('section')).includes('Active users and installed-version counts are unavailable'));
  await main.screenshot({path:join(out,'update-settings.png')});check('no renderer errors',errors.length===0);console.log(`ARTIFACTS ${out}`);
}finally{writeFileSync(join(out,'results.json'),JSON.stringify(results,null,2));if(browser)await browser.close().catch(()=>{});app?.kill();server.close();}
