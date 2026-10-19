// Actual desktop + generated local seed. No user data or public swarm is used.
import {spawn} from 'node:child_process';
import {mkdirSync,readFileSync,writeFileSync,existsSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {DatabaseSync} from 'node:sqlite';
const {chromium}=await import(process.env.CROWDM_PLAYWRIGHT||'playwright-core');
const root=resolve(import.meta.dirname,'../..');
const out=join(root,'target',`torrent-ui-${Date.now()}`);mkdirSync(out,{recursive:true});
const seedDir=join(out,'seed');
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const seed=spawn(join(root,'target/debug/examples/torrent_seed.exe'),[seedDir],{stdio:'ignore'});
const results=[];function check(name,ok){results.push({name,ok});console.log(`${ok?'PASS':'FAIL'} ${name}`);if(!ok)throw Error(name);}
let app,browser;
try{
  for(let i=0;i<200&&!existsSync(join(seedDir,'ready.json'));i++)await delay(100);
  const fixture=JSON.parse(readFileSync(join(seedDir,'ready.json')));
  const db=new DatabaseSync(join(out,'crowdm.sqlite'));
  db.exec('CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)');
  db.prepare('INSERT INTO settings VALUES (?,?)').run('settings',JSON.stringify({downloadRoot:join(out,'downloads'),organizeByCategory:true,theme:'nightwing',notifyOnComplete:false,closeToTray:false,resumeOnStartup:false}));db.close();
  const env={...process.env,CROWDM_DATA_DIR:out,CROWDM_TEST_INSTANCE:`torrent-ui-${process.pid}`,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=19341'};
  app=spawn(join(root,'target/release/CrowDM.exe'),[fixture.path],{env,stdio:'ignore'});
  for(let i=0;i<100;i++){try{if((await fetch('http://127.0.0.1:19341/json/version')).ok)break;}catch{}await delay(200);}
  browser=await chromium.connectOverCDP('http://127.0.0.1:19341');const context=browser.contexts()[0];
  const errors=[];context.pages().forEach(p=>p.on('pageerror',e=>errors.push(e.message)));context.on('page',p=>p.on('pageerror',e=>errors.push(e.message)));
  const main=context.pages().find(p=>!p.url().includes('#capture'));
  await main.getByRole('button',{name:'Open .torrent file',exact:true}).waitFor();
  const invoke=(cmd,args={})=>main.evaluate(({cmd,args})=>window.__TAURI_INTERNALS__.invoke(cmd,args),{cmd,args});
  check('isolated desktop data directory',(await invoke('app_info')).dataDir===out);
  let popup;for(let i=0;i<150;i++){popup=context.pages().find(p=>p.url().includes('#capture'));if(popup)break;await delay(100);}
  await popup.getByRole('group',{name:/Files to download/}).waitFor();
  check('cold launch accepts a .torrent path containing spaces',await popup.getByRole('group',{name:/Files to download/}).getByRole('checkbox').count()===2);
  check('torrent metadata selects its own category',(await popup.getByRole('combobox',{name:'Category',exact:true}).getAttribute('data-value'))==='torrents');
  check('torrent category is fixed while destination remains editable',await popup.getByRole('combobox',{name:'Category',exact:true}).isDisabled()&&await popup.getByLabel('Save to',{exact:true}).isEditable());
  await popup.getByRole('button',{name:'Select none',exact:true}).click();
  check('empty selection cannot start',await popup.getByRole('button',{name:'Start download',exact:true}).isDisabled());
  await popup.getByRole('checkbox',{name:/payload.bin/}).check();
  check('only selected content counts toward size',await popup.getByRole('group',{name:'Files to download (1 of 2)',exact:true}).isVisible());
  check('capture footer stays visible',await popup.locator('.capture-foot').evaluate(el=>el.getBoundingClientRect().bottom<=innerHeight+1));
  const actualDir=await popup.locator('#dir').inputValue();
  await popup.locator('#dir').fill('C:\\Downloads\\CrowDM\\Torrents');
  await popup.screenshot({path:join(out,'torrent-confirmation.png')});
  await popup.locator('#dir').fill(actualDir);
  await popup.getByRole('button',{name:'Start download',exact:true}).click();
  let job;for(let i=0;i<400;i++){job=(await invoke('list_jobs'))[0];if(job?.state==='completed'||job?.state==='failed')break;await delay(100);}
  check('native torrent finishes with actual peer data',job?.state==='completed'&&job.torrent&&job.size===16*1024*1024);
  check('selected torrent file is byte exact',readFileSync(join(job.dir,job.fileName,'payload.bin')).equals(readFileSync(join(seedDir,'payload.bin'))));
  check('unselected file is omitted',!existsSync(join(job.dir,job.fileName,'readme.txt')));
  await invoke('set_queue_running',{running:false});
  const warm=spawn(join(root,'target/release/CrowDM.exe'),[fixture.magnet],{env,stdio:'ignore'});
  await new Promise(r=>warm.on('exit',r));
  await popup.getByRole('group',{name:/Files to download/}).waitFor();
  check('warm launch accepts magnet arguments',(await invoke('capture_list'))[0].url===fixture.magnet);
  await popup.getByRole('button',{name:'Add to queue',exact:true}).click();
  await delay(300);
  check('torrent respects held queue',(await invoke('list_jobs')).some(j=>j.state==='queued'&&j.torrent));
  await main.getByRole('button',{name:'Start queue',exact:true}).click();
  for(let i=0;i<400;i++){if((await invoke('list_jobs')).filter(j=>j.state==='completed').length===2)break;await delay(100);}
  check('queued magnet completes without overwriting first copy',(await invoke('list_jobs')).filter(j=>j.state==='completed').length===2);
  check('no active torrent remains after completion',(await invoke('list_jobs')).every(j=>j.state==='completed'&&j.connections===0));
  await main.getByRole('button',{name:'Settings',exact:true}).click();
  await main.getByRole('button',{name:'Torrents',exact:true}).click();
  check('Windows registration and default-choice controls exist',await main.getByRole('button',{name:'Register with Windows',exact:true}).isVisible()&&await main.getByRole('button',{name:'Choose defaults in Windows',exact:true}).isVisible());
  await main.getByLabel('Maximum peers per torrent',{exact:true}).fill('250');
  await main.getByLabel('Upload limit',{exact:true}).fill('2 MB');
  await main.getByRole('heading',{name:'Torrents',exact:true}).click();
  for(let i=0;i<80;i++){const s=await invoke('get_settings');if(s.torrents.peerLimit===250&&s.torrents.uploadLimit>0)break;await delay(100);}
  const s=await invoke('get_settings');
  check('torrent networking preferences persist',s.torrents.peerLimit===250&&s.torrents.uploadLimit===2*1024*1024&&s.torrents.enableUtp&&s.torrents.enableUpnp);
  await main.screenshot({path:join(out,'torrent-settings.png')});
  await main.getByRole('button',{name:/All downloads/}).click();
  await invoke('plugin:window|set_size',{label:'main',value:{Logical:{width:820,height:520}}});await delay(300);
  check('new toolbar fits a small window',await main.locator('.toolbar').evaluate(el=>el.scrollWidth<=el.clientWidth+1));
  check('no unhandled renderer errors',errors.length===0);
  console.log(`ARTIFACTS ${out}`);
}finally{
  writeFileSync(join(out,'results.json'),JSON.stringify(results,null,2));
  if(browser)await browser.close().catch(()=>{});if(app)app.kill();seed.kill();
}
