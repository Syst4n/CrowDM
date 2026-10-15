// This runtime is combined with assets/themes.json by the extension builder.
// Both extension surfaces and desktop use the same palette definitions.
(() => {
  const palettes = globalThis.CROWDM_PALETTES;
  const targets = new Map();
  const media = matchMedia("(prefers-color-scheme: dark)");
  function apply(target, settings) {
    targets.set(target, settings);
    const legacy = settings.theme || "system";
    const appearance = settings.appearance || {
      mode: legacy === "light" || legacy === "sand" ? "light" : legacy === "system" ? "system" : "dark",
      palette: ["nightwing", "ocean", "forest", "sand"].includes(legacy) ? legacy : "classic",
    };
    const mode = appearance.mode === "system" ? media.matches ? "dark" : "light" : appearance.mode === "light" ? "light" : "dark";
    const palette = Object.hasOwn(palettes, appearance.palette) ? appearance.palette : "classic";
    const c = palettes[palette][mode];
    target.dataset.theme = mode; target.dataset.palette = palette;
    target.dataset.motion = settings.animations === false ? "off" : "on";
    target.dataset.reducedMotion = settings.respectReducedMotion === false ? "ignore" : "respect";
    const values = { "--bg": c.surface, "--panel": c.surface, "--ink": c.text, "--text": c.text, "--muted": c.muted, "--edge": `color-mix(in srgb, ${c.text} 26%, ${c.surface})`, "--border": `color-mix(in srgb, ${c.text} 26%, ${c.surface})`, "--hover": c.surface2, "--accent": c.accent, "--on-accent": mode === "dark" ? c.bg : "#ffffff" };
    for (const [key, value] of Object.entries(values)) target.style.setProperty(key, value);
    target.style.colorScheme = mode;
  }
  media.addEventListener("change", () => { for (const [target, settings] of targets) apply(target, settings); });
  globalThis.CrowDMTheme = { apply };
})();
