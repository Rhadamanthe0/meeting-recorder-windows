#!/bin/bash
# Crops a 3840x2160 (scale 2) VM screenshot to a window, with a small margin
# of wallpaper around it.
#
#   demo/crop.sh <shot.png> <out.png> <x> <y> <w> <h> [width]
#
# x y w h are the window's logical geometry as `hyprctl clients -j` reports it
# (at + size). The crop is scaled down to [width] pixels wide when it is wider
# (default 1600), never up.
set -euo pipefail
shot="$1" out="$2" x="$3" y="$4" w="$5" h="$6" width="${7:-1600}"
margin=24
px=$(( (x - margin) * 2 )) py=$(( (y - margin) * 2 ))
pw=$(( (w + margin * 2) * 2 )) ph=$(( (h + margin * 2) * 2 ))
(( px < 0 )) && px=0
(( py < 0 )) && py=0
magick "$shot" -crop "${pw}x${ph}+${px}+${py}" +repage -resize "${width}x>" -strip "$out"
