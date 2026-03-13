# ladder v2 · Rust 重写方案 Plan（最终版）

## 一、新增需求确认

| 需求 | 方案 |
|------|------|
| 多订阅支持 | 配置文件支持多个 `proxy-provider`，TUI 可分组显示 |
| zsh 兼容 | `ladder-env.sh` 用 POSIX sh 语法，兼容 bash/zsh/fish |
| 发布两种 bin | `ladder-bundled`（内置 mihomo）+ `ladder`（外置 mihomo） |

---

## 二、两种发布版本

### `ladder`（外置版，推荐）
- 体积小（~5MB）
- 首次运行自动从 mihomo GitHub Release 下载对应平台版本到 `~/.config/ladder/mihomo`
- 用户也可手动指定 mihomo 路径：`ladder --mihomo-bin /usr/local/bin/mihomo`

### `ladder-bundled`（内置版）
- 用 `include_bytes!` 宏在编译期将 mihomo 二进制嵌入
- 体积较大（~20MB），但真正零依赖、离线可用
- 构建时通过 build script（`build.rs`）自动下载对应平台 mihomo 并嵌入

```rust
// 运行时释放内置 mihomo
#[cfg(feature = "bundled")]
static MIHOMO_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mihomo"));

fn get_mihomo_path() -> PathBuf {
    #[cfg(feature = "bundled")]
    { extract_bundled_mihomo() }  // 释放到 ~/.cache/ladder/mihomo
    #[cfg(not(feature = "bundled"))]
    { download_or_find_mihomo() }
}
```

---

## 三、项目结构

```
ladder/
├── Cargo.toml
├── build.rs                 # bundled 版本：编译期下载 mihomo 嵌入
├── src/
│   ├── main.rs              # CLI 入口，解析参数
│   ├── config.rs            # 配置管理（多订阅 TOML）
│   ├── mihomo.rs            # Mihomo 进程管理 + 自动下载/嵌入释放
│   ├── api.rs               # Mihomo RESTful API 客户端
│   ├── watchdog.rs          # Shell PID 监控（Linux /proc + macOS kill -0）
│   ├── tui/
│   │   ├── mod.rs           # TUI 入口
│   │   ├── app.rs           # 应用状态机
│   │   └── ui.rs            # 渲染逻辑（Ratatui）
│   └── env.rs               # 生成 env export 输出
├── ladder-env.sh            # POSIX sh，兼容 bash/zsh
├── .github/
│   └── workflows/
│       └── release.yml      # 自动构建 + Release
└── README.md
```

---

## 四、多订阅设计

### 配置文件结构（`~/.config/ladder/config.toml`）

```toml
[ladder]
port = 7890
socks_port = 7891
control_port = 9090

[[subscriptions]]
name = "机场A"
url = "https://sub.airport-a.com/..."
interval = 86400

[[subscriptions]]
name = "机场B"
url = "https://sub.airport-b.com/..."
interval = 43200
```

### 自动生成 Mihomo config.yaml

ladder 启动时根据 `config.toml` 动态生成 Mihomo 配置：

```yaml
proxy-providers:
  机场A:
    type: http
    url: "https://sub.airport-a.com/..."
    interval: 86400
    health-check:
      enable: true
      url: "https://www.gstatic.com/generate_204"
      interval: 300
  机场B:
    type: http
    url: "https://sub.airport-b.com/..."
    interval: 43200
    health-check:
      enable: true
      url: "https://www.gstatic.com/generate_204"
      interval: 300

proxy-groups:
  - name: "PROXY"
    type: select
    use: [机场A, 机场B]   # 所有订阅节点合并
```

---

## 五、TUI 界面（多订阅视图）

```
┌────────────────── Ladder v2 ───────────────────┐
│ ● RUNNING  HTTP:7890  SOCKS:7891               │
├──────────────────────────────────────────────────┤
│ [机场A] [机场B] [全部]                   Tab切换 │
├──────────────────────────────────────────────────┤
│ ▶ [当前] 🇭🇰 Hong Kong 01       23ms  ★ best   │
│   🇯🇵 Japan Tokyo 02            45ms            │
│   🇸🇬 Singapore 03              88ms            │
│   ── 机场B ──────────────────────────           │
│   🇺🇸 US Los Angeles 01        142ms            │
│   🔴 Germany Frankfurt 01    timeout            │
├──────────────────────────────────────────────────┤
│ [Enter]切换  [T]测速  [A]自动最优  [U]更新  [Q]退出 │
└──────────────────────────────────────────────────┘
```

