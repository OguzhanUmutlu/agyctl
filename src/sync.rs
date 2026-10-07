use crate::proto::{replace_bytes, rewrite_proto};
use crate::quota::{is_antigravity_running, restart_local_antigravity};
use crate::ui::*;
use base64::Engine;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;


pub fn detect_remote_user(host: &str) -> Option<String> {
    if let Ok(out) = Command::new("ssh").args(["-G", host]).output() {
        if out.status.success() {
            let txt = String::from_utf8_lossy(&out.stdout);
            for line in txt.lines() {
                if let Some(user) = line.strip_prefix("user ") {
                    let u = user.trim();
                    if !u.is_empty() {
                        return Some(u.to_string());
                    }
                }
            }
        }
    }
    None
}

pub fn get_remote_home(host: &str) -> String {
    if let Ok(env_home) = std::env::var("AGYCTL_REMOTE_HOME") {
        let trimmed = env_home.trim_end_matches('/');
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Some(user) = detect_remote_user(host) {
        if user == "root" {
            return "/root".to_string();
        } else {
            return format!("/home/{}", user);
        }
    }
    "/root".to_string()
}

pub fn get_saved_host() -> Option<String> {
    let config_path = crate::cache::config_dir().join("config.json");
    if let Ok(content) = fs::read_to_string(config_path) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(h) = val.get("host").or_else(|| val.get("default_host")).and_then(|v| v.as_str()) {
                if !h.trim().is_empty() {
                    return Some(h.trim().to_string());
                }
            }
        }
    }
    None
}

pub fn save_host(host: &str) {
    let config_dir = crate::cache::config_dir();
    let _ = fs::create_dir_all(&config_dir);
    let config_path = config_dir.join("config.json");
    let mut val = if let Ok(content) = fs::read_to_string(&config_path) {
        serde_json::from_str::<serde_json::Value>(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    if let Some(obj) = val.as_object_mut() {
        obj.insert("host".to_string(), serde_json::Value::String(host.to_string()));
    }
    let _ = fs::write(config_path, serde_json::to_string_pretty(&val).unwrap_or_default());
}

pub fn detect_first_ssh_config_host() -> Option<String> {
    let home = get_local_home();
    let ssh_config = Path::new(&home).join(".ssh").join("config");
    if let Ok(content) = fs::read_to_string(ssh_config) {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(host_part) = trimmed.strip_prefix("Host ") {
                let h = host_part.trim();
                if !h.is_empty()
                    && !h.contains('*')
                    && !h.contains('?')
                    && !h.eq_ignore_ascii_case("github.com")
                    && !h.eq_ignore_ascii_case("gitlab.com")
                {
                    return Some(h.to_string());
                }
            }
        }
    }
    None
}

pub fn resolve_host(host_opt: Option<&str>) -> String {
    if let Some(h) = host_opt {
        let clean = h.trim();
        if !clean.is_empty() && clean != "remote" {
            save_host(clean);
            return clean.to_string();
        }
    }
    if let Ok(env_h) = std::env::var("AGYCTL_HOST") {
        let clean = env_h.trim();
        if !clean.is_empty() {
            return clean.to_string();
        }
    }
    if let Some(saved) = get_saved_host() {
        return saved;
    }
    if let Some(first_ssh) = detect_first_ssh_config_host() {
        return first_ssh;
    }
    "remote".to_string()
}

pub fn get_local_home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
}

pub fn get_local_antigravity_dir() -> PathBuf {
    Path::new(&get_local_home())
        .join(".gemini")
        .join("antigravity")
}

pub fn get_local_conversations_dir() -> PathBuf {
    get_local_antigravity_dir().join("conversations")
}

pub fn get_local_brain_dir() -> PathBuf {
    get_local_antigravity_dir().join("brain")
}

pub fn get_local_annotations_dir() -> PathBuf {
    get_local_antigravity_dir().join("annotations")
}

pub fn get_local_summaries_db() -> PathBuf {
    get_local_antigravity_dir().join("conversation_summaries.db")
}

