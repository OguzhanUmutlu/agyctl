use crate::cache::{load_cache, remove_cache_account, rename_cache_account, update_cache};
use crate::quota::{
    fetch_local_ls_quota_and_status, fetch_upstream_quota_summary, is_antigravity_running,
    refresh_google_oauth_token, restart_local_antigravity,
};
use crate::secret::{get_active_token, parse_jwt_payload, set_active_token};
use crate::service::is_service_active;
use crate::ui::*;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountProfile {
    pub name: String,
    pub email: String,
    pub tier: String,
    pub plan_expiration: Option<String>,
    pub created_at: String,
    pub token_data: Value,
}

pub fn get_accounts_dir() -> PathBuf {
    crate::cache::config_dir().join("accounts")
}

fn parse_rfc3339_remaining_secs(iso_str: Option<&str>) -> Option<i64> {
    let s = iso_str?.trim();
    if s.is_empty() {
        return None;
    }
    let dt = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    let now = chrono::Utc::now();
    Some(dt.signed_duration_since(now).num_seconds())
}

fn format_mins_duration(mins: u64) -> String {
    if mins < 60 {
        format!("{}m", mins)
    } else {
        let h = mins / 60;
        let m = mins % 60;
        if m == 0 {
            format!("{}h", h)
        } else {
            format!("{}h {}m", h, m)
        }
    }
}

fn expiry_urgency(weekly_reset: Option<&str>) -> f64 {
    let hours = parse_rfc3339_remaining_secs(weekly_reset)
        .map(|s| s.max(0) as f64 / 3600.0)
        .unwrap_or(168.0);
    1.0 + 1.5 / (1.0 + hours / 12.0)
}

fn weekly_expiry_hint(a: &crate::cache::AccountCacheEntry, filter: Option<&str>) -> Option<String> {
    let mut soonest: Option<i64> = None;
    let mut consider = |left: Option<f64>, reset: Option<&str>| {
        if left.unwrap_or(0.0) > 0.0 {
            if let Some(s) = parse_rfc3339_remaining_secs(reset) {
                if s > 0 {
                    soonest = Some(soonest.map_or(s, |m| m.min(s)));
                }
            }
        }
    };
    let want_g = filter.map_or(true, |f| f.eq_ignore_ascii_case("gemini"));
    let want_c = filter.map_or(true, |f| f.eq_ignore_ascii_case("claude"));
    if want_g {
        consider(a.quota.gemini_weekly, a.quota.gemini_weekly_reset.as_deref());
    }
    if want_c {
        consider(a.quota.claude_weekly, a.quota.claude_weekly_reset.as_deref());
    }
    let secs = soonest?;
    if secs < 48 * 3600 {
        Some(format!("weekly resets in {}", format_mins_duration((secs / 60) as u64)))
    } else {
        None
    }
}

fn is_quota_in_cooldown_5h(val: Option<f64>, reset: Option<&str>) -> bool {
    let v = val.unwrap_or(0.0);
    if v <= 0.1 {
        return true;
    }
    if v < 1.0 && parse_rfc3339_remaining_secs(reset).unwrap_or(0) > 0 {
        return true;
    }
    false
}

fn is_quota_in_cooldown_weekly(val: Option<f64>, reset: Option<&str>) -> bool {
    let v = val.unwrap_or(0.0);
    if v <= 0.1 {
        return true;
    }
    let secs = parse_rfc3339_remaining_secs(reset).unwrap_or(0);
    if v < 1.0 && secs > 0 && secs < 86400 {
        return true;
    }
    false
}

fn effective_usable_gemini(a: &crate::cache::AccountCacheEntry) -> f64 {
    if is_quota_in_cooldown_5h(a.quota.gemini_5h, a.quota.gemini_5h_reset.as_deref())
        || is_quota_in_cooldown_weekly(a.quota.gemini_weekly, a.quota.gemini_weekly_reset.as_deref()) {
        0.0
    } else {
        a.quota.gemini_5h.unwrap_or(0.0).min(a.quota.gemini_weekly.unwrap_or(0.0))
    }
}

fn effective_usable_claude(a: &crate::cache::AccountCacheEntry) -> f64 {
    if a.quota.claude_5h_disabled
        || is_quota_in_cooldown_5h(a.quota.claude_5h, a.quota.claude_5h_reset.as_deref())
        || is_quota_in_cooldown_weekly(a.quota.claude_weekly, a.quota.claude_weekly_reset.as_deref()) {
        0.0
    } else {
        a.quota.claude_weekly.unwrap_or(0.0).min(a.quota.claude_5h.unwrap_or(100.0))
    }
}

