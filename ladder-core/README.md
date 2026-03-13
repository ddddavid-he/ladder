# Ladder v2

> A lightweight proxy manager wrapping [Mihomo (clash-meta)](https://github.com/MetaCubeX/mihomo)

[![Build & Release](https://github.com/ddddavid/ladder/actions/workflows/release.yml/badge.svg)](https://github.com/ddddavid/ladder/actions/workflows/release.yml)

---

## Features

- 🚀 **Multi-subscription** — manage multiple airport subscriptions simultaneously
- 🖥️ **TUI** — interactive node switcher with speed test & auto-best selection
- 🐚 **Shell integration** — `source ladder.sh` transparently sets/unsets proxy env vars in current shell
- 📦 **Two release flavors** — external (small) or bundled (embeds mihomo, zero dependencies)
- 🍎 **macOS + Linux** — full support for both platforms (Apple Silicon included)
- 🔒 **Watchdog** — automatically stops proxy when the parent shell exits

---

## Quick Start

### 1. Download

From [Releases](https://github.com/ddddavid/ladder/releases), download:
- `ladder-core-linux-amd64` (or your platform)
- `ladder.sh`

```sh
# Example: Linux amd64
curl -L https://github.com/ddddavid/ladder/releases/latest/download/ladder-core-linux-amd64 -o ladder-core
curl -L https://github.com/ddddavid/ladder/releases/latest/download/ladder.sh -o ladder.sh
chmod +x ladder-core

# Place both in same directory, e.g. ~/.local/bin/
mv ladder-core ~/.local/bin/
mv ladder.sh ~/.local/bin/
```

### 2. Install shell integration

```sh
# Auto-install: injects `source ~/.config/ladder/ladder.sh` into ~/.zshrc or ~/.bashrc
ladder-core install

# Or manually:
echo 'source ~/.local/bin/ladder.sh' >> ~/.zshrc
source ~/.zshrc
```

### 3. Add subscriptions

```sh
ladder sub add "https://your-airport-url.com/sub?token=xxx" --name 机场A
ladder sub add "https://another-airport.com/sub?token=yyy" --name 机场B
ladder sub list
```

### 4. Start proxy

```sh
# Start proxy + set env vars in current shell
ladder start --env

# Start with specific subscription only
ladder start -s 机场A --env

# Start multiple subscriptions
ladder start -s 机场A -s 机场B --env
```

### 5. Use TUI

```sh
ladder tui
```

```
┌────────────────── Ladder v2 ───────────────────┐
│ ● RUNNING  HTTP:7890  SOCKS:7891               │
├──────────────────────────────────────────────────┤
│ [全部] [机场A] [机场B]                   Tab切换 │
├──────────────────────────────────────────────────┤
│ ▶ 🇭🇰 Hong Kong 01              23ms  ← current │
│   🇯🇵 Japan Tokyo 02             45ms            │
│   🇸🇬 Singapore 03               88ms            │
│   🇺🇸 US Los Angeles 01         142ms            │
│   🔴 Germany Frankfurt 01     timeout            │
├──────────────────────────────────────────────────┤
│ [↑↓]移动 [Enter]切换 [Tab]分组 [T]测速 [A]自动最优 [U]更新 [Q]退出 │
└──────────────────────────────────────────────────┘
```

### 6. Stop proxy

```sh
# Stop proxy + clear env vars
ladder stop --env

# Only clear env vars (proxy keeps running)
ladder unenv

# Only set env vars (proxy already running)
ladder env
```

---

## CLI Reference

| Command | Description |
|---------|-------------|
| `ladder start` | Start proxy (all subscriptions) |
| `ladder start --env` | Start proxy + set shell env vars |
| `ladder start -s NAME` | Start with specific subscription |
| `ladder start --auto-best` | Start and auto-select lowest latency node |
| `ladder stop` | Stop proxy |
| `ladder stop --env` | Stop proxy + unset shell env vars |
| `ladder env` | Apply proxy env vars to current shell |
| `ladder unenv` | Unset proxy env vars (proxy keeps running) |
| `ladder tui` | Open interactive TUI |
| `ladder status` | Show proxy status |
| `ladder sub add URL` | Add a subscription |
| `ladder sub list` | List all subscriptions |
| `ladder sub remove NAME` | Remove a subscription |
| `ladder update` | Force update all subscription providers |
| `ladder install` | Install shell integration |

---

## Configuration

Config file: `~/.config/ladder/config.toml`

```toml
[ladder]
port = 7890          # HTTP proxy port
socks_port = 7891    # SOCKS5 proxy port
control_port = 9090  # Mihomo REST API port
# api_secret = "your-secret"  # optional
# mihomo_bin = "/usr/local/bin/mihomo"  # optional, auto-download if not set

[[subscriptions]]
name = "机场A"
url = "https://sub.airport-a.com/..."
interval = 86400  # auto-update interval in seconds

[[subscriptions]]
name = "机场B"
url = "https://sub.airport-b.com/..."
interval = 43200
```

---

## Release Flavors

| Binary | Size | Mihomo | Notes |
|--------|------|--------|-------|
| `ladder-core-*` | ~5MB | auto-download on first run | Recommended |
| `ladder-core-bundled-*` | ~20MB | embedded at compile time | Offline / air-gapped |

---

## How Shell Integration Works

`ladder.sh` provides a `ladder` shell function (POSIX sh, works with bash/zsh/dash).

When you call commands with `--env`, the function runs `eval "$(ladder-core ...)"`, 
which evaluates the exported environment variable statements directly in the current shell — 
without needing to restart the shell.

```sh
# What ladder start --env actually does internally:
eval "$(ladder-core start --env --shell-pid $$)"
# Which sets: HTTP_PROXY, HTTPS_PROXY, ALL_PROXY, http_proxy, https_proxy, all_proxy, NO_PROXY, no_proxy
```

---

## Watchdog

When started with `--env` (which passes `--shell-pid $$`), ladder-core monitors the parent shell PID:

- **Linux**: checks `/proc/{pid}/status`
- **macOS**: uses `kill(pid, 0)` (POSIX signal 0)

If the shell exits (normally or via kill), the proxy is automatically stopped within 2 seconds.

---

## Build from Source

```sh
git clone https://github.com/ddddavid/ladder.git
cd ladder/ladder-core

# External version
cargo build --release
# Binary: target/release/ladder-core

# Bundled version (downloads mihomo at compile time)
cargo build --release --features bundled
# Binary: target/release/ladder-core
```

---

## License

MIT
