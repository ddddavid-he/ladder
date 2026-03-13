use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

use crate::config::{config_dir, data_dir, LadderConfig, Subscription};

// ─── Bundled binary (compile-time embedded) ───────────────────────────────────

#[cfg(feature = "bundled")]
static MIHOMO_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mihomo"));

// ─── Platform detection ───────────────────────────────────────────────────────

pub struct MihomoTarget {
    pub os: &'static str,
    pub arch: &'static str,
}

impl MihomoTarget {
    pub fn current() -> Result<Self> {
        let os = match std::env::consts::OS {
            "linux" => "linux",
            "macos" => "darwin",
            other => bail!("Unsupported OS: {}", other),
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            other => bail!("Unsupported arch: {}", other),
        };
        Ok(Self { os, arch })
    }

    /// GitHub release asset filename (e.g. "mihomo-linux-amd64-v1.18.0.gz")
    pub fn asset_name(&self, version: &str) -> String {
        format!("mihomo-{}-{}-{}.gz", self.os, self.arch, version)
    }

    /// Download URL
    pub fn download_url(&self, version: &str) -> String {
        format!(
            "https://github.com/MetaCubeX/mihomo/releases/download/{}/{}.gz",
            version,
            format!("mihomo-{}-{}-{}", self.os, self.arch, version)
        )
    }
}

// ─── Get or prepare mihomo binary path ────────────────────────────────────────

/// Returns path to a usable mihomo binary.
/// Priority:
///  1. User-specified path (from config or CLI)
///  2. bundled feature: extract embedded bytes to cache
///  3. Already cached at ~/.cache/ladder/mihomo
///  4. Auto-download from GitHub
pub async fn get_mihomo_path(user_specified: Option<&Path>) -> Result<PathBuf> {
    // 1. User-specified
    if let Some(p) = user_specified {
        if p.exists() {
            return Ok(p.to_path_buf());
        }
        bail!("Specified mihomo binary not found: {}", p.display());
    }

    let cache = data_dir()?;
    let cached_bin = cache.join("mihomo");

    // 2. Bundled
    #[cfg(feature = "bundled")]
    {
        if !cached_bin.exists() {
            info!("Extracting bundled mihomo binary...");
            extract_bundled(&cached_bin)?;
        }
        set_executable(&cached_bin)?;
        return Ok(cached_bin);
    }

    // 3. Already cached
    #[allow(unreachable_code)]
    if cached_bin.exists() {
        debug!("Using cached mihomo: {}", cached_bin.display());
        set_executable(&cached_bin)?;
        return Ok(cached_bin);
    }

    // 4. Auto-download
    info!("Mihomo not found, downloading...");
    download_mihomo(&cached_bin).await?;
    Ok(cached_bin)
}

// ─── Bundled extraction ───────────────────────────────────────────────────────

#[cfg(feature = "bundled")]
fn extract_bundled(dest: &Path) -> Result<()> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let mut gz = GzDecoder::new(MIHOMO_BYTES);
    let mut buf = Vec::new();
    gz.read_to_end(&mut buf)
        .context("Failed to decompress bundled mihomo")?;
    std::fs::write(dest, &buf)
        .with_context(|| format!("Failed to write mihomo to {}", dest.display()))?;
    Ok(())
}

// ─── Auto-download ────────────────────────────────────────────────────────────

pub(crate) async fn download_mihomo(dest: &Path) -> Result<()> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let target = MihomoTarget::current()?;
    let version = latest_mihomo_version().await?;
    let url = target.download_url(&version);

    info!("Downloading mihomo {} from {}", version, url);

    let resp = reqwest::get(&url)
        .await
        .with_context(|| format!("Failed to download mihomo from {}", url))?;

    if !resp.status().is_success() {
        bail!("Download failed: HTTP {}", resp.status());
    }

    let gz_bytes = resp
        .bytes()
        .await
        .context("Failed to read download body")?;

    let mut gz = GzDecoder::new(gz_bytes.as_ref());
    let mut buf = Vec::new();
    gz.read_to_end(&mut buf)
        .context("Failed to decompress downloaded mihomo")?;

    std::fs::write(dest, &buf)
        .with_context(|| format!("Failed to write mihomo to {}", dest.display()))?;
    set_executable(dest)?;

    info!("Mihomo {} downloaded to {}", version, dest.display());
    Ok(())
}

