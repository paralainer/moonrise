#!/bin/bash
# Moonrise server on macOS: installs (or updates) the server as a background service of this user.
#
#   ./install.sh                     # the moonrise-server next to this script; ffmpeg found or fetched
#   ./install.sh --binary PATH       # another moonrise-server binary (e.g. target/release/moonrise-server)
#   ./install.sh --ffmpeg PATH       # use this ffmpeg (ffprobe next to it), e.g. Homebrew's
#   ./install.sh --download-ffmpeg   # always fetch the pinned static ffmpeg build
#   ./install.sh --port 8097         # the port (the apps look for 8097)
#   ./install.sh --at-boot           # start at boot, before anyone logs in (LaunchDaemon; asks
#                                    # for your password once; see docs/install-macos.md)
#
# What goes where:
#   ~/Applications/Moonrise Server/                 the program (moonrise-server, ffmpeg, uninstall.sh)
#   ~/Library/Application Support/Moonrise Server/  its data: database, subtitles, artwork (back it up)
#   ~/Library/Caches/Moonrise Server/               stream cache (safe to delete)
#   ~/Library/Logs/Moonrise Server/server.log       the log (10 MB, then server.log.1)
#   ~/Library/LaunchAgents/org.moonrise.server.plist   (or /Library/LaunchDaemons/ with --at-boot)
#
# Running it again updates the program and restarts the server; the data stays.
set -euo pipefail

LABEL=org.moonrise.server
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$HOME/Applications/Moonrise Server"
DATA_DIR="$HOME/Library/Application Support/Moonrise Server"
CACHE_DIR="$HOME/Library/Caches/Moonrise Server"
LOG_DIR="$HOME/Library/Logs/Moonrise Server"
BINARY="$HERE/moonrise-server"
FFMPEG=""
DOWNLOAD=0
PORT=8097
AT_BOOT=0

while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BINARY=${2:?--binary needs a path}; shift 2 ;;
    --ffmpeg) FFMPEG=${2:?--ffmpeg needs a path}; shift 2 ;;
    --download-ffmpeg) DOWNLOAD=1; shift ;;
    --port) PORT=${2:?--port needs a number}; shift 2 ;;
    --at-boot) AT_BOOT=1; shift ;;
    -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
done

say() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || die "this installs the macOS service; on Linux use Docker (docs/)"
[ -f "$BINARY" ] || die "no moonrise-server at $BINARY (build it: cargo build --release in server/, then --binary server/target/release/moonrise-server)"
"$BINARY" --version >/dev/null 2>&1 || file "$BINARY" | grep -q Mach-O || die "$BINARY isn't a macOS program"

# ---------------------------------------------------------------- ffmpeg
# A static ffmpeg 9.0.2 build with VideoToolbox, libx264 and zimg (GPL; signed and notarized by
# its builder, https://ffmpeg.martin-riedl.de), pinned by checksum. Homebrew's ffmpeg works too
# (it lacks zimg: HDR is then tone-mapped by VideoToolbox only, which every supported Mac does).
FF_BASE=https://ffmpeg.martin-riedl.de/download/macos
if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ]; then
  FF_PATH=arm64/1789931890_9.0.2
  FF_SUMS="ffmpeg=c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924 ffprobe=fcbe839537485eaee7a7a8bc5cbc0f90d53617e80943e8a5b2e31cb851197ea6"
else
  FF_PATH=amd64/1789931006_9.0.2
  FF_SUMS="ffmpeg=7c6b4125b191cbf773832dc51f424cf2b6bb7da43007d1e066f95909e47cacd4 ffprobe=2322438ed2f6319a691291b247d09c69dcaa3a982460d1f269a7e1af335cfdfd"
fi

has_vt() { "$1" -hide_banner -encoders 2>/dev/null | grep -q h264_videotoolbox; }

