mod cli;
mod config;
mod env;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, SubCommands};
use config::{LadderConfig, Subscription};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // 初始化日志
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| log_level.into()),
        )
        .init();

    match cli.command {
        // 无子命令：若代理已运行则开 TUI，否则提示
        None => {
            let state = env::RuntimeState::load()?;
            if state.running {
                println!("Proxy is running. Starting TUI...");
                // TODO P5: 调用 tui::run()
                println!("[TUI not yet implemented]");
            } else {
                println!("Ladder v2 - Proxy is not running.");
                println!("Run `ladder start` to start the proxy.");
                println!("Run `ladder tui` to open TUI.");
            }
        }

        Some(Commands::Start { subscriptions, env, shell_pid, auto_best, mihomo_bin }) => {
            let cfg = LadderConfig::load()?;
            if cfg.subscriptions.is_empty() {
                anyhow::bail!("No subscriptions configured. Run `ladder sub add <URL>` first.");
            }
            let subs = cfg.filter_subscriptions(&subscriptions);
            if subs.is_empty() {
                anyhow::bail!("No matching subscriptions found for: {:?}", subscriptions);
            }

            tracing::info!("Starting proxy with {} subscription(s)...", subs.len());
            for s in &subs {
                tracing::info!("  - {} ({})", s.name, s.url);
            }

            if env {
                let vars = crate::env::EnvVars::new(cfg.ladder.port, cfg.ladder.socks_port);
                println!("{}", vars.export_statements(shell_pid));
            }

            // TODO P2: 启动 mihomo 进程
            // TODO P3: 启动 watchdog
            // TODO P4: 若 auto_best，执行自动选优
            println!("[Proxy start not yet implemented - P2]");
        }

        Some(Commands::Stop { env }) => {
            // TODO P2: 停止 mihomo 进程
            if env {
                println!("{}", crate::env::EnvVars::unset_statements());
            }
            crate::env::RuntimeState::clear()?;
            tracing::info!("Proxy stopped.");
            println!("[Proxy stop not yet implemented - P2]");
        }

        Some(Commands::Env) => {
            let state = crate::env::RuntimeState::load()?;
            if !state.running {
                eprintln!("Error: proxy is not running.");
                std::process::exit(1);
            }
            let vars = crate::env::EnvVars::new(state.http_port, state.socks_port);
            println!("{}", vars.export_statements(state.shell_pid));
        }

        Some(Commands::Unenv) => {
            println!("{}", crate::env::EnvVars::unset_statements());
        }

        Some(Commands::Tui) => {
            // TODO P5: tui::run()
            println!("[TUI not yet implemented - P5]");
        }

        Some(Commands::Status) => {
            let state = crate::env::RuntimeState::load()?;
            if state.running {
                println!("Status: RUNNING");
                println!("  HTTP  proxy: http://127.0.0.1:{}", state.http_port);
                println!("  SOCKS proxy: socks5://127.0.0.1:{}", state.socks_port);
                println!("  Control API: http://127.0.0.1:{}", state.control_port);
                if let Some(node) = &state.current_node {
                    println!("  Current node: {}", node);
                }
                if let Some(pid) = state.mihomo_pid {
                    println!("  Mihomo PID: {}", pid);
                }
            } else {
                println!("Status: STOPPED");
            }
        }

        Some(Commands::Sub { action }) => {
            let mut cfg = LadderConfig::load()?;
            match action {
                SubCommands::Add { url, name, interval } => {
                    let display_name = name.unwrap_or_else(|| {
                        // 从 URL 中提取简短名
                        url.split('/').last()
                            .unwrap_or("subscription")
                            .split('?')
                            .next()
                            .unwrap_or("subscription")
                            .to_string()
                    });
                    cfg.add_subscription(Subscription {
                        name: display_name.clone(),
                        url,
                        interval,
                    });
                    cfg.save()?;
                    println!("✓ Added subscription: {}", display_name);
                }

                SubCommands::List => {
                    if cfg.subscriptions.is_empty() {
                        println!("No subscriptions configured.");
                    } else {
                        println!("{:<20} {:<60} {}", "Name", "URL", "Interval(s)");
                        println!("{}", "-".repeat(90));
                        for s in &cfg.subscriptions {
                            println!("{:<20} {:<60} {}", s.name, s.url, s.interval);
                        }
                    }
                }

                SubCommands::Remove { name } => {
                    if cfg.remove_subscription(&name) {
                        cfg.save()?;
                        println!("✓ Removed subscription: {}", name);
                    } else {
                        eprintln!("Subscription not found: {}", name);
                        std::process::exit(1);
                    }
                }
            }
        }

        Some(Commands::Update { subscriptions }) => {
            // TODO P4: 调用 Mihomo API 强制更新 provider
            println!("[Update not yet implemented - P4], subs: {:?}", subscriptions);
        }

        Some(Commands::Install { shell }) => {
            // TODO P7: 安装 ladder.sh 到 ~/.config/ladder/，注入 shell rc
            println!("[Install not yet implemented - P7], shell: {:?}", shell);
        }
    }

    Ok(())
}
