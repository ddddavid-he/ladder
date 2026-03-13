use serde::{Deserialize, Serialize};

/// 代理环境变量组
#[derive(Debug, Clone)]
pub struct EnvVars {
    pub http_proxy: String,
    pub socks_proxy: String,
    pub no_proxy: String,
}

impl EnvVars {
    pub fn new(http_port: u16, socks_port: u16) -> Self {
        Self {
            http_proxy: format!("http://127.0.0.1:{}", http_port),
            socks_proxy: format!("socks5://127.0.0.1:{}", socks_port),
            no_proxy: "localhost,127.0.0.1,::1".to_string(),
        }
    }

    /// 生成 shell export 语句（ladder-core env / start --env 输出）
    pub fn export_statements(&self, shell_pid: Option<u32>) -> String {
        let mut lines = Vec::new();
        if let Some(pid) = shell_pid {
            lines.push(format!("export LADDER_SHELL_PID={}", pid));
        }
        lines.push(format!("export HTTP_PROXY=\"{}\"", self.http_proxy));
        lines.push(format!("export HTTPS_PROXY=\"{}\"", self.http_proxy));
        lines.push(format!("export ALL_PROXY=\"{}\"", self.socks_proxy));
        lines.push(format!("export http_proxy=\"{}\"", self.http_proxy));
        lines.push(format!("export https_proxy=\"{}\"", self.http_proxy));
        lines.push(format!("export all_proxy=\"{}\"", self.socks_proxy));
        lines.push(format!("export NO_PROXY=\"{}\"", self.no_proxy));
        lines.push(format!("export no_proxy=\"{}\"", self.no_proxy));
        lines.join("\n")
    }

    /// 生成 shell unset 语句（ladder-core unenv / stop --env 输出）
    pub fn unset_statements() -> String {
        vec![
            "unset HTTP_PROXY HTTPS_PROXY ALL_PROXY",
            "unset http_proxy https_proxy all_proxy",
            "unset NO_PROXY no_proxy",
            "unset LADDER_SHELL_PID",
        ]
        .join("\n")
    }
}

/// 运行时状态文件：~/.cache/ladder/state.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeState {
    /// ladder-core 进程 PID（自身）
    pub pid: Option<u32>,

    /// mihomo 子进程 PID
    pub mihomo_pid: Option<u32>,

    /// 绑定的父 Shell PID
    pub shell_pid: Option<u32>,

    /// HTTP 代理端口
    pub http_port: u16,

    /// SOCKS5 代理端口
    pub socks_port: u16,

    /// Mihomo API 控制端口
    pub control_port: u16,

    /// 当前选中的节点名
    pub current_node: Option<String>,

    /// 代理是否运行中
    pub running: bool,
}

impl RuntimeState {
    pub fn state_path() -> anyhow::Result<std::path::PathBuf> {
        crate::config::cache_dir().map(|d| d.join("state.json"))
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::state_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::state_path()?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn clear() -> anyhow::Result<()> {
        let path = Self::state_path()?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_statements() {
        let env = EnvVars::new(7890, 7891);
        let out = env.export_statements(Some(1234));
        assert!(out.contains("export LADDER_SHELL_PID=1234"));
        assert!(out.contains("export HTTP_PROXY=\"http://127.0.0.1:7890\""));
        assert!(out.contains("export ALL_PROXY=\"socks5://127.0.0.1:7891\""));
        assert!(out.contains("export no_proxy="));
    }

    #[test]
    fn test_unset_statements() {
        let out = EnvVars::unset_statements();
        assert!(out.contains("unset HTTP_PROXY"));
        assert!(out.contains("unset LADDER_SHELL_PID"));
    }

    #[test]
    fn test_export_without_pid() {
        let env = EnvVars::new(7890, 7891);
        let out = env.export_statements(None);
        assert!(!out.contains("LADDER_SHELL_PID"));
    }
}