/// Fetch the latest mihomo release version tag from GitHub API
pub(crate) async fn latest_mihomo_version() -> Result<String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    let client = reqwest::Client::builder()
        .user_agent("ladder-core/0.1")
        .build()?;
    let rel: Release = client
        .get("https://api.github.com/repos/MetaCubeX/mihomo/releases/latest")
        .send()
        .await
        .context("Failed to query GitHub API for mihomo version")?
        .json()
        .await
        .context("Failed to parse GitHub release JSON")?;
    Ok(rel.tag_name)
}

// ─── Set executable bit (Unix) ────────────────────────────────────────────────

fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .with_context(|| format!("Cannot stat {}", path.display()))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("Cannot chmod +x {}", path.display()))?;
    Ok(())
}

// ─── Subscription format detection ───────────────────────────────────────────

/// Detected format of a subscription file
#[derive(Debug, Clone, PartialEq)]
pub enum SubscriptionFormat {
    /// Full Clash/Mihomo config (has `proxies` + `rules` or `proxy-groups`)
    FullConfig,
    /// Only a proxies list, suitable for proxy-provider
    ProxyProvider,
}

/// Detect the format of a downloaded subscription YAML.
/// Returns `FullConfig` if the document contains both a `proxies` key and at
/// least one of `rules` / `proxy-groups` / `mode` (i.e. looks like a complete
/// Clash config).  Otherwise returns `ProxyProvider`.
pub fn detect_subscription_format(yaml_text: &str) -> SubscriptionFormat {
    let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(yaml_text) else {
        return SubscriptionFormat::ProxyProvider;
    };
    let map = match &val {
        serde_yaml::Value::Mapping(m) => m,
        _ => return SubscriptionFormat::ProxyProvider,
    };

    let has_proxies = map.contains_key(&serde_yaml::Value::String("proxies".into()));
    let has_config_keys = map.contains_key(&serde_yaml::Value::String("rules".into()))
        || map.contains_key(&serde_yaml::Value::String("proxy-groups".into()))
        || map.contains_key(&serde_yaml::Value::String("mode".into()));

    if has_proxies && has_config_keys {
        SubscriptionFormat::FullConfig
    } else {
        SubscriptionFormat::ProxyProvider
    }
}

/// Download the content of a subscription URL and return raw text.
pub async fn fetch_subscription(url: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .user_agent("clash.meta")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to fetch subscription: {}", url))?;
    if !resp.status().is_success() {
        bail!("Subscription fetch failed: HTTP {}", resp.status());
    }
    resp.text().await.context("Failed to read subscription body")
}

/// Merge a full Clash config with ladder's port/controller settings.
/// Overwrites mixed-port, socks-port, external-controller and secret,
/// then serialises back to YAML string.
pub fn merge_full_config(yaml_text: &str, cfg: &LadderConfig) -> Result<String> {
    let mut val: serde_yaml::Value =
        serde_yaml::from_str(yaml_text).context("Failed to parse subscription YAML")?;

    let map = val
        .as_mapping_mut()
        .context("Subscription YAML is not a mapping")?;

    macro_rules! set {
        ($key:expr, $v:expr) => {
            map.insert(
                serde_yaml::Value::String($key.into()),
                $v,
            );
        };
    }

    set!("mixed-port", serde_yaml::Value::Number(cfg.ladder.port.into()));
    set!("socks-port", serde_yaml::Value::Number(cfg.ladder.socks_port.into()));
    set!(
        "external-controller",
        serde_yaml::Value::String(format!("127.0.0.1:{}", cfg.ladder.control_port))
    );
    if let Some(secret) = &cfg.ladder.api_secret {
        set!("secret", serde_yaml::Value::String(secret.clone()));
    }

    serde_yaml::to_string(&val).context("Failed to serialize merged config YAML")
}

// ─── Config YAML generation ───────────────────────────────────────────────────

