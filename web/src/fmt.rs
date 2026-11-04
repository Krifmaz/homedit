//! Words and numbers as the dashboard shows them. Plain functions, tested without a browser.

use crate::model::{Check, Sample, Status};

const UNITS: [&str; 7] = ["B", "kB", "MB", "GB", "TB", "PB", "EB"];

/// A byte count with decimal units: `1500` → `1.5 kB`, `16e9` → `16 GB`; negative or not a number → `–`.
pub fn bytes(n: f64) -> String {
    if !n.is_finite() || n < 0.0 {
        return "–".into();
    }
    let (mut v, mut u) = (n, 0);
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if (v * 10.0).round() / 10.0 >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{} {}", v.round(), UNITS[0])
    } else {
        format!("{} {}", trim(v, 1), UNITS[u])
    }
}

/// `v` with at most `digits` decimals, without trailing zeros.
pub fn trim(v: f64, digits: usize) -> String {
    let s = format!("{v:.digits$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// A widget stat: numbers grouped by thousands with at most two decimals, text as it is.
pub fn stat_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            if !f.is_finite() {
                return "–".into();
            }
            let t = trim(f, 2);
            let (int, frac) = t
                .split_once('.')
                .map(|(a, b)| (a.to_string(), format!(".{b}")))
                .unwrap_or((t.clone(), String::new()));
            let (sign, digits) = int
                .strip_prefix('-')
                .map(|d| ("-", d))
                .unwrap_or(("", int.as_str()));
            let mut out = String::new();
            for (i, c) in digits.chars().enumerate() {
                if i > 0 && (digits.len() - i) % 3 == 0 {
                    out.push(',');
                }
                out.push(c);
            }
            format!("{sign}{out}{frac}")
        }
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "–".into(),
        other => other.to_string(),
    }
}

/// Initials for an icon's fallback tile: "Home Assistant" → "HA", "plex" → "PL".
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    match words.as_slice() {
        [] => "?".into(),
        [w] => w.chars().take(2).collect::<String>().to_uppercase(),
        [a, b, ..] => format!(
            "{}{}",
            a.chars().next().unwrap_or('?'),
            b.chars().next().unwrap_or('?')
        )
        .to_uppercase(),
    }
}

/// A status in words: "Up · 200 · 42 ms", "Down", "Checking…" before the first answer.
pub fn status_text(check: &Check, status: Option<&Status>) -> String {
    let Some(s) = status else {
        return "Checking…".into();
    };
    let mut parts = vec![if s.up {
        "Up".to_string()
    } else {
        "Down".to_string()
    }];
    if check.kind == "http"
        && let Some(c) = s.code
    {
        parts.push(c.to_string());
    }
    if s.up
        && let Some(ms) = s.latency_ms
    {
        parts.push(format!("{} ms", ms.round()));
    }
    parts.join(" · ")
}

/// The SVG path of the response times; a down sample, or one without a time, breaks the line.
pub fn spark_path(samples: &[Sample], width: f64, height: f64) -> String {
    let values: Vec<Option<f64>> = samples
        .iter()
        .map(|s| if s.up { s.ms } else { None })
        .collect();
    let max = values.iter().flatten().fold(1.0_f64, |a, b| a.max(*b));
    let step = if samples.len() > 1 {
        width / (samples.len() - 1) as f64
    } else {
        0.0
    };
    let mut d = String::new();
    let mut pen = false;
    for (i, v) in values.iter().enumerate() {
        match v {
            None => pen = false,
            Some(v) => {
                let x = i as f64 * step;
                let y = height - 1.0 - (v / max) * (height - 2.0);
                d.push_str(&format!("{}{x:.1} {y:.1}", if pen { "L" } else { "M" }));
                pen = true;
            }
        }
    }
    d
}

/// The sentence of a rescan's changes: the first three, and how many more.
pub fn changes_text(msgs: &[String]) -> String {
    let shown: Vec<&str> = msgs.iter().take(3).map(String::as_str).collect();
    let more = msgs.len().saturating_sub(3);
    let mut s = format!("Network changes found: {}", shown.join("; "));
    if more > 0 {
        s.push_str(&format!(" (and {more} more)"));
    }
    s
}

/// How long ago `then` was, from `now` (both Unix ms): "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(then: f64, now: f64) -> String {
    let s = ((now - then) / 1000.0).max(0.0);
    if s < 60.0 {
        "just now".into()
    } else if s < 3600.0 {
        format!("{} min ago", (s / 60.0).floor())
    } else if s < 86_400.0 {
        format!("{} h ago", (s / 3600.0).floor())
    } else {
        let d = (s / 86_400.0).floor();
        if d == 1.0 {
            "1 day ago".into()
        } else {
            format!("{d} days ago")
        }
    }
}

/// A count with its noun: `1 card`, `3 cards`.
pub fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(0.0), "0 B");
        assert_eq!(bytes(999.0), "999 B");
        assert_eq!(bytes(1500.0), "1.5 kB");
        assert_eq!(bytes(16e9), "16 GB");
        assert_eq!(bytes(999_960.0), "1 MB");
        assert_eq!(bytes(-1.0), "–");
        assert_eq!(bytes(f64::NAN), "–");
    }

    #[test]
    fn formats_stats() {
        assert_eq!(stat_value(&json!(1234567)), "1,234,567");
        assert_eq!(stat_value(&json!(-1234.567)), "-1,234.57");
        assert_eq!(stat_value(&json!(12)), "12");
        assert_eq!(stat_value(&json!("idle")), "idle");
    }

    #[test]
    fn initials_and_counts() {
        assert_eq!(initials("Home Assistant"), "HA");
        assert_eq!(initials("plex"), "PL");
        assert_eq!(initials("  "), "?");
        assert_eq!(count(1, "card", "cards"), "1 card");
        assert_eq!(count(2, "card", "cards"), "2 cards");
    }

    #[test]
    fn status_words() {
        let http = Check {
            kind: "http".into(),
            target: None,
            expect_status: None,
        };
        assert_eq!(status_text(&http, None), "Checking…");
        let s = Status {
            up: true,
            code: Some(200),
            latency_ms: Some(41.6),
            checked_at: 0.0,
            history: vec![],
        };
        assert_eq!(status_text(&http, Some(&s)), "Up · 200 · 42 ms");
        let tcp = Check {
            kind: "tcp".into(),
            ..http
        };
        let down = Status { up: false, ..s };
        assert_eq!(status_text(&tcp, Some(&down)), "Down");
    }

    #[test]
    fn sparkline_breaks_on_down() {
        let s = |up, ms: Option<f64>| Sample { t: 0.0, up, ms };
        let p = spark_path(
            &[
                s(true, Some(10.0)),
                s(true, Some(20.0)),
                s(false, None),
                s(true, Some(5.0)),
            ],
            30.0,
            22.0,
        );
        assert_eq!(p.matches('M').count(), 2);
        assert!(p.starts_with("M0.0 "));
        assert!(p.contains("L10.0 1.0"));
    }

    #[test]
    fn changes_and_times() {
        let m: Vec<String> = (1..=5).map(|i| format!("c{i}")).collect();
        assert_eq!(
            changes_text(&m),
            "Network changes found: c1; c2; c3 (and 2 more)"
        );
        assert_eq!(changes_text(&m[..1]), "Network changes found: c1");
        assert_eq!(ago(0.0, 30_000.0), "just now");
        assert_eq!(ago(0.0, 5.0 * 60_000.0), "5 min ago");
        assert_eq!(ago(0.0, 2.5 * 86_400_000.0), "2 days ago");
    }
}
