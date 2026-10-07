use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const CLOUDCODE_QUOTA_ENDPOINT: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaMetrics {
    pub gemini_weekly: Option<f64>,
    pub gemini_weekly_reset: Option<String>,
    pub gemini_5h: Option<f64>,
    pub gemini_5h_reset: Option<String>,
    pub claude_weekly: Option<f64>,
    pub claude_weekly_reset: Option<String>,
    pub claude_5h: Option<f64>,
    pub claude_5h_reset: Option<String>,
    pub claude_5h_disabled: bool,
}

pub fn is_antigravity_running() -> bool {
    Command::new("pgrep")
        .args(["-f", "antigravity"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn restart_local_antigravity() -> bool {
    let _ = Command::new("pkill").args(["-f", "antigravity"]).output();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let bin = find_antigravity_executable();
    Command::new(bin)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

fn find_antigravity_executable() -> String {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let p = Path::new(dir).join("antigravity");
            if p.is_file() {
                return p.to_string_lossy().to_string();
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let candidates = [
        format!("{}/.local/bin/antigravity", home),
        format!("{}/.antigravity/antigravity", home),
        "/usr/local/bin/antigravity".to_string(),
        "/usr/bin/antigravity".to_string(),
    ];
    for c in &candidates {
        if Path::new(c).is_file() {
            return c.clone();
        }
    }
    format!("{}/.antigravity/antigravity", home)
}

pub fn get_running_ls_credentials() -> (Option<String>, Option<String>) {
    let env_addr = std::env::var("ANTIGRAVITY_LS_ADDRESS").ok();
    let env_csrf = std::env::var("ANTIGRAVITY_CSRF_TOKEN").ok();
    if env_addr.is_some() && env_csrf.is_some() {
        return (env_addr, env_csrf);
    }

    let mut addr = None;
    let mut csrf = None;

    if let Ok(out) = Command::new("pgrep")
        .args(["-a", "-f", "language_server"])
        .output()
    {
        if out.status.success() {
            let txt = String::from_utf8_lossy(&out.stdout);
            if let Some(pos) = txt.find("--csrf_token") {
                let rem = &txt[pos + 12..];
                if let Some(token) = rem.split_whitespace().next() {
                    csrf = Some(token.to_string());
                }
            }
        }
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let log_path = PathBuf::from(home)
        .join(".config")
        .join("Antigravity")
        .join("logs")
        .join("language_server.log");

    if log_path.exists() {
        if let Ok(content) = fs::read_to_string(&log_path) {
            for line in content.lines().rev() {
                if let Some(pos) = line.find("Language server listening on random port at ") {
                    let rem = &line[pos + 44..];
                    if let Some(port_str) = rem.split_whitespace().next() {
                        addr = Some(format!("localhost:{}", port_str));
                        break;
                    }
                }
            }
        }
    }

    (addr, csrf)
}

pub fn fetch_local_ls_quota_and_status() -> (Option<Value>, Option<Value>) {
    let (addr, csrf) = get_running_ls_credentials();
    let (addr, csrf) = match (addr, csrf) {
        (Some(a), Some(c)) => (a, c),
        _ => return (None, None),
    };

    let call = |method: &str| -> Option<Value> {
        let url = format!(
            "http://{}/exa.language_server_pb.LanguageServerService/{}",
            addr, method
        );
        let resp = ureq::post(&url)
            .set("Content-Type", "application/json")
            .set("x-codeium-csrf-token", &csrf)
            .timeout(std::time::Duration::from_secs(5))
            .send_bytes(b"{}")
            .ok()?;
        resp.into_json::<Value>().ok()
    };

    let quota = call("RetrieveUserQuotaSummary").and_then(|v| v.get("response").cloned());
    let status = call("GetUserStatus").and_then(|v| v.get("userStatus").cloned());

    (quota, status)
}

pub fn get_google_oauth_credentials() -> (Option<String>, Option<String>) {
    let env_id = std::env::var("ANTIGRAVITY_OAUTH_CLIENT_ID").ok();
    let env_sec = std::env::var("ANTIGRAVITY_OAUTH_CLIENT_SECRET").ok();
    if env_id.is_some() && env_sec.is_some() {
        return (env_id, env_sec);
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let creds_file = crate::cache::config_dir().join("credentials.json");

    if creds_file.exists() {
        if let Ok(txt) = fs::read_to_string(&creds_file) {
            if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                let cid = val.get("client_id").and_then(|v| v.as_str()).map(String::from);
                let csec = val
                    .get("client_secret")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                if cid.is_some() && csec.is_some() {
                    return (cid, csec);
                }
            }
        }
    }

    let candidate_paths = [
        format!(
            "{}/.local/share/antigravity-ide/resources/app/out/main.js",
            home
        ),
        "/usr/share/antigravity-ide/resources/app/out/main.js".to_string(),
        "/opt/antigravity-ide/resources/app/out/main.js".to_string(),
    ];

    for p in &candidate_paths {
        if let Ok(txt) = fs::read_to_string(p) {
            if let (Some(cid), Some(csec)) = (extract_client_id(&txt), extract_client_secret(&txt)) {
                return (Some(cid), Some(csec));
            }
        }
    }

    (None, None)
}

fn extract_client_id(txt: &str) -> Option<String> {
    let marker = ".apps.googleusercontent.com";
    let pos = txt.find(marker)?;
    let start = txt[..pos].rfind(|c: char| !c.is_alphanumeric() && c != '-')?;
    Some(txt[start + 1..pos + marker.len()].to_string())
}

fn extract_client_secret(txt: &str) -> Option<String> {
    let marker = "GOCSPX-";
    let pos = txt.find(marker)?;
    if pos + 35 <= txt.len() {
        let sec = &txt[pos..pos + 35];
        if sec.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            return Some(sec.to_string());
        }
    }
    None
}

pub fn refresh_google_oauth_token(refresh_token: &str) -> Option<Value> {
    let (client_id, client_secret) = get_google_oauth_credentials();
    let (cid, csec) = match (client_id, client_secret) {
        (Some(i), Some(s)) => (i, s),
        _ => return None,
    };

    let params = [
        ("client_id", cid.as_str()),
        ("client_secret", csec.as_str()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];

    let resp = ureq::post("https://oauth2.googleapis.com/token")
        .timeout(std::time::Duration::from_secs(10))
        .send_form(&params)
        .ok()?;

    resp.into_json::<Value>().ok()
}

pub fn fetch_upstream_quota_summary(access_token: &str) -> Option<Value> {
    let resp = ureq::post(CLOUDCODE_QUOTA_ENDPOINT)
        .set("Authorization", &format!("Bearer {}", access_token))
        .set("Content-Type", "application/json")
        .set("User-Agent", "antigravity/2.18.1")
        .timeout(std::time::Duration::from_secs(10))
        .send_bytes(b"{}")
        .ok()?;

    resp.into_json::<Value>().ok()
}

pub fn extract_quota_metrics(quota: &Value) -> QuotaMetrics {
    let mut metrics = QuotaMetrics::default();
    let groups = match quota.get("groups").and_then(|v| v.as_array()) {
        Some(g) => g,
        None => return metrics,
    };

    for g in groups {
        let g_name = g
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let buckets = match g.get("buckets").and_then(|v| v.as_array()) {
            Some(b) => b,
            None => continue,
        };

        if g_name.contains("gemini") {
            for b in buckets {
                let window = b.get("window").and_then(|v| v.as_str()).unwrap_or("");
                let rf = b
                    .get("remainingFraction")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
                    * 100.0;
                let rt = b
                    .get("resetTime")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                if window == "weekly" {
                    metrics.gemini_weekly = Some((rf * 10.0).round() / 10.0);
                    metrics.gemini_weekly_reset = rt;
                } else if window == "5h" {
                    metrics.gemini_5h = Some((rf * 10.0).round() / 10.0);
                    metrics.gemini_5h_reset = rt;
                }
            }
        } else if g_name.contains("claude") || g_name.contains("3p") {
            for b in buckets {
                let window = b.get("window").and_then(|v| v.as_str()).unwrap_or("");
                let rf = b
                    .get("remainingFraction")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
                    * 100.0;
                let rt = b
                    .get("resetTime")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let disabled = b
                    .get("disabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if window == "weekly" {
                    metrics.claude_weekly = Some((rf * 10.0).round() / 10.0);
                    metrics.claude_weekly_reset = rt;
                } else if window == "5h" {
                    metrics.claude_5h = Some((rf * 10.0).round() / 10.0);
                    metrics.claude_5h_reset = rt;
                    metrics.claude_5h_disabled = disabled;
                }
            }
        }
    }

    metrics
}