- 实时轮询 Mihomo API 刷新延迟
- `Tab` 在不同订阅分组间切换
- `Enter` 切换节点（PUT `/proxies/PROXY`）
- `t` 触发全量测速（并发 GET delay API）
- `a` **自动选择延迟最低节点**（Auto Best）
- `u` 强制更新订阅 Provider
- `q` 退出 TUI，**代理继续运行**

### 自动选择最优节点（Auto Best）设计

**触发方式：**
- TUI 按 `a` 手动触发一次
- `ladder start --auto-best` CLI flag：启动后自动执行一次测速并选择最优节点

**执行流程：**
```
按 [A] / --auto-best
  1. 并发调用 Mihomo GET /proxies/{name}/delay 对所有可用节点测速
  2. 过滤掉 timeout（delay=0）节点
  3. 取延迟最低的节点（若多个并列取第一个）
  4. PUT /proxies/PROXY {"name": "最优节点名"} 切换
  5. TUI 状态栏显示：✓ 已切换到最优节点 HK-01 (23ms)
```

**注意事项：**
- 测速期间 TUI 节点列表显示 loading 动画，避免阻塞交互
- timeout 节点在列表中标记为 🔴，不参与自动选择
- 与 Mihomo 原生 `url-test` 组不同：这是**用户主动触发**的一次性选择，不会定时自动切换（避免意外中断连接）

---

## 六、生命周期设计（核心）

```
source ladder-env.sh (Shell PID=1234, export LADDER_SHELL_PID=$$)
  └── ladder 进程（前台或后台均可）
        ├── 启动 mihomo 子进程
        └── watchdog tokio task
              └── 每 2s 检查 Shell PID 1234 是否存活
                    └── PID 消失 → SIGTERM mihomo → 退出
```

| 操作 | 代理状态 |
|------|---------|
| `ladder` 启动 | ✅ 运行 |
| TUI 按 `q` 退出 | ✅ **继续运行** |
| 启动时的 Shell 正常 `exit` | ❌ watchdog 检测，停止代理 |
| Shell 被 `kill -9` | ❌ watchdog 检测，停止代理 |

---

## 七、macOS 兼容

**核心问题**：macOS 没有 `/proc` 文件系统，watchdog 的 PID 检查需要跨平台实现。

```rust
// watchdog.rs - 跨平台 PID 存活检测
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{}/status", pid)).exists()
    }
    #[cfg(target_os = "macos")]
    {
        // macOS 用 kill -0 探测：无权限=进程存在，ESRCH=进程不存在
        use nix::sys::signal::kill;
        use nix::unistd::Pid;
        match kill(Pid::from_raw(pid as i32), None) {
            Ok(_) => true,
            Err(nix::errno::Errno::ESRCH) => false,
            Err(_) => true,  // EPERM 等 = 进程存在但无权限
        }
    }
}
```

**Mihomo 自动下载**区分平台（通过 `std::env::consts::{OS, ARCH}` 自动选择）：

| 平台 | Mihomo Release 文件名 |
|------|----------------------|
| Linux x86_64 | `mihomo-linux-amd64-v{ver}.gz` |
| Linux arm64 | `mihomo-linux-arm64-v{ver}.gz` |
| macOS x86_64 | `mihomo-darwin-amd64-v{ver}.gz` |
| macOS arm64 (M系列) | `mihomo-darwin-arm64-v{ver}.gz` |

---

## 八、`ladder-env.sh`（POSIX 兼容）