fn score_candidate(a: &crate::cache::AccountCacheEntry, filter: Option<&str>) -> (f64, i64) {
    let g = effective_usable_gemini(a);
    let c = effective_usable_claude(a);

    let want_g = filter.map_or(true, |f| !f.eq_ignore_ascii_case("claude"));
    let want_c = filter.map_or(true, |f| !f.eq_ignore_ascii_case("gemini"));

    let (base, best) = match (want_g, want_c) {
        (true, true) => (g + c, g.max(c)),
        (true, false) => (g, g),
        _ => (c, c),
    };
    let score = if best > 0.0 { base } else { 0.0 };

    let secs = |r: Option<&str>| {
        parse_rfc3339_remaining_secs(r)
            .map(|s| s.max(0))
            .unwrap_or(86400 * 7)
    };

    let wait_g = if g > 0.0 {
        0
    } else {
        let mut w = 0;
        if is_quota_in_cooldown_5h(a.quota.gemini_5h, a.quota.gemini_5h_reset.as_deref()) {
            w = w.max(secs(a.quota.gemini_5h_reset.as_deref()));
        }
        if is_quota_in_cooldown_weekly(a.quota.gemini_weekly, a.quota.gemini_weekly_reset.as_deref()) {
            w = w.max(secs(a.quota.gemini_weekly_reset.as_deref()));
        }
        if w == 0 {
            w = secs(a.quota.gemini_5h_reset.as_deref()).max(secs(a.quota.gemini_weekly_reset.as_deref()));
        }
        w
    };

    let wait_c = if c > 0.0 {
        0
    } else {
        let mut w = 0;
        if is_quota_in_cooldown_weekly(a.quota.claude_weekly, a.quota.claude_weekly_reset.as_deref()) {
            w = w.max(secs(a.quota.claude_weekly_reset.as_deref()));
        }
        if !a.quota.claude_5h_disabled && is_quota_in_cooldown_5h(a.quota.claude_5h, a.quota.claude_5h_reset.as_deref()) {
            w = w.max(secs(a.quota.claude_5h_reset.as_deref()));
        }
        if w == 0 {
            w = secs(a.quota.claude_weekly_reset.as_deref());
        }
        w
    };

    let wait = match (want_g, want_c) {
        (true, true) => wait_g.min(wait_c),
        (true, false) => wait_g,
        _ => wait_c,
    };

    (score, wait)
}

fn calculate_smart_burn(
    history: &[crate::cache::QuotaSnapshot],
    current_val: Option<f64>,
    reset_time: Option<&str>,
    weekly_val: Option<f64>,
    weekly_reset: Option<&str>,
    now_ts: u64,
    is_5h: bool,
    is_gemini: bool,
) -> Option<String> {
    let curr = current_val?;
    if curr <= 0.0 {
        return None;
    }

    let mut points: Vec<(u64, f64)> = history
        .iter()
        .filter(|s| s.timestamp >= now_ts.saturating_sub(600))
        .filter_map(|s| {
            let v = if is_gemini {
                if is_5h { s.gemini_5h } else { s.gemini_weekly }
            } else {
                if is_5h { s.claude_5h } else { s.claude_weekly }
            };
            v.map(|val| (s.timestamp, val))
        })
        .collect();

    if points.is_empty() || points.last().map(|p| p.0 != now_ts).unwrap_or(false) {
        points.push((now_ts, curr));
    }

    if points.len() < 2 {
        return None;
    }

    let first = points.first()?;
    let last = points.last()?;
    let dt = last.0.saturating_sub(first.0);
    if dt < 60 {
        return None;
    }

    let dv = first.1 - last.1;
    if dv < 0.2 {
        return None;
    }

    let rate = dv / (dt as f64 / 60.0);
    if rate <= 0.05 || rate > 50.0 {
        return None;
    }

    let mins_to_exhaust = (curr / rate).round() as u64;
    if mins_to_exhaust > 720 {
        return None;
    }

    let reset_secs = parse_rfc3339_remaining_secs(reset_time);
    let reset_mins = reset_secs.map(|s| (s.max(0) / 60) as u64);

    if let Some(rm) = reset_mins {
        if rm < mins_to_exhaust {
            if is_5h && weekly_val.is_some() {
                let wv = weekly_val.unwrap();
                let w_exhaust_mins = (wv / rate).round() as u64;
                let w_reset_mins = parse_rfc3339_remaining_secs(weekly_reset).map(|s| (s.max(0) / 60) as u64);
                if w_reset_mins.map_or(true, |wrm| w_exhaust_mins < wrm) {
                    Some(format!(
                        "burns ~{:.1}%/m (resets in {}m before 5h exhausts; weekly exhausts in ~{})",
                        rate, rm, format_mins_duration(w_exhaust_mins)
                    ))
                } else {
                    Some(format!(
                        "burns ~{:.1}%/m (resets in {}m before exhaustion)",
                        rate, rm
                    ))
                }
            } else {
                Some(format!(
                    "burns ~{:.1}%/m (resets in {}m before exhaustion)",
                    rate, rm
                ))
            }
        } else {
            Some(format!(
                "burns ~{:.1}%/m (exhausts in ~{})",
                rate, format_mins_duration(mins_to_exhaust)
            ))
        }
    } else {
        Some(format!(
            "burns ~{:.1}%/m (exhausts in ~{})",
            rate, format_mins_duration(mins_to_exhaust)
        ))
    }
}

