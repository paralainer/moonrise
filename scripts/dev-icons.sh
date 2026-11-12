#!/usr/bin/env bash
# Make the Dev app's icons ("Moonrise Dev", the scripts' DEV=1 builds) from the app's own: the same
# logo on amber instead of black, so the two apps are told apart on a home screen. The results are
# committed; run this again only when the app's icon changes. ImageMagick runs in a container.
#
#   scripts/dev-icons.sh
#
# - clients/apple/Moonrise/Assets.xcassets/AppIcon-Dev.appiconset (iPhone, iPad, Mac): AppIcon over amber,
#   blended so its black turns amber and the logo stays as it is.
# - clients/apple/Moonrise/AssetsTV.xcassets/AppIconTV-Dev.brandassets (Apple TV): the logo layer as it is over an
#   amber back layer. The Top Shelf's static image is left out (the Dev app's Top Shelf shows the
#   extension's rows, or nothing when signed out).
# clients/apple/project.yml picks them with MOONRISE_APP_ICON and MOONRISE_APP_ICON_TV
# (clients/apple/Config/Signing.xcconfig).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AMBER="#B45309"
# Pinned (tag and digest), so a rerun draws with the ImageMagick that made the committed icons.
IMAGE="dpokidov/imagemagick:7.1.2-12@sha256:87998ec1b8127b2f73f626f74f7b05e8827f9d7605fa52da5370588f7e53cee1"
IOS_SRC="$ROOT/clients/apple/Moonrise/Assets.xcassets/AppIcon.appiconset"
IOS_DEV="$ROOT/clients/apple/Moonrise/Assets.xcassets/AppIcon-Dev.appiconset"
TV_SRC="$ROOT/clients/apple/Moonrise/AssetsTV.xcassets/App Icon & Top Shelf Image.brandassets/App Icon.imagestack"
TV_DEV="$ROOT/clients/apple/Moonrise/AssetsTV.xcassets/AppIconTV-Dev.brandassets"

rm -rf "$IOS_DEV" "$TV_DEV"
mkdir -p "$IOS_DEV" "$TV_DEV/App Icon.imagestack/Front.imagestacklayer/Content.imageset" \
    "$TV_DEV/App Icon.imagestack/Back.imagestacklayer/Content.imageset"
cp "$IOS_SRC/Contents.json" "$IOS_DEV/"
cp "$TV_SRC/Contents.json" "$TV_DEV/App Icon.imagestack/"
for layer in Front Back; do
    cp "$TV_SRC/$layer.imagestacklayer/Contents.json" "$TV_DEV/App Icon.imagestack/$layer.imagestacklayer/"
    cp "$TV_SRC/$layer.imagestacklayer/Content.imageset/Contents.json" "$TV_DEV/App Icon.imagestack/$layer.imagestacklayer/Content.imageset/"
done
cp "$TV_SRC/Front.imagestacklayer/Content.imageset/"*.png "$TV_DEV/App Icon.imagestack/Front.imagestacklayer/Content.imageset/"
cat > "$TV_DEV/Contents.json" <<'JSON'
{
  "assets" : [
    {
      "filename" : "App Icon.imagestack",
      "idiom" : "tv",
      "role" : "primary-app-icon",
      "size" : "400x240"
    }
  ],
  "info" : {
    "author" : "xcode",
    "version" : 1
  }
}
JSON

docker run --rm --user "$(id -u):$(id -g)" -v "$IOS_SRC:/src:ro" -v "$IOS_DEV:/ios" \
    -v "$TV_DEV/App Icon.imagestack/Back.imagestacklayer/Content.imageset:/back" \
    --entrypoint sh "$IMAGE" -euc "
        for f in icon-1024 icon-512; do
            magick /src/\$f.png \\( +clone -fill '$AMBER' -colorize 100 \\) +swap -compose screen -composite -strip /ios/\$f.png
        done
        magick -size 400x240 'xc:$AMBER' -strip /back/back.png
        magick -size 800x480 'xc:$AMBER' -strip /back/back@2x.png
    "
echo "==> $IOS_DEV, $TV_DEV"