pub fn get_local_projects_dir() -> PathBuf {
    Path::new(&get_local_home()).join("Projects")
}


pub fn get_ssh_rsh() -> String {
    let home = get_local_home();
    let sock_dir = Path::new(&home).join(".ssh").join("sockets");
    let _ = fs::create_dir_all(&sock_dir);
    format!(
        "ssh -o BatchMode=yes -o ConnectTimeout=6 -o ControlMaster=auto -o ControlPath={}/%r@%h:%p -o ControlPersist=10m",
        sock_dir.display()
    )
}

pub fn checkpoint_sqlite_wal(db_path: &Path) {
    if !db_path.exists() {
        return;
    }
    if let Ok(conn) = Connection::open(db_path) {
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
}

pub fn rewrite_convo_db_paths(db_path: &Path, old_uri: &str, new_uri: &str) {
    if !db_path.exists() {
        return;
    }
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let has_table: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='trajectory_metadata_blob'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
        > 0;

    if !has_table {
        return;
    }

    let mut stmt = match conn.prepare("SELECT id, data FROM trajectory_metadata_blob") {
        Ok(s) => s,
        Err(_) => return,
    };

    let rows: Vec<(i64, Vec<u8>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|iter| iter.flatten().collect())
        .unwrap_or_default();

    let old_b = old_uri.as_bytes();
    let new_b = new_uri.as_bytes();

    for (id, blob) in rows {
        if blob.windows(old_b.len()).any(|w| w == old_b) {
            let rewritten = rewrite_proto(&blob, old_b, new_b);
            let _ = conn.execute(
                "UPDATE trajectory_metadata_blob SET data = ? WHERE id = ?",
                rusqlite::params![rewritten, id],
            );
        }
    }
}

pub fn merge_summaries(
    src_db: &Path,
    dst_db: &Path,
    old_uri: &str,
    new_uri: &str,
) -> usize {
    if !src_db.exists() || !dst_db.exists() {
        return 0;
    }

    checkpoint_sqlite_wal(dst_db);
    let backup_path = dst_db.with_extension("db.bak");
    let _ = fs::copy(dst_db, backup_path);

    let src_conn = match Connection::open(src_db) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let dst_conn = match Connection::open(dst_db) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let mut col_stmt = match dst_conn.prepare("PRAGMA table_info(conversation_summaries)") {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let dst_cols: Vec<String> = col_stmt
        .query_map([], |r| r.get(1))
        .map(|iter| iter.flatten().collect())
        .unwrap_or_default();

    let mut src_stmt = match src_conn.prepare("SELECT * FROM conversation_summaries") {
        Ok(s) => s,
        Err(_) => return 0,
    };

    let col_names: Vec<String> = src_stmt
        .column_names()
        .into_iter()
        .map(String::from)
        .collect();

    let rows = match src_stmt.query([]) {
        Ok(r) => r,
        Err(_) => return 0,
    };

    let old_b = old_uri.as_bytes();
    let new_b = new_uri.as_bytes();
    let raw_old = old_uri.replace("file://", "");
    let raw_new = new_uri.replace("file://", "");

    let mut rows_iter = rows;
    let mut updated_count = 0;

    while let Ok(Some(row)) = rows_iter.next() {
        let mut row_map: HashMap<String, rusqlite::types::Value> = HashMap::new();
        for (i, name) in col_names.iter().enumerate() {
            if let Ok(val) = row.get::<_, rusqlite::types::Value>(i) {
                row_map.insert(name.clone(), val);
            }
        }

        let cid = match row_map.get("conversation_id") {
            Some(rusqlite::types::Value::Text(s)) => s.clone(),
            _ => continue,
        };

        let src_mod = match row_map.get("last_modified_time") {
            Some(rusqlite::types::Value::Text(s)) => s.clone(),
            _ => String::new(),
        };

        let existing_mod: Option<String> = dst_conn
            .query_row(
                "SELECT last_modified_time FROM conversation_summaries WHERE conversation_id = ?",
                [&cid],
                |r| r.get(0),
            )
            .ok();

        if let Some(existing) = existing_mod {
            if existing >= src_mod {
                continue;
            }
        }

        if let Some(rusqlite::types::Value::Text(u)) = row_map.get_mut("workspace_uris") {
            *u = u.replace(old_uri, new_uri).replace(&raw_old, &raw_new);
        }

        if let Some(rusqlite::types::Value::Blob(b)) = row_map.get_mut("raw_summary") {
            if b.windows(old_b.len()).any(|w| w == old_b) {
                *b = rewrite_proto(b, old_b, new_b);
            }
        }

        let valid_cols: Vec<&String> = dst_cols.iter().filter(|c| row_map.contains_key(*c)).collect();
        let col_str = valid_cols
            .iter()
            .map(|c| format!("`{}`", c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = valid_cols
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ");

        let query = format!(
            "INSERT OR REPLACE INTO conversation_summaries ({}) VALUES ({})",
            col_str, placeholders
        );

        let vals: Vec<&rusqlite::types::Value> = valid_cols
            .iter()
            .filter_map(|c| row_map.get(*c))
            .collect();

        if dst_conn
            .execute(&query, rusqlite::params_from_iter(vals))
            .is_ok()
        {
            updated_count += 1;
        }
    }

    updated_count
}

pub fn localize_brain_paths(host: &str) {
    let r_home = get_remote_home(host);
    let old_uri = format!("file://{}/Projects/", r_home);
    let new_uri = format!("file://{}/Projects/", get_local_home());
    let old_fs = format!("{}/Projects/", r_home);
    let new_fs = format!("{}/Projects/", get_local_home());

    let old_uri_b = old_uri.as_bytes();
    let new_uri_b = new_uri.as_bytes();
    let old_fs_b = old_fs.as_bytes();
    let new_fs_b = new_fs.as_bytes();

    for entry in walkdir::WalkDir::new(get_local_brain_dir()).into_iter().flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                if matches!(ext, "md" | "json" | "jsonl" | "txt" | "log") {
                    if let Ok(bytes) = fs::read(path) {
                        if bytes.windows(old_uri_b.len()).any(|w| w == old_uri_b)
                            || bytes.windows(old_fs_b.len()).any(|w| w == old_fs_b)
                        {
                            let step1 = replace_bytes(&bytes, old_uri_b, new_uri_b);
                            let step2 = replace_bytes(&step1, old_fs_b, new_fs_b);
                            let _ = fs::write(path, step2);
                        }
                    }
                }
            }
        }
    }
}

pub fn get_local_conversations() -> HashMap<String, (String, String, String)> {
    let db = get_local_summaries_db();
    if !db.exists() {
        return HashMap::new();
    }
    checkpoint_sqlite_wal(&db);

    let conn = match Connection::open(&db) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    let mut stmt = match conn.prepare(
        "SELECT conversation_id, title, last_modified_time, workspace_uris FROM conversation_summaries",
    ) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };

    stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            (
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            ),
        ))
    })
    .map(|iter| iter.flatten().collect())
    .unwrap_or_default()
}

