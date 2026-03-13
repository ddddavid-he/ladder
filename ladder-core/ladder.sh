#!/bin/sh
# ladder.sh - Shell wrapper for ladder-core
# Usage: source ladder.sh
# Supports: bash, zsh, dash (POSIX sh)
# fish users: use `bass source ladder.sh` or configure manually

# ─── Locate ladder-core binary ───────────────────────────────────────────────

_ladder_core_bin() {
    # 1. Same directory as this script
    if [ -n "${BASH_SOURCE[0]}" ]; then
        _ld_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
    elif [ -n "${0}" ] && [ "${0}" != "sh" ] && [ "${0}" != "bash" ] && [ "${0}" != "zsh" ]; then
        _ld_dir="$(cd "$(dirname "${0}")" 2>/dev/null && pwd)"
    else
        _ld_dir=""
    fi

    if [ -n "$_ld_dir" ] && [ -x "$_ld_dir/ladder-core" ]; then
        echo "$_ld_dir/ladder-core"
        return 0
    fi

    # 2. PATH
    if command -v ladder-core >/dev/null 2>&1; then
        echo "ladder-core"
        return 0
    fi

    # 3. ~/.config/ladder/
    if [ -x "$HOME/.config/ladder/ladder-core" ]; then
        echo "$HOME/.config/ladder/ladder-core"
        return 0
    fi

    echo "Error: ladder-core not found. Please install it to PATH or ~/.config/ladder/" >&2
    return 1
}

# ─── Main ladder function ─────────────────────────────────────────────────────

ladder() {
    _lc_bin=$(_ladder_core_bin) || return 1

    case "$1" in
        start)
            # Check if --env flag is present
            _has_env=0
            for _arg in "$@"; do
                [ "$_arg" = "--env" ] && _has_env=1 && break
            done

            if [ "$_has_env" = "1" ]; then
                # Capture export statements and eval them into current shell
                # Pass --shell-pid $$ so watchdog monitors this shell
                eval "$("$_lc_bin" "$@" --shell-pid $$)"
            else
                "$_lc_bin" "$@"
            fi
            ;;

        stop)
            _has_env=0
            for _arg in "$@"; do
                [ "$_arg" = "--env" ] && _has_env=1 && break
            done

            if [ "$_has_env" = "1" ]; then
                eval "$("$_lc_bin" "$@")"
            else
                "$_lc_bin" "$@"
            fi
            ;;

        unenv)
            # Only clear env vars, do not stop proxy
            eval "$("$_lc_bin" unenv)"
            ;;

        env)
            # Apply current proxy env vars to this shell
            eval "$("$_lc_bin" env)"
            ;;

        *)
            # tui, status, sub, update, install, etc. - pass through
            "$_lc_bin" "$@"
            ;;
    esac
}

# ─── Export ladder function (bash only) ──────────────────────────────────────
[ -n "$BASH_VERSION" ] && export -f ladder 2>/dev/null || true
