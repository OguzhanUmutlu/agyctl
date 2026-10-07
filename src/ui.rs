use chrono::Datelike;
use colored::Colorize;

pub fn dim(text: &str) -> String {
    text.dimmed().to_string()
}

pub fn bold(text: &str) -> String {
    text.bold().to_string()
}

pub fn green(text: &str) -> String {
    text.green().to_string()
}

pub fn yellow(text: &str) -> String {
    text.yellow().to_string()
}

pub fn red(text: &str) -> String {
    text.red().to_string()
}

pub fn format_age(seconds: u64) -> String {
    if seconds < 60 {
        format!("{}s ago", seconds)
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else {
        format!("{}h ago", seconds / 3600)
    }
}

pub fn format_status_extended(
    is_active: bool,
    is_best: bool,
    is_exhausted: bool,
    width: usize,
    short: bool,
) -> String {
    if is_active {
        green(&format!("{:<width$}", "active"))
    } else if is_best {
        let label = if short { "next" } else { "ready (best next)" };
        bold(&yellow(&format!("{:<width$}", label)))
    } else if is_exhausted {
        let label = if short { "out" } else { "exhausted" };
        red(&format!("{:<width$}", label))
    } else {
        dim(&format!("{:<width$}", "ready"))
    }
}

pub fn truncate_str(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else {
        let prefix: String = s.chars().take(max_len.saturating_sub(2)).collect();
        format!("{}..", prefix)
    }
}

pub fn clean_tier(t: &str, max_len: usize) -> String {
    let lower = t.trim().to_lowercase();
    let simplified = if lower.contains("ultra") {
        "AI Ultra"
    } else if lower.contains("pro") {
        "AI Pro"
    } else if lower.contains("google") {
        "Google"
    } else if lower.is_empty() || lower == "unknown" {
        "AI Pro"
    } else {
        t.trim()
    };
    truncate_str(simplified, max_len)
}

pub fn format_pct(val: Option<f64>, width: usize) -> String {
    match val {
        Some(v) => {
            let s = format!("{:>width$}", format!("{:.1}%", v));
            if v < 1.0 {
                red(&s)
            } else if v < 20.0 {
                yellow(&s)
            } else {
                s
            }
        }
        None => dim(&format!("{:>width$}", "-")),
    }
}

pub fn format_claude_pct(val: Option<f64>, width: usize) -> String {
    match val {
        Some(v) => {
            if v <= 0.0 {
                red(&format!("{:>width$}", "exhausted"))
            } else {
                let s = format!("{:>width$}", format!("{:.1}%", v));
                if v < 1.0 {
                    red(&s)
                } else if v < 20.0 {
                    yellow(&s)
                } else {
                    s
                }
            }
        }
        None => dim(&format!("{:>width$}", "-")),
    }
}

pub fn format_plan_expiration(val: Option<&str>, width: usize) -> String {
    let raw = match val {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => return dim(&format!("{:<width$}", "Auto")),
    };

    let clean = raw.split('T').next().unwrap_or(raw);
    if let Ok(dt) = chrono::NaiveDate::parse_from_str(clean, "%Y-%m-%d") {
        let today = chrono::Local::now().date_naive();
        let days = (dt - today).num_days();
        let month = match dt.month() {
            1 => "Jan",
            2 => "Feb",
            3 => "Mar",
            4 => "Apr",
            5 => "May",
            6 => "Jun",
            7 => "Jul",
            8 => "Aug",
            9 => "Sep",
            10 => "Oct",
            11 => "Nov",
            _ => "Dec",
        };
        let label = format!("{} {} ({}d)", month, dt.day(), days);

        if days < 0 {
            red(&format!("{:<width$}", "expired"))
        } else if days == 0 {
            yellow(&format!("{:<width$}", "today"))
        } else if days <= 7 {
            yellow(&format!("{:<width$}", label))
        } else {
            format!("{:<width$}", label)
        }
    } else {
        format!("{:<width$}", truncate_str(clean, width))
    }
}

pub fn format_relative_time(iso_str: Option<&str>) -> String {
    let s = match iso_str {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return dim("N/A"),
    };

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        let now = chrono::Utc::now();
        let diff = dt.signed_duration_since(now).num_seconds();
        if diff <= 0 {
            return dim("refreshed");
        }
        let days = diff / 86400;
        let rem = diff % 86400;
        let hours = rem / 3600;
        let mins = (rem % 3600) / 60;
        if days > 0 {
            format!("in {}d {}h", days, hours)
        } else if hours > 0 {
            format!("in {}h {}m", hours, mins)
        } else {
            format!("in {}m", mins)
        }
    } else {
        s.chars().take(16).collect()
    }
}