fn term_width() -> usize {
    if let Ok(c) = std::env::var("COLUMNS") {
        if let Ok(n) = c.trim().parse::<usize>() {
            return n;
        }
    }
    if let Some((terminal_size::Width(w), _)) = terminal_size::terminal_size() {
        return w as usize;
    }
    1000
}

struct Layout {
    name_w: usize,
    email_w: usize,
    tier_short: bool,
    plan_compact: bool,
    status: u8,
    model: u8,
}

impl Layout {
    fn tier_w(&self) -> usize {
        if self.tier_short { 5 } else { 8 }
    }

    fn plan_w(&self) -> usize {
        if self.plan_compact { 5 } else { 14 }
    }

    fn total(&self) -> usize {
        let mut t = 3 + self.name_w + 1 + self.tier_w() + 1 + self.email_w + 1 + self.plan_w();
        t += match self.model {
            1 => 9 + 9,
            2 => 11 + 9,
            _ => 9 + 9 + 11,
        };
        t += match self.status {
            0 => 2 + 17,
            1 => 2 + 6,
            _ => 0,
        };
        t
    }

    fn fit(&mut self, width: usize, min_email: usize) {
        let t = self.total();
        if t > width {
            let floor = min_email.min(self.email_w);
            self.email_w = self.email_w.saturating_sub(t - width).max(floor);
        }
    }

    fn compute(width: usize, model: u8, max_email: usize) -> Layout {
        let mut l = Layout {
            name_w: 8,
            email_w: max_email.max(5),
            tier_short: false,
            plan_compact: false,
            status: 0,
            model,
        };
        l.fit(width, 12);
        if l.total() <= width {
            return l;
        }
        l.tier_short = true;
        l.fit(width, 12);
        if l.total() <= width {
            return l;
        }
        l.plan_compact = true;
        l.fit(width, 12);
        if l.total() <= width {
            return l;
        }
        l.status = 1;
        l.fit(width, 12);
        if l.total() <= width {
            return l;
        }
        l.fit(width, 8);
        if l.total() <= width {
            return l;
        }
        l.status = 2;
        l.fit(width, 8);
        if l.total() <= width {
            return l;
        }
        l.name_w = 6;
        l.fit(width, 6);
        l
    }

    fn assemble(&self, marker: &str, cells: Vec<String>, nums: Vec<String>, status: Option<String>) -> String {
        let mut s = marker.to_string();
        for c in cells.iter().chain(nums.iter()) {
            s.push(' ');
            s.push_str(c);
        }
        if let Some(st) = status {
            s.push_str("  ");
            s.push_str(&st);
        }
        s
    }

    fn header(&self) -> String {
        let plan = if self.plan_compact { "EXP" } else { "EXPIRATION" };
        let cells = vec![
            dim(&format!("{:<w$}", "NAME", w = self.name_w)),
            dim(&format!("{:<w$}", "TIER", w = self.tier_w())),
            dim(&format!("{:<w$}", "EMAIL", w = self.email_w)),
            dim(&format!("{:<w$}", plan, w = self.plan_w())),
        ];
        let g = dim(&format!("{:>8}", "GEMINI"));
        let h5 = dim(&format!("{:>8}", "5-HOUR"));
        let c = dim(&format!("{:>10}", "CLAUDE"));
        let nums = match self.model {
            1 => vec![g, h5],
            2 => vec![c, h5],
            _ => vec![g, h5, c],
        };
        let status = match self.status {
            0 => Some(dim(&format!("{:<17}", "STATUS"))),
            1 => Some(dim(&format!("{:<6}", "STATUS"))),
            _ => None,
        };
        self.assemble("  ", cells, nums, status)
    }
}

