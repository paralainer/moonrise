#!/usr/bin/env bash
# Makes the TV app's icon and home-screen banner from the Apple TV app's artwork (the Moonrise mark in
# clients/apple/Moonrise/AssetsTV.xcassets), with ImageMagick in a container. The results are committed; run this
# again only when that artwork changes.
#
#   clients/android/tools/artwork.sh
#
# - tv/src/main/res/drawable-xhdpi/banner.png: the launcher's banner (320 x 180 dp), the Moonrise mark and
#   the app's name on black.
# - tv/src/main/res/mipmap-xxxhdpi/ic_launcher_foreground.png: the adaptive icon's foreground (108 dp,
#   the Moonrise mark inside the 66 dp safe zone); its background is @color/ic_launcher_background (black).
# - tv/src/debug/res/drawable-xhdpi/banner.png: the Dev app's banner (the debug build, "Moonrise Dev"),
#   the same with "Dev" in amber, so the two apps are told apart on the home screen. Its icon is the
#   same logo on amber (@color/ic_launcher_background in src/debug/res/values/colors.xml).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
STACK="$ROOT/clients/apple/Moonrise/AssetsTV.xcassets/App Icon & Top Shelf Image.brandassets/App Icon - App Store.imagestack"
RES="$HERE/tv/src/main/res"
DEV_RES="$HERE/tv/src/debug/res"
mkdir -p "$RES/mipmap-xxxhdpi" "$RES/drawable-xhdpi" "$DEV_RES/drawable-xhdpi"

docker run --rm -e OWNER="$(id -u):$(id -g)" \
    -v "$STACK/Front.imagestacklayer/Content.imageset:/in:ro" -v "$RES:/res" -v "$DEV_RES:/devres" \
    alpine:3.20 sh -euc '
        apk add -q --no-cache imagemagick font-inter >/dev/null 2>&1 || { echo "apk failed" >&2; exit 1; }
        cd /tmp
        # The mark alone: the front layer (transparent around it), trimmed to its edge and its glow.
        magick /in/front-store.png -trim +repage -background none -gravity center -extent "%[fx:max(w,h)]x%[fx:max(w,h)]" mark.png
        # Adaptive icon foreground: 432 px (108 dp at xxxhdpi), the Moonrise mark 264 px (66 dp safe zone).
        magick mark.png -resize 264x264 -background none -gravity center -extent 432x432 -strip -depth 8 -define png:color-type=6 /res/mipmap-xxxhdpi/ic_launcher_foreground.png
        # Banner: 640 x 360 (320 x 180 dp at xhdpi), the Moonrise mark and "Moonrise" centred as one line.
        magick -background none -fill "#F4ECE6" -font /usr/share/fonts/inter/InterDisplay-SemiBold.otf -pointsize 72 -kerning -1 label:Moonrise -trim +repage name.png
        magick mark.png -resize 168x168 r.png
        magick r.png \( -size 30x1 xc:none \) name.png -background none -gravity center +append row.png
        magick -size 640x360 xc:black row.png -gravity center -composite -strip -depth 8 -define png:color-type=2 /res/drawable-xhdpi/banner.png
        # The Dev app: "Moonrise" and "Dev" (amber), a little smaller to fit the same width.
        font=/usr/share/fonts/inter/InterDisplay-SemiBold.otf
        magick -background none -fill "#F4ECE6" -font $font -pointsize 64 -kerning -1 label:Moonrise -trim +repage dn.png
        magick -background none -fill "#F59E0B" -font $font -pointsize 64 -kerning -1 label:Dev -trim +repage dd.png
        magick dn.png \( -size 18x1 xc:none \) dd.png -background none -gravity south +append dname.png
        magick r.png -resize 150x150 dr.png
        magick dr.png \( -size 26x1 xc:none \) dname.png -background none -gravity center +append drow.png
        magick -size 640x360 xc:black drow.png -gravity center -composite -strip -depth 8 -define png:color-type=2 /devres/drawable-xhdpi/banner.png
        chown "$OWNER" /res/drawable-xhdpi/banner.png /res/mipmap-xxxhdpi/ic_launcher_foreground.png /devres/drawable-xhdpi/banner.png
    '
echo "==> $RES/drawable-xhdpi/banner.png, $RES/mipmap-xxxhdpi/ic_launcher_foreground.png, $DEV_RES/drawable-xhdpi/banner.png"
