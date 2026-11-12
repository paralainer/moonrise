#!/usr/bin/env bash
# Install the last tvOS sim build on the Apple TV 4K simulator and launch it signed in (like sim.sh).
# Env seams as in sim.sh: OPEN, AUTOPLAY, AT, AUDIO, QUALITY, BURN, SEEK, REPORT, AWAY, LOSSLESS, MOONRISE_URL, MOONRISE_CREDS_FILE, SEED=0; the app's stderr
# is /tmp/moonrise-tv-stderr.log on the Mac. SIM=<udid>: another simulator than SIM_TV. DEV=1: the Dev
# app (Moonrise Dev, <bundle id>.dev, from scripts/build.sh tvos with DEV=1). The launch is sim.sh's
# (sim_launch in common.sh): waited for (LAUNCH_WAIT), tried once more, then an error (exit 1).
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
SIM=${SIM:-$SIM_TV}
need_mac
# No simulator set: the one scripts/sim-create.sh makes ("Moonrise Apple TV"), made on first use.
if [ -z "$SIM" ]; then
  SIM=$("$MOONRISE_ROOT/scripts/sim-create.sh" tv)
  echo "note: no SIM_TV set: using the simulator $SIM (scripts/sim-create.sh tv); set SIM_TV=$SIM in scripts/mac.env to keep it" >&2
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
printf '%s\n%s\n' "$U" "$P" | ssh "$MAC" "read -r U; read -r P
SIM=$SIM; $(sim_launch /tmp/moonrise-tv-stderr.log)
boot
xcrun simctl terminate \$SIM $APP_ID 2>/dev/null || true
export SIMCTL_CHILD_MOONRISE_DV=${DV:-}  # DV=1: the app asks for Dolby Vision (MoonriseServer.dolbyVisionDisplay)
xcrun simctl install \$SIM ${APP:-~/$MAC_DIR/build-tvos$BUILD_SUFFIX/Build/Products/Debug-appletvsimulator/Moonrise.app}
export SIMCTL_CHILD_MOONRISE_NO_REPORT=$([ "${REPORT:-}" = 1 ] && echo 0 || echo 1) SIMCTL_CHILD_MOONRISE_FORCE_TAILNET=${AWAY:-} SIMCTL_CHILD_MOONRISE_OPEN_ITEM=${OPEN:-} SIMCTL_CHILD_MOONRISE_DISCOVER=${DISCOVER:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY=${AUTOPLAY:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_AT=${AT:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_AUDIO=${AUDIO:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_QUALITY=${QUALITY:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_BURN=${BURN:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_SEEK=${SEEK:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_SUB=${SUB:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_SUBSEARCH=${SUBSEARCH:-} SIMCTL_CHILD_MOONRISE_LOG_OVERLAY=${OVERLAY:-} SIMCTL_CHILD_MOONRISE_AUTODOWNLOAD=${DL:-} SIMCTL_CHILD_MOONRISE_PLAY_DOWNLOAD=${PLAYDL:-} SIMCTL_CHILD_MOONRISE_DELETE_DOWNLOAD=${DELDL:-} SIMCTL_CHILD_MOONRISE_LIBRARY='${LIB:-}' SIMCTL_CHILD_MOONRISE_RECOVERY_GIVEUP=${GIVEUP:-} SIMCTL_CHILD_MOONRISE_OPEN_SUBSEARCH=${SUBSEARCHUI:-} SIMCTL_CHILD_MOONRISE_DEMO=${DEMO:-} SIMCTL_CHILD_MOONRISE_MANAGE=${MANAGE:-} SIMCTL_CHILD_MOONRISE_AUTOPLAY_VERSION=${VERSION:-} SIMCTL_CHILD_MOONRISE_LOSSLESS=${LOSSLESS:-}
  export SIMCTL_CHILD_MOONRISE_URL='$MOONRISE_URL' SIMCTL_CHILD_MOONRISE_USER=\"\$U\" SIMCTL_CHILD_MOONRISE_PASS=\"\$P\"
  # stderr (MoonriseTiming, MoonrisePlayerError…) goes to /tmp/moonrise-tv-stderr.log on the Mac.
  start"
