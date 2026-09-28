const B = globalThis.browser ?? globalThis.chrome;
const stateEl = document.getElementById("state");
const catchEl = document.getElementById("catch");
const noteEl = document.getElementById("note");

function render(s) {
  if (!s) return;
  if (s.connected) {
    stateEl.textContent = "Connected to CrowDM";
    stateEl.className = "state ok";
  } else {
    stateEl.textContent = s.error ? `Not connected: ${s.error}` : "CrowDM starts when you download something.";
    stateEl.className = "state";
  }
  catchEl.checked = s.settings.enabled && !s.pausedHere;
  catchEl.disabled = !s.settings.enabled;
  noteEl.textContent = s.settings.enabled ? "" : "Catching is turned off in CrowDM's settings.";
}

B.runtime.sendMessage({ type: "getStatus" }).then(render);
B.runtime.onMessage.addListener((m) => m?.type === "status" && render(m.status));
catchEl.addEventListener("change", () => B.runtime.sendMessage({ type: "setPausedHere", paused: !catchEl.checked }).then(render));
document.getElementById("open").addEventListener("click", () => {
  B.runtime.sendMessage({ type: "openApp" });
  window.close();
});
