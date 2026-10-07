use crate::ui::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn get_systemd_user_service_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    Path::new(&home)
        .join(".config")
        .join("systemd")
        .join("user")
        .join("agyctl.service")
}

pub fn is_service_active() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "agyctl.service"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn manage_service(action: &str) {
    match action {
        "enable" | "install" => {
            let service_path = get_systemd_user_service_path();
            if let Some(parent) = service_path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            let agyctl_bin = find_self_executable();
            let unit = format!(
                "[Unit]\nDescription=Antigravity Quota Cache Daemon (agyctl)\nAfter=network.target\n\n[Service]\nType=simple\nExecStart={} daemon --interval 10\nRestart=always\nRestartSec=10\n\n[Install]\nWantedBy=default.target\n",
                agyctl_bin
            );

            if let Err(e) = fs::write(&service_path, unit) {
                eprintln!("{}", red(&format!("Failed to write service unit: {}", e)));
                std::process::exit(1);
            }

            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();

            let status = Command::new("systemctl")
                .args(["--user", "enable", "--now", "agyctl.service"])
                .status();

            match status {
                Ok(s) if s.success() => {
                    println!(
                        "{} Service agyctl.service enabled and started",
                        green("✓")
                    );
                }
                _ => {
                    eprintln!("{}", red("Failed to enable agyctl.service"));
                    std::process::exit(1);
                }
            }
        }
        "disable" => {
            let status = Command::new("systemctl")
                .args(["--user", "disable", "--now", "agyctl.service"])
                .status();

            match status {
                Ok(s) if s.success() => {
                    println!("{} Service agyctl.service disabled and stopped", green("✓"));
                }
                _ => {
                    eprintln!("{}", red("Failed to disable agyctl.service"));
                    std::process::exit(1);
                }
            }
        }
        "status" => {
            let _ = Command::new("systemctl")
                .args(["--user", "status", "agyctl.service"])
                .status();
        }
        "logs" => {
            let _ = Command::new("journalctl")
                .args([
                    "--user",
                    "-u",
                    "agyctl.service",
                    "-n",
                    "40",
                    "--no-pager",
                ])
                .status();
        }
        "start" | "stop" | "restart" => {
            let res = Command::new("systemctl")
                .args(["--user", action, "agyctl.service"])
                .status();
            match res {
                Ok(s) if s.success() => {
                    println!("{} Service agyctl.service {}ed", green("✓"), action);
                }
                _ => {
                    eprintln!("{}", red(&format!("Failed to {} service", action)));
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("{}", red(&format!("Unknown action: {}", action)));
            std::process::exit(1);
        }
    }
}

fn find_self_executable() -> String {
    if let Ok(exe) = std::env::current_exe() {
        return exe.to_string_lossy().to_string();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    format!("{}/.local/bin/agyctl", home)
}
