use crate::account::{get_accounts_dir, AccountProfile};
use crate::quota::{
    extract_quota_metrics, fetch_local_ls_quota_and_status, fetch_upstream_quota_summary,
    is_antigravity_running, refresh_google_oauth_token, QuotaMetrics,
};
use crate::secret::{get_active_token, parse_jwt_payload};
use crate::ui::clean_tier;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    pub timestamp: u64,
    pub gemini_5h: Option<f64>,
    pub gemini_weekly: Option<f64>,
    pub claude_5h: Option<f64>,
    pub claude_weekly: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountCacheEntry {
    pub name: String,
    pub email: String,
    pub tier: String,
    pub is_active: bool,
    pub plan_expiration: Option<String>,
    pub token_expiry: Option<String>,
    pub quota: QuotaMetrics,
    pub last_refreshed: String,
    pub error: Option<String>,
    #[serde(default)]
    pub history: Vec<QuotaSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheData {
    pub updated_at: String,
    pub updated_timestamp: u64,
    pub antigravity_running: bool,
    pub active_account_email: Option<String>,
    pub accounts: HashMap<String, AccountCacheEntry>,
}

const OTHER_ACCOUNT_TTL_SECS: i64 = 60;

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    Path::new(&home).join(".config").join("agyctl")
}

pub fn get_cache_file() -> PathBuf {
    config_dir().join("cache.json")
}

pub fn load_cache(max_age_seconds: Option<u64>) -> Option<CacheData> {
    let path = get_cache_file();
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    let data: CacheData = serde_json::from_str(&content).ok()?;
    if let Some(max_age) = max_age_seconds {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now.saturating_sub(data.updated_timestamp) > max_age {
            return None;
        }
    }
    Some(data)
}

pub fn rename_cache_account(old_name: &str, new_name: &str, tier: Option<&str>, expires: Option<Option<&str>>) {
    if let Some(mut cache) = load_cache(None) {
        if let Some(mut entry) = cache.accounts.remove(old_name) {
            entry.name = new_name.to_string();
            if let Some(t) = tier {
                entry.tier = t.to_string();
            }
            if let Some(exp) = expires {
                entry.plan_expiration = exp.map(String::from);
            }
            cache.accounts.insert(new_name.to_string(), entry);
            let cache_file = get_cache_file();
            let tmp_file = cache_file.with_extension("tmp");
            if let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
                if fs::write(&tmp_file, bytes).is_ok() {
                    let _ = fs::rename(&tmp_file, &cache_file);
                }
            }
        }
    }
}

pub fn remove_cache_account(name: &str) {
    if let Some(mut cache) = load_cache(None) {
        if cache.accounts.remove(name).is_some() {
            let cache_file = get_cache_file();
            let tmp_file = cache_file.with_extension("tmp");
            if let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
                if fs::write(&tmp_file, bytes).is_ok() {
                    let _ = fs::rename(&tmp_file, &cache_file);
                }
            }
        }
    }
}

pub fn update_cache(verbose: bool) -> CacheData {
    update_cache_scoped(verbose, RefreshScope::Stale)
}

#[derive(Clone, Copy, PartialEq)]
pub enum RefreshScope {
    Stale,
    ActiveOnly,
    All,
}

