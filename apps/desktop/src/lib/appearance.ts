import themes from "../../../../assets/themes.json";
import type { Appearance, Palette, Settings } from "./api";

export { themes };
export function effectiveAppearance(s: Pick<Settings, "theme" | "appearance"> | undefined): Appearance {
  if (s?.appearance) return s.appearance;
  const theme = s?.theme ?? "system";
  return {
    mode: theme === "light" || theme === "sand" ? "light" : theme === "system" ? "system" : "dark",
    palette: ["nightwing", "ocean", "forest", "sand"].includes(theme) ? theme as Palette : "classic",
  };
}

export function paletteStyle(palette: Palette, mode: "light" | "dark") {
  const c = themes[palette][mode];
  return {
    "--bg": c.bg, "--surface": c.surface, "--surface-2": c.surface2,
    "--surface-3": `color-mix(in srgb, ${c.text} 12%, ${c.surface})`,
    "--text": c.text, "--text-2": c.muted,
    "--text-3": `color-mix(in srgb, ${c.text} 65%, ${c.surface})`,
    "--border": `color-mix(in srgb, ${c.text} 14%, ${c.surface})`,
    "--border-strong": `color-mix(in srgb, ${c.text} 26%, ${c.surface})`,
    "--accent": c.accent,
    "--accent-hover": `color-mix(in srgb, ${c.accent} 86%, ${mode === "dark" ? "white" : "black"})`,
    "--accent-text": mode === "dark" ? c.bg : "#ffffff",
    "--accent-soft": `color-mix(in srgb, ${c.accent} 13%, transparent)`,
    "--accent-soft-2": `color-mix(in srgb, ${c.accent} 24%, transparent)`,
    "--track": `color-mix(in srgb, ${c.text} 12%, ${c.surface})`,
    "--logo-ink": c.text, "--logo-accent": c.accent,
    colorScheme: mode,
  } as React.CSSProperties;
}
