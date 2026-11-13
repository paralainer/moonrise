# Brand art

The Moonrise icon artwork. Nothing here is built into an app: the apps use the
images in their own asset folders (below), and the scripts that make icons read those.

| File | What it is |
|---|---|
| `icon-glass-moonrise-play.png` | The Moonrise icon artwork, 1024 x 1024: a glass film spool with a play symbol. The Unraid template (`server/packaging/unraid/moonrise-server.xml`) shows it as the server's icon, from this path on GitHub |

## The apps' illustrations

Two illustrations ship in the apps, besides the icons. Both are original to Moonrise: they were
generated for it from text prompts alone (no source images) with the open image model
[MOSS-Image](https://github.com/OpenMOSS/MOSS-Image) (Apache-2.0), then resized. They are covered
by the apps' licence (MPL-2.0), like the rest of `clients/`.

| Image | Where | What it shows |
|---|---|---|
| `WelcomeTheater` | Apple: `WelcomeTheater.imageset` in both asset catalogs; Android phone: `drawable-nodpi/welcome_theater.jpg` | An empty home theatre with a glowing blank screen, behind the welcome screen |
| `EmptyDownloads` | Apple: `EmptyDownloads.imageset` in both asset catalogs; Android phone: `drawable-nodpi/empty_downloads.jpg` | An amber film strip curling into a download arrow over film canisters, on the empty Downloads screen |

The demo's posters and backdrops (`clients/apple/Moonrise/Demo/Resources/`) are the Blender
Foundation's, under Creative Commons licences: see [docs/demo.md](../../docs/demo.md).

## Where the apps' images are, and how icons are made

- **Apple apps:** `clients/apple/Moonrise/Assets.xcassets` (iPhone, iPad, Mac: `AppIcon`,
  `EmptyDownloads`, `WelcomeTheater`) and `clients/apple/Moonrise/AssetsTV.xcassets` (Apple TV: the
  layered app icon and Top Shelf images in `App Icon & Top Shelf Image.brandassets`, and
  `MoonriseMark`). These are edited by hand.
- **Dev app icons:** `scripts/dev-icons.sh` makes `AppIcon-Dev` and `AppIconTV-Dev` (the same logo on
  amber) from `AppIcon` and the Apple TV icon's layers.
- **Android TV:** `clients/android/tools/artwork.sh` makes the launcher icon's foreground and the
  home-screen banners (the release app's and the Dev app's) from the front layer of the Apple TV App
  Store icon. `clients/android/tv/src/main/res/drawable-nodpi/moonrise_mark.png` is the mark the app
  shows on its own screens.

Both scripts run ImageMagick in Docker, and their results are committed: run one again only when its
source images change.

## Licence

The icon artwork here, and the app icons made from it, are the Moonrise logo: a trademark, not
covered by the code licences (MPL-2.0 and AGPL-3.0) and not free to reuse in a fork. A fork or
redistribution uses its own name and icon. See [TRADEMARKS.md](../../TRADEMARKS.md).