fetch_ffmpeg() {
  local tmp; tmp=$(mktemp -d)
  for pair in $FF_SUMS; do
    local tool=${pair%%=*} sum=${pair#*=}
    say "downloading $tool ($FF_BASE/$FF_PATH)"
    curl -fsSL --retry 3 -o "$tmp/$tool.zip" "$FF_BASE/$FF_PATH/$tool.zip" || die "download failed: $tool"
    [ "$(shasum -a 256 "$tmp/$tool.zip" | cut -d' ' -f1)" = "$sum" ] || die "$tool.zip: checksum mismatch"
    unzip -oq "$tmp/$tool.zip" -d "$tmp" && install -m 755 "$tmp/$tool" "$APP_DIR/$tool"
  done
  rm -rf "$tmp"
  FFMPEG="$APP_DIR/ffmpeg"
}

mkdir -p "$APP_DIR" "$DATA_DIR" "$CACHE_DIR" "$LOG_DIR"
chmod 700 "$DATA_DIR"

if [ -z "$FFMPEG" ] && [ "$DOWNLOAD" = 0 ]; then
  if [ -x "$APP_DIR/ffmpeg" ] && has_vt "$APP_DIR/ffmpeg"; then FFMPEG="$APP_DIR/ffmpeg"     # kept from an earlier install
  else
    for c in /opt/homebrew/bin/ffmpeg /usr/local/bin/ffmpeg; do
      if [ -x "$c" ] && has_vt "$c"; then FFMPEG=$c; break; fi
    done
  fi
fi
[ -z "$FFMPEG" ] && fetch_ffmpeg
[ -x "$FFMPEG" ] || die "no ffmpeg at $FFMPEG"
FFPROBE="$(dirname "$FFMPEG")/ffprobe"
[ -x "$FFPROBE" ] || die "no ffprobe next to $FFMPEG"
has_vt "$FFMPEG" || say "warning: $FFMPEG has no VideoToolbox encoder: transcoding will use the CPU"
say "ffmpeg: $FFMPEG ($("$FFMPEG" -hide_banner -version | head -1 | cut -d' ' -f1-3))"

# ---------------------------------------------------------------- the program
PLIST_AGENT="$HOME/Library/LaunchAgents/$LABEL.plist"
PLIST_DAEMON="/Library/LaunchDaemons/$LABEL.plist"
# No one logged in to the desktop (a Mac managed over SSH): the user's background session instead.
GUI=1
launchctl print "gui/$(id -u)" >/dev/null 2>&1 || GUI=0
stop_service() {
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
  launchctl bootout "user/$(id -u)/$LABEL" 2>/dev/null || true
  if [ -f "$PLIST_DAEMON" ]; then sudo launchctl bootout "system/$LABEL" 2>/dev/null || true; fi
}
stop_service
install -m 755 "$BINARY" "$APP_DIR/moonrise-server"
[ -f "$HERE/uninstall.sh" ] && install -m 755 "$HERE/uninstall.sh" "$APP_DIR/uninstall.sh"
# A copy downloaded with a browser is quarantined; this script is how you said you trust it.
xattr -dr com.apple.quarantine "$APP_DIR" 2>/dev/null || true
say "installed $("$APP_DIR/moonrise-server" --version 2>/dev/null || echo moonrise-server) in $APP_DIR"

xml() { sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' <<<"$1"; }
plist() {
  cat <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
$( [ "$AT_BOOT" = 1 ] && printf '  <key>UserName</key><string>%s</string>\n' "$(id -un)" )
$( [ "$AT_BOOT" = 0 ] && [ "$GUI" = 0 ] && printf '  <key>LimitLoadToSessionType</key><string>Background</string>\n' )
  <key>ProgramArguments</key>
  <array><string>$(xml "$APP_DIR/moonrise-server")</string><string>serve</string></array>
  <key>WorkingDirectory</key><string>$(xml "$DATA_DIR")</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HOME</key><string>$(xml "$HOME")</string>
    <key>MOONRISE_LISTEN</key><string>0.0.0.0:$PORT</string>
    <key>MOONRISE_DATA_DIR</key><string>$(xml "$DATA_DIR")</string>
    <key>MOONRISE_CACHE_DIR</key><string>$(xml "$CACHE_DIR")</string>
    <key>MOONRISE_FFMPEG</key><string>$(xml "$FFMPEG")</string>
    <key>MOONRISE_FFPROBE</key><string>$(xml "$FFPROBE")</string>
    <key>MOONRISE_LOG_FILE</key><string>$(xml "$LOG_DIR/server.log")</string>
    <key>MOONRISE_RESTART</key><string>exit</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>ProcessType</key><string>Standard</string>
  <key>SoftResourceLimits</key><dict><key>NumberOfFiles</key><integer>8192</integer></dict>
  <key>StandardOutPath</key><string>$(xml "$LOG_DIR/launchd.log")</string>
  <key>StandardErrorPath</key><string>$(xml "$LOG_DIR/launchd.log")</string>
</dict>
</plist>
EOF
}

if [ "$AT_BOOT" = 1 ]; then
  rm -f "$PLIST_AGENT"
  plist | sudo tee "$PLIST_DAEMON" >/dev/null
  sudo chown root:wheel "$PLIST_DAEMON" && sudo chmod 644 "$PLIST_DAEMON"
  sudo launchctl bootstrap system "$PLIST_DAEMON"
  say "started at boot from now on (LaunchDaemon, as $(id -un))"
else
  [ -f "$PLIST_DAEMON" ] && { sudo rm -f "$PLIST_DAEMON"; }
  mkdir -p "$(dirname "$PLIST_AGENT")"
  plist > "$PLIST_AGENT"
  if [ "$GUI" = 1 ]; then
    launchctl bootstrap "gui/$(id -u)" "$PLIST_AGENT"
    say "starts when you log in (LaunchAgent)"
  else
    launchctl bootstrap "user/$(id -u)" "$PLIST_AGENT"
    say "no one is logged in to this Mac's desktop: started in your background session."
    say "It won't start by itself after a restart until you log in; to start at boot, run: $0 --at-boot"
  fi
fi

# ---------------------------------------------------------------- up?
say "waiting for the server on port $PORT"
for _ in $(seq 1 60); do
  curl -fs "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -fs "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 || die "the server didn't start: see $LOG_DIR/server.log"
IF=$(route -n get default 2>/dev/null | awk '/interface:/{print $2}')
IP=$(ipconfig getifaddr "${IF:-en0}" 2>/dev/null || ipconfig getifaddr en0 2>/dev/null || echo "<this Mac>")
CODE=$(grep -o 'setup code: [0-9 ]*' "$LOG_DIR/server.log" 2>/dev/null | tail -1 | cut -d: -f2 | xargs || true)
HW=$(grep -o 'transcoding with .*' "$LOG_DIR/server.log" 2>/dev/null | tail -1 | sed 's/ backend=.*//' || true)
echo
echo "  Moonrise server is running: http://$IP:$PORT/"
[ -n "$HW" ] && echo "  ${HW}"
if [ -n "$CODE" ] && curl -fs "http://127.0.0.1:$PORT/v1/info" | grep -q '"setup":"unclaimed"'; then
  echo "  Setup code: $CODE   (open the Moonrise app, pick this server, enter the code)"
fi
echo "  Log: $LOG_DIR/server.log    Uninstall: \"$APP_DIR/uninstall.sh\""
