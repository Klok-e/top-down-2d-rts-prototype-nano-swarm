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

# Player Worker: round body with small work notch/dot.
"${im[@]}" -size 64x64 xc:none \
    -fill '#1c392e' -draw 'circle 32,32 32,10' \
    -fill '#66e38a' -draw 'circle 32,32 32,14' \
    -fill '#b9ffd0' -draw 'circle 43,22 43,17' \
    -fill '#1c392e' -draw 'rectangle 29,8 35,18' \
    "$assets_dir/worker_nanobot.png"

# Player Hauler: cargo body with visible forward cab at +Y.
"${im[@]}" -size 64x64 xc:none \
    -fill '#4a3910' -draw 'roundrectangle 14,18 50,50 8,8' \
    -fill '#f2c94c' -draw 'roundrectangle 18,20 46,50 8,8' \
    -fill '#fff3a3' -draw 'rectangle 24,30 40,46' \
    -fill '#fff3a3' -draw 'polygon 32,8 44,22 20,22' \
    -fill '#4a3910' -draw 'rectangle 10,30 18,40' \
    -fill '#4a3910' -draw 'rectangle 46,30 54,40' \
    "$assets_dir/hauler_nanobot.png"

# Player Defender: triangular shield silhouette with bright core.
"${im[@]}" -size 64x64 xc:none \
    -fill '#15284d' -draw 'polygon 32,8 56,52 8,52' \
    -fill '#3f7fff' -draw 'polygon 32,13 50,49 14,49' \
    -fill '#a9c9ff' -draw 'circle 32,36 32,27' \
    "$assets_dir/defender_nanobot.png"

# Opponent Worker: same worker silhouette, hostile red palette.
"${im[@]}" -size 64x64 xc:none \
    -fill '#4a1818' -draw 'circle 32,32 32,10' \
    -fill '#d85a5a' -draw 'circle 32,32 32,14' \
    -fill '#ffc1b8' -draw 'circle 43,22 43,17' \
    -fill '#4a1818' -draw 'rectangle 29,8 35,18' \
    "$assets_dir/opponent_worker_nanobot.png"

# Opponent Hauler: same cargo silhouette, hostile orange palette.
"${im[@]}" -size 64x64 xc:none \
    -fill '#4a2410' -draw 'roundrectangle 14,18 50,50 8,8' \
    -fill '#e88932' -draw 'roundrectangle 18,20 46,50 8,8' \
    -fill '#ffd0a0' -draw 'rectangle 24,30 40,46' \
    -fill '#ffd0a0' -draw 'polygon 32,8 44,22 20,22' \
    -fill '#4a2410' -draw 'rectangle 10,30 18,40' \
    -fill '#4a2410' -draw 'rectangle 46,30 54,40' \
    "$assets_dir/opponent_hauler_nanobot.png"

# Opponent Defender: same shield silhouette, hostile magenta palette.
"${im[@]}" -size 64x64 xc:none \
    -fill '#4a102d' -draw 'polygon 32,8 56,52 8,52' \
    -fill '#d64c89' -draw 'polygon 32,13 50,49 14,49' \
    -fill '#ffc0d9' -draw 'circle 32,36 32,27' \
    "$assets_dir/opponent_defender_nanobot.png"
