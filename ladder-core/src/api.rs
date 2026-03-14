use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info};

// ─── API Client ───────────────────────────────────────────────────────────────

pub struct MihomoApi {
    client: Client,
    base_url: String,
}

impl MihomoApi {
    pub fn new(control_port: u16, secret: Option<&str>) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(s) = secret {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", s))
                    .context("Invalid API secret")?,
            );
        }
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(10))
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self {
            client,
            base_url: format!("http://127.0.0.1:{}", control_port),
        })
    }

    /// Wait until the Mihomo API is ready (retry up to max_tries * 500ms)
    pub async fn wait_ready(&self, max_tries: u32) -> Result<()> {
        for i in 0..max_tries {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if self.version().await.is_ok() {
                info!("Mihomo API ready after {}ms", (i + 1) * 500);
                return Ok(());
            }
        }
        bail!("Mihomo API not ready after {}ms", max_tries * 500)
    }

    // ─── Basic ───────────────────────────────────────────────────────────────

    /// GET /version
    pub async fn version(&self) -> Result<String> {
        #[derive(Deserialize)]
        struct Ver {
            version: String,
        }
        let r: Ver = self
            .client
            .get(format!("{}/version", self.base_url))
            .send()
            .await?
            .json()
            .await?;
        Ok(r.version)
    }

    // ─── Proxies ─────────────────────────────────────────────────────────────

    /// GET /proxies - Returns all proxy names and their info
    pub async fn get_proxies(&self) -> Result<HashMap<String, ProxyInfo>> {
        #[derive(Deserialize)]
        struct Resp {
            proxies: HashMap<String, ProxyInfo>,
        }
        let resp = match self
            .client
            .get(format!("{}/proxies", self.base_url))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                bail!(
                    "GET /proxies failed — Mihomo API unreachable at {} (is mihomo running?): {}",
                    self.base_url,
                    e
                );
            }
        };

        let status = resp.status();
        let text = resp
            .text()
            .await
            .context("Failed to read /proxies response body")?;

        if !status.is_success() {
            let preview: String = text.chars().take(200).collect();
            bail!("/proxies returned HTTP {}: {}", status, preview);
        }

        let r: Resp = serde_json::from_str(&text)
            .with_context(|| {
                let preview: String = text.chars().take(500).collect();
                format!(
                    "Failed to parse /proxies response (len={}).\nPreview: {}",
                    text.len(),
                    preview,
                )
            })?;
        Ok(r.proxies)
    }

    /// GET /proxies/{group} - Get a specific proxy group info
    pub async fn get_proxy_group(&self, group: &str) -> Result<ProxyInfo> {
        let r: ProxyInfo = self
            .client
            .get(format!("{}/proxies/{}", self.base_url, group))
            .send()
            .await
            .with_context(|| format!("GET /proxies/{} failed", group))?
            .json()
            .await
            .context("Failed to parse proxy group response")?;
        Ok(r)
    }

    /// PUT /proxies/{group} - Switch selected node in a proxy group
    pub async fn select_proxy(&self, group: &str, node_name: &str) -> Result<()> {
        #[derive(Serialize)]
        struct Body {
            name: String,
        }
        let resp = self
            .client
            .put(format!("{}/proxies/{}", self.base_url, group))
            .json(&Body { name: node_name.to_string() })
            .send()
            .await
            .with_context(|| format!("PUT /proxies/{} failed", group))?;

        if resp.status().is_success() {
            info!("Switched {} to node: {}", group, node_name);
            Ok(())
        } else {
            bail!("Select proxy failed: HTTP {}", resp.status())
        }
    }

    /// GET /proxies/{name}/delay - Test latency of a specific node
    /// Returns 0 if timeout or error
    pub async fn get_delay(&self, proxy_name: &str, test_url: &str) -> Result<u64> {
        #[derive(Deserialize)]
        struct Resp {
            delay: Option<u64>,
        }
        let resp = self
            .client
            .get(format!("{}/proxies/{}/delay", self.base_url, urlencoding(proxy_name)))
            .query(&[("url", test_url), ("timeout", "3000")])
            .send()
            .await
            .with_context(|| format!("GET /proxies/{}/delay failed", proxy_name))?;

        if resp.status().is_success() {
            let r: Resp = resp.json().await?;
            Ok(r.delay.unwrap_or(0))
        } else {
            // timeout / unreachable → treat as 0
            Ok(0)
        }
    }

    // ─── Proxy Providers ─────────────────────────────────────────────────────

    /// GET /providers/proxies - List all proxy providers
    pub async fn get_providers(&self) -> Result<HashMap<String, ProviderInfo>> {
        #[derive(Deserialize)]
        struct Resp {
            providers: HashMap<String, ProviderInfo>,
        }
        let r: Resp = self
            .client
            .get(format!("{}/providers/proxies", self.base_url))
            .send()
            .await
            .context("GET /providers/proxies failed")?
            .json()
            .await
            .context("Failed to parse providers response")?;
        Ok(r.providers)
    }

    /// PUT /providers/proxies/{name} - Force update a specific provider
    pub async fn update_provider(&self, name: &str) -> Result<()> {
        let resp = self
            .client
            .put(format!("{}/providers/proxies/{}", self.base_url, name))
            .send()
            .await
            .with_context(|| format!("PUT /providers/proxies/{} failed", name))?;
        if resp.status().is_success() {
            info!("Updated provider: {}", name);
            Ok(())
        } else {
            bail!("Update provider failed: HTTP {}", resp.status())
        }
    }

    /// PUT /configs?force=true - Reload mihomo config.
    ///
    /// Strategy:
    /// 1. Try sending config content via `payload` field (avoids Mihomo's IsSafePath check).
    /// 2. If that fails with an HTTP error, fall back to sending `path` field.
    pub async fn reload_config(&self, config_path: &std::path::Path) -> Result<()> {
        let url = format!("{}/configs?force=true", self.base_url);

        // Strategy 1: send config content directly via `payload`
        let yaml_content = std::fs::read_to_string(config_path)
            .with_context(|| format!("Cannot read config file: {}", config_path.display()))?;

        let resp = match self
            .client
            .put(&url)
            .json(&serde_json::json!({ "payload": yaml_content }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // Connection-level failure: Mihomo API is unreachable
                bail!(
                    "Mihomo API unreachable at {} (is mihomo running?): {}",
                    self.base_url,
                    e
                );
            }
        };

        if resp.status().is_success() {
            info!("Mihomo config reloaded via payload from {}", config_path.display());
            return Ok(());
        }

        let status1 = resp.status();
        let body1 = resp.text().await.unwrap_or_default();
        let body1_preview: String = body1.chars().take(200).collect();
        debug!("Reload via payload failed: HTTP {} — {}", status1, body1_preview);

        // Strategy 2: fall back to sending file path
        let resp = self
            .client
            .put(&url)
            .json(&serde_json::json!({ "path": config_path.to_string_lossy() }))
            .send()
            .await
            .with_context(|| format!("Mihomo API unreachable at {}", self.base_url))?;

        if resp.status().is_success() {
            info!("Mihomo config reloaded via path from {}", config_path.display());
            return Ok(());
        }

        let status2 = resp.status();
        let body2 = resp.text().await.unwrap_or_default();
        let body2_preview: String = body2.chars().take(200).collect();
        bail!(
            "Reload config failed.\n  payload attempt: HTTP {} — {}\n  path attempt: HTTP {} — {}",
            status1, body1_preview,
            status2, body2_preview,
        )
    }

    // ─── Auto-Best selection ─────────────────────────────────────────────────

    /// Test all nodes in a group concurrently and select the lowest-latency one.
    /// Nodes with delay==0 (timeout) are excluded.
    /// Returns the selected node name and its delay.
    pub async fn auto_best(
        &self,
        group: &str,
        test_url: &str,
    ) -> Result<(String, u64)> {
        let group_info = self.get_proxy_group(group).await?;
        let all_nodes = group_info.all.unwrap_or_default();

        if all_nodes.is_empty() {
            bail!("No nodes available in group {}", group);
        }

        info!("Testing {} nodes in group {}...", all_nodes.len(), group);

        // Concurrently test all nodes
        let mut tasks = Vec::new();
        for node in &all_nodes {
            let api_clone = MihomoApi {
                client: self.client.clone(),
                base_url: self.base_url.clone(),
            };
            let node_clone = node.clone();
            let url_clone = test_url.to_string();
            tasks.push(tokio::spawn(async move {
                let delay = api_clone.get_delay(&node_clone, &url_clone).await.unwrap_or(0);
                (node_clone, delay)
            }));
        }

        let mut results: Vec<(String, u64)> = Vec::new();
        for t in tasks {
            if let Ok((name, delay)) = t.await {
                debug!("  {} → {}ms", name, if delay == 0 { "timeout".to_string() } else { delay.to_string() });
                if delay > 0 {
                    results.push((name, delay));
                }
            }
        }

        if results.is_empty() {
            bail!("All nodes timed out in group {}", group);
        }

        // Sort by delay ascending
        results.sort_by_key(|(_, d)| *d);
        let (best_node, best_delay) = results.into_iter().next().unwrap();

        // Switch to best node
        self.select_proxy(group, &best_node).await?;
        info!("Auto-best: selected {} ({}ms)", best_node, best_delay);

        Ok((best_node, best_delay))
    }
}

