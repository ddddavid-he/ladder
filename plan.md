# ladder v2 · Rust 重写方案 Plan（最终版）

## 一、新增需求确认

| 需求 | 方案 |
|------|------|
| 多订阅支持 | 配置文件支持多个 `proxy-provider`，TUI 可分组显示 |
| zsh / bash 兼容 | `ladder.sh` 用 POSIX sh 语法，`source` 后提供 `ladder` shell 函数 |
| 发布两种 bin | `ladder-core-bundled`（内置 mihomo）+ `ladder-core`（外置 mihomo） |
| 环境变量透传 | Rust bin 改名为 `ladder-core`；`ladder.sh` 封装为 `ladder` 函数，自动 eval 处理透传 |

---

## 二、两种发布版本

### `ladder-core`（外置版，推荐）
- Rust 编译产物，体积小（~5MB）
- 首次运行自动从 mihomo GitHub Release 下载对应平台版本到 `~/.config/ladder/mihomo`
- 用户也可手动指定 mihomo 路径：`ladder-core --mihomo-bin /usr/local/bin/mihomo`

### `ladder-core-bundled`（内置版）
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
├── Cargo.toml               # binary name: ladder-core
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
│   └── env.rs               # 生成 env export / unset 输出
├── ladder.sh                # Shell 封装脚本：source 后提供 ladder 函数（透传环境变量）
├── .github/
│   └── workflows/
│       └── release.yml      # 自动构建 + Release
└── README.md
```

**Cargo.toml 关键配置：**
```toml
[[bin]]
name = "ladder-core"
path = "src/main.rs"
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
- `q` 退出 TUI，**代理继续运行**，**不操作环境变量**

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
source ladder.sh  →  ladder start -s 机场A
  (Shell PID=1234, export LADDER_SHELL_PID=$$)
  └── ladder-core 进程（后台运行）
        ├── 启动 mihomo 子进程
        └── watchdog tokio task
              └── 每 2s 检查 Shell PID 1234 是否存活
                    └── PID 消失 → SIGTERM mihomo → 退出
```

| 操作 | 代理状态 |
|------|---------|
| `ladder start` 启动 | ✅ 运行 |
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

## 八、`ladder.sh`（Shell 封装脚本）

### 设计原则

- Rust bin 名为 `ladder-core`，**不直接暴露给用户**
- `ladder.sh` 提供 `ladder` shell 函数，用户 `source ladder.sh` 后直接使用 `ladder` 命令
- 凡是涉及环境变量的操作（`start --env`、`stop --env`、`unenv`）由 shell 函数自动 `eval`，实现透传
- 其余子命令（`tui`、`status`、`sub`、`update` 等）直接透传给 `ladder-core`

### 安装方式

```sh
# 手动：下载后 source
source /path/to/ladder.sh

# 推荐：写入 ~/.zshrc / ~/.bashrc（ladder install 自动完成）
echo 'source ~/.config/ladder/ladder.sh' >> ~/.zshrc
```

### `ladder.sh` 完整内容

```sh
#!/bin/sh
# ladder.sh - Shell wrapper for ladder-core
# Usage: source ladder.sh
# Supports: bash, zsh, dash (POSIX sh)
# fish users: use `bass source ladder.sh`

# 找到 ladder-core 的位置（同目录 > PATH）
_ladder_core_bin() {
    _dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd)"
    if [ -x "$_dir/ladder-core" ]; then
        echo "$_dir/ladder-core"
    elif command -v ladder-core >/dev/null 2>&1; then
        echo "ladder-core"
    else
        echo "Error: ladder-core not found. Please install it first." >&2
        return 1
    fi
}

ladder() {
    _core=$(_ladder_core_bin) || return 1

    case "$1" in
        start)
            # 含 --env flag：启动代理并透传环境变量到当前 Shell
            if echo "$@" | grep -q -- '--env'; then
                export LADDER_SHELL_PID=$$
                eval "$("$_core" "$@" --shell-pid $$)"
            else
                "$_core" "$@"
            fi
            ;;
        stop)
            # 含 --env flag：停止代理并清除当前 Shell 环境变量
            if echo "$@" | grep -q -- '--env'; then
                eval "$("$_core" "$@")"
            else
                "$_core" "$@"
            fi
            ;;
        unenv)
            # 仅清除环境变量，不停代理
            eval "$("$_core" unenv)"
            ;;
        env)
            # 输出当前 export 语句（用户主动调用时也可直接 eval）
            eval "$("$_core" env)"
            ;;
        *)
            # tui / status / sub / update 等：直接透传
            "$_core" "$@"
            ;;
    esac
}

# 使 ladder 函数在子 shell 中可用（bash only）
[ -n "$BASH_VERSION" ] && export -f ladder 2>/dev/null || true
```

### 用户使用体验

```sh
# 启动代理，当前 Shell 立即生效
ladder start -s 机场A

# 启动代理并设置环境变量（内部自动 eval，无需手写）
ladder start -s 机场A --env

# 关闭代理 + 清除环境变量
ladder stop --env

# 仅清除当前 Shell 的环境变量（代理继续运行）
ladder unenv

# 开启 TUI（直接透传给 ladder-core）
ladder tui