fn print_account_row(
    l: &Layout,
    a: &crate::cache::AccountCacheEntry,
    is_active: bool,
    is_best: bool,
    is_exhausted: bool,
) {
    let marker = if is_active {
        green("* ")
    } else {
        "  ".to_string()
    };

    let tier_txt = clean_tier(&a.tier, 8);
    let tier_txt = if l.tier_short {
        tier_txt.strip_prefix("AI ").unwrap_or(&tier_txt).to_string()
    } else {
        tier_txt
    };

    let plan = if l.plan_compact {
        format_plan_compact(a.plan_expiration.as_deref(), l.plan_w())
    } else {
        format_plan_expiration(a.plan_expiration.as_deref(), l.plan_w())
    };

    let cells = vec![
        format!("{:<w$}", truncate_str(&a.name, l.name_w), w = l.name_w),
        format!("{:<w$}", truncate_str(&tier_txt, l.tier_w()), w = l.tier_w()),
        format!("{:<w$}", truncate_str(&a.email, l.email_w), w = l.email_w),
        plan,
    ];

    let gw = format_weekly_pct_reset(a.quota.gemini_weekly, a.quota.gemini_weekly_reset.as_deref(), 8);
    let g5 = format_pct_reset(a.quota.gemini_5h, a.quota.gemini_5h_reset.as_deref(), 8);
    let cw = format_claude_reset(a.quota.claude_weekly, a.quota.claude_weekly_reset.as_deref(), 10);
    let c5 = if a.quota.claude_5h_disabled {
        dim(&format!("{:>8}", "off"))
    } else {
        format_pct_reset(a.quota.claude_5h, a.quota.claude_5h_reset.as_deref(), 8)
    };
    let nums = match l.model {
        1 => vec![gw, g5],
        2 => vec![cw, c5],
        _ => vec![gw, g5, cw],
    };

    let status = match l.status {
        0 => Some(format_status_extended(is_active, is_best, is_exhausted, 17, false)),
        1 => Some(format_status_extended(is_active, is_best, is_exhausted, 6, true)),
        _ => None,
    };

    println!("{}", l.assemble(&marker, cells, nums, status));
}