// ─── URL encoding helper ──────────────────────────────────────────────────────

fn urlencoding(s: &str) -> String {
    // Simple percent-encoding for proxy names with spaces
    s.replace(' ', "%20")
}

// ─── Data types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ProxyInfo {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub proxy_type: String,
    /// Current selected node (for select/url-test groups)
    pub now: Option<String>,
    /// All nodes in this group
    pub all: Option<Vec<String>>,
    /// Latency history
    #[serde(default)]
    pub history: Option<Vec<HistoryEntry>>,
    /// Whether the proxy is alive
    pub alive: Option<bool>,
    /// Provider name (non-empty when the proxy comes from a proxy-provider)
    #[serde(rename = "provider-name", default)]
    pub provider_name: Option<String>,
    /// Extra delay histories per test URL (mihomo returns map[string]ProxyState)
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
}

impl ProxyInfo {
    /// Returns the most recent delay (0 if not available)
    pub fn latest_delay(&self) -> u64 {
        self.history
            .as_ref()
            .and_then(|h| h.last())
            .map(|e| e.delay)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryEntry {
    #[serde(default)]
    pub time: String,
    #[serde(default, deserialize_with = "deserialize_delay")]
    pub delay: u64,
}

/// Deserialize delay field flexibly: accept integers, floats, or strings
fn deserialize_delay<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    struct DelayVisitor;
    impl<'de> de::Visitor<'de> for DelayVisitor {
        type Value = u64;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a number or numeric string")
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<u64, E> { Ok(v) }
        fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<u64, E> {
            Ok(if v < 0 { 0 } else { v as u64 })
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<u64, E> {
            Ok(if v < 0.0 { 0 } else { v as u64 })
        }
        fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<u64, E> {
            v.parse::<u64>().map_err(de::Error::custom)
        }
    }
    deserializer.deserialize_any(DelayVisitor)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    #[serde(rename = "vehicleType")]
    pub vehicle_type: Option<String>,
    pub proxies: Option<Vec<ProxyInfo>>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding("HK 01"), "HK%2001");
        assert_eq!(urlencoding("NoSpace"), "NoSpace");
    }

