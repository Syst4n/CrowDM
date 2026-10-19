// Run only when the project owner explicitly says to send this update to users.
// This prepares the reviewed manifest. Commit/push it as a separate action.
import {readFileSync,writeFileSync} from 'node:fs';
const [version,notesPath]=process.argv.slice(2);
if(!/^\d+\.\d+\.\d+$/.test(version??'')||!notesPath)throw Error('Usage: node scripts/announce-update.mjs VERSION WHATS_NEW.json');
const whatsNew=JSON.parse(readFileSync(notesPath,'utf8'));
if(!Array.isArray(whatsNew)||!whatsNew.length||whatsNew.length>40||whatsNew.some(n=>typeof n!=='string'||n.length>1000))throw Error('Provide a JSON array of 1–40 short plain-text release notes.');
const r=await fetch(`https://api.github.com/repos/Syst4n/CrowDM/releases/tags/v${version}`,{headers:{'User-Agent':'CrowDM-announcement'}});
if(!r.ok)throw Error(`Release lookup failed: ${r.status}`);
const release=await r.json();
const name=`CrowDM_${version}_x64-setup.exe`,asset=release.assets.find(a=>a.name===name);
if(release.draft||!asset?.digest?.match(/^sha256:[a-f0-9]{64}$/))throw Error('Publish the tested installer with a GitHub SHA-256 asset digest first.');
const installerUrl=`https://github.com/Syst4n/CrowDM/releases/download/v${version}/${name}`;
if(asset.browser_download_url!==installerUrl)throw Error('Unexpected installer asset URL.');
const extension=JSON.parse(readFileSync(new URL('../extensions/package.json',import.meta.url))).version;
if(extension!==version)throw Error('Build and test a matching extension before announcing this version.');
const feed={schema:1,release:{version,whatsNew,installerUrl,sha256:asset.digest.slice(7),extensionVersion:extension}};
writeFileSync(new URL('../updates/windows-x64.json',import.meta.url),JSON.stringify(feed,null,2)+'\n');
console.log(`Prepared ${version} announcement. Review the feed, then commit and push it only with the owner’s explicit approval to send this update to users.`);
