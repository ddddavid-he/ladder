use anyhow::Result;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::api::{MihomoApi, ProxyInfo};
use crate::config::LadderConfig;
use crate::env::RuntimeState;

// ─── App State ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Testing,    // speed test in progress
    AutoBest,   // auto-best selection in progress
    Updating,   // subscription update in progress
}

/// Messages sent from background tasks back to the event loop
pub enum BgMessage {
    /// A single node delay result (name, delay)
    DelayResult(String, u64),
    /// Speed test finished
    SpeedTestDone,
    /// Auto-best finished: selected node name and delay
    AutoBestDone(Result<(String, u64)>),
    /// Subscription update finished
    UpdateDone(Result<Vec<crate::mihomo::SubscriptionUpdateReport>>),
    /// Status message update
    Status(String),
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

    /// Channel receiver for background task messages
    pub bg_rx: mpsc::UnboundedReceiver<BgMessage>,
    /// Channel sender (cloned into background tasks)
    pub bg_tx: mpsc::UnboundedSender<BgMessage>,
    /// Cancellation token for the current background operation
    pub cancel_token: Option<CancellationToken>,

    /// Progress counter for speed test (completed / total)
    pub test_progress: Option<(usize, usize)>,
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

        let (bg_tx, bg_rx) = mpsc::unbounded_channel();

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
            bg_rx,
            bg_tx,
            cancel_token: None,
            test_progress: None,
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

    /// Run speed test on all visible nodes (non-blocking: spawns a background task)
    pub fn run_speed_test(&mut self) {
        let api = match &self.api {
            Some(_) => MihomoApi::new(
                self.state.control_port,
                self.cfg.ladder.api_secret.as_deref(),
            ).ok(),
            None => return,
        };
        let Some(api) = api else { return };

        self.mode = AppMode::Testing;
        let node_names: Vec<String> = self.nodes.iter().map(|n| n.name.clone()).collect();
        let total = node_names.len();
        self.test_progress = Some((0, total));
        self.set_status(format!("测速中... (0/{})", total));

        let tx = self.bg_tx.clone();
        let cancel = CancellationToken::new();
        self.cancel_token = Some(cancel.clone());

        tokio::spawn(async move {
            for (i, name) in node_names.iter().enumerate() {
                if cancel.is_cancelled() {
                    let _ = tx.send(BgMessage::Status("⚠ 测速已取消".to_string()));
                    let _ = tx.send(BgMessage::SpeedTestDone);
                    return;
                }
                let d = api
                    .get_delay(name, "https://www.gstatic.com/generate_204")
                    .await
                    .unwrap_or(0);
                let _ = tx.send(BgMessage::DelayResult(name.clone(), d));
                let _ = tx.send(BgMessage::Status(format!(
                    "测速中... ({}/{})",
                    i + 1,
                    node_names.len()
                )));
            }
            let _ = tx.send(BgMessage::Status("✓ 测速完成".to_string()));
            let _ = tx.send(BgMessage::SpeedTestDone);
        });
    }

