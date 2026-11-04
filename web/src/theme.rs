//! The dashboard's look on the kit's themes: dark is the kit's default (Encore), light is the kit's light theme,
//! system follows the browser between the two. A picked accent replaces the theme's own; the background, the
//! frosted cards and the custom CSS go on top.

use crate::model::{Background, Settings, ThemeMode};

/// The kit theme for a mode and the browser's preference: `None` is the default (Encore), else `light`.
pub fn kit_theme(mode: ThemeMode, prefers_light: bool) -> Option<&'static str> {
    match mode {
        ThemeMode::Dark => None,
        ThemeMode::Light => Some("light"),
        ThemeMode::System => prefers_light.then_some("light"),
    }
}

/// `#rgb` or `#rrggbb` as red, green and blue.
pub fn hex_rgb(c: &str) -> Option<(u8, u8, u8)> {
    let h = c.trim().strip_prefix('#')?;
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h.to_string(),
        _ => return None,
    };
    let p = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok();
    Some((p(0)?, p(2)?, p(4)?))
}

/// The text colour on an accent: dark on light accents, white on dark ones.
pub fn on_accent((r, g, b): (u8, u8, u8)) -> &'static str {
    let l = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
    if l > 150.0 { "#16161f" } else { "#ffffff" }
}

/// A CSS `url()`, with characters that would break out of it left out.
pub fn css_url(url: &str) -> String {
    let clean: String = url
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\n' | '\r'))
        .collect();
    format!("url(\"{clean}\")")
}

/// The CSS `filter` of a background, "" for none.
pub fn background_filter(bg: &Background) -> String {
    let mut parts = Vec::new();
    if bg.blur > 0.0 {
        parts.push(format!("blur({}px)", bg.blur));
    }
    if bg.saturate != 100.0 {
        parts.push(format!("saturate({}%)", bg.saturate));
    }
    if bg.brightness != 100.0 {
        parts.push(format!("brightness({}%)", bg.brightness));
    }
    parts.join(" ")
}

/// The id of the `<style>` with the custom CSS.
pub const CUSTOM_CSS_ID: &str = "homedit-custom-css";

fn prefers_light() -> bool {
    kit_leptos::rt::window()
        .and_then(|w| {
            w.match_media("(prefers-color-scheme: light)")
                .ok()
                .flatten()
        })
        .is_some_and(|m| m.matches())
}

/// Puts `s` on `<html>` and `<head>`; safe to call again on every change.
pub fn apply(s: &Settings) {
    let Some(doc) = kit_leptos::rt::document() else {
        return;
    };
    let Some(root) = doc.document_element() else {
        return;
    };
    let set = |k: &str, v: Option<&str>| {
        let _ = match v {
            Some(v) => root.set_attribute(k, v),
            None => root.remove_attribute(k),
        };
    };
    set("data-theme", kit_theme(s.theme, prefers_light()));
    let style = root.dyn_ref_style();
    if let Some(st) = &style {
        match hex_rgb(&s.accent) {
            Some(rgb) => {
                let _ = st.set_property("--accent", s.accent.trim());
                let _ = st.set_property("--accent-2", s.accent.trim());
                let _ =
                    st.set_property("--accent-rgb", &format!("{}, {}, {}", rgb.0, rgb.1, rgb.2));
                let _ = st.set_property("--on-accent", on_accent(rgb));
            }
            None => {
                for p in ["--accent", "--accent-2", "--accent-rgb", "--on-accent"] {
                    let _ = st.remove_property(p);
                }
            }
        }
        if s.card_blur > 0.0 {
            let _ = st.set_property("--card-blur", &format!("{}px", s.card_blur));
        } else {
            let _ = st.remove_property("--card-blur");
        }
    }
    set("data-card-blur", (s.card_blur > 0.0).then_some(""));
    set(
        "data-background",
        s.background
            .as_ref()
            .filter(|b| !b.url.is_empty())
            .map(|_| ""),
    );
    doc.set_title(if s.title.is_empty() {
        "homedit"
    } else {
        &s.title
    });
    let el = match doc.get_element_by_id(CUSTOM_CSS_ID) {
        Some(e) => e,
        None => {
            let Ok(e) = doc.create_element("style") else {
                return;
            };
            e.set_id(CUSTOM_CSS_ID);
            if let Some(h) = doc.head() {
                let _ = h.append_child(&e);
            }
            e
        }
    };
    if el.text_content().unwrap_or_default() != s.custom_css {
        el.set_text_content(Some(&s.custom_css));
    }
}

trait StyleOf {
    fn dyn_ref_style(&self) -> Option<web_sys::CssStyleDeclaration>;
}

impl StyleOf for web_sys::Element {
    fn dyn_ref_style(&self) -> Option<web_sys::CssStyleDeclaration> {
        use wasm_bindgen::JsCast;
        self.dyn_ref::<web_sys::HtmlElement>().map(|h| h.style())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes() {
        assert_eq!(kit_theme(ThemeMode::Dark, true), None);
        assert_eq!(kit_theme(ThemeMode::Light, false), Some("light"));
        assert_eq!(kit_theme(ThemeMode::System, true), Some("light"));
        assert_eq!(kit_theme(ThemeMode::System, false), None);
    }

    #[test]
    fn accents_and_backgrounds() {
        assert_eq!(hex_rgb("#10b981"), Some((16, 185, 129)));
        assert_eq!(hex_rgb("#fff"), Some((255, 255, 255)));
        assert_eq!(hex_rgb("blue"), None);
        assert_eq!(on_accent((255, 255, 255)), "#16161f");
        assert_eq!(on_accent((16, 16, 120)), "#ffffff");
        assert_eq!(css_url("a\"b\\c.jpg"), "url(\"abc.jpg\")");
        let mut b = Background::new("/x.jpg");
        assert_eq!(background_filter(&b), "");
        b.blur = 4.0;
        b.saturate = 120.0;
        assert_eq!(background_filter(&b), "blur(4px) saturate(120%)");
    }
}