```sh
#!/bin/sh
# POSIX sh: compatible with bash, zsh, dash
# fish 用户请使用 bass: bass source ladder-env.sh
export LADDER_SHELL_PID=$$
_port=$(cat "${LADDER_HOME:-$HOME/.config/ladder}/port" 2>/dev/null || echo 7890)
_sport=$(cat "${LADDER_HOME:-$HOME/.config/ladder}/socks-port" 2>/dev/null || echo 7891)

export HTTP_PROXY="http://127.0.0.1:$_port"
export HTTPS_PROXY="http://127.0.0.1:$_port"
export ALL_PROXY="socks5://127.0.0.1:$_sport"
export http_proxy="$HTTP_PROXY"
export https_proxy="$HTTPS_PROXY"
export all_proxy="$ALL_PROXY"
export NO_PROXY="localhost,127.0.0.1,::1"
export no_proxy="$NO_PROXY"

echo "✓ proxy → HTTP:$_port  SOCKS5:$_sport  (shell PID: $$)"
```

---

## 九、GitHub Actions 自动构建

### 触发时机
- 推送 `v*` tag（如 `v0.1.0`）时触发构建 + 发布 Release
- PR 时仅构建，不发布

### 构建矩阵

| 目标 | Runner | 编译方式 | 产物 |
|------|--------|---------|------|
| `x86_64-unknown-linux-musl` | `ubuntu-latest` | cross 工具链 | `ladder-linux-amd64` |
| `aarch64-unknown-linux-musl` | `ubuntu-latest` | cross 工具链 | `ladder-linux-arm64` |
| `x86_64-apple-darwin` | `macos-latest` | 原生 cargo | `ladder-macos-amd64` |
| `aarch64-apple-darwin` | `macos-latest` | 原生 cargo | `ladder-macos-arm64` |

> Linux 用 musl 静态编译（零 glibc 依赖），macOS 用系统 SDK 原生编译。

### 产物清单（每次 Release）

| 文件 | 说明 |
|------|------|
| `ladder-linux-amd64` | Linux x86_64，musl 静态 |
| `ladder-linux-arm64` | Linux arm64，musl 静态 |
| `ladder-macos-amd64` | macOS Intel |
| `ladder-macos-arm64` | macOS Apple Silicon |
| `ladder-bundled-linux-amd64` | 同上，内置 mihomo |
| `ladder-bundled-linux-arm64` | 同上，内置 mihomo |
| `ladder-bundled-macos-amd64` | 同上，内置 mihomo |
| `ladder-bundled-macos-arm64` | 同上，内置 mihomo |
| `ladder-env.sh` | 环境变量脚本（POSIX sh） |
| `checksums.txt` | SHA256 校验 |

### workflow 逻辑概览

```
push v* tag
  ├── job: build-external (4个平台)
  │     cargo build --release
  │     Linux: cross + musl target
  │     macOS: native cargo
  │
  ├── job: build-bundled (4个平台)
  │     build.rs 下载对应平台 mihomo bin
  │     cargo build --release --features bundled
  │
  └── job: release
        needs: [build-external, build-bundled]
        收集全部 8 个 artifacts
        生成 checksums.txt
        创建 GitHub Release 并上传所有文件
```

---

## 十、Rust 技术栈

| 依赖 | 用途 |
|------|------|
| `ratatui` | TUI 渲染框架（配合 crossterm 后端） |
| `tokio` | 异步运行时（watchdog + API 轮询） |
| `reqwest` | HTTP 客户端（调 Mihomo RESTful API + 下载 mihomo） |
| `serde / serde_json` | JSON 序列化（解析 API 响应） |
| `serde_yaml` | YAML 生成（生成 Mihomo config.yaml） |
| `toml` | 配置文件解析 |
| `clap` | CLI 参数解析（subcommand 风格） |
| `directories` | XDG 路径（`~/.config/ladder/`） |
| `nix` | Unix syscall（进程信号、PID 检查） |

---

## 十一、CLI 子命令

