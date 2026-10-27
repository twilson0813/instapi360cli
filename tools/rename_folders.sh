#!/usr/bin/env bash
#
# rename_folders.sh — rename legacy mediaId-named asset folders to the
# capture-stem scheme (VID_<date>_<time>), matching the current CLI.
#
# Scope: only directories whose name is all-digits (the old mediaId folders)
# AND whose modification time is older than a cutoff (default: 5 AM today).
# The new name is derived from the .insv/.insp/.mp4 file inside the folder.
#
# Usage:
#   ./rename_folders.sh <dir>            # DRY RUN (prints planned renames)
#   ./rename_folders.sh <dir> --apply    # actually rename
#   ./rename_folders.sh <dir> --cutoff "2026-10-27 05:00:00" [--apply]
#
set -u

DIR="${1:-}"
[[ -z "$DIR" || ! -d "$DIR" ]] && { echo "usage: $0 <dir> [--apply] [--cutoff 'YYYY-MM-DD HH:MM:SS']" >&2; exit 1; }
shift

APPLY=0
CUTOFF=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --apply) APPLY=1 ;;
        --cutoff) CUTOFF="$2"; shift ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
    shift
done

# Default cutoff: 5 AM today (local time).
[[ -z "$CUTOFF" ]] && CUTOFF="$(date +%Y-%m-%d) 05:00:00"

# Make a reference file stamped at the cutoff so we can compare mtimes portably.
REF="$(mktemp)"
# macOS `date -j` parses an explicit format; -t sets a touch timestamp.
TS="$(date -j -f "%Y-%m-%d %H:%M:%S" "$CUTOFF" "+%Y%m%d%H%M.%S" 2>/dev/null)" || {
    echo "could not parse cutoff '$CUTOFF'" >&2; rm -f "$REF"; exit 1; }
touch -t "$TS" "$REF"

echo "Directory : $DIR"
echo "Cutoff    : $CUTOFF  (only folders modified at/before this are considered)"
echo "Mode      : $([[ $APPLY -eq 1 ]] && echo APPLY || echo 'DRY RUN')"
echo

# Derive VID_<date>_<time> stem from a camera filename.
stem() {
    local n="$1" prefix date time
    n="${n##*/}"
    IFS='_' read -r prefix date time _rest <<<"$n"
    time="${time%%.*}"
    if [[ "$prefix" =~ ^[A-Za-z]+$ && "$date" =~ ^[0-9]{8}$ && "$time" =~ ^[0-9]{6}$ ]]; then
        echo "${prefix}_${date}_${time}"
    fi
}

renamed=0; skipped=0; conflicts=0
for d in "$DIR"/*/; do
    d="${d%/}"
    base="$(basename "$d")"
    # only all-digit (mediaId) folders
    [[ "$base" =~ ^[0-9]+$ ]] || continue
    # only folders at/older than the cutoff
    if [[ "$d" -nt "$REF" ]]; then
        skipped=$((skipped+1)); continue
    fi
    # find a media file inside to derive the name
    src="$(find "$d" -maxdepth 1 -type f \( -iname '*.insv' -o -iname '*.insp' -o -iname '*.mp4' \) | head -1)"
    [[ -z "$src" ]] && { echo "SKIP  $base  (no media file inside)"; skipped=$((skipped+1)); continue; }
    newname="$(stem "$src")"
    [[ -z "$newname" ]] && { echo "SKIP  $base  (unrecognized name: $(basename "$src"))"; skipped=$((skipped+1)); continue; }
    target="$DIR/$newname"
    if [[ -e "$target" ]]; then
        echo "CONFLICT  $base -> $newname  (target exists, leaving as-is)"
        conflicts=$((conflicts+1)); continue
    fi
    echo "RENAME    $base -> $newname"
    if [[ $APPLY -eq 1 ]]; then
        mv "$d" "$target"
    fi
    renamed=$((renamed+1))
done

rm -f "$REF"
echo
echo "Planned/done renames: $renamed | skipped: $skipped | conflicts: $conflicts"
[[ $APPLY -eq 0 ]] && echo "(dry run — nothing changed; re-run with --apply to perform)"
