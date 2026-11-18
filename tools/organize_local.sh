#!/usr/bin/env bash
#
# organize_local.sh — sort a flat camera folder (e.g. an SD card's
# DCIM/Camera01) into the same per-capture layout insta360cli downloads use:
#
#   <dest>/VID_20260809_131521/VID_20260809_131521_00_001.insv
#   <dest>/VID_20260809_131521/LRV_20260809_131521_01_001.lrv
#   <dest>/IMG_20260810_114318/IMG_20260810_114318_00_014.insp
#
# Folder name = capture stem PREFIX_<date>_<time>. LRV (low-res preview)
# files are grouped with their VID capture, so the folder is VID_..., the
# same name the cloud download gives that clip.
#
# Usage:
#   ./organize_local.sh <source> [dest] [--move] [--dry-run]
#
#   source     flat folder of camera files (searched one level deep)
#   dest       where to build the layout (default: <source>-organized)
#   --move     move files instead of copying (default: copy, source untouched)
#   --dry-run  print what would happen, change nothing
#
# Existing files at the destination with the same size are skipped, so it is
# safe to re-run. Files that don't match the camera naming are left alone.
#
set -u

SRC=""; DEST=""; MOVE=0; DRY=0
for a in "$@"; do
    case "$a" in
        --move) MOVE=1 ;;
        --dry-run|-n) DRY=1 ;;
        -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
        -*) echo "unknown option: $a" >&2; exit 1 ;;
        *) if [[ -z "$SRC" ]]; then SRC="$a"; elif [[ -z "$DEST" ]]; then DEST="$a"; else echo "too many arguments" >&2; exit 1; fi ;;
    esac
done
[[ -z "$SRC" || ! -d "$SRC" ]] && { echo "usage: $0 <source> [dest] [--move] [--dry-run]" >&2; exit 1; }
SRC="${SRC%/}"
[[ -z "$DEST" ]] && DEST="${SRC}-organized"
DEST="${DEST%/}"

echo "Source : $SRC"
echo "Dest   : $DEST"
echo "Mode   : $([[ $MOVE -eq 1 ]] && echo move || echo copy)$([[ $DRY -eq 1 ]] && echo ' (DRY RUN)')"
echo

# PREFIX_YYYYMMDD_HHMMSS from a camera filename; LRV maps to VID.
stem() {
    local n="$1" prefix date time
    IFS='_' read -r prefix date time _rest <<<"$n"
    time="${time%%.*}"
    [[ "$prefix" =~ ^[A-Za-z]+$ && "$date" =~ ^[0-9]{8}$ && "$time" =~ ^[0-9]{6}$ ]] || return 1
    [[ "$(echo "$prefix" | tr '[:lower:]' '[:upper:]')" == "LRV" ]] && prefix="VID"
    echo "${prefix}_${date}_${time}"
}

fsize() { stat -f %z "$1" 2>/dev/null || stat -c %s "$1"; }

done_n=0; skip_n=0; other_n=0; folders=()
while IFS= read -r -d '' f; do
    name="$(basename "$f")"
    [[ "$name" == .* ]] && continue
    s="$(stem "$name")" || { echo "IGNORE  $name"; other_n=$((other_n+1)); continue; }
    target="$DEST/$s/$name"
    if [[ -e "$target" && "$(fsize "$target")" == "$(fsize "$f")" ]]; then
        skip_n=$((skip_n+1)); continue
    fi
    echo "$([[ $MOVE -eq 1 ]] && echo MOVE || echo COPY)    $name -> $s/"
    if [[ $DRY -eq 0 ]]; then
        mkdir -p "$DEST/$s" || exit 1
        if [[ $MOVE -eq 1 ]]; then mv "$f" "$target"; else cp -p "$f" "$target"; fi || { echo "FAILED  $name" >&2; exit 1; }
    fi
    done_n=$((done_n+1))
done < <(find "$SRC" -maxdepth 1 -type f -print0 | sort -z)

echo
echo "Files $([[ $DRY -eq 1 ]] && echo planned || echo done): $done_n | already there: $skip_n | ignored: $other_n"
[[ $DRY -eq 1 ]] && echo "(dry run, nothing changed)"
exit 0
