#!/usr/bin/env bash
# Install the last sim build on the shared simulator and launch it signed in to the Moonrise server.
# Credentials go MOONRISE_CREDS_FILE or scripts/local/creds.sh -> ssh stdin -> SIMCTL_CHILD_* env; never written to disk.
# Usage: scripts/sim.sh [--reset]   (--reset wipes app data first; APP=<path on the Mac> installs another build;
#        MARK=<item id>=watched|unwatched marks it 8 s after launch (changes the account's watch state);
#        REPORT=1 lets playback report progress to the server: it changes the account's watch state)
#        EXTPLAY="8:on,16:sub=t:3,24:sub=none,30:off" simulates AirPlay (seconds after playback first starts)
#        GIVEUP=<secs>: stream recovery gives up after that long without the server (default 60)
#        EPISODES=<secs>: opens the player's Episodes sheet that long after the player shows
#        VERSION=<file id>: switches the playing title to that version 6 s after it starts (as its menu does)
#        MANAGE=libraries|users|services|playback|activity|about|log opens Manage Server there (admins)
#        SCRUB=<secs>: the scrubber shows its thumbnail preview there, as if dragged (with PIN=1)
#        DEV=1: the Dev app (Moonrise Dev, <bundle id>.dev, from scripts/build.sh sim with DEV=1)
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
SIM=${SIM:-$SIM_PHONE}
need_mac
# No simulator set: the one scripts/sim-create.sh makes ("Moonrise iPhone"), made on first use.
if [ -z "$SIM" ]; then
  SIM=$("$MOONRISE_ROOT/scripts/sim-create.sh" phone)
  echo "note: no SIM_PHONE set: using the simulator $SIM (scripts/sim-create.sh phone); set SIM_PHONE=$SIM in scripts/mac.env to keep it" >&2
fi
# Signs in to the Moonrise server (MOONRISE_URL, default MOONRISE_SERVER_URL from mac.env) with MOONRISE_CREDS_FILE (a
# chmod-600 file: user name, then password), else what scripts/local/creds.sh moonrise prints (git-ignored).
# SEED=0 signs in to nothing (a fresh install shows onboarding).
MOONRISE_URL="${MOONRISE_URL:-${MOONRISE_SERVER_URL:-}}"
[ "${SEED:-}" = 0 ] && MOONRISE_URL=
U=; P=
if [ -n "$MOONRISE_URL" ]; then
  if [ -n "${MOONRISE_CREDS_FILE:-}" ]; then { read -r U; read -r P; } < "$MOONRISE_CREDS_FILE"
  elif [ -x "$MOONRISE_ROOT/scripts/local/creds.sh" ]; then { read -r U; read -r P; } < <("$MOONRISE_ROOT/scripts/local/creds.sh" moonrise)
  else echo "no credentials for $MOONRISE_URL: set MOONRISE_CREDS_FILE (a chmod 600 file: user name, then password), add an executable scripts/local/creds.sh that prints them for \`creds.sh moonrise\`, or SEED=0 to sign in to nothing" >&2; exit 1; fi
fi
# AWAY=1: the app's embedded Tailscale node signs in with a fresh ephemeral, pre-authorised auth key
# (`scripts/local/creds.sh tailscale-key`) and the LAN is treated as unreachable.
K=
if [ "${AWAY:-}" = 1 ]; then
  [ -x "$MOONRISE_ROOT/scripts/local/creds.sh" ] || { echo "AWAY=1 needs an executable scripts/local/creds.sh that prints a Tailscale auth key for \`creds.sh tailscale-key\`" >&2; exit 1; }
  K=$("$MOONRISE_ROOT/scripts/local/creds.sh" tailscale-key)
