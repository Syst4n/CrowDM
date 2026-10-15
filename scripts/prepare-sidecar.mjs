// Builds crowdm-host and places it where the Tauri bundler expects the
// sidecar (src-tauri/binaries/crowdm-host-<target-triple>.exe). Also builds
// the browser extensions that the installer ships as resources.
//
// Usage: node scripts/prepare-sidecar.mjs [--release]
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const release = process.argv.includes("--release");
const run = (cmd, args, cwd = root) => execFileSync(cmd, args, { cwd, stdio: "inherit" });

const triple = execFileSync("rustc", ["-vV"], { encoding: "utf8" })
  .split("\n")
  .find((l) => l.startsWith("host:"))
  .slice(5)
  .trim();

run("cargo", ["build", "-p", "crowdm-host", ...(release ? ["--release"] : [])]);
const exe = process.platform === "win32" ? ".exe" : "";
const built = join(root, "target", release ? "release" : "debug", `crowdm-host${exe}`);
const outDir = join(root, "apps", "desktop", "src-tauri", "binaries");
mkdirSync(outDir, { recursive: true });
copyFileSync(built, join(outDir, `crowdm-host-${triple}${exe}`));
console.log(`sidecar ready for ${triple}`);

run("node", ["build.mjs"], join(root, "extensions"));
if (process.platform === "win32") {
  // Prefer the runner's PowerShell 7. Launching Windows PowerShell from it
  // can inherit a module path that hides built-in commands such as Get-FileHash.
  let powershell = "pwsh.exe";
  try { execFileSync(powershell, ["-NoProfile", "-NonInteractive", "-Command", "exit 0"], { stdio: "ignore" }); }
  catch { powershell = "powershell.exe"; }
  run(powershell, ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", join(root, "scripts", "prepare-media.ps1")]);
}
