# Remote access with Tailscale

Moonrise never opens your server to the internet. To watch away from home, the apps reach the server
through [Tailscale](https://tailscale.com), a private network between your own devices.

1. Install Tailscale on the server's computer (or NAS: Synology, Unraid and TrueNAS have Tailscale
   packages/apps) and sign in. The server needs nothing else: Moonrise answers on every address of the
   machine, the tailnet's (100.x) included, and Manage Server → Services shows "Remote access:
   on" with that address.
2. In the app on iPhone, iPad, Mac, an Android phone or tablet, or Android TV / Google TV:
   **Settings → Remote Access → Connect Through Tailscale**, and sign in to the same tailnet. On a
   phone the sign-in opens in the browser; a TV shows it as a QR code to scan with your phone, and
   its short address to open on any device. The app joins as its own device: no Tailscale app, and
   on Android it doesn't take the system's VPN slot. At home it keeps using the local network;
   away, it switches to the tailnet by itself, during playback too.
3. Apple TV: install the Tailscale app on the Apple TV (tvOS can't embed it), or use it only at home.

Without Tailscale the apps work on the home network, and downloads play anywhere.

Building the apps yourself: remote access needs the Tailscale library, built from source:

- Apple: `clients/apple/Vendor/TailscaleKit.xcframework` (`scripts/build-tailscalekit.sh`, needs Go).
- Android: `clients/android/vendor/moonrise-tailscale.aar` (`scripts/build-tailscale-android.sh`,
  needs only Docker: it builds Go, gomobile and the NDK into its own image and cache).

Without it an app builds and runs without the Remote Access section (see [building.md](building.md)).

The embedded node talks to Tailscale's coordination service and its relays only while Connect
Through Tailscale is on, as any Tailscale device does. The Android apps' node sends no logs to
Tailscale.
