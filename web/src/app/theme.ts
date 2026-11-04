/**
 * Applies dashboard settings to the document: theme, accent colour, title
 * and custom CSS.
 */
import { useEffect } from "react";
import type { Background, Settings } from "../api/types";

/** A CSS `url()` for `url`, with characters that would break out of it removed. */
export function cssUrl(url: string): string {
  return `url("${url.replace(/["\\\n\r]/g, "")}")`;
}

/** CSS `filter` for a background's blur, saturation and brightness; `""` if none. */
export function backgroundFilter(bg: Background): string {
  const parts: string[] = [];
  if (bg.blur > 0) parts.push(`blur(${bg.blur}px)`);
  if (bg.saturate !== 100) parts.push(`saturate(${bg.saturate}%)`);
  if (bg.brightness !== 100) parts.push(`brightness(${bg.brightness}%)`);
  return parts.join(" ");
}

/** Id of the <style> element holding the user's custom CSS. */
export const CUSTOM_CSS_ID = "homedit-custom-css";

/** Applies `settings` to `<html>` and `<head>`; safe to call repeatedly. */
export function applySettings(
  settings: Settings,
  root: HTMLElement = document.documentElement,
): void {
  if (settings.theme === "system") delete root.dataset.theme;
  else root.dataset.theme = settings.theme;
  if (CSS.supports?.("color", settings.accent)) root.style.setProperty("--accent", settings.accent);
  else root.style.removeProperty("--accent");
  if (settings.title) document.title = settings.title;
  if (settings.cardBlur > 0) {
    root.style.setProperty("--card-blur", `${settings.cardBlur}px`);
    root.dataset.cardBlur = "";
  } else {
    root.style.removeProperty("--card-blur");
    delete root.dataset.cardBlur;
  }
  if (settings.background?.url) root.dataset.background = "";
  else delete root.dataset.background;
  let style = document.getElementById(CUSTOM_CSS_ID) as HTMLStyleElement | null;
  if (!style) {
    style = document.createElement("style");
    style.id = CUSTOM_CSS_ID;
    document.head.appendChild(style);
  }
  if (style.textContent !== settings.customCss) style.textContent = settings.customCss;
}

/** Keeps the document in sync with `settings` (live as they are edited). */
export function useApplySettings(settings: Settings | undefined): void {
  useEffect(() => {
    if (settings) applySettings(settings);
  }, [settings]);
}