pub fn cmd_list(refresh: bool, model_filter: Option<&str>, auto_switch: bool, restart: bool) {
    let mut cache = if refresh {
        update_cache(false)
    } else {
        load_cache(Some(20)).unwrap_or_else(|| update_cache(false))
    };

    let mut active_opt = cache.accounts.values().find(|a| a.is_active).cloned();

    let is_gemini_filter = model_filter.map_or(false, |f| f.eq_ignore_ascii_case("gemini"));
    let is_claude_filter = model_filter.map_or(false, |f| f.eq_ignore_ascii_case("claude"));

    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut candidates: Vec<crate::cache::AccountCacheEntry> = cache
        .accounts
        .values()
        .filter(|a| !a.is_active && a.error.is_none())
        .cloned()
        .collect();

    let act_g = active_opt.as_ref().map(effective_usable_gemini).unwrap_or(0.0);
    let act_c = active_opt.as_ref().map(effective_usable_claude).unwrap_or(0.0);

    let need_g = 0.25 + (100.0 - act_g) / 100.0;
    let need_c = 0.25 + (100.0 - act_c) / 100.0;
    let rank = |a: &crate::cache::AccountCacheEntry| -> f64 {
        let g = effective_usable_gemini(a);
        let c = effective_usable_claude(a);
        let tier = if a.tier.to_lowercase().contains("ultra") {
            0.5
        } else {
            0.0
        };
        let g = g * expiry_urgency(a.quota.gemini_weekly_reset.as_deref());
        let c = c * expiry_urgency(a.quota.claude_weekly_reset.as_deref());
        match model_filter {
            Some(f) if f.eq_ignore_ascii_case("gemini") => g + tier,
            Some(f) if f.eq_ignore_ascii_case("claude") => c + tier,
            _ => g * need_g + c * need_c + tier,
        }
    };

    candidates.sort_by(|a, b| {
        let (score_a, wait_a) = score_candidate(a, model_filter);
        let (score_b, wait_b) = score_candidate(b, model_filter);
        match (score_a > 0.0, score_b > 0.0) {
            (true, true) => rank(b).partial_cmp(&rank(a)).unwrap_or(std::cmp::Ordering::Equal),
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (false, false) => wait_a.cmp(&wait_b),
        }
    });

    let best_cand = candidates.first().cloned();

    let target_exhausted = if is_gemini_filter {
        act_g <= 0.0
    } else if is_claude_filter {
        act_c <= 0.0
    } else {
        act_g <= 0.0 && act_c <= 0.0
    };

    if auto_switch && target_exhausted {
        if let Some(ref bc) = best_cand {
            let (score, _) = score_candidate(bc, model_filter);
            if score > 0.0 {
                println!("{} Auto-switching to '{}'...", green("⚡"), bold(&bc.name));
                cmd_switch(&bc.name, restart, false, None);
                cache = update_cache(false);
                active_opt = cache.accounts.values().find(|a| a.is_active).cloned();
            }
        }
    }

    let age_str = format_age(now_ts.saturating_sub(cache.updated_timestamp));
    let daemon_str = if is_service_active() {
        green("daemon active")
    } else {
        dim("daemon inactive")
    };

    println!();
    println!(
        "{}  {}  {}",
        bold("Accounts & Quotas"),
        dim(&format!("cached {}", age_str)),
        daemon_str
    );
    println!();

    if let Some(ref act) = active_opt {
        println!(
            "{} {} ({}, {})",
            bold("Current:"),
            bold(&act.name),
            act.email,
            clean_tier(&act.tier, 10)
        );

        if !is_claude_filter {
            let g5_str = format_pct_reset(act.quota.gemini_5h, act.quota.gemini_5h_reset.as_deref(), 6);
            let gw_str = format_weekly_pct_reset(act.quota.gemini_weekly, act.quota.gemini_weekly_reset.as_deref(), 6);
            let burn_g5 = calculate_smart_burn(
                &act.history,
                act.quota.gemini_5h,
                act.quota.gemini_5h_reset.as_deref(),
                act.quota.gemini_weekly,
                act.quota.gemini_weekly_reset.as_deref(),
                now_ts,
                true,
                true,
            );
            let extra = burn_g5.map(|b| format!("  ·  {}", b)).unwrap_or_default();
            println!("  Gemini:  5-Hour: {}  Weekly: {}{}", g5_str, gw_str, extra);
        }

        if !is_gemini_filter {
            let c5_str = if act.quota.claude_5h_disabled {
                red("disabled")
            } else {
                format_pct_reset(act.quota.claude_5h, act.quota.claude_5h_reset.as_deref(), 6)
            };
            let cw_str = format_claude_reset(act.quota.claude_weekly, act.quota.claude_weekly_reset.as_deref(), 8);
            let burn_cw = calculate_smart_burn(
                &act.history,
                act.quota.claude_weekly,
                act.quota.claude_weekly_reset.as_deref(),
                None,
                None,
                now_ts,
                false,
                false,
            );
            let extra = burn_cw.map(|b| format!("  ·  {}", b)).unwrap_or_default();
            println!("  Claude:  5-Hour: {}  Weekly: {}{}", c5_str, cw_str, extra);
        }

        if model_filter.is_none() {
            if act_g <= 0.0 && act_c > 10.0 {
                println!(
                    "  {}",
                    yellow(&format!(
                        "Notice: Gemini exhausted in current account. Claude has {:.1}% remaining.",
                        act_c
                    ))
                );
            } else if act_c <= 0.0 && act_g > 10.0 {
                println!(
                    "  {}",
                    yellow(&format!(
                        "Notice: Claude exhausted in current account. Gemini has {:.1}% remaining.",
                        act_g
                    ))
                );
            }
        }

        if let Some(ref bc) = best_cand {
            let (score, reset_s) = score_candidate(bc, model_filter);
            if score > 0.0 {
                let bc_g = effective_usable_gemini(bc);
                let bc_c = effective_usable_claude(bc);
                let summary = if is_gemini_filter {
                    format!("Gemini: {:.1}%", bc_g)
                } else if is_claude_filter {
                    format!("Claude: {:.1}%", bc_c)
                } else {
                    format!("Gemini {:.1}%, Claude {:.1}%", bc_g, bc_c)
                };
                let summary = match weekly_expiry_hint(bc, model_filter) {
                    Some(h) => format!("{} · {}", summary, h),
                    None => summary,
                };
                if target_exhausted {
                    println!(
                        "  {} Switch to '{}' ({} · {}) -> {}",
                        bold("Recommendation:"),
                        bold(&bc.name),
                        summary,
                        clean_tier(&bc.tier, 10),
                        green(&format!("agyctl switch {}", bc.name))
                    );
                } else {
                    println!(
                        "  {} '{}' ({} · {}) -> agyctl switch {}",
                        dim("Next fallback:"),
                        bold(&bc.name),
                        summary,
                        clean_tier(&bc.tier, 10),
                        bc.name
                    );
                }
            } else {
                let reset_mins = (reset_s / 60) as u64;
                println!(
                    "  {} All alternative accounts currently exhausted. Next reset: '{}' in ~{}.",
                    yellow("Notice:"),
                    bold(&bc.name),
                    format_mins_duration(reset_mins)
                );
            }
        }
        println!();
    }

    let model_kind: u8 = if is_gemini_filter {
        1
    } else if is_claude_filter {
        2
    } else {
        0
    };
    let max_email = cache
        .accounts
        .values()
        .map(|a| a.email.chars().count())
        .max()
        .unwrap_or(5);
    let layout = Layout::compute(term_width(), model_kind, max_email);
    println!("{}", layout.header());

    if let Some(ref act) = active_opt {
        print_account_row(&layout, act, true, false, false);
        println!();
    }

    for (idx, cand) in candidates.iter().enumerate() {
        let (score, _) = score_candidate(cand, model_filter);
        let is_best = idx == 0 && score > 0.0;
        let is_exhausted = score <= 0.0;
        print_account_row(&layout, cand, false, is_best, is_exhausted);
    }

    for a in cache.accounts.values().filter(|a| a.error.is_some()) {
        let c_name = format!("{:<w$}", truncate_str(&a.name, layout.name_w), w = layout.name_w);
        let c_email = format!("{:<w$}", truncate_str(&a.email, layout.email_w), w = layout.email_w);
        println!(
            "   {} {} {}",
            c_name,
            c_email,
            red(&format!("error: {}", a.error.as_deref().unwrap_or("unknown")))
        );
    }

    println!();
}