    #[test]
    fn test_proxy_info_latest_delay_empty() {
        let p = ProxyInfo {
            name: "test".to_string(),
            proxy_type: "ss".to_string(),
            now: None,
            all: None,
            history: None,
            alive: None,
            provider_name: None,
            extra: None,
        };
        assert_eq!(p.latest_delay(), 0);
    }

    #[test]
    fn test_proxy_info_latest_delay() {
        let p = ProxyInfo {
            name: "test".to_string(),
            proxy_type: "ss".to_string(),
            now: None,
            all: None,
            history: Some(vec![
                HistoryEntry { time: "2024-01-01".to_string(), delay: 50 },
                HistoryEntry { time: "2024-01-02".to_string(), delay: 23 },
            ]),
            alive: Some(true),
            provider_name: None,
            extra: None,
        };
        assert_eq!(p.latest_delay(), 23);
    }

    #[test]
    fn test_parse_mihomo_proxies_response() {
        // Simulated Mihomo /proxies response with all the fields it actually returns
        let json = r#"{
            "proxies": {
                "DIRECT": {
                    "type": "Direct",
                    "id": "some-uuid",
                    "name": "DIRECT",
                    "alive": true,
                    "udp": true,
                    "uot": false,
                    "xudp": false,
                    "tfo": false,
                    "mptcp": false,
                    "smux": false,
                    "interface": "",
                    "routing-mark": 0,
                    "provider-name": "",
                    "dialer-proxy": "",
                    "history": [],
                    "extra": {}
                },
                "🔰 节点选择": {
                    "type": "Selector",
                    "now": "HK-01",
                    "all": ["HK-01", "JP-01", "US-01"],
                    "testUrl": "",
                    "hidden": false,
                    "icon": "",
                    "name": "🔰 节点选择",
                    "alive": true,
                    "udp": true,
                    "uot": false,
                    "xudp": false,
                    "tfo": false,
                    "mptcp": false,
                    "smux": false,
                    "interface": "",
                    "routing-mark": 0,
                    "provider-name": "",
                    "dialer-proxy": "",
                    "history": [],
                    "extra": {}
                },
                "♻️ 自动选择": {
                    "type": "URLTest",
                    "now": "HK-01",
                    "all": ["HK-01", "JP-01"],
                    "testUrl": "https://www.gstatic.com/generate_204",
                    "expectedStatus": "",
                    "fixed": "",
                    "hidden": false,
                    "icon": "",
                    "name": "♻️ 自动选择",
                    "alive": true,
                    "udp": true,
                    "uot": false,
                    "xudp": false,
                    "tfo": false,
                    "mptcp": false,
                    "smux": false,
                    "interface": "",
                    "routing-mark": 0,
                    "provider-name": "",
                    "dialer-proxy": "",
                    "history": [
                        {"time": "2024-03-14T12:00:00.000Z", "delay": 23}
                    ],
                    "extra": {
                        "https://www.gstatic.com/generate_204": {
                            "alive": true,
                            "history": [
                                {"time": "2024-03-14T12:00:00.000Z", "delay": 23}
                            ]
                        }
                    }
                },
                "HK-01": {
                    "type": "Trojan",
                    "id": "uuid-hk01",
                    "name": "HK-01",
                    "alive": true,
                    "udp": true,
                    "uot": false,
                    "xudp": false,
                    "tfo": false,
                    "mptcp": false,
                    "smux": false,
                    "interface": "",
                    "routing-mark": 0,
                    "provider-name": "WestWorld",
                    "dialer-proxy": "",
                    "history": [
                        {"time": "2024-03-14T12:00:00.000Z", "delay": 45}
                    ],
                    "extra": {
                        "https://www.gstatic.com/generate_204": {
                            "alive": true,
                            "history": [
                                {"time": "2024-03-14T12:00:00.000Z", "delay": 45}
                            ]
                        }
                    }
                }
            }
        }"#;

        #[derive(Deserialize)]
        struct Resp {
            proxies: HashMap<String, ProxyInfo>,
        }

        let r: Resp = serde_json::from_str(json).expect("Should parse Mihomo proxies response");
        assert_eq!(r.proxies.len(), 4);
        assert_eq!(r.proxies["DIRECT"].proxy_type, "Direct");
        assert_eq!(r.proxies["🔰 节点选择"].now, Some("HK-01".to_string()));
        assert_eq!(r.proxies["♻️ 自动选择"].all.as_ref().unwrap().len(), 2);
        assert_eq!(r.proxies["HK-01"].latest_delay(), 45);
    }

    #[test]
    fn test_mihomo_api_new() {
        let api = MihomoApi::new(9090, None);
        assert!(api.is_ok());
        let api = api.unwrap();
        assert_eq!(api.base_url, "http://127.0.0.1:9090");
    }

    #[test]
    fn test_mihomo_api_with_secret() {
        let api = MihomoApi::new(9090, Some("my-secret"));
        assert!(api.is_ok());
    }
}
