use anyhow::Result;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::env::RuntimeState;

/// Check if a process with the given PID is alive.
/// Linux: checks /proc/{pid}/status existence
/// macOS: uses kill(pid, 0) via nix
pub fn is_pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{}/status", pid)).exists()
    }

    #[cfg(target_os = "macos")]
    {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;
        match kill(Pid::from_raw(pid as i32), None) {
            Ok(_) => true,
            Err(nix::errno::Errno::ESRCH) => false,
            // EPERM means process exists but we lack permission
            Err(_) => true,
        }
    }

    // Fallback for unsupported platforms: assume alive
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        true
    }
}

/// Watchdog configuration
pub struct WatchdogConfig {
    /// PID of the parent shell to monitor
    pub shell_pid: u32,
    /// PID of the mihomo process to kill when shell dies
    pub mihomo_pid: u32,
    /// Poll interval (default: 2s)
    pub interval: Duration,
}

impl WatchdogConfig {
    pub fn new(shell_pid: u32, mihomo_pid: u32) -> Self {
        Self {
            shell_pid,
            mihomo_pid,
            interval: Duration::from_secs(2),
        }
    }
}

/// Spawn the watchdog task.
/// Returns a JoinHandle. The task runs until the shell PID disappears,
/// at which point it sends SIGTERM to mihomo and cleans up state.
pub fn spawn_watchdog(config: WatchdogConfig) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(
            "Watchdog started: monitoring shell PID {} → mihomo PID {}",
            config.shell_pid, config.mihomo_pid
        );

        loop {
            tokio::time::sleep(config.interval).await;

            if !is_pid_alive(config.shell_pid) {
                warn!(
                    "Shell PID {} is gone. Stopping mihomo PID {}...",
                    config.shell_pid, config.mihomo_pid
                );

                // Stop mihomo
                if let Err(e) = crate::mihomo::stop_mihomo(config.mihomo_pid).await {
                    warn!("Error stopping mihomo: {}", e);
                }

                // Clear runtime state
                if let Err(e) = RuntimeState::clear() {
                    warn!("Error clearing runtime state: {}", e);
                }

                info!("Watchdog: cleanup complete. Exiting.");
                break;
            } else {
                debug!("Watchdog: shell PID {} is alive", config.shell_pid);
            }
        }
    })
}

/// A guard that stops mihomo when dropped (useful in main for graceful shutdown)
pub struct MihomoGuard {
    pub pid: u32,
}

impl Drop for MihomoGuard {
    fn drop(&mut self) {
        // Synchronous best-effort SIGTERM on drop
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        let _ = kill(Pid::from_raw(self.pid as i32), Signal::SIGTERM);
        info!("MihomoGuard: sent SIGTERM to PID {}", self.pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_self_pid_alive() {
        // Current process should be alive
        let pid = std::process::id();
        assert!(is_pid_alive(pid), "Current process should be detected as alive");
    }

    #[test]
    fn test_nonexistent_pid_dead() {
        // PID 999999 is very unlikely to exist
        // Note: on some systems this might technically pass if PID wrap-around happens,
        // but in practice this is reliable for test purposes
        assert!(
            !is_pid_alive(999999),
            "Non-existent PID should be detected as dead"
        );
    }

    #[test]
    fn test_pid_1_alive() {
        // PID 1 (init/systemd) should always be alive on Linux/macOS
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        assert!(is_pid_alive(1), "PID 1 should always be alive");
    }
}