```
ladder                        # 若代理已运行则开 TUI，否则先启动代理再开 TUI
ladder start                  # 仅启动代理（不开 TUI），使用配置文件中第一个订阅
ladder start -s <NAME>        # 仅启动代理，指定订阅（按名称）
ladder start -s <NAME> --env  # 启动代理，并输出 shell export 语句以应用代理
ladder stop                   # 手动停止代理（不清除环境变量）
ladder stop --env             # 停止代理，并输出 shell unset 语句以清除代理环境变量
ladder env                    # 输出当前 export 语句（代理已运行时）
ladder unenv                  # 输出 unset 语句，仅清除环境变量，不停止代理
ladder tui                    # 仅开 TUI（连接已运行的代理）
ladder sub add <URL>          # 添加订阅
ladder sub list               # 列出所有订阅
ladder sub remove <NAME>      # 删除订阅
ladder update                 # 强制更新所有订阅
ladder status                 # 打印当前状态（运行中/已停止、当前节点、端口）
```

### 纯 CLI 启动代理并应用代理环境变量

无 TUI 场景（如脚本、CI 环境）下，用户希望一条命令完成：启动代理 + 当前 Shell 应用代理。

**设计方案：`--env` flag + eval 模式**

```sh
# 方式一：eval 模式（推荐，一行搞定）
eval "$(ladder start -s 机场A --env)"

# 方式二：source 模式（等价）
ladder start -s 机场A --env > /tmp/ladder-env.sh && source /tmp/ladder-env.sh
```

`ladder start --env` 输出内容（POSIX sh，eval 安全）：

```sh
export LADDER_SHELL_PID=$$
export HTTP_PROXY="http://127.0.0.1:7890"
export HTTPS_PROXY="http://127.0.0.1:7890"
export ALL_PROXY="socks5://127.0.0.1:7891"
export http_proxy="http://127.0.0.1:7890"
export https_proxy="http://127.0.0.1:7890"
export all_proxy="socks5://127.0.0.1:7891"
export NO_PROXY="localhost,127.0.0.1,::1"
export no_proxy="localhost,127.0.0.1,::1"
# ladder: proxy started (PID 12345), sub=机场A, HTTP=7890 SOCKS5=7891
```

> `$$` 在 eval 展开时是当前 Shell 的 PID，watchdog 因此锚定到正确的 Shell。

### 清除代理环境变量（unenv）

与 `--env` 对称，提供两种清除方式：

**方式一：停止代理同时清除环境变量**
```sh
eval "$(ladder stop --env)"
```

**方式二：仅清除环境变量，不停止代理（保留代理进程）**
```sh
eval "$(ladder unenv)"
```

`ladder unenv` / `ladder stop --env` 输出内容：
```sh
unset HTTP_PROXY HTTPS_PROXY ALL_PROXY
unset http_proxy https_proxy all_proxy
unset NO_PROXY no_proxy
# ladder: proxy env cleared
```

**推荐使用方式（与 1.0 体验对齐）：**
```sh
# 开启
eval "$(ladder start -s 机场A --env)"

# 关闭（停代理 + 清变量）
eval "$(ladder stop --env)"

# 仅清除变量（代理保持后台运行，其他终端仍可使用）
eval "$(ladder unenv)"
```

> **设计说明**：`ladder stop` 只停止代理进程，**不**自动清除环境变量（因为无法操作父 Shell 的变量），清除必须通过 `eval` + unset 输出来完成，这与 1.0 的 `unset_proxy` 思路一致。

| 命令 | 行为 |
|------|------|
| `ladder start` | 使用配置中所有订阅（全量节点） |
| `ladder start -s 机场A` | 仅加载"机场A"的订阅，生成精简 mihomo 配置 |
| `ladder start -s 机场A -s 机场B` | 加载多个指定订阅 |

---

## 十二、实施阶段

| 阶段 | 内容 |
|------|------|
| **P1** | 项目骨架 + `config.rs`（多订阅 TOML 读写） |
| **P2** | `mihomo.rs`：自动下载（外置）+ `build.rs`（内置） |
| **P3** | `watchdog.rs`：Linux `/proc` + macOS `kill -0` 双实现 |
| **P4** | `api.rs`：Mihomo RESTful 客户端（节点列表/切换/测速/更新订阅） |
| **P5** | TUI：节点列表（多订阅分组 Tab）+ 切换 + 测速 |
| **P6** | CLI 子命令（`start/stop/status/sub` 等） |
| **P7** | `release.yml` GitHub Actions + `ladder-env.sh` + README |