# 查看状态
ladder status
```

> **关键点**：用户无需手写 `eval`，`ladder.sh` 的 shell 函数自动处理所有需要透传的场景。

---

## 九、GitHub Actions 自动构建

### 触发时机
- 推送 `v*` tag（如 `v0.1.0`）时触发构建 + 发布 Release
- PR 时仅构建，不发布

### 构建矩阵

| 目标 | Runner | 编译方式 | 产物 |
|------|--------|---------|------|
| `x86_64-unknown-linux-musl` | `ubuntu-latest` | cross 工具链 | `ladder-core-linux-amd64` |
| `aarch64-unknown-linux-musl` | `ubuntu-latest` | cross 工具链 | `ladder-core-linux-arm64` |
| `x86_64-apple-darwin` | `macos-latest` | 原生 cargo | `ladder-core-macos-amd64` |
| `aarch64-apple-darwin` | `macos-latest` | 原生 cargo | `ladder-core-macos-arm64` |

> Linux 用 musl 静态编译（零 glibc 依赖），macOS 用系统 SDK 原生编译。

### 产物清单（每次 Release）

| 文件 | 说明 |
|------|------|
| `ladder-core-linux-amd64` | Linux x86_64，musl 静态，外置 mihomo |
| `ladder-core-linux-arm64` | Linux arm64，musl 静态，外置 mihomo |
| `ladder-core-macos-amd64` | macOS Intel，外置 mihomo |
| `ladder-core-macos-arm64` | macOS Apple Silicon，外置 mihomo |
| `ladder-core-bundled-linux-amd64` | 同上，内置 mihomo |
| `ladder-core-bundled-linux-arm64` | 同上，内置 mihomo |
| `ladder-core-bundled-macos-amd64` | 同上，内置 mihomo |
| `ladder-core-bundled-macos-arm64` | 同上，内置 mihomo |
| `ladder.sh` | Shell 封装脚本（POSIX sh，source 后提供 ladder 命令） |
| `checksums.txt` | SHA256 校验 |

### workflow 逻辑概览

```
push v* tag
  ├── job: build-external (4个平台)
  │     cargo build --release  (binary: ladder-core)
  │     Linux: cross + musl target
  │     macOS: native cargo
  │
  ├── job: build-bundled (4个平台)
  │     build.rs 下载对应平台 mihomo bin
  │     cargo build --release --features bundled  (binary: ladder-core-bundled)
  │
  └── job: release
        needs: [build-external, build-bundled]
        收集全部 8 个 artifacts + ladder.sh
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

## 十一、CLI 子命令（`ladder-core` 原始接口）

> 用户通过 `ladder.sh` 封装的 `ladder` 函数调用，底层实际执行 `ladder-core`。

```
ladder-core                           # 若代理已运行则开 TUI，否则先启动代理再开 TUI
ladder-core start                     # 仅启动代理（不开 TUI），使用所有订阅
ladder-core start -s <NAME>           # 仅启动代理，指定订阅（按名称）
ladder-core start -s <NAME> --env     # 启动代理，stdout 输出 shell export 语句
ladder-core start --shell-pid <PID>   # 启动时绑定 Shell PID（由 ladder.sh 自动传入）
ladder-core stop                      # 手动停止代理
ladder-core stop --env                # 停止代理，stdout 输出 shell unset 语句
ladder-core env                       # stdout 输出当前 export 语句（代理运行时）
ladder-core unenv                     # stdout 输出 unset 语句
ladder-core tui                       # 仅开 TUI（连接已运行的代理）
ladder-core sub add <URL>             # 添加订阅
ladder-core sub list                  # 列出所有订阅
ladder-core sub remove <NAME>         # 删除订阅
ladder-core update                    # 强制更新所有订阅
ladder-core status                    # 打印当前状态（运行中/已停止、当前节点、端口）
ladder-core install                   # 安装：将 ladder.sh 写入 ~/.config/ladder/，并注入 ~/.zshrc / ~/.bashrc
```

### `--env` 输出格式（ladder-core stdout）

```sh
export LADDER_SHELL_PID=1234
export HTTP_PROXY="http://127.0.0.1:7890"
export HTTPS_PROXY="http://127.0.0.1:7890"
export ALL_PROXY="socks5://127.0.0.1:7891"
export http_proxy="http://127.0.0.1:7890"
export https_proxy="http://127.0.0.1:7890"
export all_proxy="socks5://127.0.0.1:7891"
export NO_PROXY="localhost,127.0.0.1,::1"
export no_proxy="localhost,127.0.0.1,::1"
```

### `unenv` 输出格式（ladder-core stdout）

```sh
unset HTTP_PROXY HTTPS_PROXY ALL_PROXY
unset http_proxy https_proxy all_proxy
unset NO_PROXY no_proxy LADDER_SHELL_PID
```

### `-s` 订阅过滤说明

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
| **P5** | TUI：节点列表（多订阅分组 Tab）+ 切换 + 测速 + Auto Best |
| **P6** | CLI 子命令（`start/stop/status/sub/install` 等）+ `env.rs` 输出 |
| **P7** | `ladder.sh` 封装脚本 + `release.yml` GitHub Actions + README |
