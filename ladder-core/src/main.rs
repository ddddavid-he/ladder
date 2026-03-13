mod cli;
mod config;
mod env;
mod mihomo;
mod watchdog;
mod api;
mod tui;

use std::io::Write;

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
                tui::run().await?;
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

            // Get mihomo binary
            let bin_path = mihomo::get_mihomo_path(mihomo_bin.as_deref()).await?;

            // Generate mihomo config.yaml
            let config_path = mihomo::write_config_yaml(&cfg, &subs)?;

            // Spawn mihomo
            let proc = mihomo::spawn_mihomo(&bin_path, &config_path).await?;
            let mihomo_pid = proc.pid;

            // Save runtime state
            let mut state = crate::env::RuntimeState {
                pid: Some(std::process::id()),
                mihomo_pid: Some(mihomo_pid),
                shell_pid,
                http_port: cfg.ladder.port,
                socks_port: cfg.ladder.socks_port,
                control_port: cfg.ladder.control_port,
                current_node: None,
                running: true,
            };
            state.save()?;

            // Output env vars if requested
            if env {
                let vars = crate::env::EnvVars::new(cfg.ladder.port, cfg.ladder.socks_port);
                println!("{}", vars.export_statements(shell_pid));
            }

            tracing::info!("Proxy started. Mihomo PID: {}", mihomo_pid);

            // P3: Start watchdog if shell_pid is set
            if let Some(spid) = shell_pid {
                tracing::info!("Watchdog started for shell PID {}", spid);
                let wdcfg = watchdog::WatchdogConfig::new(spid, mihomo_pid);
                let _watchdog = watchdog::spawn_watchdog(wdcfg);
                // Keep ladder-core alive; exit when Ctrl+C or watchdog finishes
                tokio::signal::ctrl_c().await.ok();
            }

            // P4: if auto_best, wait for API and run auto-best
            if auto_best {
                let mihomo_api = api::MihomoApi::new(cfg.ladder.control_port, cfg.ladder.api_secret.as_deref())?;
                if let Err(e) = mihomo_api.wait_ready(10).await {
                    tracing::warn!("API not ready for auto-best: {}", e);
                } else {
                    match mihomo_api.auto_best("PROXY", "https://www.gstatic.com/generate_204").await {
                        Ok((node, delay)) => tracing::info!("Auto-best: {} ({}ms)", node, delay),
                        Err(e) => tracing::warn!("Auto-best failed: {}", e),
                    }
                }
            }
            drop(proc);
        }

        Some(Commands::Stop { env }) => {
            let state = crate::env::RuntimeState::load()?;
            if state.running {
                if let Some(pid) = state.mihomo_pid {
                    mihomo::stop_mihomo(pid).await?;
                }
            }
            if env {
                println!("{}", crate::env::EnvVars::unset_statements());
            }
            crate::env::RuntimeState::clear()?;
            tracing::info!("Proxy stopped.");
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
            tui::run().await?;
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
            let cfg = LadderConfig::load()?;
            let state = crate::env::RuntimeState::load()?;
            if !state.running {
                anyhow::bail!("Proxy is not running. Start it first with `ladder start`.");
            }
            let mihomo_api = api::MihomoApi::new(cfg.ladder.control_port, cfg.ladder.api_secret.as_deref())?;
            let subs = cfg.filter_subscriptions(&subscriptions);
            if subs.is_empty() {
                anyhow::bail!("No matching subscriptions to update.");
            }
            for s in &subs {
                print!("Updating subscription: {}... ", s.name);
                match mihomo_api.update_provider(&s.name).await {
                    Ok(_) => println!("✓"),
                    Err(e) => println!("✗ {}", e),
                }
            }
        }

        Some(Commands::Install { shell }) => {
            let config = config::config_dir()?;
            let ladder_sh_src = std::env::current_exe()?
                .parent()
                .map(|p| p.join("ladder.sh"))
                .filter(|p| p.exists());

            // Write ladder.sh to config dir
            let dest_sh = config.join("ladder.sh");
            if let Some(src) = ladder_sh_src {
                std::fs::copy(&src, &dest_sh)?;
                println!("✓ Copied ladder.sh to {}", dest_sh.display());
            } else {
                // Embed ladder.sh content directly (fallback)
                let sh_content = include_str!("../ladder.sh");
                std::fs::write(&dest_sh, sh_content)?;
                println!("✓ Written ladder.sh to {}", dest_sh.display());
            }

            // Detect shell
            let shell_name = shell.unwrap_or_else(|| {
                std::env::var("SHELL").unwrap_or_default()
                    .split('/')
                    .last()
                    .unwrap_or("bash")
                    .to_string()
            });

            let rc_file = match shell_name.as_str() {
                "zsh" => dirs_rc("zsh"),
                "bash" => dirs_rc("bash"),
                other => {
                    println!("Unknown shell: {}. Please manually add to your rc:", other);
                    println!("  source {}", dest_sh.display());
                    return Ok(());
                }
            };

            let source_line = format!("\nsource {}\n", dest_sh.display());
            let existing = std::fs::read_to_string(&rc_file).unwrap_or_default();
            if !existing.contains(&dest_sh.display().to_string()) {
                std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&rc_file)?
                    .write_all(source_line.as_bytes())?;
                println!("✓ Added source line to {}", rc_file.display());
            } else {
                println!("✓ Already installed in {}", rc_file.display());
            }
            println!("Run: source {} (or open a new terminal)", rc_file.display());
        }
    }

    Ok(())
}

/// Return the rc file path for the given shell
fn dirs_rc(shell: &str) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    match shell {
        "zsh" => std::path::PathBuf::from(format!("{}/.zshrc", home)),
        "bash" => {
            // Prefer .bash_profile on macOS, .bashrc on Linux
            let bash_profile = std::path::PathBuf::from(format!("{}/.bash_profile", home));
            if bash_profile.exists() && cfg!(target_os = "macos") {
                bash_profile
            } else {
                std::path::PathBuf::from(format!("{}/.bashrc", home))
            }
        }
        _ => std::path::PathBuf::from(format!("{}/.bashrc", home)),
    }
}
