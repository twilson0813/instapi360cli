#!/usr/bin/env bash
#
# instapi360-ui — a small interactive menu over the `instapi360` CLI.
#
# Drives the common workflows without memorizing flags: check who you are,
# list your cloud media, filter by a shoot-date range, estimate the download
# size, and download (with resume) to a folder or a mounted Samba share.
#
# Usage:  ./instapi360-ui.sh
#
# It locates the `instapi360` binary from PATH, or falls back to ~/instapi360
# or ~/.cargo/bin/instapi360. Set INSTAPI360_BIN to override.

set -u

# ---- small helpers -----------------------------------------------------------
bold()  { printf "\033[1m%s\033[0m\n" "$*"; }
dim()   { printf "\033[2m%s\033[0m\n" "$*"; }
err()   { printf "\033[31m%s\033[0m\n" "$*" >&2; }
pause() { read -r -p "Press Enter to continue… " _; }

# ---- locate the binary -------------------------------------------------------
# An executable *file* (not a directory that merely has the search bit set —
# a project folder named `instapi360` would otherwise be picked by mistake).
is_exe() { [[ -f "$1" && -x "$1" ]]; }

# Directory this script lives in (so a binary sitting next to the UI is found).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" >/dev/null 2>&1 && pwd)"

find_bin() {
    # 1) explicit override wins.
    if [[ -n "${INSTAPI360_BIN:-}" ]] && is_exe "${INSTAPI360_BIN}"; then
        echo "${INSTAPI360_BIN}"; return 0
    fi
    # 2) on PATH.
    if command -v instapi360 >/dev/null 2>&1; then
        command -v instapi360; return 0
    fi
    # 3) common standalone locations, in priority order.
    local cand
    for cand in \
        "$SCRIPT_DIR/instapi360" \
        "$PWD/instapi360" \
        "$HOME/.cargo/bin/instapi360" \
        "$HOME/instapi360/instapi360" \
        "$HOME/instapi360/target/release/instapi360" \
        "$HOME/instapi360.bin" \
        "$HOME/bin/instapi360" \
        "$HOME/instapi360"; do
        is_exe "$cand" && { echo "$cand"; return 0; }
    done
    return 1
}

BIN="$(find_bin)" || {
    err "Could not find the 'instapi360' binary."
    echo "Looked on PATH and in: this script's folder, \$PWD, ~/.cargo/bin," >&2
    echo "~/instapi360[/target/release], ~/instapi360.bin, ~/bin." >&2
    echo >&2
    echo "Fix: point the UI at your standalone binary, e.g.:" >&2
    echo "    INSTAPI360_BIN=\$HOME/instapi360-cli ./instapi360-ui" >&2
    echo "or drop the binary next to this script and name it 'instapi360'." >&2
    exit 1
}

# Current date-range selection (empty = all).
SINCE=""
UNTIL=""
# Default output directory.
OUTDIR="${INSTAPI360_OUT:-$HOME/Insta360}"

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
    dim  "Paste the 'insta_access_token' value from your browser cookie on cloud.insta360.com"
    read -r -p "Token: " tok
    [[ -z "$tok" ]] && { echo "No token entered."; echo; pause; return; }
    "$BIN" import-token "$tok" && echo "Stored." || err "import failed"
    echo; pause
}

# ---- menu --------------------------------------------------------------------
menu() {
    # Only clear the screen on a real terminal, so piped/non-TTY runs stay readable.
    [[ -t 1 ]] && clear
    bold "instapi360  —  $BIN"
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
