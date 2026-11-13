# Sourced by the scripts here. Builds run on a Mac over SSH (this repository's tooling was made on
# Linux with a Mac as the build machine); everything machine-specific is in scripts/mac.env
# (git-ignored; copy scripts/mac.env.example), everything app-specific in
# clients/apple/Config/Signing*.xcconfig.
#
# A fresh clone has neither personal file, and sourcing this doesn't fail for that: the scripts get
# the public defaults (bundle id org.example.moonrise, no team, no build Mac), and only the steps
# that need more ask for it: need_mac (anything over SSH) and need_signing (device installs, OTA,
# TestFlight). `<script> --help` prints the script's header comment.
MOONRISE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The Apple apps' project (project.yml, Config/, the targets), relative to the repository root.
APPLE_DIR=clients/apple

case "${1:-}" in
  -h|--help) awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;;
esac

# The build Mac: scripts/mac.env (see scripts/mac.env.example for each name). Without it every
# name still gets a default here, as the scripts run under set -u.
# Device ids given in the environment win over mac.env's (PHONE_DEVICE=<id> scripts/deploy-phone.sh).
ENV_PHONE_DEVICE=${PHONE_DEVICE:-}; ENV_IPAD_DEVICE=${IPAD_DEVICE:-}; ENV_TV_DEVICE=${TV_DEVICE:-}
ENV_RELEASE_MAC=${RELEASE_MAC:-}
if [ -f "$MOONRISE_ROOT/scripts/mac.env" ]; then
  . "$MOONRISE_ROOT/scripts/mac.env"
fi
PHONE_DEVICE=${ENV_PHONE_DEVICE:-${PHONE_DEVICE:-}}; IPAD_DEVICE=${ENV_IPAD_DEVICE:-${IPAD_DEVICE:-}}; TV_DEVICE=${ENV_TV_DEVICE:-${TV_DEVICE:-}}
MAC=${MAC:-}
# The Mac for signed distribution builds (release-apple.sh, store-shots.sh, ota.sh; use_release_mac
# below): the environment's RELEASE_MAC wins over mac.env's.
RELEASE_MAC=${ENV_RELEASE_MAC:-${RELEASE_MAC:-}}; PERSONAL_MACS=${PERSONAL_MACS:-}
MAC_DIR_GIVEN=${MAC_DIR:-}
MAC_DIR=${MAC_DIR:-video-player}
SIM_PHONE=${SIM_PHONE:-}; SIM_TV=${SIM_TV:-}
MAC_PATH=${MAC_PATH:-'export PATH=$HOME/bin:/usr/local/bin:/opt/homebrew/bin:$PATH'}
MAC_UNLOCK=${MAC_UNLOCK:-}
ASC_ENV=${ASC_ENV:-}; ASC_KEY_PATH=${ASC_KEY_PATH:-}
TSKIT_MAC=${TSKIT_MAC:-$MAC}
MOONRISE_SERVER_URL=${MOONRISE_SERVER_URL:-}
SHOT_DIR=${SHOT_DIR:-/tmp/moonrise-shots}

# Local mode, for running the scripts on a Mac itself: MAC=local (the default on macOS when no MAC
# is set). What the scripts would send over SSH runs in a local shell (from the home folder, as
# over SSH). No Remote Login needed. MOONRISE_UNAME stands in for uname -s in the scripts' tests.
# Where it builds: unless MAC_DIR is set, in the checkout itself (the build folders and the
# generated project are git-ignored; TailscaleKit, built here, is found); a checkout outside the
# home folder builds in its own copy, ~/moonrise-build. With MAC_DIR set, the tree is copied to
# ~/$MAC_DIR with a local rsync as it would be to a remote Mac (skipped when that is the checkout),
# but never into a folder that exists and isn't an earlier build copy (an rsync --delete there
# would overwrite it): never into some other ~/video-player.
if [ -z "$MAC" ] && [ "${MOONRISE_UNAME:-$(uname -s 2>/dev/null)}" = Darwin ]; then
  MAC=local
fi
if [ "$MAC" = local ] && [ -z "$MAC_DIR_GIVEN" ]; then
  case "$MOONRISE_ROOT" in
    "$HOME"/?*) MAC_DIR=${MOONRISE_ROOT#"$HOME"/} ;;
    *) MAC_DIR=moonrise-build ;;
  esac
