use anyhow::Result;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::api::{MihomoApi, ProxyInfo};
use crate::config::LadderConfig;
use crate::env::RuntimeState;

// ─── App State ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Testing,    // speed test in progress
    AutoBest,   // auto-best selection in progress
}

#[derive(Debug, Clone)]
pub struct NodeEntry {
    pub name: String,
    pub delay: u64,      // 0 = timeout / unknown
    pub is_current: bool,
    pub subscription: String, // which subscription this node belongs to
}

/// Tab filter
#[derive(Debug, Clone, PartialEq)]
pub enum TabFilter {
    All,
    Sub(String),
}

impl TabFilter {
    pub fn label(&self) -> String {
        match self {
            TabFilter::All => "全部".to_string(),
            TabFilter::Sub(s) => s.clone(),
        }
    }
}

pub struct App {
    pub cfg: LadderConfig,
    pub state: RuntimeState,
    pub api: Option<MihomoApi>,

    /// All nodes
    pub nodes: Vec<NodeEntry>,

    /// Tab list: [All, Sub1, Sub2, ...]
    pub tabs: Vec<TabFilter>,
    pub active_tab: usize,

    /// Cursor position in the visible node list
    pub cursor: usize,

    /// Current selected proxy node name
    pub current_node: Option<String>,

    pub mode: AppMode,
    pub status_msg: Option<String>,
    pub status_msg_time: Option<Instant>,

    /// Should quit
    pub should_quit: bool,

    /// Last refresh time
    pub last_refresh: Instant,
}

impl App {
    pub fn new(cfg: LadderConfig, state: RuntimeState) -> Result<Self> {
        let api = if state.running {
            Some(MihomoApi::new(
                state.control_port,
                cfg.ladder.api_secret.as_deref(),
            )?)
        } else {
            None
        };

        // Build tabs: All + one per subscription
        let mut tabs = vec![TabFilter::All];
        for sub in &cfg.subscriptions {
            tabs.push(TabFilter::Sub(sub.name.clone()));
        }

        Ok(Self {
            cfg,
            state,
            api,
            nodes: Vec::new(),
            tabs,
            active_tab: 0,
            cursor: 0,
            current_node: None,
            mode: AppMode::Normal,
            status_msg: None,
            status_msg_time: None,
            should_quit: false,
            last_refresh: Instant::now() - Duration::from_secs(60), // trigger immediate refresh
        })
    }

    /// Visible nodes for the current tab
    pub fn visible_nodes(&self) -> Vec<&NodeEntry> {
        match &self.tabs[self.active_tab] {
            TabFilter::All => self.nodes.iter().collect(),
            TabFilter::Sub(name) => self.nodes.iter().filter(|n| &n.subscription == name).collect(),
        }
    }

