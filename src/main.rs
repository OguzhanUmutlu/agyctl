mod account;
mod cache;
mod cli;
mod proto;
mod quota;
mod secret;
mod service;
mod sync;
mod ui;

use clap::Parser;
use cli::*;

fn main() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(|s| s.as_str())
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if msg.contains("Broken pipe") {
            std::process::exit(0);
        }
        default_hook(info);
    }));

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Account(cmd)) => match cmd {
            AccountCommands::List(a) => {
                account::cmd_list(a.refresh, a.model.as_deref(), a.auto_switch, a.restart)
            }
            AccountCommands::Current => account::cmd_current(),
            AccountCommands::Edit(a) => account::cmd_edit(
                &a.target,
                a.new_name.as_deref(),
                a.tier.as_deref(),
                a.expires.as_deref(),
            ),
            AccountCommands::Switch(a) => {
                account::cmd_switch(&a.name, a.restart, a.remote, a.get_host())
            }
            AccountCommands::Usage(a) => account::cmd_usage(a.name.as_deref(), a.all),
            AccountCommands::Delete(a) => account::cmd_delete(&a.name),
        },
        Some(Commands::Sync(cmd)) => match cmd {
            SyncCommands::Status(a) => sync::cmd_status(a.get_host()),
            SyncCommands::Pull(a) => {
                sync::cmd_pull(a.get_host(), &a.project, a.projects, a.dry_run, a.restart)
            }
            SyncCommands::Push(a) => {
                sync::cmd_push(a.get_host(), &a.project, a.projects, a.dry_run, a.no_restart)
            }
        },
        Some(Commands::Project(cmd)) => match cmd {
            ProjectCommands::List(a) => sync::cmd_project_list(a.get_host(), a.local_only),
            ProjectCommands::Pull(a) => {
                let host = sync::resolve_host(a.get_host());
                if a.all {
                    let local_c = sync::get_local_conversations();
                    let remote_c = sync::get_remote_conversations(&host);
                    for (_, (_, _, uris)) in local_c.iter().chain(remote_c.iter()) {
                        if let Some(p) = sync::extract_project_name(uris) {
                            sync::sync_single_project(&p, "pull", &host, a.dry_run);
                        }
                    }
                } else if let Some(n) = a.name {
                    sync::sync_single_project(&n, "pull", &host, a.dry_run);
                } else {
                    eprintln!("Specify a project name or pass --all");
                    std::process::exit(1);
                }
            }
            ProjectCommands::Push(a) => {
                let host = sync::resolve_host(a.get_host());
                if a.all {
                    let local_c = sync::get_local_conversations();
                    let remote_c = sync::get_remote_conversations(&host);
                    for (_, (_, _, uris)) in local_c.iter().chain(remote_c.iter()) {
                        if let Some(p) = sync::extract_project_name(uris) {
                            sync::sync_single_project(&p, "push", &host, a.dry_run);
                        }
                    }
                } else if let Some(n) = a.name {
                    sync::sync_single_project(&n, "push", &host, a.dry_run);
                } else {
                    eprintln!("Specify a project name or pass --all");
                    std::process::exit(1);
                }
            }
        },
        Some(Commands::Service(a)) => service::manage_service(&a.action),
        Some(Commands::Daemon(a)) => cache::run_daemon(a.interval, a.once),
        Some(Commands::List(a)) => {
            account::cmd_list(a.refresh, a.model.as_deref(), a.auto_switch, a.restart)
        }
        Some(Commands::Current) => account::cmd_current(),
        Some(Commands::Refresh(a)) => {
            let scope = if a.all {
                cache::RefreshScope::All
            } else {
                cache::RefreshScope::ActiveOnly
            };
            cache::update_cache_scoped(false, scope);
            account::cmd_list(false, None, false, false);
        }
        Some(Commands::Switch(a)) => {
            account::cmd_switch(&a.name, a.restart, a.remote, a.get_host())
        }
        Some(Commands::Edit(a)) => account::cmd_edit(
            &a.target,
            a.new_name.as_deref(),
            a.tier.as_deref(),
            a.expires.as_deref(),
        ),
        Some(Commands::Usage(a)) => account::cmd_usage(a.name.as_deref(), a.all),
        Some(Commands::Status(a)) => sync::cmd_status(a.get_host()),
        Some(Commands::Pull(a)) => {
            sync::cmd_pull(a.get_host(), &a.project, a.projects, a.dry_run, a.restart)
        }
        Some(Commands::Push(a)) => {
            sync::cmd_push(a.get_host(), &a.project, a.projects, a.dry_run, a.no_restart)
        }
        None => account::cmd_list(false, None, false, false),
    }
}
