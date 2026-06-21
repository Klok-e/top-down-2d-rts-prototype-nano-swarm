#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
assets_dir="$repo_root/assets"

if command -v magick >/dev/null 2>&1; then
    im=(magick)
elif command -v convert >/dev/null 2>&1; then
    im=(convert)
else
    echo "error: ImageMagick required (magick or convert)" >&2
    exit 1
fi

mkdir -p "$assets_dir"

# Resource Deposit: five flat teal/cyan rock-like diamonds/hexes, transparent background.
"${im[@]}" -size 64x64 xc:none \
    -fill '#28c7c7' -draw 'polygon 18,12 29,18 27,31 15,33 9,22' \
    -fill '#45d6ff' -draw 'polygon 38,10 50,18 48,31 35,33 30,21' \
    -fill '#159aa6' -draw 'polygon 29,26 42,33 39,49 25,52 18,39' \
    -fill '#63e6d2' -draw 'polygon 14,34 25,40 23,52 11,55 6,44' \
    -fill '#1fb8d1' -draw 'polygon 45,34 57,42 53,55 40,53 35,42' \
    "$assets_dir/resource_deposit.png"

# Production Facility: flat square base, cyan core, four small ports.
"${im[@]}" -size 64x64 xc:none \
    -fill '#2f3437' -draw 'rectangle 14,14 50,50' \
    -fill '#485057' -draw 'rectangle 20,20 44,44' \
    -fill '#38d6d6' -draw 'circle 32,32 32,23' \
    -fill '#66e38a' -draw 'rectangle 29,8 35,14' \
    -fill '#66e38a' -draw 'rectangle 29,50 35,56' \
    -fill '#66e38a' -draw 'rectangle 8,29 14,35' \
    -fill '#66e38a' -draw 'rectangle 50,29 56,35' \
    "$assets_dir/production_facility.png"