fi
# local_path <arg>: "local:~/x" -> "$HOME/x", "local:/x" -> "/x"; anything else as it is.
local_path() {
  case "$1" in
    'local:~/'*) printf '%s\n' "$HOME/${1#local:\~/}" ;;
    'local:~') printf '%s\n' "$HOME" ;;
    local:*) printf '%s\n' "${1#local:}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}
if [ "$MAC" = local ]; then
  # ssh [options] local [command]: the command in bash here, from the home folder; stdin is
  # passed on, as ssh does.
  ssh() {
    while [ $# -gt 0 ]; do
      case "$1" in
        -[bcDEeFIiJLlmOoPpQRSWw]) shift 2 ;;
        -*) shift ;;
        *) break ;;
      esac
    done
    shift || true   # the host, "local"
    [ $# -gt 0 ] || return 0
    (cd "$HOME" && bash -c "$*")
  }
  # rsync with local:<path> arguments: the same copy on this machine.
  rsync() {
    local a dest='' args=()
    for a in "$@"; do a=$(local_path "$a"); args+=("$a"); dest=$a; done
    if [ -d "$dest" ] && [ "$(cd "$dest" && pwd -P)" = "$(cd "$MOONRISE_ROOT" && pwd -P)" ]; then
      return 0
    fi
    # A build copy has scripts/common.sh and no .git (the copies leave .git out); anything else
    # that isn't empty (another folder, another checkout) is left alone.
    if [ -d "$dest" ] && [ -n "$(ls -A "$dest" 2>/dev/null)" ] && { [ ! -f "$dest/scripts/common.sh" ] || [ -e "$dest/.git" ]; }; then
      echo "error: $dest exists and isn't a build copy of this repository: not copying into it (set MAC_DIR to a new folder, relative to your home)" >&2
      return 1
    fi
    command rsync "${args[@]}"
  }
  # scp [options] local:<from> <to>: a plain copy.
  scp() {
    while [ $# -gt 0 ]; do case "$1" in -[iFPoJSc]) shift 2 ;; -*) shift ;; *) break ;; esac; done
    local a args=()
    for a in "$@"; do args+=("$(local_path "$a")"); done
    command cp "${args[@]}"
  }
fi

# Signed distribution builds (release-apple.sh and store-shots.sh for the App Store, ota.sh for the
# ad-hoc IPA) run on RELEASE_MAC only, the build Mac with the distribution certificates (user@host).
# MAC and TSKIT_MAC are never used there: they may name someone's own Mac. Hosts in PERSONAL_MACS
# (mac.env, space-separated, user@ optional, any case) are refused.
host_of() { local h=${1##*@}; printf '%s\n' "$h" | tr '[:upper:]' '[:lower:]'; }
# personal_mac <user@host>: whether that host is listed in PERSONAL_MACS.
personal_mac() {
  local p
  for p in $PERSONAL_MACS; do
    [ "$(host_of "$p")" != "$(host_of "$1")" ] || return 0
  done
  return 1
}
# release_mac_ok: RELEASE_MAC is set, is user@host, and isn't in PERSONAL_MACS. Prints why not.
release_mac_ok() {
  if [ -z "$RELEASE_MAC" ]; then
    echo "error: no release Mac: set RELEASE_MAC=user@host (the build Mac with the distribution certificates) in scripts/mac.env or the environment; MAC isn't used for signed distribution builds" >&2
    return 1
  fi
  if [ "$RELEASE_MAC" = local ]; then
    echo "error: RELEASE_MAC=local isn't supported: name the Mac as user@host (Remote Login on)" >&2
    return 1
  fi
  if personal_mac "$RELEASE_MAC"; then
    echo "error: RELEASE_MAC $RELEASE_MAC is listed in PERSONAL_MACS: signed distribution builds never run on a personal Mac" >&2
    return 1
  fi
}
# test_mac_ok: the app's tests (test-apple.sh, check.sh's Apple builds) may run on MAC: it isn't
# listed in PERSONAL_MACS. Prints why not.
test_mac_ok() {
  if [ -n "$MAC" ] && [ "$MAC" != local ] && personal_mac "$MAC"; then
    echo "MAC $MAC is listed in PERSONAL_MACS: the app's tests don't run on a personal Mac (run with MAC=user@host, a build Mac)" >&2
    return 1
  fi
}
# use_release_mac <folder>: from here on, MAC is RELEASE_MAC (checked with release_mac_ok before
# reaching it), the folder there is ~/<folder>, ssh/rsync/scp are the real ones (no local mode), and
# TSKIT_MAC is empty. Exported, so the scripts this one runs (build.sh,
# build-tailscalekit.sh) do the same when they source this file, whatever mac.env says.
use_release_mac() {
  unset -f ssh rsync scp 2>/dev/null || true
  MAC=$RELEASE_MAC; TSKIT_MAC=
  MAC_DIR=$1
  export RELEASE_MAC MOONRISE_RELEASE_DIR=$1
}
if [ -n "${MOONRISE_RELEASE_DIR:-}" ]; then
  use_release_mac "$MOONRISE_RELEASE_DIR"
  release_mac_ok || exit 1
fi

# The app's identity: clients/apple/Config/Signing.local.xcconfig (git-ignored) wins over
# clients/apple/Config/Signing.xcconfig.
# The Apple project was once at the repository root; a git-ignored Signing.local.xcconfig
# left in the old place isn't moved by git, so stop with the command that moves it (rather than
# build the app under the public id).
if [ -f "$MOONRISE_ROOT/Config/Signing.local.xcconfig" ] && [ ! -f "$MOONRISE_ROOT/$APPLE_DIR/Config/Signing.local.xcconfig" ]; then
  echo "error: Config/Signing.local.xcconfig is in its old place: the Apple project moved to $APPLE_DIR/, so move it: mv '$MOONRISE_ROOT/Config/Signing.local.xcconfig' '$MOONRISE_ROOT/$APPLE_DIR/Config/'" >&2
  exit 1
fi
signing_value() {
  local f v
  for f in "$MOONRISE_ROOT/$APPLE_DIR/Config/Signing.local.xcconfig" "$MOONRISE_ROOT/$APPLE_DIR/Config/Signing.xcconfig"; do
    [ -f "$f" ] || continue
    v=$(sed -n "s/^$1 *= *//p" "$f" | head -1 | tr -d '[:space:]')
    if [ -n "$v" ]; then printf '%s\n' "$v"; return 0; fi
  done
  return 0
}
APP_ID=$(signing_value MOONRISE_BUNDLE_ID)
APP_ID=${APP_ID:-org.example.moonrise}
BASE_APP_ID=$APP_ID
APP_TEAM=$(signing_value MOONRISE_DEVELOPMENT_TEAM)
# Two apps side by side on one device. The default is the app itself: Moonrise, the bundle id above.
# DEV=1 is "Moonrise Dev", <bundle id>.dev with the URL scheme moonrise-dev and an amber icon: a
# separate app with its own sign-in, keychain, data, widgets and Top Shelf, for trying out changes and for tests, so the
# real app (and its sign-in) stays as it is. Its builds go to build-<platform>-dev on the Mac, and
# build, sim, tvsim and deploy-* act on it. On Mac it is "Moonrise Dev.app". ota.sh and
# release-apple.sh ship the app itself only, and refuse DEV=1 (no_dev).
if [ "${DEV:-}" = 1 ]; then
  APP_ID="$APP_ID.dev"
  APP_NAME="Moonrise Dev"
  BUILD_SUFFIX=-dev
  # Passed to xcodebuild, after the xcconfig: the bundle id (the extensions follow it), the name, the scheme.
  VARIANT_SETTINGS="MOONRISE_BUNDLE_ID=$APP_ID 'MOONRISE_APP_NAME=$APP_NAME' MOONRISE_URL_SCHEME=moonrise-dev MOONRISE_APP_ICON=AppIcon-Dev MOONRISE_APP_ICON_TV=AppIconTV-Dev"
else
  APP_NAME="Moonrise"
  BUILD_SUFFIX=
  VARIANT_SETTINGS=
fi
if [ ! -f "$MOONRISE_ROOT/$APPLE_DIR/Config/Signing.local.xcconfig" ] && [ -z "${MOONRISE_SIGNING_NOTED:-}" ]; then
  echo "note: no $APPLE_DIR/Config/Signing.local.xcconfig, so the app is $APP_ID with no team (simulator builds only); copy $APPLE_DIR/Config/Signing.local.xcconfig.example to sign for devices" >&2
  export MOONRISE_SIGNING_NOTED=1
fi

# sim_launch <log>: the Mac's side of launching $APP_ID on a simulator (sim.sh, tvsim.sh), as shell
# for their ssh command: it defines boot and start, which use SIM (the simulator, set on the Mac)
# and the SIMCTL_CHILD_* environment set before start. The app's output goes to <log> on the Mac
# (simctl's --stdout/--stderr would put it inside the simulator's own /tmp). simctl launch
# --console-pty blocks for as long as the app runs (and holds back its own "<id>: <pid>" line), so
# it runs in the background, and the launch is waited for until the simulator lists the app as
# running, or simctl exits (a failed launch). The old fixed 2 s wait then one relaunch gave up on
# launches after a simulator's first boot, which took up to a minute on the Intel Mac. A
# launch that fails, or isn't up in LAUNCH_WAIT seconds (default 90), is stopped (by its PID) and
# tried once more; then it is an error: exit 1, with simctl's output.
sim_launch() {
  cat <<EOF
boot() { xcrun simctl list devices 2>/dev/null | grep -qF "\$SIM) (Booted)" || echo 'booting the simulator (a first boot can take minutes)'
  xcrun simctl bootstatus \$SIM -b >/dev/null 2>&1 || xcrun simctl boot \$SIM 2>/dev/null || true; }
launch() { : > $1; nohup xcrun simctl launch --console-pty \$SIM $APP_ID >$1 2>&1 & L=\$!; }
running() { xcrun simctl spawn \$SIM launchctl list 2>/dev/null | grep -qF 'UIKitApplication:$APP_ID['; }
started() { i=0; while [ \$i -lt ${LAUNCH_WAIT:-90} ]; do
    running && { sleep 1; running; return; }
    kill -0 \$L 2>/dev/null || { sleep 1; running; return; }
    [ \$i = 5 ] && echo 'waiting for the app to start (a first launch after a boot can take a minute)'
    sleep 1; i=\$((i + 1)); done
  return 1; }
start() { launch
  started && { echo launched; return; }
  kill \$L 2>/dev/null; echo 'not running: launching again; simctl said:'; head -c 600 $1 | LC_ALL=C tr -cd '[:print:]\n'; echo
  launch
  started && { echo launched; return; }
  kill \$L 2>/dev/null
  echo 'error: $APP_ID did not start on the simulator; simctl said:' >&2; head -c 1500 $1 | LC_ALL=C tr -cd '[:print:]\n' >&2; echo >&2; exit 1; }
EOF
}

# no_dev <script>: for the scripts that ship the app itself (OTA, the App Store), which have no Dev
# variant: stop (exit 2) when DEV=1 is set, rather than build one app and publish under another's id.
no_dev() {
  [ "${DEV:-}" = 1 ] || return 0
  echo "error: $1 ships Moonrise itself, not the Dev app: run it without DEV=1 (the Dev app goes on devices with DEV=1 scripts/deploy-phone.sh, deploy-tv.sh, deploy-mac.sh, deploy-android-tv.sh)" >&2
  exit 2
}

# need <value> <message>: stop with the message when the value is empty.
need() {
  [ -n "$1" ] && return 0
  echo "$2" >&2
  exit 1
}

# For anything that runs on the build Mac.
need_mac() {
  need "$MAC" "no build Mac: copy scripts/mac.env.example to scripts/mac.env and set MAC (or run with MAC=user@host); on a Mac itself the scripts build locally (MAC=local, the default there; docs/building.md)"
}

# For steps that sign the app ($1 names the step): a bundle id of your own, and your team.
need_signing() {
  [ "$BASE_APP_ID" != org.example.moonrise ] && [ -n "$APP_TEAM" ] && return 0
  echo "$1 signs the app, so it needs your own bundle id and team: copy $APPLE_DIR/Config/Signing.local.xcconfig.example to $APPLE_DIR/Config/Signing.local.xcconfig and set MOONRISE_BUNDLE_ID and MOONRISE_DEVELOPMENT_TEAM (docs/building.md). Simulator builds need neither: scripts/build.sh sim" >&2
  exit 1
}

# The build stamp the apps show next to their version in Settings: the commit (git short sha),
# with -dirty when tracked files have uncommitted changes. scripts/build.sh passes it to xcodebuild.
build_stamp() {
  local sha
  sha=$(git -C "$MOONRISE_ROOT" rev-parse --short HEAD 2>/dev/null) || { echo unknown; return 0; }
  [ -z "$(git -C "$MOONRISE_ROOT" status --porcelain --untracked-files=no 2>/dev/null)" ] || sha="$sha-dirty"
  printf '%s\n' "$sha"
}

# Connecting to the build Mac: a Mac can answer ping and port 22 and still let an SSH
# handshake time out now and then (the banner exchange), and a retry gets through. So the
# reachability check and the copy are tried up to MAC_SSH_TRIES times (default 3), MAC_SSH_TIMEOUT
# seconds each to connect (default 10), waiting MAC_SSH_BACKOFF (default 2) seconds times the
# attempt number in between.
#
# Only these two are retried: `ssh true` (mac_reachable) and rsync (rsync_retry), both harmless to
# run twice. A long or non-idempotent remote command (a build, the tests, an install) is never
# retried: from its exit status and stderr alone ssh can't tell a connection that never opened from
# one dropped mid-run (OpenSSH prints "Connection reset/closed by HOST port 22" in both cases), and a
# second run would start a second build next to the first. Callers run need_mac_reachable right
# before such a command, then one plain ssh (with MAC_SSH_OPTS).
MAC_SSH_TRIES=${MAC_SSH_TRIES:-3}; MAC_SSH_TIMEOUT=${MAC_SSH_TIMEOUT:-10}; MAC_SSH_BACKOFF=${MAC_SSH_BACKOFF:-2}
MAC_SSH_OPTS=(-o ConnectTimeout="$MAC_SSH_TIMEOUT")
# What ssh says when a connection failed before a session began; mac_reachable tries again only on
# these (a refused key or an unknown host fails at once). Safe there because its command is `true`.
SSH_NO_SESSION_RE='banner exchange|kex_exchange_identification|connect to host .* port [0-9]+: |Connection (closed|reset) by .* port [0-9]+'

# rsync_retry <rsync arguments>: rsync over ssh with ConnectTimeout, tried again (as above) when
# the connection failed or broke (rsync exit 255, 12, 10, 30 or 35). A copy is idempotent: a
# second run finishes it.
rsync_retry() {
  local try=1 rc
  while :; do
    rsync -e "ssh -o ConnectTimeout=$MAC_SSH_TIMEOUT" "$@" && return 0 || rc=$?
    case "$rc" in 255|12|10|30|35) ;; *) return "$rc" ;; esac
    [ "$try" -lt "$MAC_SSH_TRIES" ] || return "$rc"
    echo "note: copying to the build Mac failed (rsync exit $rc); trying again in $((try * MAC_SSH_BACKOFF)) s (attempt $((try + 1)) of $MAC_SSH_TRIES)" >&2
    sleep $((try * MAC_SSH_BACKOFF))
    try=$((try + 1))
  done
}

# mac_reachable: whether the build Mac answers over SSH now: `ssh $MAC true`, tried again (as
# above) when ssh exits 255 with a no-session error. Prints the retry notes, and ssh's last error
# when it gives up.
mac_reachable() {
  [ -n "$MAC" ] || return 1
  local try=1 rc err
  while :; do
    err=$(ssh "${MAC_SSH_OPTS[@]}" -o BatchMode=yes "$MAC" true 2>&1 >/dev/null) && return 0 || rc=$?
    if [ "$rc" != 255 ] || ! grep -qE "$SSH_NO_SESSION_RE" <<<"$err" || [ "$try" -ge "$MAC_SSH_TRIES" ]; then
      [ -z "$err" ] || tail -1 <<<"$err" | sed 's/^/  ssh: /' >&2
      return 1
    fi
    echo "note: SSH to the build Mac didn't connect ($(grep -E "$SSH_NO_SESSION_RE" <<<"$err" | tail -1)); trying again in $((try * MAC_SSH_BACKOFF)) s (attempt $((try + 1)) of $MAC_SSH_TRIES)" >&2
    sleep $((try * MAC_SSH_BACKOFF))
    try=$((try + 1))
  done
}

# need_mac_reachable: stop unless the build Mac answers over SSH now (before a long build).
need_mac_reachable() {
  need_mac
  mac_reachable && return 0
  echo "error: can't reach the build Mac $MAC over SSH, $MAC_SSH_TRIES attempts (asleep, off the network, or another MAC=?)" >&2
  exit 1
}

# need_device <device id> <name>: stop unless the build Mac can reach the paired device now, so a
# deploy fails before the build rather than after it. <name> says which setting it came from.
need_device() {
  local out tries=0
  need_mac_reachable
  while :; do
    out=$(ssh "$MAC" "xcrun devicectl device info details --device '$1' --timeout 30 2>&1 >/dev/null") && return 0
    tries=$((tries + 1))
    # A paired device on Wi-Fi often drops the first connection; a second try settles it.
    if [ $tries -ge 2 ] || grep -q 'not found' <<<"$out"; then break; fi
    sleep 3
  done
  echo "$out" | grep -m1 '^ERROR' >&2 || echo "$out" | tail -2 >&2
  echo "error: $2 isn't reachable from $MAC (is it awake, on the network and paired: xcrun devicectl list devices)" >&2
  exit 1
}

# app_built <app path in MAC_DIR> <Info.plist inside it>: prints "<CFBundleVersion> <build stamp>"
# of the app just built on the Mac.
app_built() {
  ssh "$MAC" "cd ~/$MAC_DIR && p='$1/$2' && /usr/libexec/PlistBuddy -c 'Print CFBundleVersion' \"\$p\" && /usr/libexec/PlistBuddy -c 'Print MoonriseBuildStamp' \"\$p\"" | paste -sd' ' -
}

# install_on_device <device id> <name> <app path in MAC_DIR>: install the built app on a paired
# device and check that the device took it. Exits 1 with devicectl's answer when it didn't: a
# non-zero exit, an ERROR line, or no install record (databaseSequenceNumber) for this bundle id.
# Retries once, as device installs often fail once with a reset connection.
install_on_device() {
  local out code tries=0 built version stamp listed
  built=$(app_built "$3" Info.plist) || { echo "error: no built app at $3 on $MAC" >&2; exit 1; }
  version=${built%% *}; stamp=${built#* }
  echo "==> installing $APP_NAME ($APP_ID) $stamp (build $version) on $2"
  while :; do
    set +e
    out=$(ssh "$MAC" "cd ~/$MAC_DIR && xcrun devicectl device install app --device '$1' --timeout 300 '$3' 2>&1")
    code=$?
    set -e
    if [ $code -eq 0 ] && grep -q databaseSequenceNumber <<<"$out" && ! grep -q '^ERROR' <<<"$out" \
        && grep -qF "bundleID: $APP_ID" <<<"$out"; then
      break
    fi
    echo "$out" | grep -v '^[0-9:]*  ' | tail -4 >&2
    if [ $code -eq 0 ] && grep -q 'bundleID: ' <<<"$out" && ! grep -qF "bundleID: $APP_ID" <<<"$out"; then
      echo "error: the build installed as $(sed -n 's/.*bundleID: //p' <<<"$out" | head -1), not $APP_ID ($APPLE_DIR/Config/Signing.local.xcconfig didn't reach the Mac?)" >&2
      exit 1
    fi
    tries=$((tries + 1))
    if [ $tries -ge 2 ]; then
      echo "error: install on $2 failed (exit $code): the device still has its previous build" >&2
      exit 1
    fi
    echo "==> retrying the install" >&2
    sleep 3
  done
  # Where devicectl can list the device's apps, check the version there too. (Older devicectl
  # lists no apps on newer devices; the install record above is then the confirmation.)
  listed=$(ssh "$MAC" "t=\$(mktemp) && xcrun devicectl device info apps --device '$1' --bundle-id '$APP_ID' --timeout 30 --json-output \$t >/dev/null 2>&1; plutil -extract result.apps.0.bundleVersion raw -o \$t.v \$t >/dev/null 2>&1 && cat \$t.v; rm -f \$t \$t.v" || true)
  if [ -n "$listed" ] && [ "$listed" != "$version" ]; then
    echo "error: the install reported success, but $2 has build $listed, not $version" >&2
    exit 1
  fi
  echo "==> $2 now has $APP_NAME $stamp (build $version; install record $(sed -n 's/.*databaseSequenceNumber: //p' <<<"$out" | head -1)$([ -n "$listed" ] && echo ", confirmed by the device's app list"))"
}
