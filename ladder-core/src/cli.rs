use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "ladder-core",
    version,
    about = "Ladder v2 - A proxy manager wrapping Mihomo",
    long_about = None
)]
pub struct Cli {
    /// Verbose logging
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the proxy (detached background process)
    Start {
        /// Subscription names to use (can specify multiple, default: all)
        #[arg(short = 's', long = "sub", value_name = "NAME")]
        subscriptions: Vec<String>,

        /// Output shell export statements to stdout (for eval by env.sh)
        #[arg(long)]
        env: bool,

        /// Shell PID to monitor (passed by env.sh, triggers watchdog)
        #[arg(long, value_name = "PID")]
        shell_pid: Option<u32>,

        /// Auto-select the lowest latency node after startup
        #[arg(long)]
        auto_best: bool,

        /// Path to mihomo binary (overrides auto-download)
        #[arg(long, value_name = "PATH")]
        mihomo_bin: Option<std::path::PathBuf>,
    },

    /// Stop the running proxy
    Stop {
        /// Also output unset statements to stdout (for eval by env.sh)
        #[arg(long)]
        env: bool,
    },

    /// Print current proxy env export statements (for eval)
    Env,

    /// Print proxy env unset statements (for eval)
    Unenv,

    /// Open TUI (auto-starts proxy if not running)
    Tui,

    /// Show proxy status
    Status,

    /// Subscription management
    Sub {
        #[command(subcommand)]
        action: SubCommands,
    },

    /// Add a subscription (alias of `sub add`)
    Add {
        /// Subscription URL
        url: String,
        /// Display name
        #[arg(short, long)]
        name: Option<String>,
        /// Update interval in seconds (default: 86400)
        #[arg(short, long, default_value = "86400")]
        interval: u64,
    },

    /// Force update all (or specified) subscription providers
    Update {
        /// Subscription name to update (default: all)
        #[arg(short = 's', long = "sub", value_name = "NAME")]
        subscriptions: Vec<String>,
    },

    /// Install: write env.sh to ~/.config/ladder/ and print shell rc instructions
    Install {
        /// Shell type: bash, zsh, fish (auto-detect if not specified)
        #[arg(long, value_name = "SHELL")]
        shell: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum SubCommands {
    /// Add a subscription
    Add {
        /// Subscription URL
        url: String,
        /// Display name
        #[arg(short, long)]
        name: Option<String>,
        /// Update interval in seconds (default: 86400)
        #[arg(short, long, default_value = "86400")]
        interval: u64,
    },

    /// List all subscriptions
    List,

    /// Remove a subscription by name
    Remove {
        /// Subscription name
        name: String,
    },
}