pub fn cmd_current() {
    let tok = match get_active_token() {
        Some(t) => t,
        None => {
            eprintln!("{}", red("No active account token found"));
            std::process::exit(1);
        }
    };

    let id_tok = tok.get("id_token").and_then(|v| v.as_str()).unwrap_or("");
    let jwt = parse_jwt_payload(id_tok);
    let email = jwt
        .as_ref()
        .and_then(|j| j.get("email"))
        .and_then(|e| e.as_str())
        .unwrap_or("Unknown");
    let name = jwt
        .as_ref()
        .and_then(|j| j.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");
    let expiry = tok
        .get("token")
        .and_then(|t| t.get("expiry"))
        .and_then(|e| e.as_str())
        .unwrap_or("Unknown");
    let method = tok
        .get("auth_method")
        .and_then(|m| m.as_str())
        .unwrap_or("consumer");

    println!();
    println!("{}", bold("Active Account"));
    if !name.is_empty() {
        println!("  {}     {}", dim("Name:"), name);
    }
    println!("  {}    {}", dim("Email:"), email);
    println!("  {}     {}", dim("Auth:"), method);
    println!("  {}  {}", dim("Expires:"), expiry);
    println!();
}

pub fn cmd_edit(
    target: &str,
    new_name: Option<&str>,
    tier: Option<&str>,
    expires: Option<&str>,
) {
    let accounts_dir = get_accounts_dir();
    let (old_clean, new_clean) = match new_name {
        Some(nn) => (target.trim().to_lowercase(), Some(nn.trim().to_lowercase())),
        None => {
            let direct_file = accounts_dir.join(format!("{}.json", target.trim().to_lowercase()));
            if direct_file.exists() && (tier.is_some() || expires.is_some()) {
                (target.trim().to_lowercase(), None)
            } else {
                let cache = load_cache(None).unwrap_or_else(|| update_cache(false));
                let current_name = cache
                    .accounts
                    .values()
                    .find(|a| a.is_active)
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "account1".to_string());
                (current_name, Some(target.trim().to_lowercase()))
            }
        }
    };

    let old_file = accounts_dir.join(format!("{}.json", old_clean));
    if !old_file.exists() {
        eprintln!("{}", red(&format!("Account '{}' not found", old_clean)));
        std::process::exit(1);
    }

    let content = match fs::read_to_string(&old_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", red(&format!("Failed to read profile: {}", e)));
            std::process::exit(1);
        }
    };

    let mut profile: AccountProfile = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", red(&format!("Failed to parse profile: {}", e)));
            std::process::exit(1);
        }
    };

    if let Some(ref nn) = new_clean {
        if nn != &old_clean {
            let new_file = accounts_dir.join(format!("{}.json", nn));
            if new_file.exists() {
                eprintln!("{}", red(&format!("Account '{}' already exists", nn)));
                std::process::exit(1);
            }
            profile.name = nn.clone();
        }
    }

    if let Some(t) = tier {
        profile.tier = clean_tier(t, 20);
    }
    if let Some(e) = expires {
        profile.plan_expiration = if e.trim().is_empty() {
            None
        } else {
            Some(e.trim().to_string())
        };
    }

    let final_name = new_clean.as_deref().unwrap_or(&old_clean);
    let final_file = accounts_dir.join(format!("{}.json", final_name));

    if let Ok(bytes) = serde_json::to_string_pretty(&profile) {
        let _ = fs::write(&final_file, bytes);
        if final_name != old_clean {
            let _ = fs::remove_file(&old_file);
        }
        println!("{} Updated account '{}'", green("✓"), final_name);
        rename_cache_account(
            &old_clean,
            final_name,
            tier,
            expires.map(|e| if e.trim().is_empty() { None } else { Some(e.trim()) }),
        );
    }
}