    /// Move cursor up
    pub fn cursor_up(&mut self) {
        let len = self.visible_nodes().len();
        if len == 0 { return; }
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor down
    pub fn cursor_down(&mut self) {
        let len = self.visible_nodes().len();
        if len == 0 { return; }
        if self.cursor < len - 1 {
            self.cursor += 1;
        }
    }

    /// Cycle to next tab
    pub fn next_tab(&mut self) {
        self.active_tab = (self.active_tab + 1) % self.tabs.len();
        self.cursor = 0;
    }

    /// Cycle to prev tab
    pub fn prev_tab(&mut self) {
        if self.active_tab == 0 {
            self.active_tab = self.tabs.len() - 1;
        } else {
            self.active_tab -= 1;
        }
        self.cursor = 0;
    }

    /// Refresh node list from Mihomo API
    pub async fn refresh_nodes(&mut self) {
        let api = match &self.api {
            Some(_) => MihomoApi::new(
                self.state.control_port,
                self.cfg.ladder.api_secret.as_deref(),
            ).ok(),
            None => return,
        };
        let Some(api) = api else { return };

        match api.get_proxies().await {
            Ok(proxies) => {
                self.nodes = build_node_list(&proxies, &self.cfg);
                // Find current selected node in the main selectable group
                let current_node = proxies.get("PROXY")
                    .or_else(|| proxies.get("🔰 节点选择"))
                    .or_else(|| proxies.get("节点选择"))
                    .or_else(|| proxies.get("Proxy"))
                    .or_else(|| {
                        proxies.values().find(|info| {
                            info.proxy_type.eq_ignore_ascii_case("Selector") && info.all.is_some()
                        })
                    })
                    .and_then(|g| g.now.clone());
                self.current_node = current_node.clone();
                for node in &mut self.nodes {
                    node.is_current = Some(&node.name) == current_node.as_ref();
                }
                self.last_refresh = Instant::now();
            }
            Err(e) => {
                self.set_status(format!("Refresh error: {}", e));
            }
        }
    }

    /// Detect the main selectable proxy group name.
    /// In proxy-provider mode it's "PROXY"; in FullConfig mode it's the first selector group.
    pub async fn detect_main_group(&self) -> Option<String> {
        let api = MihomoApi::new(
            self.state.control_port,
            self.cfg.ladder.api_secret.as_deref(),
        ).ok()?;
        let proxies = api.get_proxies().await.ok()?;
        for name in &["PROXY", "🔰 节点选择", "节点选择", "Proxy"] {
            if proxies.contains_key(*name) {
                return Some(name.to_string());
            }
        }
        // Fall back to first Selector with an `all` list
        proxies.iter()
            .find(|(_, info)| info.proxy_type.eq_ignore_ascii_case("Selector") && info.all.is_some())
            .map(|(name, _)| name.clone())
    }

    /// Select the node under cursor
    pub async fn select_current(&mut self) {
        let api = match &self.api {
            Some(_) => MihomoApi::new(
                self.state.control_port,
                self.cfg.ladder.api_secret.as_deref(),
            ).ok(),
            None => return,
        };
        let Some(api) = api else { return };

        let visible: Vec<String> = self.visible_nodes().iter().map(|n| n.name.clone()).collect();
        if visible.is_empty() { return; }
        let node_name = visible[self.cursor].clone();

        let group_name = self.detect_main_group().await.unwrap_or_else(|| "PROXY".to_string());

        match api.select_proxy(&group_name, &node_name).await {
            Ok(_) => {
                self.current_node = Some(node_name.clone());
                for n in &mut self.nodes {
                    n.is_current = n.name == node_name;
                }
                self.set_status(format!("✓ 已切换到 {}", node_name));
            }
            Err(e) => {
                self.set_status(format!("切换失败: {}", e));
            }
        }
    }

    /// Run speed test on all visible nodes
    pub async fn run_speed_test(&mut self) {
        let api = match &self.api {
            Some(_) => MihomoApi::new(
                self.state.control_port,
                self.cfg.ladder.api_secret.as_deref(),
            ).ok(),
            None => return,
        };
        let Some(api) = api else { return };

        self.mode = AppMode::Testing;
        self.set_status("测速中...".to_string());

        let node_names: Vec<String> = self.nodes.iter().map(|n| n.name.clone()).collect();
        let mut delays = HashMap::new();

        for name in &node_names {
            let d = api
                .get_delay(name, "https://www.gstatic.com/generate_204")
                .await
                .unwrap_or(0);
            delays.insert(name.clone(), d);
        }

        for node in &mut self.nodes {
            if let Some(d) = delays.get(&node.name) {
                node.delay = *d;
            }
        }

        self.mode = AppMode::Normal;
        self.set_status("✓ 测速完成".to_string());
    }

    /// Auto-select best node
    pub async fn auto_best(&mut self) {
        let api = match &self.api {
            Some(_) => MihomoApi::new(
                self.state.control_port,
                self.cfg.ladder.api_secret.as_deref(),
            ).ok(),
            None => return,
        };
        let Some(api) = api else { return };

        self.mode = AppMode::AutoBest;
        self.set_status("🔍 自动选优中...".to_string());

        let group_name = self.detect_main_group().await.unwrap_or_else(|| "PROXY".to_string());

        match api
            .auto_best(&group_name, "https://www.gstatic.com/generate_204")
            .await
        {
            Ok((node, delay)) => {
                self.current_node = Some(node.clone());
                for n in &mut self.nodes {
                    n.is_current = n.name == node;
                }
                self.set_status(format!("✓ 已切换到最优节点 {} ({}ms)", node, delay));
            }
            Err(e) => {
                self.set_status(format!("自动选优失败: {}", e));
            }
        }

        self.mode = AppMode::Normal;
    }

    /// Force-update all subscriptions
    pub async fn update_providers(&mut self) {
        if self.api.is_none() {
            return;
        }

        self.set_status("更新订阅中...".to_string());

        let subs: Vec<_> = self.cfg.subscriptions.iter().collect();
        match crate::mihomo::update_running_subscriptions(&self.cfg, &subs).await {
            Ok(reports) => {
                let failed = reports.iter().filter(|report| !report.success).count();
                if failed == 0 {
                    self.set_status(format!("✓ 已更新 {} 个订阅", reports.len()));
                } else {
                    self.set_status(format!("更新完成，{} 个成功，{} 个失败", reports.len() - failed, failed));
                }
                self.refresh_nodes().await;
            }
            Err(e) => {
                self.set_status(format!("更新失败: {}", e));
            }
        }
    }

    pub fn set_status(&mut self, msg: String) {
        self.status_msg = Some(msg);
        self.status_msg_time = Some(Instant::now());
    }

    /// Clear status message after 3 seconds
    pub fn tick_status(&mut self) {
        if let Some(t) = self.status_msg_time {
            if t.elapsed() > Duration::from_secs(3) {
                self.status_msg = None;
                self.status_msg_time = None;
            }
        }
    }
}

// ─── Build node list from proxies map ────────────────────────────────────────

/// Proxy group types that are "selectable" in Mihomo
const GROUP_TYPES: &[&str] = &[
    "Selector", "URLTest", "Fallback", "LoadBalance",
    "select", "url-test", "fallback", "load-balance",
];

fn is_group_type(t: &str) -> bool {
    GROUP_TYPES.iter().any(|&gt| gt.eq_ignore_ascii_case(t))
}

/// Reserved / internal proxy names that should not be shown as nodes
fn is_reserved_proxy(name: &str) -> bool {
    matches!(name, "DIRECT" | "REJECT" | "REJECT-DROP" | "COMPATIBLE" | "PASS" | "GLOBAL")
}

fn build_node_list(
    proxies: &HashMap<String, ProxyInfo>,
    cfg: &LadderConfig,
) -> Vec<NodeEntry> {
    let mut result = Vec::new();

    // Strategy 1: match by subscription name (proxy-provider mode)
    for sub in &cfg.subscriptions {
        if let Some(provider_group) = proxies.get(&sub.name) {
            if let Some(all) = &provider_group.all {
                for node_name in all {
                    let info = proxies.get(node_name);
                    let delay = info.map(|i| i.latest_delay()).unwrap_or(0);
                    result.push(NodeEntry {
                        name: node_name.clone(),
                        delay,
                        is_current: false,
                        subscription: sub.name.clone(),
                    });
                }
            }
        }
    }

    // Strategy 2: if nothing found via subscription names, scan all proxy groups
    // (FullConfig mode – the groups are named by the config itself, not by ladder)
    if result.is_empty() {
        // Collect all group names so we can exclude them from the node list
        let group_names: std::collections::HashSet<&str> = proxies
            .iter()
            .filter(|(_, info)| is_group_type(&info.proxy_type))
            .map(|(name, _)| name.as_str())
            .collect();

        // Find the "main" selectable group: prefer "PROXY", then first Selector
        let main_group = proxies.get("PROXY")
            .or_else(|| {
                // Look for groups with known names (common in Chinese Clash configs)
                for candidate in &["🔰 节点选择", "节点选择", "Proxy", "proxy"] {
                    if let Some(g) = proxies.get(*candidate) {
                        return Some(g);
                    }
                }
                // Fall back to first Selector group
                proxies.values().find(|info| {
                    info.proxy_type.eq_ignore_ascii_case("Selector") && info.all.is_some()
                })
            });

        if let Some(group) = main_group {
            if let Some(all) = &group.all {
                for node_name in all {
                    // Skip groups and reserved names – only show actual proxy nodes
                    if group_names.contains(node_name.as_str()) || is_reserved_proxy(node_name) {
                        continue;
                    }
                    let info = proxies.get(node_name);
                    let delay = info.map(|i| i.latest_delay()).unwrap_or(0);
                    // Try to determine provider name from the proxy info
                    let sub_name = info
                        .and_then(|i| i.extra.as_ref())
                        .and_then(|_| None::<String>) // extra doesn't hold provider; fall back
                        .unwrap_or_else(|| "default".to_string());
                    result.push(NodeEntry {
                        name: node_name.clone(),
                        delay,
                        is_current: false,
                        subscription: sub_name,
                    });
                }
            }
        }
    }

    result
}
