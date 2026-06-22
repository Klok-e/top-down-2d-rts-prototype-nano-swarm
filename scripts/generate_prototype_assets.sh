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

# Planned Production Facility: foundation pad, transparent ghost core, four anchor sockets.
"${im[@]}" -size 64x64 xc:none \
    -fill '#26303380' -draw 'rectangle 14,14 50,50' \
    -fill none -stroke '#7ee7e7cc' -strokewidth 3 -draw 'rectangle 18,18 46,46' \
    -fill '#38d6d680' -stroke none -draw 'circle 32,32 32,24' \
    -fill '#66e38aaa' -draw 'rectangle 29,8 35,14' \
    -fill '#66e38aaa' -draw 'rectangle 29,50 35,56' \
    -fill '#66e38aaa' -draw 'rectangle 8,29 14,35' \
    -fill '#66e38aaa' -draw 'rectangle 50,29 56,35' \
    "$assets_dir/planned_production_facility.png"

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

# Planned Source Stockpile: dashed intake tray outline with faint mineral dots.
"${im[@]}" -size 64x64 xc:none \
    -fill '#0e3a3a66' -draw 'polygon 14,25 50,25 44,50 20,50' \
    -fill none -stroke '#63e6d2cc' -strokewidth 3 -draw 'line 14,25 25,25 line 33,25 50,25 line 50,25 46,38 line 44,45 44,50 line 44,50 31,50 line 23,50 20,50 line 20,50 16,37 line 14,31 14,25' \
    -fill '#45d6ff99' -stroke none -draw 'circle 26,35 26,31' \
    -fill '#28c7c799' -draw 'circle 36,39 36,35' \
    -fill '#63e6d299' -draw 'circle 31,44 31,41' \
    "$assets_dir/planned_source_stockpile.png"

# Source Stockpile: open intake tray with teal mineral chunks.
"${im[@]}" -size 64x64 xc:none \
    -fill '#123f3f' -draw 'polygon 12,23 52,23 46,52 18,52' \
    -fill '#1f6b68' -draw 'polygon 18,29 46,29 42,47 22,47' \
    -fill '#45d6ff' -draw 'polygon 24,31 31,34 29,42 21,42 19,36' \
    -fill '#28c7c7' -draw 'polygon 36,32 44,37 41,45 33,44 31,37' \
    -fill '#63e6d2' -draw 'polygon 29,42 36,45 34,51 26,50 24,45' \
    -fill '#102a2a' -draw 'rectangle 13,21 51,26' \
    "$assets_dir/source_stockpile.png"

# Planned Sink Stockpile: ghost depot footprint with faint storage blocks.
"${im[@]}" -size 64x64 xc:none \
    -fill '#3a321066' -draw 'roundrectangle 13,17 51,51 5,5' \
    -fill none -stroke '#f2c94ccc' -strokewidth 3 -draw 'line 13,17 25,17 line 34,17 51,17 line 51,17 51,30 line 51,39 51,51 line 51,51 38,51 line 29,51 13,51 line 13,51 13,38 line 13,29 13,17' \
    -fill '#fff3a399' -stroke none -draw 'rectangle 21,29 31,41' \
    -fill '#66e38a99' -draw 'rectangle 34,25 44,42' \
    "$assets_dir/planned_sink_stockpile.png"

# Sink Stockpile: enclosed storage depot with logistics accents.
"${im[@]}" -size 64x64 xc:none \
    -fill '#4a3910' -draw 'roundrectangle 12,16 52,52 6,6' \
    -fill '#f2c94c' -draw 'roundrectangle 17,21 47,47 4,4' \
    -fill '#fff3a3' -draw 'rectangle 22,28 31,41' \
    -fill '#66e38a' -draw 'rectangle 34,26 43,41' \
    -fill '#4a3910' -draw 'rectangle 14,31 50,36' \
    -fill '#fff3a3' -draw 'rectangle 27,10 37,16' \
    "$assets_dir/sink_stockpile.png"

# Planned Charger: ghost charging pad ring and faint battery core.
"${im[@]}" -size 64x64 xc:none \
    -fill '#15284d55' -draw 'circle 32,34 32,10' \
    -fill none -stroke '#82aaffcc' -strokewidth 4 -draw 'circle 32,34 32,12' \
    -fill '#a9c9ff88' -stroke none -draw 'rectangle 27,22 37,42' \
    -fill '#a9c9ff88' -draw 'rectangle 29,18 35,22' \
    -fill '#3f7fff99' -draw 'rectangle 29,32 35,39' \
    "$assets_dir/planned_charger.png"

# Charger: powered charging node with battery core and contact prongs.
"${im[@]}" -size 64x64 xc:none \
    -fill '#15284d' -draw 'circle 32,34 32,9' \
    -fill '#3f7fff' -draw 'circle 32,34 32,13' \
    -fill '#a9c9ff' -draw 'rectangle 25,20 39,44' \
    -fill '#d7e7ff' -draw 'rectangle 28,16 36,20' \
    -fill '#66e38a' -draw 'rectangle 28,32 36,41' \
    -fill '#15284d' -draw 'rectangle 17,31 24,37' \
    -fill '#15284d' -draw 'rectangle 40,31 47,37' \
    "$assets_dir/charger.png"

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