pub fn cmd_switch(name: &str, restart: bool, remote: bool, host_opt: Option<&str>) {
    let clean_name = name.trim().to_lowercase();
    let file_path = get_accounts_dir().join(format!("{}.json", clean_name));

    if !file_path.exists() {
        eprintln!("{}", red(&format!("Profile '{}' does not exist", clean_name)));
        std::process::exit(1);
    }

    let content = fs::read_to_string(&file_path).unwrap_or_default();
    let profile: AccountProfile = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", red(&format!("Corrupt profile JSON: {}", e)));
            std::process::exit(1);
        }
    };

    println!();
    println!(
        "Switching to '{}' ({}, {})...",
        bold(&clean_name),
        profile.email,
        profile.tier
    );

    if !set_active_token(&profile.token_data) {
        eprintln!("{}", yellow("Warning: Failed to set token in SecretService"));
    } else {
        println!("  {} Local session updated", green("✓"));
    }

    let should_remote = remote || host_opt.is_some();
    if should_remote {
        let resolved = crate::sync::resolve_host(host_opt);
        let host = resolved.as_str();
        println!("  Syncing to remote ({})...", host);
        let token_json = serde_json::to_string(&profile.token_data).unwrap_or_default();
        let b64 = base64::prelude::BASE64_STANDARD.encode(token_json.as_bytes());
        let py_cmd = format!(
            "python3 -c \"import base64; open('/root/.gemini/jetski-standalone-oauth-token', 'wb').write(base64.b64decode('{}'))\" && systemctl restart antigravity-desktop.service",
            b64
        );
        let res = Command::new("ssh")
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=6", host, &py_cmd])
            .output();

        match res {
            Ok(o) if o.status.success() => {
                println!("  {} Remote token updated and service restarted", green("✓"));
            }
            _ => {
                eprintln!("  {} Failed to update remote session", red("✗"));
            }
        }
    }

    if restart {
        if restart_local_antigravity() {
            println!("  {} Antigravity restarted", green("✓"));
        } else {
            eprintln!("  {} Failed to restart Antigravity", red("✗"));
        }
    } else if is_antigravity_running() {
        println!(
            "  {}",
            dim("Note: Antigravity is running. Restart it to apply the new session.")
        );
    }

    println!();
}

pub fn cmd_delete(name: &str) {
    let clean_name = name.trim().to_lowercase();
    let file_path = get_accounts_dir().join(format!("{}.json", clean_name));

    if !file_path.exists() {
        eprintln!("{}", red(&format!("Profile '{}' not found", clean_name)));
        std::process::exit(1);
    }

    let _ = fs::remove_file(&file_path);
    println!("{} Deleted profile '{}'", green("✓"), clean_name);
    remove_cache_account(&clean_name);
}

