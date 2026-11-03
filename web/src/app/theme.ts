/**
 * Applies dashboard settings to the document: theme, accent colour, title
 * and custom CSS.
 */
import { useEffect } from "react";
import type { Settings } from "../api/types";

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
