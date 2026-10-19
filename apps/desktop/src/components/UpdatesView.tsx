import { useEffect, useState } from "react";
import { api, errorText, type AnnouncedUpdate, type DownloadCounts, type BrowserStatus } from "../lib/api";
import { useStore } from "../lib/store";

let cached: AnnouncedUpdate | null = null;
export function UpdateNotice() {
  const settings = useStore(s=>s.settings);
  const [release,setRelease] = useState<AnnouncedUpdate|null>(cached);
  const [expanded,setExpanded] = useState(false);
  useEffect(()=>{
    if(!settings?.updates.checkAutomatically)return;
    let alive=true;
    const check=()=>{const last=Number(localStorage.getItem("crowdm.lastUpdateCheck")??0);if(Date.now()-last<24*60*60*1000)return;
      void api.checkUpdate().then(r=>{localStorage.setItem("crowdm.lastUpdateCheck",String(Date.now()));cached=r;if(alive)setRelease(r);}).catch(()=>{});};
    const timer=window.setTimeout(check,5000);const interval=window.setInterval(check,60*60*1000);
    return()=>{alive=false;clearTimeout(timer);clearInterval(interval);};
  },[settings?.updates.checkAutomatically]);
  if(!release||settings?.updates.dismissedVersion===release.version)return null;
  return <div className="update-banner"><span>CrowDM {release.version} is available</span><button className="btn ghost" onClick={()=>setExpanded(v=>!v)}>What’s New</button><button className="btn ghost" onClick={()=>{
    if(!settings)return;void api.saveSettings({...settings,updates:{...settings.updates,dismissedVersion:release.version}}).then(s=>useStore.getState().setSettings(s)).catch(e=>useStore.getState().showToast(errorText(e),"error"));
  }}>Later</button>{expanded&&<div className="update-banner-detail"><ReleaseDetails release={release}/></div>}</div>;
}
function ReleaseDetails({release}:{release:AnnouncedUpdate}){
  const [id,setId]=useState<string|null>(null);
  const [error,setError]=useState("");
  const [busy,setBusy]=useState(false);
  const job=useStore(s=>id?s.jobs[id]:undefined);
  const action=async()=>{setBusy(true);setError("");try{if(job?.state==="completed"){await api.installUpdate(job.id);}else{setId(await api.downloadUpdate());}}catch(e){setError(errorText(e));}finally{setBusy(false);}};
  return <div className="release-details"><h3>What’s New in {release.version}</h3><ul>{release.whatsNew.map((s,i)=><li key={i}>{s}</li>)}</ul><button className="btn primary" disabled={busy||job?.state==="downloading"} onClick={()=>void action()}>{job?.state==="completed"?"Pause downloads and open installer":job?.state==="downloading"?"Downloading and verifying…":"Download verified installer"}</button><p className="hint">You choose when to install. The installer preserves your history and settings. Extension version for this update: {release.extensionVersion}.</p>{error&&<p role="alert">{error}</p>}</div>;
}
export function UpdatesView(){
  const settings=useStore(s=>s.settings);
  const [version,setVersion]=useState("");
  const [release,setRelease]=useState<AnnouncedUpdate|null>(cached);
  const [status,setStatus]=useState("");
  const [busy,setBusy]=useState(false);
  const [counts,setCounts]=useState<DownloadCounts|null>(null);
  const [browsers,setBrowsers]=useState<BrowserStatus|null>(null);
  useEffect(()=>{void api.appInfo().then(i=>setVersion(i.version));void api.browserStatus().then(setBrowsers).catch(()=>{});},[]);
  const check=async()=>{setBusy(true);setStatus("");try{const r=await api.checkUpdate();cached=r;setRelease(r);setStatus(r?`Version ${r.version} is available.`:"You have the latest announced update. Preview builds on GitHub may be newer.");}catch(e){setStatus(errorText(e));}finally{setBusy(false);}};
  return <section><h2>Updates and downloads</h2><p>Installed version: <strong>{version}</strong></p><label className="check"><input type="checkbox" checked={settings?.updates.checkAutomatically??true} onChange={e=>{if(settings)void api.saveSettings({...settings,updates:{...settings.updates,checkAutomatically:e.target.checked}}).then(s=>useStore.getState().setSettings(s)).catch(e=>setStatus(errorText(e)));}}/><span>Check for announced updates daily<div className="hint">One quiet in-app notice. No automatic installation. Ordinary commits and unannounced previews do not trigger it.</div></span></label><div><button className="btn" disabled={busy} onClick={()=>void check()}>Check for updates</button></div>{status&&<p role="status">{status}</p>}{release&&<ReleaseDetails release={release}/>}
  <h3>Browser extension updates</h3><p className="hint">App updates include matching extension files. Unpacked Chrome/Edge extensions need a manual reload; Firefox temporary extensions need to be loaded again. A future store release can update extensions automatically.</p>
  <p>Bundled extension: {version}</p>{browsers?.connected.map((b,i)=><p key={i}>{b.browser}: {b.extensionVersion}{b.extensionVersion!==version?" — update or reload recommended":" — up to date"}</p>)}{!browsers?.connected.length&&<p className="hint">No browser connected. Connect CrowDM’s extension to check its version.</p>}
  <ol className="extension-steps"><li>Install the app update first.</li><li>Open the extension files below. For Chrome or Edge, open Manage extensions, enable Developer mode, then reload CrowDM. If its folder moved, use Load unpacked and choose the chromium folder.</li><li>For Firefox, open about:debugging → This Firefox → Load Temporary Add-on, then choose firefox/manifest.json.</li><li>Refresh video tabs that were already open, then return here to check the connected version.</li></ol><button className="btn" onClick={()=>void api.openExtensionsFolder().catch(e=>setStatus(errorText(e)))}>Open extension files</button>
  <h3>Community download counter</h3><p className="hint">Public GitHub installer and portable download counts, by version. Downloads can include repeat downloads and automated requests. Active users and installed-version counts are unavailable; CrowDM sends no usage reports.</p><button className="btn" disabled={busy} onClick={()=>{setBusy(true);void api.downloadCounts().then(setCounts).catch(e=>setStatus(errorText(e))).finally(()=>setBusy(false));}}>Refresh download counts</button>{counts&&<><p><strong>{counts.total.toLocaleString()}</strong> app downloads on GitHub</p><table className="counts-table"><thead><tr><th>Version</th><th>Downloads</th></tr></thead><tbody>{counts.versions.map(v=><tr key={v.version}><td>{v.version}</td><td>{v.downloads.toLocaleString()}</td></tr>)}</tbody></table></>}
  </section>;
}
