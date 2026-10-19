// Quiet selected-link control. Selection is read locally; nothing is sent
// until the user clicks. A closed shadow root isolates it from page styles.
(()=>{
  if(window.__crowdmSelection)return;window.__crowdmSelection=true;
  const B=globalThis.browser??globalThis.chrome;
  let settings={},paused=false,host,button,label,chosen=[],timer=0,sending=false,identity=location.href;
  const allowed=()=>settings.enabled!==false&&!paused&&(!B.extension?.inIncognitoContext||settings.includePrivate===true)
    &&!settings.excludedSites?.some(s=>location.hostname===s||location.hostname.endsWith(`.${s}`));
  function apply(){if(host)globalThis.CrowDMTheme?.apply(host,settings);if(!allowed())hide();}
  B.storage.local.get(["settings","pausedHere"]).then(r=>{settings=r.settings||{};paused=!!r.pausedHere;apply();});
  B.storage.onChanged.addListener(c=>{if(c.settings)settings=c.settings.newValue||{};if(c.pausedHere)paused=!!c.pausedHere.newValue;apply();});
  function urls(text){const found=[],seen=new Set();for(const m of text.matchAll(/(?:https?:\/\/|magnet:\?)[^\s<>"']+/gi)){
    const raw=m[0].replace(/&amp;/g,"&");try{const u=new URL(raw);if(u.username||u.password)continue;const key=u.protocol==="magnet:"?u.href:(u.hash="",u.href);if(!seen.has(key)){seen.add(key);found.push(raw);}if(found.length>1000)return [];}catch{}
  }return found;}
  function selection(){
    const active=document.activeElement;if(active?.closest('input,textarea,[contenteditable="true"],[contenteditable=""]'))return null;
    const s=window.getSelection();if(!s||s.isCollapsed||!s.rangeCount)return null;
    const text=s.toString();if(text.length>512*1024)return null;
    const parts=[text];const range=s.getRangeAt(0);
    // Hyperlinked labels may say “part 1” rather than showing their URL.
    const fragment=range.cloneContents();let count=0;
    for(const anchor of fragment.querySelectorAll('a[href]')){if(++count>1000)break;try{const u=new URL(anchor.getAttribute('href'),document.baseURI);if(["http:","https:","magnet:"].includes(u.protocol))parts.push(u.href);}catch{}}
    const links=urls(parts.join("\n"));
    // A multi-line Range can include full-width <li>/<p> boxes. Anchor the
    // control to selected text glyphs instead of the page's right edge.
    const common=range.commonAncestorContainer;const container=common.nodeType===Node.TEXT_NODE?common.parentNode:common;
    const walker=document.createTreeWalker(container,NodeFilter.SHOW_TEXT);const rects=[];let node,visited=0;
    while((node=walker.nextNode())&&visited++<5000){if(!node.textContent.trim()||!range.intersectsNode(node))continue;
      const textRange=document.createRange();textRange.setStart(node,node===range.startContainer?range.startOffset:0);textRange.setEnd(node,node===range.endContainer?range.endOffset:node.length);
      for(const r of textRange.getClientRects())if(r.width&&r.height)rects.push(r);
    }
    const rect=rects.length?{left:Math.min(...rects.map(r=>r.left)),right:Math.max(...rects.map(r=>r.right)),top:Math.min(...rects.map(r=>r.top)),bottom:Math.max(...rects.map(r=>r.bottom))}:range.getBoundingClientRect();
    return links.length?{links,rect}:null;
  }
  function build(){
    host=document.createElement('crowdm-selection-button');host.style.cssText='all:initial;position:fixed;z-index:2147483647;display:none;';
    const root=host.attachShadow({mode:'closed'});
    root.innerHTML=`<style>:host{--panel:#161a20;--ink:#f3f5f7;--accent:#38c1b0;--hover:#263e40}button{all:initial;display:inline-flex;align-items:center;justify-content:center;height:34px;padding:0 9px;border-radius:8px;background:var(--panel);color:var(--ink);font:600 13px/1 'Segoe UI',system-ui,sans-serif;box-shadow:0 2px 10px #0005;cursor:pointer;opacity:.3;transition:opacity .18s ease,background .15s ease}button:hover,button:focus-visible,button.busy{opacity:1;background:var(--hover)}button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}svg{width:20px;height:20px;flex:none}.label{max-width:0;opacity:0;overflow:hidden;white-space:nowrap;transition:max-width .18s ease,opacity .18s ease,margin .18s ease}button:hover .label,button:focus-visible .label,button.busy .label{max-width:260px;opacity:1;margin-left:7px}:host([data-motion="off"]) *{transition:none!important}@media(prefers-reduced-motion:reduce){:host(:not([data-reduced-motion="ignore"])) *{transition:none!important}}</style><button type="button" aria-label="Collect selected links with CrowDM"><svg viewBox="0 0 256 256" aria-hidden="true"><g fill="currentColor"><path d="M28 68 100 104V124L48 98ZM42 107 100 136V156L56 134Z"/><path d="M28 68 100 104V124L48 98ZM42 107 100 136V156L56 134Z" transform="translate(256 0) scale(-1 1)"/><path fill-rule="evenodd" d="M112 96V72L102 78 108 59C112 40 124 30 140 30H153L147 38 162 45C179 47 188 57 186 76L177 68 165 65H156L144 80V96ZM135 47 155 53 142 59Z"/><path fill="var(--accent)" d="M112 104H144V164H168L128 204 88 164H112Z"/><path d="M76 204H88V216H168V204H180V228H76Z"/></g></svg><span class="label"></span></button>`;
    button=root.querySelector('button');label=root.querySelector('.label');
    button.addEventListener('pointerdown',e=>e.preventDefault());
    button.addEventListener('click',async e=>{e.preventDefault();e.stopPropagation();if(sending||!allowed())return;
      const current=selection();if(!current||current.links.join('\n')!==chosen.join('\n')||identity!==location.href){hide();return;}
      sending=true;button.classList.add('busy');label.textContent='Opening Link Grabber…';
      try{const r=await B.runtime.sendMessage({type:'batchLinks',text:chosen.join('\n')});if(r?.ok){label.textContent='Collected in CrowDM';button.title='Links are ready to review in Link Grabber';setTimeout(hide,1400);}else{label.textContent='Could not collect links';button.title=r?.error||'Open CrowDM and try again.';}}
      catch{label.textContent='Could not connect';button.title='Open CrowDM and try again.';}
      finally{sending=false;button.classList.remove('busy');}
    });document.documentElement.appendChild(host);apply();
    new ResizeObserver(place).observe(host);
  }
  function place(){if(!host||host.style.display==='none')return;const s=selection();if(!s){hide();return;}const r=s.rect;
    if(r.bottom<0||r.top>innerHeight){hide();return;}
    const w=host.getBoundingClientRect().width||38;host.style.left=`${Math.max(8,Math.min(innerWidth-w-8,r.right+8))}px`;
    host.style.top=`${Math.max(8,Math.min(innerHeight-44,r.top))}px`;
  }
  function hide(){if(host)host.style.display='none';chosen=[];}
  function refresh(){if(sending)return;if(!allowed()){hide();return;}const s=selection();if(!s){hide();return;}if(!host)build();identity=location.href;chosen=s.links;label.textContent=`Collect ${chosen.length} link${chosen.length===1?'':'s'} with CrowDM`;button.title=label.textContent;host.style.display='block';place();}
  const schedule=()=>{clearTimeout(timer);timer=setTimeout(refresh,130);};
  document.addEventListener('selectionchange',schedule);document.addEventListener('pointerup',schedule);document.addEventListener('keyup',schedule);
  addEventListener('scroll',()=>{if(!sending)place();},true);addEventListener('resize',place);
  document.addEventListener('keydown',e=>{if(e.key==='Escape')hide();});
  for(const event of ['popstate','hashchange','yt-navigate-start'])addEventListener(event,hide,true);
  setInterval(()=>{if(identity!==location.href){identity=location.href;hide();}},300);
})();