pub fn make_bar(fraction: f64, length: usize) -> String {
    let filled = ((fraction * length as f64).round() as usize).min(length);
    let empty = length - filled;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

pub fn format_plan_compact(val: Option<&str>, width: usize) -> String {
    let raw = match val {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => return dim(&format!("{:<width$}", "Auto")),
    };
    let clean = raw.split('T').next().unwrap_or(raw);
    if let Ok(dt) = chrono::NaiveDate::parse_from_str(clean, "%Y-%m-%d") {
        let days = (dt - chrono::Local::now().date_naive()).num_days();
        if days < 0 {
            red(&format!("{:<width$}", "ended"))
        } else if days == 0 {
            yellow(&format!("{:<width$}", "today"))
        } else if days <= 7 {
            yellow(&format!("{:<width$}", format!("{}d", days)))
        } else {
            format!("{:<width$}", format!("{}d", days))
        }
    } else {
        format!("{:<width$}", truncate_str(clean, width))
    }
}

pub fn format_reset_in(reset: Option<&str>) -> Option<String> {
    let dt = chrono::DateTime::parse_from_rfc3339(reset?.trim()).ok()?;
    let secs = dt.signed_duration_since(chrono::Utc::now()).num_seconds();
    if secs <= 0 {
        return None;
    }
    let mins = (secs + 59) / 60;
    let d = mins / 1440;
    let h = (mins % 1440) / 60;
    let m = mins % 60;
    Some(if d > 0 {
        format!("{}d{}h", d, h)
    } else if h > 0 {
        format!("{}h{:02}m", h, m)
    } else {
        format!("{}m", m)
    })
}

pub fn format_reset_in_same_day(reset: Option<&str>) -> Option<String> {
    let dt = chrono::DateTime::parse_from_rfc3339(reset?.trim()).ok()?;
    let secs = dt.signed_duration_since(chrono::Utc::now()).num_seconds();
    if secs <= 0 || secs >= 86400 {
        return None;
    }
    let mins = (secs + 59) / 60;
    if mins >= 1440 {
        return None;
    }
    let h = mins / 60;
    let m = mins % 60;
    Some(if h > 0 {
        format!("{}h{:02}m", h, m)
    } else {
        format!("{}m", m)
    })
}

pub fn format_pct_reset(val: Option<f64>, reset: Option<&str>, width: usize) -> String {
    if let Some(v) = val {
        if v < 1.0 {
            if let Some(t) = format_reset_in(reset) {
                return red(&format!("{:>width$}", t));
            }
        }
    }
    format_pct(val, width)
}

pub fn format_weekly_pct_reset(val: Option<f64>, reset: Option<&str>, width: usize) -> String {
    if let Some(v) = val {
        if v <= 0.1 {
            if let Some(t) = format_reset_in(reset) {
                return red(&format!("{:>width$}", t));
            }
        } else if v < 1.0 {
            if let Some(t) = format_reset_in_same_day(reset) {
                return red(&format!("{:>width$}", t));
            }
        }
    }
    format_pct(val, width)
}

pub fn format_claude_reset(val: Option<f64>, reset: Option<&str>, width: usize) -> String {
    if let Some(v) = val {
        if v <= 0.1 {
            if let Some(t) = format_reset_in(reset) {
                return red(&format!("{:>width$}", t));
            }
        } else if v < 1.0 {
            if let Some(t) = format_reset_in_same_day(reset) {
                return red(&format!("{:>width$}", t));
            }
        }
    }
    format_claude_pct(val, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weekly_reset_same_day() {
        let now = chrono::Utc::now();
        let in_4h = (now + chrono::Duration::hours(4)).to_rfc3339();
        let in_3d = (now + chrono::Duration::days(3)).to_rfc3339();

        let s1 = format_weekly_pct_reset(Some(0.4), Some(&in_4h), 8);
        assert!(s1.contains("4h00m") || s1.contains("4h01m"));

        let s2 = format_weekly_pct_reset(Some(0.4), Some(&in_3d), 8);
        assert!(s2.contains("0.4%"));

        let s3 = format_weekly_pct_reset(Some(50.0), Some(&in_4h), 8);
        assert!(s3.contains("50.0%"));

        let s4 = format_weekly_pct_reset(Some(0.0), Some(&in_3d), 8);
        assert!(s4.contains("3d"));
    }
}
