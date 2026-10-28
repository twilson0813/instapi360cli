#!/usr/bin/env bash
#
# insta360cli-ui — a small interactive menu over the `insta360cli` CLI.
#
# Drives the common workflows without memorizing flags: check who you are,
# list your cloud media, filter by a shoot-date range, estimate the download
# size, and download (with resume) to a folder or a mounted Samba share.
#
# Usage:  ./insta360cli-ui.sh
#
# It locates the `insta360cli` binary from PATH, or falls back to common spots
# (next to this script, ~/insta360cli, ~/.cargo/bin, …). The older `instapi360`
# name is still accepted. Set INSTA360CLI_BIN to override.

set -u

# ---- small helpers -----------------------------------------------------------
bold()  { printf "\033[1m%s\033[0m\n" "$*"; }
dim()   { printf "\033[2m%s\033[0m\n" "$*"; }
err()   { printf "\033[31m%s\033[0m\n" "$*" >&2; }
pause() { read -r -p "Press Enter to continue… " _; }

# ---- locate the binary -------------------------------------------------------
# An executable *file* (not a directory that merely has the search bit set —
# a project folder sharing the binary's name would otherwise be picked).
is_exe() { [[ -f "$1" && -x "$1" ]]; }

# Directory this script lives in (so a binary sitting next to the UI is found).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" >/dev/null 2>&1 && pwd)"

find_bin() {
    # 1) explicit override wins (new var, then legacy).
    local override="${INSTA360CLI_BIN:-${INSTAPI360_BIN:-}}"
    if [[ -n "$override" ]] && is_exe "$override"; then
        echo "$override"; return 0
    fi
    # 2) on PATH — current name, then legacy.
    local name
    for name in insta360cli instapi360; do
        if command -v "$name" >/dev/null 2>&1; then
            command -v "$name"; return 0
        fi
    done
    # 3) common standalone locations, for each name, in priority order.
    local base cand
    for name in insta360cli instapi360; do
        for base in \
            "$SCRIPT_DIR" \
            "$PWD" \
            "$HOME/.cargo/bin" \
            "$HOME/$name" \
            "$HOME/$name/target/release" \
            "$HOME/bin"; do
            cand="$base/$name"
            is_exe "$cand" && { echo "$cand"; return 0; }
        done
    done
    return 1
}

BIN="$(find_bin)" || {
    err "Could not find the 'insta360cli' binary."
    echo "Looked on PATH and in: this script's folder, \$PWD, ~/.cargo/bin," >&2
    echo "~/insta360cli[/target/release], ~/bin (and the legacy 'instapi360' name)." >&2
    echo >&2
    echo "Fix: point the UI at your standalone binary, e.g.:" >&2
    echo "    INSTA360CLI_BIN=\$HOME/insta360cli ./insta360cli-ui" >&2
    echo "or drop the binary next to this script and name it 'insta360cli'." >&2
    exit 1
}

# Current date-range selection (empty = all).
SINCE=""
UNTIL=""

# Read the last-used output dir the CLI saved (so it isn't re-entered). The CLI
# stores it in config.json under its OS config dir.
saved_output_dir() {
    local cfg
    for cfg in \
        "$HOME/Library/Application Support/com.instapi360.instapi360/config.json" \
        "$HOME/.config/instapi360/config.json"; do
        if [[ -f "$cfg" ]]; then
            # extract "output_dir": "…"  (no jq dependency)
            sed -n 's/.*"output_dir"[[:space:]]*:[[:space:]]*"\(.*\)".*/\1/p' "$cfg" | head -1
            return
        fi
    done
}

# Default output directory: env var > saved config > ~/Insta360.
OUTDIR="${INSTA360CLI_OUT:-${INSTAPI360_OUT:-$(saved_output_dir)}}"
[[ -z "$OUTDIR" ]] && OUTDIR="$HOME/Insta360"

range_label() {
    if [[ -z "$SINCE" && -z "$UNTIL" ]]; then echo "all dates"
    elif [[ -n "$SINCE" && -n "$UNTIL" ]]; then echo "$SINCE … $UNTIL"
    elif [[ -n "$SINCE" ]]; then echo "since $SINCE"
    else echo "until $UNTIL"; fi
}

