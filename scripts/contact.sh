#!/bin/bash
# Contact sheet of cropped shots: contact.sh OUT.png TILE-COLUMNS FILES...
# (host tool; the crop is the setup card at 1x)
out=$1; cols=$2; shift 2
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
i=0
for f in "$@"; do
    magick "$f" -crop 880x700+520+190 +repage -gravity north -background '#888' -fill white -pointsize 18 -splice 0x26 -annotate +0+3 "$(basename "$f" .png)" "$tmp/$(printf %03d $i).png"
    i=$((i+1))
done
montage "$tmp"/*.png -tile "${cols}x" -geometry +3+3 -background '#666' "$out"