    /// Auto-select best node (non-blocking: spawns a background task)
    pub fn auto_best(&mut self) {
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

        let tx = self.bg_tx.clone();
        let cancel = CancellationToken::new();
        self.cancel_token = Some(cancel.clone());

        // We need the group name. Detect it first, then spawn the task.
        let control_port = self.state.control_port;
        let api_secret = self.cfg.ladder.api_secret.clone();

        tokio::spawn(async move {
            // Detect main group
            let group_name = {
                let detect_api = match MihomoApi::new(control_port, api_secret.as_deref()) {
                    Ok(a) => a,
                    Err(_) => {
                        let _ = tx.send(BgMessage::AutoBestDone(Err(anyhow::anyhow!(
                            "无法创建 API 客户端"
                        ))));
                        return;
                    }
                };
                let proxies = match detect_api.get_proxies().await {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = tx.send(BgMessage::AutoBestDone(Err(e)));
                        return;
                    }
                };
                let mut found = None;
                for name in &["PROXY", "🔰 节点选择", "节点选择", "Proxy"] {
                    if proxies.contains_key(*name) {
                        found = Some(name.to_string());
                        break;
                    }
                }
                found.or_else(|| {
                    proxies.iter()
                        .find(|(_, info)| {
                            info.proxy_type.eq_ignore_ascii_case("Selector")
                                && info.all.is_some()
                        })
                        .map(|(name, _)| name.clone())
                }).unwrap_or_else(|| "PROXY".to_string())
            };

            if cancel.is_cancelled() {
                let _ = tx.send(BgMessage::Status("⚠ 自动选优已取消".to_string()));
                let _ = tx.send(BgMessage::AutoBestDone(Err(anyhow::anyhow!("cancelled"))));
                return;
            }

            let result = api
                .auto_best(&group_name, "https://www.gstatic.com/generate_204")
                .await;
            let _ = tx.send(BgMessage::AutoBestDone(result));
        });
    }

    /// Force-update all subscriptions (non-blocking: spawns a background task)
    pub fn update_providers(&mut self) {
        if self.api.is_none() {
            return;
        }

        self.mode = AppMode::Updating;
        self.set_status("更新订阅中...".to_string());

        let tx = self.bg_tx.clone();
        let cfg = self.cfg.clone();
        let cancel = CancellationToken::new();
        self.cancel_token = Some(cancel.clone());

        tokio::spawn(async move {
            let subs: Vec<_> = cfg.subscriptions.iter().collect();
            let result = crate::mihomo::update_running_subscriptions(&cfg, &subs).await;
            if !cancel.is_cancelled() {
                let _ = tx.send(BgMessage::UpdateDone(result));
            } else {
                let _ = tx.send(BgMessage::Status("⚠ 更新已取消".to_string()));
                let _ = tx.send(BgMessage::UpdateDone(Err(anyhow::anyhow!("cancelled"))));
            }
        });
    }

    pub fn set_status(&mut self, msg: String) {
        self.status_msg = Some(msg);
        self.status_msg_time = Some(Instant::now());
    }

    /// Clear status message after 3 seconds
    pub fn tick_status(&mut self) {
        if let Some(t) = self.status_msg_time {
            if t.elapsed() > Duration::from_secs(3) && self.mode == AppMode::Normal {
                self.status_msg = None;
                self.status_msg_time = None;
            }
        }
    }

    /// Cancel the current background operation (if any)
    pub fn cancel_current(&mut self) {
        if let Some(token) = self.cancel_token.take() {
            token.cancel();
        }
        self.mode = AppMode::Normal;
        self.test_progress = None;
        self.set_status("⚠ 操作已取消".to_string());
    }

    /// Process all pending messages from background tasks (non-blocking).
    /// Should be called every tick in the event loop.
    pub async fn process_bg_messages(&mut self) {
        // Drain all available messages without blocking
        loop {
            match self.bg_rx.try_recv() {
                Ok(msg) => match msg {
                    BgMessage::DelayResult(name, delay) => {
                        for node in &mut self.nodes {
                            if node.name == name {
                                node.delay = delay;
                            }
                        }
                        // Update progress counter
                        if let Some((ref mut done, total)) = self.test_progress {
                            *done += 1;
                            if *done >= total {
                                self.test_progress = None;
                            }
                        }
                    }
                    BgMessage::SpeedTestDone => {
                        self.mode = AppMode::Normal;
                        self.cancel_token = None;
                        self.test_progress = None;
                    }
                    BgMessage::AutoBestDone(result) => {
                        match result {
                            Ok((node, delay)) => {
                                self.current_node = Some(node.clone());
                                for n in &mut self.nodes {
                                    n.is_current = n.name == node;
                                }
                                self.set_status(format!(
                                    "✓ 已切换到最优节点 {} ({}ms)",
                                    node, delay
                                ));
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                if msg != "cancelled" {
                                    self.set_status(format!("自动选优失败: {}", msg));
                                }
                            }
                        }
                        self.mode = AppMode::Normal;
                        self.cancel_token = None;
                    }
                    BgMessage::UpdateDone(result) => {
                        match result {
                            Ok(reports) => {
                                let failed =
                                    reports.iter().filter(|report| !report.success).count();
                                if failed == 0 {
                                    self.set_status(format!(
                                        "✓ 已更新 {} 个订阅",
                                        reports.len()
                                    ));
                                } else {
                                    self.set_status(format!(
                                        "更新完成，{} 个成功，{} 个失败",
                                        reports.len() - failed,
                                        failed
                                    ));
                                }
                                // Trigger a refresh after update
                                self.refresh_nodes().await;
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                if msg != "cancelled" {
                                    self.set_status(format!("更新失败: {}", msg));
                                }
                            }
                        }
                        self.mode = AppMode::Normal;
                        self.cancel_token = None;
                    }
                    BgMessage::Status(msg) => {
                        self.set_status(msg);
                    }
                },
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
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
                    // Use provider-name from Mihomo API if available; otherwise
                    // try to match against configured subscription names.
                    let sub_name = info
                        .and_then(|i| {
                            i.provider_name.as_ref()
                                .filter(|pn| !pn.is_empty())
                                .cloned()
                        })
                        .or_else(|| {
                            // In FullConfig mode without providers, use the first subscription name
                            cfg.subscriptions.first().map(|s| s.name.clone())
                        })
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