pub fn cmd_usage(name: Option<&str>, all: bool) {
    if all {
        cmd_list(false, None, false, false);
        return;
    }

    if is_antigravity_running() && name.is_none() {
        let (quota, status) = fetch_local_ls_quota_and_status();
        if let Some(q) = quota {
            let email = status
                .as_ref()
                .and_then(|s| s.get("email"))
                .and_then(|e| e.as_str())
                .unwrap_or("Current User");
            let tier = status
                .as_ref()
                .and_then(|s| s.get("userTier"))
                .and_then(|u| u.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("Pro");

            println!();
            println!(
                "{} ({}, {})",
                bold("Usage & Limits"),
                email,
                tier
            );
            print_quota_groups(&q);
            println!();
            return;
        }
    }

    let token = match name {
        Some(p_name) => {
            let file_path = get_accounts_dir().join(format!("{}.json", p_name.to_lowercase()));
            if !file_path.exists() {
                eprintln!("{}", red(&format!("Profile '{}' not found", p_name)));
                std::process::exit(1);
            }
            let content = fs::read_to_string(&file_path).unwrap_or_default();
            let profile: AccountProfile = serde_json::from_str(&content).unwrap();
            let token_obj = profile.token_data.get("token").cloned().unwrap_or(Value::Null);
            let mut at = token_obj
                .get("access_token")
                .and_then(|v| v.as_str())
                .map(String::from);
            let rt = token_obj
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(String::from);

            let quota = at.as_deref().and_then(fetch_upstream_quota_summary);
            if quota.is_none() && rt.is_some() {
                if let Some(refreshed) = refresh_google_oauth_token(rt.as_deref().unwrap()) {
                    if let Some(new_at) = refreshed.get("access_token").and_then(|v| v.as_str()) {
                        at = Some(new_at.to_string());
                    }
                }
            }
            (at, profile.email, profile.tier)
        }
        None => {
            let tok = match get_active_token() {
                Some(t) => t,
                None => {
                    eprintln!("{}", red("No active account token found"));
                    std::process::exit(1);
                }
            };
            let id_tok = tok.get("id_token").and_then(|v| v.as_str()).unwrap_or("");
            let jwt = parse_jwt_payload(id_tok);
            let email = jwt
                .as_ref()
                .and_then(|j| j.get("email"))
                .and_then(|e| e.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let token_obj = tok.get("token").cloned().unwrap_or(Value::Null);
            let mut at = token_obj
                .get("access_token")
                .and_then(|v| v.as_str())
                .map(String::from);
            let rt = token_obj
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(String::from);

            let quota = at.as_deref().and_then(fetch_upstream_quota_summary);
            if quota.is_none() && rt.is_some() {
                if let Some(refreshed) = refresh_google_oauth_token(rt.as_deref().unwrap()) {
                    if let Some(new_at) = refreshed.get("access_token").and_then(|v| v.as_str()) {
                        at = Some(new_at.to_string());
                    }
                }
            }
            (at, email, "Pro".to_string())
        }
    };

    let access_token = match token.0 {
        Some(at) => at,
        None => {
            eprintln!("{}", red("No valid access token available"));
            std::process::exit(1);
        }
    };

    let quota = match fetch_upstream_quota_summary(&access_token) {
        Some(q) => q,
        None => {
            eprintln!("{}", red("Failed to fetch quota from API"));
            std::process::exit(1);
        }
    };

    println!();
    println!(
        "{} ({}, {})",
        bold("Usage & Limits"),
        token.1,
        token.2
    );
    print_quota_groups(&quota);
    println!();
}

fn print_quota_groups(quota: &Value) {
    let groups = match quota.get("groups").and_then(|v| v.as_array()) {
        Some(g) => g,
        None => {
            println!("  {}", dim("No quota group data available"));
            return;
        }
    };

    for g in groups {
        let g_name = g
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or("Models");
        println!();
        println!("  {}", bold(g_name));

        if let Some(buckets) = g.get("buckets").and_then(|v| v.as_array()) {
            for b in buckets {
                let b_name = b
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Limit");
                let rem_frac = b
                    .get("remainingFraction")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let disabled = b
                    .get("disabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let reset_time = b
                    .get("resetTime")
                    .and_then(|v| v.as_str());

                if disabled {
                    println!(
                        "    {:<24} {}  {}",
                        b_name,
                        dim("[----------]"),
                        dim("disabled (limit reached)")
                    );
                } else {
                    let pct = rem_frac * 100.0;
                    let bar = make_bar(rem_frac, 10);
                    let pct_str = if pct <= 0.0 {
                        red("0.0% exhausted")
                    } else if pct < 20.0 {
                        yellow(&format!("{:5.1}% remaining", pct))
                    } else {
                        format!("{:5.1}% remaining", pct)
                    };

                    let rel = format_relative_time(reset_time);
                    println!(
                        "    {:<24} {}  {}  {}",
                        b_name,
                        bar,
                        pct_str,
                        dim(&format!("resets {}", rel))
                    );
                }
            }
        }
    }
}