/// Generate a Mihomo config.yaml from LadderConfig + filtered subscriptions
pub fn generate_config_yaml(cfg: &LadderConfig, subs: &[&Subscription]) -> Result<String> {
    if subs.is_empty() {
        bail!("No subscriptions provided to generate config");
    }

    let mut proxy_providers = serde_yaml::Mapping::new();
    for sub in subs {
        let mut provider = serde_yaml::Mapping::new();
        provider.insert(
            serde_yaml::Value::String("type".into()),
            serde_yaml::Value::String("http".into()),
        );
        provider.insert(
            serde_yaml::Value::String("url".into()),
            serde_yaml::Value::String(sub.url.clone()),
        );
        provider.insert(
            serde_yaml::Value::String("interval".into()),
            serde_yaml::Value::Number(sub.interval.into()),
        );
        // health-check
        let mut hc = serde_yaml::Mapping::new();
        hc.insert(
            serde_yaml::Value::String("enable".into()),
            serde_yaml::Value::Bool(true),
        );
        hc.insert(
            serde_yaml::Value::String("url".into()),
            serde_yaml::Value::String("https://www.gstatic.com/generate_204".into()),
        );
        hc.insert(
            serde_yaml::Value::String("interval".into()),
            serde_yaml::Value::Number(300u64.into()),
        );
        provider.insert(
            serde_yaml::Value::String("health-check".into()),
            serde_yaml::Value::Mapping(hc),
        );
        proxy_providers.insert(
            serde_yaml::Value::String(sub.name.clone()),
            serde_yaml::Value::Mapping(provider),
        );
    }

    // proxy-groups: use all subscription names
    let use_list: Vec<serde_yaml::Value> = subs
        .iter()
        .map(|s| serde_yaml::Value::String(s.name.clone()))
        .collect();
    let mut proxy_group = serde_yaml::Mapping::new();
    proxy_group.insert(
        serde_yaml::Value::String("name".into()),
        serde_yaml::Value::String("PROXY".into()),
    );
    proxy_group.insert(
        serde_yaml::Value::String("type".into()),
        serde_yaml::Value::String("select".into()),
    );
    proxy_group.insert(
        serde_yaml::Value::String("use".into()),
        serde_yaml::Value::Sequence(use_list),
    );

    // auto select group
    let use_list2: Vec<serde_yaml::Value> = subs
        .iter()
        .map(|s| serde_yaml::Value::String(s.name.clone()))
        .collect();
    let mut auto_group = serde_yaml::Mapping::new();
    auto_group.insert(
        serde_yaml::Value::String("name".into()),
        serde_yaml::Value::String("AUTO".into()),
    );
    auto_group.insert(
        serde_yaml::Value::String("type".into()),
        serde_yaml::Value::String("url-test".into()),
    );
    auto_group.insert(
        serde_yaml::Value::String("use".into()),
        serde_yaml::Value::Sequence(use_list2),
    );
    auto_group.insert(
        serde_yaml::Value::String("url".into()),
        serde_yaml::Value::String("https://www.gstatic.com/generate_204".into()),
    );
    auto_group.insert(
        serde_yaml::Value::String("interval".into()),
        serde_yaml::Value::Number(300u64.into()),
    );

    let mut root = serde_yaml::Mapping::new();

    // basic settings
    root.insert(
        serde_yaml::Value::String("mixed-port".into()),
        serde_yaml::Value::Number(cfg.ladder.port.into()),
    );
    root.insert(
        serde_yaml::Value::String("socks-port".into()),
        serde_yaml::Value::Number(cfg.ladder.socks_port.into()),
    );
    root.insert(
        serde_yaml::Value::String("allow-lan".into()),
        serde_yaml::Value::Bool(false),
    );
    root.insert(
        serde_yaml::Value::String("mode".into()),
        serde_yaml::Value::String("rule".into()),
    );
    root.insert(
        serde_yaml::Value::String("log-level".into()),
        serde_yaml::Value::String("info".into()),
    );

    // external-controller
    let ctrl = format!("127.0.0.1:{}", cfg.ladder.control_port);
    root.insert(
        serde_yaml::Value::String("external-controller".into()),
        serde_yaml::Value::String(ctrl),
    );
    if let Some(secret) = &cfg.ladder.api_secret {
        root.insert(
            serde_yaml::Value::String("secret".into()),
            serde_yaml::Value::String(secret.clone()),
        );
    }

    root.insert(
        serde_yaml::Value::String("proxy-providers".into()),
        serde_yaml::Value::Mapping(proxy_providers),
    );
    root.insert(
        serde_yaml::Value::String("proxy-groups".into()),
        serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::Mapping(proxy_group),
            serde_yaml::Value::Mapping(auto_group),
        ]),
    );

    // basic rules
    let rules = vec![
        "DOMAIN-SUFFIX,local,DIRECT",
        "IP-CIDR,127.0.0.0/8,DIRECT",
        "IP-CIDR,172.16.0.0/12,DIRECT",
        "IP-CIDR,192.168.0.0/16,DIRECT",
        "IP-CIDR,10.0.0.0/8,DIRECT",
        "GEOIP,CN,DIRECT",
        "MATCH,PROXY",
    ];
    root.insert(
        serde_yaml::Value::String("rules".into()),
        serde_yaml::Value::Sequence(
            rules
                .iter()
                .map(|r| serde_yaml::Value::String(r.to_string()))
                .collect(),
        ),
    );

    serde_yaml::to_string(&serde_yaml::Value::Mapping(root))
        .context("Failed to serialize config YAML")
}

