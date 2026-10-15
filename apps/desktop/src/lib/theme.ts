import { useEffect } from "react";
import type { Settings } from "./api";
import { effectiveAppearance, paletteStyle } from "./appearance";

/** Applies the theme setting (following Windows when "system"). */
export function useTheme(settings: Settings | undefined | null) {
  const { mode, palette } = effectiveAppearance(settings ?? undefined);
  const animations = settings?.animations !== false;
  const respectReducedMotion = settings?.respectReducedMotion !== false;
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    const apply = () => {
      const root = document.documentElement;
      const resolved = mode === "system" ? mq.matches ? "dark" : "light" : mode;
      const previous = `${root.dataset.theme}:${root.dataset.palette}`;
      root.dataset.theme = resolved;
      root.dataset.mode = mode;
      root.dataset.palette = palette;
      root.dataset.motion = animations ? "on" : "off";
      root.dataset.reducedMotion = respectReducedMotion ? "respect" : "ignore";
      for (const [key, value] of Object.entries(paletteStyle(palette, resolved))) {
        if (key === "colorScheme") root.style.colorScheme = resolved;
        else root.style.setProperty(key, String(value));
      }
      if (animations && (!respectReducedMotion || !reduced.matches) && previous !== "undefined:undefined" && previous !== `${resolved}:${palette}`) {
        root.animate([{ opacity: .86 }, { opacity: 1 }], { duration: 260, easing: "ease-out" });
      }
    };
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [mode, palette, animations, respectReducedMotion]);
}
