use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 获取配置目录：~/.config/ladder/
pub fn config_dir() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "ladder")
        .context("Failed to determine config directory")?;
    let path = dirs.config_dir().to_path_buf();
    fs::create_dir_all(&path).context("Failed to create config directory")?;
    Ok(path)
}

/// 获取缓存目录：~/.cache/ladder/
pub fn cache_dir() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "ladder")
        .context("Failed to determine cache directory")?;
    let path = dirs.cache_dir().to_path_buf();
    fs::create_dir_all(&path).context("Failed to create cache directory")?;
    Ok(path)
}

/// 主配置文件：~/.config/ladder/config.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LadderConfig {
    #[serde(default)]
    pub ladder: LadderSettings,

    #[serde(default, rename = "subscriptions")]
    pub subscriptions: Vec<Subscription>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LadderSettings {
    /// HTTP 代理端口
    #[serde(default = "default_port")]
    pub port: u16,

    /// SOCKS5 代理端口
    #[serde(default = "default_socks_port")]
    pub socks_port: u16,

    /// Mihomo RESTful API 控制端口
    #[serde(default = "default_control_port")]
    pub control_port: u16,

    /// Mihomo API Secret（可选）
    #[serde(default)]
    pub api_secret: Option<String>,

    /// 外置 mihomo bin 路径（不设则自动下载）
    #[serde(default)]
    pub mihomo_bin: Option<PathBuf>,
}

impl Default for LadderSettings {
    fn default() -> Self {
        Self {
            port: default_port(),
            socks_port: default_socks_port(),
            control_port: default_control_port(),
            api_secret: None,
            mihomo_bin: None,
        }
    }
}

fn default_port() -> u16 { 7890 }
fn default_socks_port() -> u16 { 7891 }
fn default_control_port() -> u16 { 9090 }

/// 单个订阅
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    /// 显示名称（唯一标识）
    pub name: String,

    /// 订阅 URL
    pub url: String,

    /// 自动更新间隔（秒），默认 86400（24h）
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 { 86400 }

impl LadderConfig {
    /// 读取配置文件，若不存在则返回默认值
    pub fn load() -> Result<Self> {
        let path = config_dir()?.join("config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("Failed to parse config: {}", path.display()))
    }

    /// 保存配置文件
    pub fn save(&self) -> Result<()> {
        let path = config_dir()?.join("config.toml");
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        fs::write(&path, content)
            .with_context(|| format!("Failed to write config: {}", path.display()))
    }

    /// 获取指定名称的订阅
    pub fn get_subscription(&self, name: &str) -> Option<&Subscription> {
        self.subscriptions.iter().find(|s| s.name == name)
    }

    /// 添加订阅（名称已存在则更新 URL）
    pub fn add_subscription(&mut self, sub: Subscription) {
        if let Some(existing) = self.subscriptions.iter_mut().find(|s| s.name == sub.name) {
            existing.url = sub.url;
            existing.interval = sub.interval;
        } else {
            self.subscriptions.push(sub);
        }
    }

    /// 删除订阅
    pub fn remove_subscription(&mut self, name: &str) -> bool {
        let before = self.subscriptions.len();
        self.subscriptions.retain(|s| s.name != name);
        self.subscriptions.len() < before
    }

    /// 筛选订阅（传入名称列表，空则返回全部）
    pub fn filter_subscriptions(&self, names: &[String]) -> Vec<&Subscription> {
        if names.is_empty() {
            self.subscriptions.iter().collect()
        } else {
            self.subscriptions
                .iter()
                .filter(|s| names.contains(&s.name))
                .collect()
        }
    }
}

impl Default for LadderConfig {
    fn default() -> Self {
        Self {
            ladder: LadderSettings::default(),
            subscriptions: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = LadderConfig::default();
        assert_eq!(cfg.ladder.port, 7890);
        assert_eq!(cfg.ladder.socks_port, 7891);
        assert_eq!(cfg.ladder.control_port, 9090);
        assert!(cfg.subscriptions.is_empty());
    }

    #[test]
    fn test_add_remove_subscription() {
        let mut cfg = LadderConfig::default();
        cfg.add_subscription(Subscription {
            name: "机场A".to_string(),
            url: "https://example.com/sub".to_string(),
            interval: 86400,
        });
        assert_eq!(cfg.subscriptions.len(), 1);
        assert!(cfg.get_subscription("机场A").is_some());

        let removed = cfg.remove_subscription("机场A");
        assert!(removed);
        assert!(cfg.subscriptions.is_empty());
    }

    #[test]
    fn test_filter_subscriptions() {
        let mut cfg = LadderConfig::default();
        cfg.add_subscription(Subscription { name: "A".to_string(), url: "u1".to_string(), interval: 86400 });
        cfg.add_subscription(Subscription { name: "B".to_string(), url: "u2".to_string(), interval: 86400 });
        cfg.add_subscription(Subscription { name: "C".to_string(), url: "u3".to_string(), interval: 86400 });

        let filtered = cfg.filter_subscriptions(&["A".to_string(), "C".to_string()]);
        assert_eq!(filtered.len(), 2);

        let all = cfg.filter_subscriptions(&[]);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_toml_roundtrip() {
        let mut cfg = LadderConfig::default();
        cfg.add_subscription(Subscription {
            name: "测试".to_string(),
            url: "https://test.example.com/sub?token=abc".to_string(),
            interval: 43200,
        });
        let serialized = toml::to_string_pretty(&cfg).unwrap();
        let deserialized: LadderConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.subscriptions[0].name, "测试");
        assert_eq!(deserialized.subscriptions[0].interval, 43200);
    }
}