pub fn get_remote_conversations(host: &str) -> HashMap<String, (String, String, String)> {
    let r_home = get_remote_home(host);
    let py = format!(
        "import sqlite3, json\ncon = sqlite3.connect('{}/.gemini/antigravity/conversation_summaries.db')\nrows = con.execute('SELECT conversation_id, title, last_modified_time, workspace_uris FROM conversation_summaries').fetchall()\nprint(json.dumps([list(r) for r in rows]))\ncon.close()\n",
        r_home
    );
    let b64 = base64::prelude::BASE64_STANDARD.encode(py.as_bytes());
    let cmd = format!("python3 -c \"import base64; exec(base64.b64decode('{}').decode())\"", b64);

    let res = Command::new("ssh")
        .args([
            "-o", "BatchMode=yes",
            "-o", "ConnectTimeout=6",
            host,
            &cmd,
        ])
        .output();

    let mut map = HashMap::new();
    if let Ok(o) = res {
        if o.status.success() {
            let out_str = String::from_utf8_lossy(&o.stdout);
            if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&out_str) {
                for item in items {
                    if let Value::Array(cols) = item {
                        if cols.len() >= 4 {
                            let cid = cols[0].as_str().unwrap_or("").to_string();
                            let title = cols[1].as_str().unwrap_or("").to_string();
                            let lmod = cols[2].as_str().unwrap_or("").to_string();
                            let uris = cols[3].as_str().unwrap_or("").to_string();
                            map.insert(cid, (title, lmod, uris));
                        }
                    }
                }
            }
        }
    }
    map
}

