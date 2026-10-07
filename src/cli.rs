use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "agyctl", version = "0.1.0", about = "Antigravity Control & Synchronization")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(subcommand, aliases = ["auth", "profile", "accounts"])]
    Account(AccountCommands),

    #[command(subcommand, aliases = ["history"])]
    Sync(SyncCommands),

    #[command(subcommand, aliases = ["projects"])]
    Project(ProjectCommands),

    #[command(alias = "svc")]
    Service(ServiceArgs),

    Daemon(DaemonArgs),

    #[command(alias = "ls")]
    List(AccountListArgs),

    #[command(alias = "whoami")]
    Current,

    #[command(alias = "update")]
    Refresh(RefreshArgs),

    Switch(AccountSwitchArgs),

    #[command(alias = "rename")]
    Edit(AccountEditArgs),

    #[command(aliases = ["quota", "limits"])]
    Usage(AccountUsageArgs),

    Status(SyncStatusArgs),

    Pull(SyncPullArgs),

    Push(SyncPushArgs),
}

#[derive(Subcommand, Debug)]
pub enum AccountCommands {
    #[command(alias = "ls")]
    List(AccountListArgs),
    Current,
    #[command(alias = "rename")]
    Edit(AccountEditArgs),
    Switch(AccountSwitchArgs),
    #[command(aliases = ["quota", "limits"])]
    Usage(AccountUsageArgs),
    #[command(alias = "rm")]
    Delete(AccountDeleteArgs),
}

#[derive(Args, Debug, Clone)]
pub struct AccountListArgs {
    #[arg(value_name = "MODEL")]
    pub model: Option<String>,
    #[arg(short, long)]
    pub refresh: bool,
    #[arg(short = 'a', long)]
    pub auto_switch: bool,
    #[arg(long)]
    pub restart: bool,
}

#[derive(Args, Debug)]
pub struct AccountEditArgs {
    pub target: String,
    pub new_name: Option<String>,
    #[arg(short, long)]
    pub tier: Option<String>,
    #[arg(short, long)]
    pub expires: Option<String>,
}

#[derive(Args, Debug)]
pub struct AccountSwitchArgs {
    pub name: String,
    #[arg(value_name = "HOST")]
    pub host: Option<String>,
    #[arg(long)]
    pub restart: bool,
    #[arg(long)]
    pub remote: bool,
    #[arg(long = "host")]
    pub host_flag: Option<String>,
}

impl AccountSwitchArgs {
    pub fn get_host(&self) -> Option<&str> {
        self.host.as_deref().or(self.host_flag.as_deref())
    }
}

#[derive(Args, Debug)]
pub struct AccountUsageArgs {
    pub name: Option<String>,
    #[arg(short, long)]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct AccountDeleteArgs {
    pub name: String,
}

#[derive(Subcommand, Debug)]
pub enum SyncCommands {
    Status(SyncStatusArgs),
    Pull(SyncPullArgs),
    Push(SyncPushArgs),
}

#[derive(Args, Debug)]
pub struct SyncStatusArgs {
    #[arg(value_name = "HOST")]
    pub host: Option<String>,
    #[arg(long = "host")]
    pub host_flag: Option<String>,
}

impl SyncStatusArgs {
    pub fn get_host(&self) -> Option<&str> {
        self.host.as_deref().or(self.host_flag.as_deref())
    }
}

#[derive(Args, Debug)]
pub struct SyncPullArgs {
    #[arg(value_name = "HOST")]
    pub host: Option<String>,
    #[arg(long = "host")]
    pub host_flag: Option<String>,
    #[arg(short, long)]
    pub project: Vec<String>,
    #[arg(long)]
    pub projects: bool,
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    #[arg(long)]
    pub restart: bool,
}

impl SyncPullArgs {
    pub fn get_host(&self) -> Option<&str> {
        self.host.as_deref().or(self.host_flag.as_deref())
    }
}

#[derive(Args, Debug)]
pub struct SyncPushArgs {
    #[arg(value_name = "HOST")]
    pub host: Option<String>,
    #[arg(long = "host")]
    pub host_flag: Option<String>,
    #[arg(short, long)]
    pub project: Vec<String>,
    #[arg(long)]
    pub projects: bool,
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    #[arg(long)]
    pub no_restart: bool,
}

impl SyncPushArgs {
    pub fn get_host(&self) -> Option<&str> {
        self.host.as_deref().or(self.host_flag.as_deref())
    }
}

#[derive(Subcommand, Debug)]
pub enum ProjectCommands {
    #[command(alias = "ls")]
    List(ProjectListArgs),
    Pull(ProjectPullArgs),
    Push(ProjectPushArgs),
}

#[derive(Args, Debug)]
pub struct ProjectListArgs {
    #[arg(value_name = "HOST")]
    pub host: Option<String>,
    #[arg(long = "host")]
    pub host_flag: Option<String>,
    #[arg(long)]
    pub local_only: bool,
}

impl ProjectListArgs {
    pub fn get_host(&self) -> Option<&str> {
        self.host.as_deref().or(self.host_flag.as_deref())
    }
}

#[derive(Args, Debug)]
pub struct ProjectPullArgs {
    pub name: Option<String>,
    #[arg(value_name = "HOST")]
    pub host: Option<String>,
    #[arg(long = "host")]
    pub host_flag: Option<String>,
    #[arg(long)]
    pub all: bool,
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

impl ProjectPullArgs {
    pub fn get_host(&self) -> Option<&str> {
        self.host.as_deref().or(self.host_flag.as_deref())
    }
}

#[derive(Args, Debug)]
pub struct ProjectPushArgs {
    pub name: Option<String>,
    #[arg(value_name = "HOST")]
    pub host: Option<String>,
    #[arg(long = "host")]
    pub host_flag: Option<String>,
    #[arg(long)]
    pub all: bool,
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

impl ProjectPushArgs {
    pub fn get_host(&self) -> Option<&str> {
        self.host.as_deref().or(self.host_flag.as_deref())
    }
}

#[derive(Args, Debug)]
pub struct ServiceArgs {
    pub action: String,
}

#[derive(Args, Debug)]
pub struct DaemonArgs {
    #[arg(long, default_value_t = 10)]
    pub interval: u64,
    #[arg(long)]
    pub once: bool,
}

#[derive(Args, Debug)]
pub struct RefreshArgs {
    #[arg(short, long)]
    pub all: bool,
}