/// Write generated config.yaml to ~/.config/ladder/mihomo-config.yaml
pub fn write_config_yaml(cfg: &LadderConfig, subs: &[&Subscription]) -> Result<PathBuf> {
    let yaml = generate_config_yaml(cfg, subs)?;
    let path = config_dir()?.join("mihomo-config.yaml");
    std::fs::write(&path, yaml)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    info!("Generated mihomo config: {}", path.display());
    Ok(path)
}

/// Smart config writer: detects subscription format and chooses the best strategy.
///
/// - Single `FullConfig`: merges port/controller settings directly into it.
/// - All `ProxyProvider`: uses the standard proxy-provider generation.
/// - Mixed / multiple full configs: falls back to proxy-provider mode.
///
/// Returns `(path, strategy_description)`.
pub async fn smart_write_config_yaml(
    cfg: &LadderConfig,
    subs: &[&Subscription],
) -> Result<(PathBuf, String)> {
    if subs.is_empty() {
        bail!("No subscriptions provided");
    }

    // Fetch all subscription texts and detect formats
    let mut fetched: Vec<(&Subscription, String, SubscriptionFormat)> = Vec::new();
    for s in subs {
        let text = fetch_subscription(&s.url).await
            .with_context(|| format!("Failed to fetch subscription: {}", s.name))?;
        let fmt = detect_subscription_format(&text);
        info!("Subscription '{}' detected as: {:?}", s.name, fmt);
        fetched.push((s, text, fmt));
    }

    let all_full = fetched.iter().all(|(_, _, f)| *f == SubscriptionFormat::FullConfig);
    let all_provider = fetched.iter().all(|(_, _, f)| *f == SubscriptionFormat::ProxyProvider);

    let path = config_dir()?.join("mihomo-config.yaml");

    if all_full && fetched.len() == 1 {
        // Single full config: merge port/controller directly
        let (s, text, _) = &fetched[0];
        let yaml = merge_full_config(text, cfg)
            .with_context(|| format!("Failed to merge full config for '{}'", s.name))?;
        std::fs::write(&path, &yaml)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        info!("Smart config: using full config from '{}'", s.name);
        Ok((path, format!("full-config ({})", s.name)))
    } else if all_provider {
        // All are proxy-provider format: use standard generation
        let config_path = write_config_yaml(cfg, subs)?;
        Ok((config_path, "proxy-provider".to_string()))
    } else {
        // Mixed or multiple full configs: fall back to proxy-provider
        warn!(
            "Mixed subscription formats detected, falling back to proxy-provider mode."
        );
        let config_path = write_config_yaml(cfg, subs)?;
        Ok((config_path, "proxy-provider (mixed fallback)".to_string()))
    }
}

// ─── Process management ───────────────────────────────────────────────────────

pub struct MihomoProcess {
    pub child: Child,
    pub pid: u32,
}

