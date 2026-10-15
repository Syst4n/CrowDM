// Builds the unpacked Chromium (Chrome, Edge) and Firefox extensions into
// build/chromium and build/firefox from the shared sources in src/.
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const { version } = JSON.parse(readFileSync(join(here, "package.json"), "utf8"));

// Public key only: it fixes the Chromium extension id to
// pomhhnfijfeecghaflldjibhaofbiabm, which the native host manifest allows.
const CHROMIUM_KEY = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA7a/K4yh3JEDWd+Idd4Lrn4lY9qijKefbz+gJ/p5jDHYKFksVQr137Q83fUYoWTeOBejH3esu+UCxi4dyuzEqn+agOaAOy/bREDq3BAaLa8qKdiIF6b/9IaeC1vp2LsKoiEZ7MQ7d5qOWNZjQ1Y+fVdG9QxmTHcCcDq4eu/vEtK1hp9lWhBV4u7vX9zxEB0FSOrlCfeX7bYcFPMK8ZKj1NK51cH9eIMzlHAc2xdGDlThV8ebM4O1TyxoFOorQxnvG/JTp9j7vLch1QjqRWVDtb1bTZrSKjUTwCm+IkA2wY5wUcfCtjdiFoJfHIQmz308MX6MIWNvjPrJa3KdnrNNt6QIDAQAB";

const common = {
  manifest_version: 3,
  name: "CrowDM",
  short_name: "CrowDM",
  version,
  description: "Sends your downloads to CrowDM, the download manager for Windows.",
  icons: { 16: "icons/icon-16.png", 32: "icons/icon-32.png", 48: "icons/icon-48.png", 128: "icons/icon-128.png" },
  permissions: ["downloads", "nativeMessaging", "contextMenus", "cookies", "storage", "webRequest", "webNavigation"],
  host_permissions: ["<all_urls>"],
  action: { default_title: "CrowDM", default_popup: "popup.html", default_icon: { 16: "icons/icon-16.png", 32: "icons/icon-32.png" } },
  content_scripts: [{ matches: ["http://*/*", "https://*/*"], js: ["themes.js", "video.js"], all_frames: true, run_at: "document_idle" }],
};

const targets = {
  chromium: {
    ...common,
    key: CHROMIUM_KEY,
    minimum_chrome_version: "116",
    background: { service_worker: "background.js" },
  },
  firefox: {
    ...common,
    background: { scripts: ["background.js"] },
    browser_specific_settings: { gecko: { id: "crowdm@syst4n.github.io", strict_min_version: "128.0" } },
  },
};

for (const [name, manifest] of Object.entries(targets)) {
  const out = join(here, "build", name);
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  cpSync(join(here, "src"), out, { recursive: true });
  const palettes = JSON.parse(readFileSync(join(here, "../assets/themes.json"), "utf8"));
  writeFileSync(join(out, "themes.js"), `globalThis.CROWDM_PALETTES = ${JSON.stringify(palettes)};\n` + readFileSync(join(here, "src/theme-runtime.js"), "utf8"));
  writeFileSync(join(out, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
  console.log("built", out);
}
