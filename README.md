# Ladder v2

[English](README-en.md)

> 一个基于 [Mihomo (clash-meta)](https://github.com/MetaCubeX/mihomo) 的轻量级代理管理器

[![Build & Release](https://github.com/ddddavid/ladder/actions/workflows/release.yml/badge.svg)](https://github.com/ddddavid/ladder/actions/workflows/release.yml)

---

## 项目简介

`Ladder` 是一个面向 macOS 和 Linux 的**会话级代理管理工具**。
它在 `mihomo` 之上提供了更友好的 CLI、TUI、多订阅管理，以及 Shell 环境变量接入能力。

当前的 v2 主实现位于 `ladder-core/`。
旧版纯脚本实现保存在 `1.0/` 目录中。

## 特性

- 🚀 **多订阅支持**：可同时管理多个机场订阅
- 🖥️ **TUI 交互界面**：支持节点切换、测速、自动选择最低延迟节点
- 🐚 **Shell 集成**：通过 `source env.sh` 在当前 Shell 中透明设置/清理代理环境变量
- 📦 **两种发布形态**：外置版体积更小；`bundled` 版内置 `mihomo`，零外部依赖
- 🍎 **支持 macOS + Linux**：包含 Apple Silicon
- 🔒 **Watchdog**：父 Shell 退出后自动停止代理

---

## 快速开始

### 1. 下载

从 [Releases](https://github.com/ddddavid/ladder/releases) 下载适合你平台的二进制：

- `ladder-core-linux-amd64`
- `ladder-core-linux-arm64`
- `ladder-core-macos-amd64`
- `ladder-core-macos-arm64`
- 或对应的 `ladder-core-bundled-*` 版本

```sh
# 以 Linux amd64 为例
curl -L https://github.com/ddddavid/ladder/releases/latest/download/ladder-core-linux-amd64 -o ladder-core
chmod +x ladder-core
mv ladder-core ~/.local/bin/
```

### 2. 安装 Shell 集成

`env.sh` 由二进制在安装时自动生成，所以不需要单独下载脚本文件。

```sh
# 写入 ~/.config/ladder/env.sh，并输出你需要添加到 shell rc 的命令
ladder-core install

# 然后手动加入到 shell 配置文件
# zsh 示例：
echo 'source ~/.config/ladder/env.sh' >> ~/.zshrc
source ~/.zshrc
```

### 3. 添加订阅

```sh
ladder sub add "https://your-airport-url.com/sub?token=xxx" --name 机场A
ladder sub add "https://another-airport.com/sub?token=yyy" --name 机场B
ladder sub list
```

### 4. 启动代理

```sh
# 启动代理，并在当前 shell 中设置环境变量
ladder start --env

# 只使用指定订阅启动
ladder start -s 机场A --env

# 同时使用多个订阅启动
ladder start -s 机场A -s 机场B --env
```

### 5. 使用 TUI

```sh
ladder tui
```

### 6. 停止代理

```sh
# 停止代理并清理环境变量
ladder stop --env

# 只清理环境变量，代理继续运行
ladder unenv

# 代理已运行时，仅重新应用环境变量
ladder env
```

---

## CLI 参考

| 命令 | 说明 |
|------|------|
| `ladder start` | 使用全部订阅启动代理 |
| `ladder start --env` | 启动代理并在当前 Shell 应用环境变量 |
| `ladder start -s NAME` | 使用指定订阅启动 |
| `ladder start --auto-best` | 启动后自动切换到最低延迟节点 |
| `ladder stop` | 停止代理 |
| `ladder stop --env` | 停止代理并清理环境变量 |
| `ladder env` | 在当前 Shell 应用代理环境变量 |
| `ladder unenv` | 清理环境变量但不停止代理 |
| `ladder tui` | 打开交互式 TUI |
| `ladder status` | 查看当前代理状态 |
| `ladder sub add URL` | 添加订阅 |
| `ladder sub list` | 列出订阅 |
| `ladder sub remove NAME` | 删除订阅 |
| `ladder update` | 强制更新订阅 |
| `ladder install` | 安装 Shell 集成 |

---

## 配置文件

配置文件路径：`~/.config/ladder/config.toml`

```toml
[ladder]
port = 7890
socks_port = 7891
control_port = 9090
# api_secret = "your-secret"
# mihomo_bin = "/usr/local/bin/mihomo"

[[subscriptions]]
name = "机场A"
url = "https://sub.airport-a.com/..."
interval = 86400

[[subscriptions]]
name = "机场B"
url = "https://sub.airport-b.com/..."
interval = 43200
```

---

## 发布版本说明

| 二进制 | 体积 | Mihomo | 说明 |
|--------|------|--------|------|
| `ladder-core-*` | ~5MB | 首次运行自动下载 | 推荐 |
| `ladder-core-bundled-*` | ~20MB | 编译时内置 | 适合离线或隔离环境 |

---

## Shell 集成原理

`env.sh` 会提供一个 `ladder` Shell 函数，适用于 `bash`、`zsh`、`dash` 等 POSIX 风格 Shell。

当你使用带 `--env` 的命令时，这个函数会执行 `eval "$(ladder-core ...)"`，从而把 `ladder-core` 输出的环境变量语句直接作用到当前 Shell。

```sh
# ladder start --env 的内部行为
eval "$(ladder-core start --env --shell-pid $$)"
# 会设置：HTTP_PROXY, HTTPS_PROXY, ALL_PROXY, http_proxy, https_proxy, all_proxy, NO_PROXY, no_proxy
```

---

## Watchdog

当通过 `--env` 启动时（内部会传入 `--shell-pid $$`），`ladder-core` 会持续监控父 Shell：

- **Linux**：检查 `/proc/{pid}/status`
- **macOS**：使用 `kill(pid, 0)`

一旦父 Shell 退出，代理会在大约 2 秒内自动停止。

---

## 从源码构建

```sh
git clone https://github.com/ddddavid/ladder.git
cd ladder
cd ladder-core

# 外置版
cargo build --release

# bundled 版（编译时下载并内置 mihomo）
cargo build --release --features bundled
```

---

## License

MIT
