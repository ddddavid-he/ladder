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
                if let Some(proxy_group) = proxies.get("PROXY") {
                    self.current_node = proxy_group.now.clone();
                }
                let current = self.current_node.clone();
                for node in &mut self.nodes {
                    node.is_current = Some(&node.name) == current.as_ref();
                }
                self.last_refresh = Instant::now();
            }
            Err(e) => {
                self.set_status(format!("Refresh error: {}", e));
            }
        }
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

        match api.select_proxy("PROXY", &node_name).await {
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
            Some(a) => MihomoApi::new(
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

        match api
            .auto_best("PROXY", "https://www.gstatic.com/generate_204")
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

    /// Force-update all subscription providers
    pub async fn update_providers(&mut self) {
        let api = match &self.api {
            Some(_) => MihomoApi::new(
                self.state.control_port,
                self.cfg.ladder.api_secret.as_deref(),
            ).ok(),
            None => return,
        };
        let Some(api) = api else { return };

        self.set_status("更新订阅中...".to_string());

        let names: Vec<String> = self.cfg.subscriptions.iter().map(|s| s.name.clone()).collect();
        for name in &names {
            let _ = api.update_provider(name).await;
        }

        self.set_status("✓ 订阅已更新".to_string());
        self.refresh_nodes().await;
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

fn build_node_list(
    proxies: &HashMap<String, ProxyInfo>,
    cfg: &LadderConfig,
) -> Vec<NodeEntry> {
    let mut result = Vec::new();

    // For each subscription, collect its nodes in order
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

    result
}