pub fn extract_project_name(uri: &str) -> Option<String> {
    if let Some(pos) = uri.find("Projects/") {
        let rem = &uri[pos + 9..];
        let name = rem
            .split(['/', '"', '\'', ']', '['])
            .next()?
            .trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

pub fn sync_single_project(name: &str, direction: &str, host: &str, dry_run: bool) -> bool {
    let local = get_local_projects_dir().join(name);
    let remote = format!("{}/Projects/{}", get_remote_home(host), name);
    let rsh = format!("-e={}", get_ssh_rsh());

    let excludes = [
        "--exclude=target",
        "--exclude=node_modules",
        "--exclude=dist",
        "--exclude=build",
        "--exclude=__pycache__",
        "--exclude=.venv",
        "--exclude=*.pyc",
        "--exclude=.DS_Store",
    ];

    println!(
        "  {} project '{}' [{}]",
        dim("Syncing"),
        bold(name),
        direction
    );

    if direction == "pull" {
        let _ = fs::create_dir_all(&local);
        let mut args = vec!["-avz", "--update", &rsh];
        if dry_run {
            args.push("-n");
        }
        args.extend(excludes);
        let remote_src = format!("{}:{}/", host, remote);
        let local_dst = format!("{}/", local.display());
        args.push(&remote_src);
        args.push(&local_dst);

        let res = Command::new("rsync").args(&args).status();
        match res {
            Ok(s) if s.success() => {
                println!("    {} Pulled successfully", green("✓"));
                true
            }
            _ => {
                eprintln!("    {} Pull failed", red("✗"));
                false
            }
        }
    } else {
        if !local.exists() {
            eprintln!("    {} Local directory does not exist", red("✗"));
            return false;
        }

        let _ = Command::new("ssh")
            .args([
                "-o", "BatchMode=yes",
                "-o", "ConnectTimeout=6",
                host,
                &format!("mkdir -p {}/Projects", get_remote_home(host)),
            ])
            .status();

        let mut args = vec!["-avz", "--update", &rsh];
        if dry_run {
            args.push("-n");
        }
        args.extend(excludes);
        let local_src = format!("{}/", local.display());
        let remote_dst = format!("{}:{}/", host, remote);
        args.push(&local_src);
        args.push(&remote_dst);

        let res = Command::new("rsync").args(&args).status();
        match res {
            Ok(s) if s.success() => {
                println!("    {} Pushed successfully", green("✓"));
                true
            }
            _ => {
                eprintln!("    {} Push failed", red("✗"));
                false
            }
        }
    }
}

pub fn cmd_status(host_opt: Option<&str>) {
    let resolved = resolve_host(host_opt);
    let host = resolved.as_str();
    println!();
    println!("Connecting to {}...", dim(host));
    let local = get_local_conversations();
    let remote = get_remote_conversations(host);

    let local_ids: HashSet<_> = local.keys().cloned().collect();
    let remote_ids: HashSet<_> = remote.keys().cloned().collect();

    let only_remote = remote_ids.difference(&local_ids).count();
    let only_local = local_ids.difference(&remote_ids).count();

    let mut newer_remote = 0;
    let mut newer_local = 0;
    for id in local_ids.intersection(&remote_ids) {
        let l_time = &local[id].1;
        let r_time = &remote[id].1;
        if r_time > l_time {
            newer_remote += 1;
        } else if l_time > r_time {
            newer_local += 1;
        }
    }

    let mut projects = HashSet::new();
    for (_, (_, _, uris)) in local.iter().chain(remote.iter()) {
        if let Some(p) = extract_project_name(uris) {
            projects.insert(p);
        }
    }

    println!();
    println!("{} ({})", bold("History Status"), host);
    println!("  {:<26} {}", dim("Local Conversations:"), local.len());
    println!("  {:<26} {}", dim("Remote Conversations:"), remote.len());
    println!("  {:<26} {}", dim("Only on Remote (pull):"), only_remote);
    println!("  {:<26} {}", dim("Only on Local (push):"), only_local);
    println!("  {:<26} {}", dim("Newer on Remote:"), newer_remote);
    println!("  {:<26} {}", dim("Newer on Local:"), newer_local);

    if !projects.is_empty() {
        let mut p_list: Vec<_> = projects.into_iter().collect();
        p_list.sort();
        println!("  {:<26} {}", dim("Projects in History:"), p_list.join(", "));
    }
    println!();
}

pub fn cmd_pull(
    host_opt: Option<&str>,
    projects: &[String],
    all_projects: bool,
    dry_run: bool,
    restart: bool,
) {
    let resolved = resolve_host(host_opt);
    let host = resolved.as_str();
    println!();
    println!("{} Pulling from {}...", bold("Sync:"), host);

    let rsh = format!("-e={}", get_ssh_rsh());
    let r_home = get_remote_home(host);
    let l_home = get_local_home();
    let dry_arg = if dry_run { vec!["-n"] } else { vec![] };

    let tmp_dir = std::env::temp_dir().join(format!("agyctl_pull_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp_dir);

    let remote_db = format!("{}:{}/.gemini/antigravity/conversation_summaries.db", host, r_home);
    let staged_db = tmp_dir.join("remote_summaries.db");

    let _ = Command::new("rsync")
        .args(["-az", &rsh])
        .args(&dry_arg)
        .arg(&remote_db)
        .arg(&staged_db)
        .status();

    let convos_dir = get_local_conversations_dir();
    let _ = fs::create_dir_all(&convos_dir);
    let remote_convos = format!("{}:{}/.gemini/antigravity/conversations/", host, r_home);
    let local_convos = format!("{}/", convos_dir.display());

    println!("  Syncing conversation files...");
    let _ = Command::new("rsync")
        .args([
            "-avz",
            "--update",
            "--exclude=*-wal",
            "--exclude=*-shm",
            &rsh,
        ])
        .args(&dry_arg)
        .arg(&remote_convos)
        .arg(&local_convos)
        .status();

    if !dry_run {
        let old_uri = format!("file://{}/Projects/", r_home);
        let new_uri = format!("file://{}/Projects/", l_home);
        if let Ok(entries) = fs::read_dir(&convos_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("db") {
                    rewrite_convo_db_paths(&p, &old_uri, &new_uri);
                }
            }
        }
    }

    let brain_dir = get_local_brain_dir();
    let _ = fs::create_dir_all(&brain_dir);
    let remote_brain = format!("{}:{}/.gemini/antigravity/brain/", host, r_home);
    let local_brain = format!("{}/", brain_dir.display());

    println!("  Syncing artifacts and transcripts...");
    let _ = Command::new("rsync")
        .args([
            "-avz",
            "--update",
            "--exclude=*.sock",
            "--exclude=.tempmediaStorage",
            &rsh,
        ])
        .args(&dry_arg)
        .arg(&remote_brain)
        .arg(&local_brain)
        .status();

    if !dry_run {
        localize_brain_paths(host);
    }

    let annotations_dir = get_local_annotations_dir();
    let _ = fs::create_dir_all(&annotations_dir);
    let remote_annotations = format!("{}:{}/.gemini/antigravity/annotations/", host, r_home);
    let local_annotations = format!("{}/", annotations_dir.display());

    let _ = Command::new("rsync")
        .args(["-avz", "--update", &rsh])
        .args(&dry_arg)
        .arg(&remote_annotations)
        .arg(&local_annotations)
        .status();

    if !dry_run && staged_db.exists() {
        let old_uri = format!("file://{}/Projects/", r_home);
        let new_uri = format!("file://{}/Projects/", l_home);
        let merged = merge_summaries(&staged_db, &get_local_summaries_db(), &old_uri, &new_uri);
        println!("  Merged {} summaries into local history", merged);
    }

    let _ = fs::remove_dir_all(&tmp_dir);

    let mut targets = HashSet::new();
    for p in projects {
        targets.insert(p.clone());
    }
    if all_projects {
        let local_conv = get_local_conversations();
        let remote_conv = get_remote_conversations(host);
        for (_, (_, _, uris)) in local_conv.iter().chain(remote_conv.iter()) {
            if let Some(p) = extract_project_name(uris) {
                targets.insert(p);
            }
        }
    }

    for p in targets {
        sync_single_project(&p, "pull", host, dry_run);
    }

    println!("{} Pull completed", green("✓"));

    if restart && is_antigravity_running() {
        println!("  Restarting local Antigravity...");
        let _ = restart_local_antigravity();
    }
    println!();
}

pub fn cmd_push(
    host_opt: Option<&str>,
    projects: &[String],
    all_projects: bool,
    dry_run: bool,
    no_restart: bool,
) {
    let resolved = resolve_host(host_opt);
    let host = resolved.as_str();
    println!();
    println!("{} Pushing to {}...", bold("Sync:"), host);

    let rsh = format!("-e={}", get_ssh_rsh());
    let r_home = get_remote_home(host);
    let l_home = get_local_home();
    let dry_arg = if dry_run { vec!["-n"] } else { vec![] };

    checkpoint_sqlite_wal(&get_local_summaries_db());
    let convos_dir = get_local_conversations_dir();
    if let Ok(entries) = fs::read_dir(&convos_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("db") {
                checkpoint_sqlite_wal(&p);
            }
        }
    }

    let tmp_dir = std::env::temp_dir().join(format!("agyctl_push_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp_dir);
    let convo_staging = tmp_dir.join("conversations");
    let _ = fs::create_dir_all(&convo_staging);

    let old_uri = format!("file://{}/Projects/", l_home);
    let new_uri = format!("file://{}/Projects/", r_home);

    if let Ok(entries) = fs::read_dir(&convos_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("db") {
                let staged = convo_staging.join(p.file_name().unwrap());
                let _ = fs::copy(&p, &staged);
                rewrite_convo_db_paths(&staged, &old_uri, &new_uri);
            }
        }
    }

    let remote_convos = format!("{}:{}/.gemini/antigravity/conversations/", host, r_home);
    let local_staged = format!("{}/", convo_staging.display());

    println!("  Uploading conversation databases...");
    let _ = Command::new("rsync")
        .args([
            "-avz",
            "--update",
            &rsh,
        ])
        .args(&dry_arg)
        .arg(&local_staged)
        .arg(&remote_convos)
        .status();

    let remote_brain = format!("{}:{}/.gemini/antigravity/brain/", host, r_home);
    let local_brain = format!("{}/", get_local_brain_dir().display());

    println!("  Uploading brain artifacts...");
    let _ = Command::new("rsync")
        .args([
            "-avz",
            "--update",
            "--exclude=*.sock",
            "--exclude=.tempmediaStorage",
            &rsh,
        ])
        .args(&dry_arg)
        .arg(&local_brain)
        .arg(&remote_brain)
        .status();

    let remote_annotations = format!("{}:{}/.gemini/antigravity/annotations/", host, r_home);
    let local_annotations = format!("{}/", get_local_annotations_dir().display());

    let _ = Command::new("rsync")
        .args(["-avz", "--update", &rsh])
        .args(&dry_arg)
        .arg(&local_annotations)
        .arg(&remote_annotations)
        .status();

    let remote_stage_db = "/tmp/local_summaries_staging.db";
    let _ = Command::new("rsync")
        .args(["-az", &rsh])
        .arg(get_local_summaries_db().to_string_lossy().to_string())
        .arg(format!("{}:{}", host, remote_stage_db))
        .status();

    let py_merge = format!(
        "import sqlite3\nsrc = sqlite3.connect('{}')\ndst = sqlite3.connect('{}/.gemini/antigravity/conversation_summaries.db')\ncols = [c[1] for c in dst.execute('PRAGMA table_info(conversation_summaries)').fetchall()]\nupdated = 0\nfor r in src.execute('SELECT * FROM conversation_summaries').fetchall():\n    d = dict(zip([c[0] for c in src.execute('PRAGMA table_info(conversation_summaries)').fetchall()], r))\n    cid = d['conversation_id']\n    ex = dst.execute('SELECT last_modified_time FROM conversation_summaries WHERE conversation_id = ?', (cid,)).fetchone()\n    if ex and ex[0] >= d['last_modified_time']: continue\n    if d.get('workspace_uris'): d['workspace_uris'] = d['workspace_uris'].replace('{}', '{}')\n    valid = [c for c in cols if c in d]\n    q = f\"INSERT OR REPLACE INTO conversation_summaries ({{', '.join(valid)}}) VALUES ({{', '.join(['?']*len(valid))}})\"\n    dst.execute(q, [d[c] for c in valid])\n    updated += 1\ndst.commit()\nsrc.close()\ndst.close()\nprint(f'Remote merged {{updated}} summaries')\n",
        remote_stage_db, r_home, old_uri, new_uri
    );
    let b64 = base64::prelude::BASE64_STANDARD.encode(py_merge.as_bytes());
    let _ = Command::new("ssh")
        .args([
            "-o", "BatchMode=yes",
            "-o", "ConnectTimeout=6",
            host,
            &format!("python3 -c \"import base64; exec(base64.b64decode('{}').decode())\" && rm -f {}", b64, remote_stage_db),
        ])
        .status();

    let _ = fs::remove_dir_all(&tmp_dir);

    let mut targets = HashSet::new();
    for p in projects {
        targets.insert(p.clone());
    }
    if all_projects {
        let local_conv = get_local_conversations();
        let remote_conv = get_remote_conversations(host);
        for (_, (_, _, uris)) in local_conv.iter().chain(remote_conv.iter()) {
            if let Some(p) = extract_project_name(uris) {
                targets.insert(p);
            }
        }
    }

    for p in targets {
        sync_single_project(&p, "push", host, dry_run);
    }

    if !no_restart && !dry_run {
        println!("  Restarting remote service...");
        let _ = Command::new("ssh")
            .args([
                "-o", "BatchMode=yes",
                "-o", "ConnectTimeout=6",
                host,
                "systemctl restart antigravity-desktop.service",
            ])
            .status();
    }

    println!("{} Push completed", green("✓"));
    println!();
}

pub fn cmd_project_list(host_opt: Option<&str>, local_only: bool) {
    let resolved = resolve_host(host_opt);
    let host = resolved.as_str();
    let local_dir = get_local_projects_dir();
    let mut names = HashSet::new();

    if let Ok(entries) = fs::read_dir(&local_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(n) = entry.file_name().to_str() {
                    names.insert(n.to_string());
                }
            }
        }
    }

    let local_conv = get_local_conversations();
    for (_, (_, _, uris)) in &local_conv {
        if let Some(p) = extract_project_name(uris) {
            names.insert(p);
        }
    }

    if !local_only {
        let remote_conv = get_remote_conversations(host);
        for (_, (_, _, uris)) in &remote_conv {
            if let Some(p) = extract_project_name(uris) {
                names.insert(p);
            }
        }
    }

    let mut sorted: Vec<_> = names.into_iter().collect();
    sorted.sort();

    println!();
    println!("{}", bold("Known Projects"));
    println!(
        "  {:<20} {:<32} {}",
        dim("NAME"),
        dim("LOCAL PATH"),
        dim("STATUS")
    );

    for name in sorted {
        let local_p = local_dir.join(&name);
        let exists = local_p.exists();
        let status = if exists {
            green("local")
        } else {
            dim("remote only")
        };
        let disp_path = format!("~/Projects/{}", name);
        println!("  {:<20} {:<32} {}", name, disp_path, status);
    }
    println!();
}
