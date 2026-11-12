# Remote access with Tailscale

Moonrise never opens your server to the internet. To watch away from home, the apps reach the server
through [Tailscale](https://tailscale.com), a private network between your own devices.

1. Install Tailscale on the server's computer (or NAS: Synology, Unraid and TrueNAS have Tailscale
   packages/apps) and sign in. The server needs nothing else: Moonrise answers on every address of the
   machine, the tailnet's (100.x) included, and Manage Server → Services shows "Remote access:
   on" with that address.
2. In the app on iPhone, iPad or Mac: **Settings → Remote Access → Connect Through Tailscale**,
   and sign in to the same tailnet. The app joins as its own device: no Tailscale app or VPN
   needed. At home it keeps using the local network; away, it switches to the tailnet by itself.
3. Apple TV: install the Tailscale app on the Apple TV (tvOS can't embed it), or use it only at home.

Without Tailscale the apps work on the home network, and downloads play anywhere.

Building the app yourself: remote access needs `clients/apple/Vendor/TailscaleKit.xcframework`
(`scripts/build-tailscalekit.sh`, needs Go). Without it the app builds without the Remote Access
section (see [building.md](building.md)).