/// Spawn mihomo process
pub async fn spawn_mihomo(bin: &Path, config: &Path) -> Result<MihomoProcess> {
    info!("Spawning mihomo: {} -f {}", bin.display(), config.display());
    let child = Command::new(bin)
        .args(["-f", &config.to_string_lossy()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to spawn mihomo: {}", bin.display()))?;
    let pid = child.id().context("Failed to get mihomo PID")?;
    info!("Mihomo started with PID {}", pid);
    Ok(MihomoProcess { child, pid })
}

/// Gracefully stop mihomo: try SIGTERM first, then SIGKILL after 3s
pub async fn stop_mihomo(pid: u32) -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let nix_pid = Pid::from_raw(pid as i32);
    info!("Sending SIGTERM to mihomo PID {}", pid);
    match kill(nix_pid, Signal::SIGTERM) {
        Ok(_) => {}
        Err(nix::errno::Errno::ESRCH) => {
            warn!("Mihomo PID {} already gone", pid);
            return Ok(());
        }
        Err(e) => warn!("SIGTERM failed: {}", e),
    }

    // Wait up to 3s
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        match kill(nix_pid, None) {
            Err(nix::errno::Errno::ESRCH) => {
                info!("Mihomo exited cleanly");
                return Ok(());
            }
            _ => {}
        }
    }

    // Force kill
    warn!("Mihomo did not exit in 3s, sending SIGKILL");
    let _ = kill(nix_pid, Signal::SIGKILL);
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LadderConfig, LadderSettings, Subscription};

    fn make_config() -> LadderConfig {
        LadderConfig {
            ladder: LadderSettings::default(),
            subscriptions: vec![
                Subscription {
                    name: "机场A".to_string(),
                    url: "https://sub.example.com/a".to_string(),
                    interval: 86400,
                },
                Subscription {
                    name: "机场B".to_string(),
                    url: "https://sub.example.com/b".to_string(),
                    interval: 43200,
                },
            ],
        }
    }

    #[test]
    fn test_generate_config_yaml_two_subs() {
        let cfg = make_config();
        let subs: Vec<&Subscription> = cfg.subscriptions.iter().collect();
        let yaml = generate_config_yaml(&cfg, &subs).unwrap();
        assert!(yaml.contains("机场A"));
        assert!(yaml.contains("机场B"));
        assert!(yaml.contains("proxy-providers"));
        assert!(yaml.contains("proxy-groups"));
        assert!(yaml.contains("PROXY"));
        assert!(yaml.contains("external-controller"));
        assert!(yaml.contains("7890") || yaml.contains("mixed-port"));
    }

    #[test]
    fn test_generate_config_yaml_empty_fails() {
        let cfg = make_config();
        assert!(generate_config_yaml(&cfg, &[]).is_err());
    }

    #[test]
    fn test_mihomo_target_url() {
        let t = MihomoTarget { os: "linux", arch: "amd64" };
        let url = t.download_url("v1.18.0");
        assert!(url.contains("linux-amd64"));
        assert!(url.contains("v1.18.0"));
        assert!(url.contains("github.com"));
    }

    #[test]
    fn test_generate_config_rules() {
        let cfg = make_config();
        let subs: Vec<&Subscription> = cfg.subscriptions.iter().collect();
        let yaml = generate_config_yaml(&cfg, &subs).unwrap();
        assert!(yaml.contains("GEOIP,CN,DIRECT"));
        assert!(yaml.contains("MATCH,PROXY"));
    }

    // ─── Format detection tests ───────────────────────────────────────────────

    #[test]
    fn test_detect_full_config() {
        let yaml = r#"
port: 7890
mode: rule
proxies:
  - name: node1
    type: ss
    server: example.com
    port: 443
proxy-groups:
  - name: PROXY
    type: select
    proxies: [node1]
rules:
  - MATCH,PROXY
"#;
        assert_eq!(detect_subscription_format(yaml), SubscriptionFormat::FullConfig);
    }

    #[test]
    fn test_detect_proxy_provider() {
        let yaml = r#"
proxies:
  - name: node1
    type: ss
    server: example.com
    port: 443
  - name: node2
    type: vmess
    server: example.com
    port: 8443
"#;
        assert_eq!(detect_subscription_format(yaml), SubscriptionFormat::ProxyProvider);
    }

    #[test]
    fn test_detect_invalid_yaml() {
        assert_eq!(detect_subscription_format("not: valid: yaml: ["), SubscriptionFormat::ProxyProvider);
    }

    #[test]
    fn test_merge_full_config_overrides_ports() {
        let yaml = r#"
mixed-port: 1234
socks-port: 5678
external-controller: "127.0.0.1:9999"
mode: rule
proxies: []
proxy-groups: []
rules:
  - MATCH,DIRECT
"#;
        let cfg = make_config();
        let merged = merge_full_config(yaml, &cfg).unwrap();
        let val: serde_yaml::Value = serde_yaml::from_str(&merged).unwrap();
        let map = val.as_mapping().unwrap();
        assert_eq!(
            map[&serde_yaml::Value::String("mixed-port".into())],
            serde_yaml::Value::Number(cfg.ladder.port.into())
        );
        assert_eq!(
            map[&serde_yaml::Value::String("socks-port".into())],
            serde_yaml::Value::Number(cfg.ladder.socks_port.into())
        );
        let ctrl = map[&serde_yaml::Value::String("external-controller".into())]
            .as_str()
            .unwrap();
        assert!(ctrl.contains(&cfg.ladder.control_port.to_string()));
    }

}