fi
# The launch (sim_launch in common.sh): the simulator is booted fully first, the launch is waited
# for until the app is up (LAUNCH_WAIT, default 90 s), and one that fails or doesn't come up is tried
# once more; still not running, it is an error (exit 1, with simctl's output).
RESET="${1:-}"
PIN="${PIN:-}"
printf '%s\n%s\n%s\n' "$U" "$P" "$K" | ssh "$MAC" "PIN=${PIN:-}; OPEN=${OPEN:-}; NATIVE=${NATIVE:-}; AUTOPLAY=${AUTOPLAY:-}; AT=${AT:-}; AUDIO=${AUDIO:-}; QUALITY=${QUALITY:-}; BURN=${BURN:-}; SEEK=${SEEK:-}; SUB=${SUB:-}; SUBSEARCH=${SUBSEARCH:-}; OVERLAY=${OVERLAY:-}; DL=${DL:-}; PLAYDL=${PLAYDL:-}; DELDL=${DELDL:-}; AWAY=${AWAY:-}; read -r U; read -r P; read -r K
SIM=$SIM; $(sim_launch /tmp/moonrise-stderr.log)
boot
xcrun simctl terminate \$SIM $APP_ID 2>/dev/null || true
[ '$RESET' = --reset ] && xcrun simctl uninstall \$SIM $APP_ID 2>/dev/null || true
xcrun simctl install \$SIM ${APP:-~/$MAC_DIR/build-sim$BUILD_SUFFIX/Build/Products/Debug-iphonesimulator/Moonrise.app}
export SIMCTL_CHILD_MOONRISE_NO_REPORT=$([ "${REPORT:-}" = 1 ] && echo 0 || echo 1) SIMCTL_CHILD_MOONRISE_OPEN_ITEM=${OPEN:-} SIMCTL_CHILD_MOONRISE_DISCOVER=${DISCOVER:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY=${AUTOPLAY:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_AT=${AT:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_AUDIO=${AUDIO:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_QUALITY=${QUALITY:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_BURN=${BURN:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_SEEK=${SEEK:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_SUB=${SUB:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_SUBSEARCH=${SUBSEARCH:-} SIMCTL_CHILD_MOONRISE_LOG_OVERLAY=${OVERLAY:-} SIMCTL_CHILD_MOONRISE_AUTODOWNLOAD=${DL:-} SIMCTL_CHILD_MOONRISE_PLAY_DOWNLOAD=${PLAYDL:-} SIMCTL_CHILD_MOONRISE_DELETE_DOWNLOAD=${DELDL:-} SIMCTL_CHILD_MOONRISE_FORCE_TAILNET=${AWAY:-} SIMCTL_CHILD_MOONRISE_TS_AUTHKEY=\"\$K\" SIMCTL_CHILD_MOONRISE_NATIVE_SUBS=${NATIVE:-} SIMCTL_CHILD_MOONRISE_PIN_CONTROLS=${PIN:-} SIMCTL_CHILD_MOONRISE_SLOW_ANIM=${SLOW:-} SIMCTL_CHILD_MOONRISE_LANDSCAPE=${LAND:-} SIMCTL_CHILD_MOONRISE_LIBRARY='${LIB:-}' SIMCTL_CHILD_MOONRISE_MARK=${MARK:-} SIMCTL_CHILD_MOONRISE_EXTPLAY='${EXTPLAY:-}' SIMCTL_CHILD_MOONRISE_RECOVERY_GIVEUP=${GIVEUP:-} SIMCTL_CHILD_MOONRISE_OPEN_EPISODES=${EPISODES:-} SIMCTL_CHILD_MOONRISE_OPEN_SUBSEARCH=${SUBSEARCHUI:-} SIMCTL_CHILD_MOONRISE_DEMO=${DEMO:-} SIMCTL_CHILD_MOONRISE_MANAGE=${MANAGE:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_VERSION=${VERSION:-} SIMCTL_CHILD_MOONRISE_LOSSLESS=${LOSSLESS:-} SIMCTL_CHILD_MOONRISE_SCRUB_AT=${SCRUB:-}
  export SIMCTL_CHILD_MOONRISE_URL='$MOONRISE_URL' SIMCTL_CHILD_MOONRISE_USER=\"\$U\" SIMCTL_CHILD_MOONRISE_PASS=\"\$P\"
  start"