# Build the --since/--until args array from the current selection.
range_args() {
    local -a a=()
    [[ -n "$SINCE" ]] && a+=(--since "$SINCE")
    [[ -n "$UNTIL" ]] && a+=(--until "$UNTIL")
    printf '%s\n' "${a[@]}"
}

# ---- actions -----------------------------------------------------------------
do_whoami() {
    bold "Account"
    "$BIN" whoami || err "whoami failed — is a token imported? (option 7)"
    echo; pause
}

do_list() {
    bold "Cloud media ($(range_label))"
    # shellcheck disable=SC2046
    "$BIN" list --all $(range_args) || err "list failed"
    echo; pause
}

set_range() {
    bold "Set shoot-date range"
    dim  "Formats: YYYY-MM-DD, or relative age like 7d / 12h / 3w. Blank = open."
    read -r -p "Since (start, blank = none): " s
    read -r -p "Until (end,   blank = none): " u
    SINCE="$s"; UNTIL="$u"
    echo "Range set to: $(range_label)"; echo; pause
}

set_outdir() {
    bold "Set output directory"
    dim  "Point this at a local folder or a mounted Samba share, e.g. /Volumes/Share/Insta360"
    read -r -p "Output dir [$OUTDIR]: " d
    [[ -n "$d" ]] && OUTDIR="$d"
    if mount | grep -qi "$(dirname "$OUTDIR")"; then
        dim "(looks like it may be on a mounted volume — good for Samba)"
    fi
    echo "Output dir: $OUTDIR"; echo; pause
}

do_dry_run() {
    bold "Estimate download ($(range_label))"
    dim  "Resolving sizes… this queries each matching item."
    # shellcheck disable=SC2046
    "$BIN" download all --out "$OUTDIR" $(range_args) --dry-run || err "dry-run failed"
    echo; pause
}

do_download() {
    bold "Download ($(range_label)) → $OUTDIR"
    echo "This downloads every matching item (originals + proxies), resuming partials."
    read -r -p "Proceed? [y/N] " yn
    case "$yn" in
        [yY]*) ;;
        *) echo "Cancelled."; echo; pause; return;;
    esac
    mkdir -p "$OUTDIR" || { err "cannot create $OUTDIR"; pause; return; }
    # shellcheck disable=SC2046
    "$BIN" download all --out "$OUTDIR" $(range_args) --resume \
        && bold "Done → $OUTDIR" \
        || err "download ended with an error (re-run to resume)"
    echo; pause
}

import_token() {
    bold "Import session token"
    echo "How to get it:"
    dim  "  1. Sign in at https://cloud.insta360.com in your browser"
    dim  "  2. DevTools (F12) → Application → Cookies → cloud.insta360.com"
    dim  "  3. Copy the value of the 'insta_access_token' cookie"
    echo
    read -r -p "Paste token (blank to cancel): " tok
    [[ -z "$tok" ]] && { echo "Cancelled."; echo; pause; return; }
    if "$BIN" import-token "$tok"; then
        echo "Stored. Verifying…"
        "$BIN" whoami
    else
        err "import failed — check you copied the whole token value"
    fi
    echo; pause
}

# ---- menu --------------------------------------------------------------------
menu() {
    # Only clear the screen on a real terminal, so piped/non-TTY runs stay readable.
    [[ -t 1 ]] && clear
    bold "insta360cli  —  $BIN"
    dim  "Range: $(range_label)    Output: $OUTDIR"
    echo
    echo "  1) Who am I"
    echo "  2) List media"
    echo "  3) Set date range"
    echo "  4) Set output directory"
    echo "  5) Estimate size (dry run)"
    echo "  6) Download matching media"
    echo "  7) Import session token"
    echo "  q) Quit"
    echo
    read -r -p "Choose: " choice
    case "$choice" in
        1) do_whoami ;;
        2) do_list ;;
        3) set_range ;;
        4) set_outdir ;;
        5) do_dry_run ;;
        6) do_download ;;
        7) import_token ;;
        q|Q) exit 0 ;;
        *) ;;
    esac
}

while true; do menu; done