pub fn update_cache_scoped(verbose: bool, scope: RefreshScope) -> CacheData {
    let accounts_dir = get_accounts_dir();
    let _ = fs::create_dir_all(&accounts_dir);

    let prev_cache = load_cache(None);

    let current_token = get_active_token();
    let mut current_email = None;
    if let Some(tok) = &current_token {
        if let Some(id_tok) = tok.get("id_token").and_then(|v| v.as_str()) {
            if let Some(jwt) = parse_jwt_payload(id_tok) {
                current_email = jwt.get("email").and_then(|e| e.as_str()).map(String::from);
            }
        }
    }

    let running = is_antigravity_running();
    let now = chrono::Local::now();
    let now_iso = now.to_rfc3339();
    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let (live_quota, live_status) = if running {
        fetch_local_ls_quota_and_status()
    } else {
        (None, None)
    };

    let mut accounts_map = HashMap::new();
    let mut seen_emails = Vec::new();

    if let Ok(entries) = fs::read_dir(&accounts_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    accounts_map.insert(
                        name.clone(),
                        AccountCacheEntry {
                            name,
                            email: "Unknown".into(),
                            tier: "Pro".into(),
                            is_active: false,
                            plan_expiration: None,
                            token_expiry: None,
                            quota: QuotaMetrics::default(),
                            last_refreshed: now_iso.clone(),
                            error: Some(e.to_string()),
                            history: Vec::new(),
                        },
                    );
                    continue;
                }
            };

            let mut profile: AccountProfile = match serde_json::from_str(&content) {
                Ok(p) => p,
                Err(e) => {
                    accounts_map.insert(
                        name.clone(),
                        AccountCacheEntry {
                            name,
                            email: "Unknown".into(),
                            tier: "Pro".into(),
                            is_active: false,
                            plan_expiration: None,
                            token_expiry: None,
                            quota: QuotaMetrics::default(),
                            last_refreshed: now_iso.clone(),
                            error: Some(e.to_string()),
                            history: Vec::new(),
                        },
                    );
                    continue;
                }
            };

            let is_active = current_email.as_deref() == Some(&profile.email);
            seen_emails.push(profile.email.clone());

            let mut tier = profile.tier.clone();
            let mut quota_metrics = QuotaMetrics::default();
            let mut refreshed_at = now_iso.clone();

            let reusable = if is_active || scope == RefreshScope::All {
                None
            } else {
                prev_cache
                    .as_ref()
                    .and_then(|c| c.accounts.get(&name))
                    .filter(|p| p.error.is_none() && p.quota.gemini_5h.is_some())
                    .filter(|p| {
                        scope == RefreshScope::ActiveOnly
                            || chrono::DateTime::parse_from_rfc3339(&p.last_refreshed)
                            .map(|dt| {
                                let age = now.signed_duration_since(dt).num_seconds();
                                (0..OTHER_ACCOUNT_TTL_SECS).contains(&age)
                            })
                            .unwrap_or(false)
                    })
            };

            if is_active && live_quota.is_some() {
                if let Some(lq) = &live_quota {
                    quota_metrics = extract_quota_metrics(lq);
                }
                if let Some(ls) = &live_status {
                    if let Some(t) = ls.get("userTier").and_then(|u| u.get("name")).and_then(|n| n.as_str()) {
                        tier = t.to_string();
                    }
                }
            } else if let Some(p) = reusable {
                quota_metrics = p.quota.clone();
                refreshed_at = p.last_refreshed.clone();
            } else {
                let token_obj = profile.token_data.get("token").cloned().unwrap_or(Value::Null);
                let mut access_token = token_obj
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let refresh_token = token_obj
                    .get("refresh_token")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                let mut quota = access_token
                    .as_deref()
                    .and_then(fetch_upstream_quota_summary);

                if quota.is_none() && refresh_token.is_some() {
                    if let Some(refreshed) = refresh_google_oauth_token(refresh_token.as_deref().unwrap()) {
                        if let Some(new_at) = refreshed.get("access_token").and_then(|v| v.as_str()) {
                            access_token = Some(new_at.to_string());
                            if let Some(tok_mut) = profile.token_data.get_mut("token").and_then(|v| v.as_object_mut()) {
                                tok_mut.insert("access_token".to_string(), Value::String(new_at.to_string()));
                                let _ = fs::write(&path, serde_json::to_string_pretty(&profile).unwrap_or_default());
                            }
                            quota = fetch_upstream_quota_summary(new_at);
                        }
                    }
                }

                if let Some(q) = quota {
                    quota_metrics = extract_quota_metrics(&q);
                }
            }

            let token_expiry = profile
                .token_data
                .get("token")
                .and_then(|t| t.get("expiry"))
                .and_then(|e| e.as_str())
                .map(String::from);

            let mut history = prev_cache
                .as_ref()
                .and_then(|c| c.accounts.get(&name))
                .map(|a| a.history.clone())
                .unwrap_or_default();

            let should_append = history
                .last()
                .map_or(true, |last| now_ts.saturating_sub(last.timestamp) >= 15);

            if should_append {
                history.push(QuotaSnapshot {
                    timestamp: now_ts,
                    gemini_5h: quota_metrics.gemini_5h,
                    gemini_weekly: quota_metrics.gemini_weekly,
                    claude_5h: quota_metrics.claude_5h,
                    claude_weekly: quota_metrics.claude_weekly,
                });
            }

            history.retain(|s| now_ts.saturating_sub(s.timestamp) <= 2700);

            accounts_map.insert(
                name.clone(),
                AccountCacheEntry {
                    name,
                    email: profile.email,
                    tier,
                    is_active,
                    plan_expiration: profile.plan_expiration,
                    token_expiry,
                    quota: quota_metrics,
                    last_refreshed: refreshed_at,
                    error: None,
                    history,
                },
            );
        }
    }

    if let Some(active_email) = &current_email {
        if !seen_emails.contains(active_email) {
            let mut tier = "AI Pro".to_string();
            if let Some(ls) = &live_status {
                if let Some(t) = ls.get("userTier").and_then(|u| u.get("name")).and_then(|n| n.as_str()) {
                    tier = clean_tier(t, 20);
                }
            }
            let quota_metrics = live_quota
                .as_ref()
                .map(extract_quota_metrics)
                .unwrap_or_default();

            let mut n = 1;
            let auto_name = loop {
                let cand = format!("account{}", n);
                let f = accounts_dir.join(format!("{}.json", cand));
                if !f.exists() && !accounts_map.contains_key(&cand) {
                    break cand;
                }
                n += 1;
            };

            let auto_profile = AccountProfile {
                name: auto_name.clone(),
                email: active_email.clone(),
                tier: tier.clone(),
                plan_expiration: None,
                created_at: now_iso.clone(),
                token_data: current_token.clone().unwrap_or(Value::Null),
            };
            let _ = fs::write(
                accounts_dir.join(format!("{}.json", auto_name)),
                serde_json::to_string_pretty(&auto_profile).unwrap_or_default(),
            );

            let mut history = prev_cache
                .as_ref()
                .and_then(|c| c.accounts.get(&auto_name))
                .map(|a| a.history.clone())
                .unwrap_or_default();

            let should_append = history
                .last()
                .map_or(true, |last| now_ts.saturating_sub(last.timestamp) >= 15);

            if should_append {
                history.push(QuotaSnapshot {
                    timestamp: now_ts,
                    gemini_5h: quota_metrics.gemini_5h,
                    gemini_weekly: quota_metrics.gemini_weekly,
                    claude_5h: quota_metrics.claude_5h,
                    claude_weekly: quota_metrics.claude_weekly,
                });
            }

            history.retain(|s| now_ts.saturating_sub(s.timestamp) <= 2700);

            accounts_map.insert(
                auto_name.clone(),
                AccountCacheEntry {
                    name: auto_name,
                    email: active_email.clone(),
                    tier,
                    is_active: true,
                    plan_expiration: None,
                    token_expiry: current_token
                        .as_ref()
                        .and_then(|t| t.get("token"))
                        .and_then(|t| t.get("expiry"))
                        .and_then(|e| e.as_str())
                        .map(String::from),
                    quota: quota_metrics,
                    last_refreshed: now_iso.clone(),
                    error: None,
                    history,
                },
            );
        }
    }

    let cache_data = CacheData {
        updated_at: now_iso,
        updated_timestamp: now_ts,
        antigravity_running: running,
        active_account_email: current_email,
        accounts: accounts_map,
    };

    let cache_file = get_cache_file();
    if let Some(parent) = cache_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp_file = cache_file.with_extension("tmp");
    if let Ok(bytes) = serde_json::to_vec_pretty(&cache_data) {
        if fs::write(&tmp_file, bytes).is_ok() {
            let _ = fs::rename(&tmp_file, &cache_file);
        }
    }

    if verbose {
        eprintln!(
            "Updated quota cache for {} accounts",
            cache_data.accounts.len()
        );
    }

    cache_data
}

pub fn run_daemon(interval_seconds: u64, run_once: bool) {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    let _ = ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    });

    while running.load(Ordering::SeqCst) {
        update_cache(true);
        if run_once {
            break;
        }
        for _ in 0..interval_seconds {
            if !running.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}
