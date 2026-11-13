#!/usr/bin/env bash
# The Apple apps on the App Store: store builds to TestFlight, the listing, App Review. Runs on the
# build Mac RELEASE_MAC (below) with fastlane (clients/apple/fastlane) and the App Store Connect API
# key in ASC_ENV there; the review contact comes from REVIEW_ENV there. Nothing secret is in the
# repository. One App Store Connect app for iPhone, iPad, Apple TV and Mac, bundle id
# MOONRISE_BUNDLE_ID (clients/apple/Config/Signing.local.xcconfig). docs/publishing.md has the steps.
#
#   scripts/release-apple.sh ids                 register the bundle ids (app, .widgets, .topshelf) when missing
#   scripts/release-apple.sh profiles [P|all]    App Store profiles for P (ios, tvos, mac): made when missing,
#                                                installed on the Mac
#   scripts/release-apple.sh ios|tvos|mac        a store build: the next build number (one past P's latest on
#                                                TestFlight; BUILD_NUMBER overrides), signed archive, then the
#                                                .ipa (.pkg for the Mac) in ~/<RELEASE_DIR>/build-release-P on the Mac
#   scripts/release-apple.sh validate P          read only: App Store Connect checks that build as an upload would
#   scripts/release-apple.sh upload P            validate, then upload it to TestFlight
#   scripts/release-apple.sh next-build P        read only: the build number the next P build gets
#   scripts/release-apple.sh app                 read only: whether the App Store Connect app record exists
#   scripts/release-apple.sh builds              read only: the latest uploads, their processing, TestFlight groups
#   scripts/release-apple.sh status              read only: versions, attached builds, texts, screenshots
#   scripts/release-apple.sh check-metadata      the listing's texts against App Store limits (no Mac needed)
#   scripts/release-apple.sh metadata [P,P..]    texts, URLs, categories, App Review notes and contact (default all)
#   scripts/release-apple.sh screenshots [P,P..] the screenshots in clients/apple/fastlane/screenshots
#   scripts/release-apple.sh testers             the internal TestFlight group "Team" (every build, the team's admins)
#   scripts/release-apple.sh review [P,P..] [ios_build:N] [tvos_build:N] [mac_build:N]
#                                                review notes and contact, and the build, on the editable versions
#   scripts/release-apple.sh submit P,P.. --yes  submit for App Review (manual release once approved)
#
# The Mac: RELEASE_MAC (user@host; the environment wins over scripts/mac.env), the build Mac that has
# the distribution certificates. MAC and FASTLANE_MAC are never used here: they may name someone's own
# Mac. Hosts in PERSONAL_MACS (scripts/mac.env, space-separated, user@ optional) are refused.
#
# The store version is MARKETING_VERSION in clients/apple/project.yml (MOONRISE_VERSION overrides).
# The builds happen in their own copy on the Mac, ~/$RELEASE_DIR (default moonrise-release), with
# TailscaleKit there (made the first time with scripts/build-tailscalekit.sh). Store builds come from
# a clean tree (ALLOW_DIRTY=1 allows uncommitted changes, for trying the pipeline).
set -euo pipefail
ASKED=${RELEASE_MAC:-}; . "$(dirname "${BASH_SOURCE[0]}")/common.sh"
no_dev scripts/release-apple.sh
DIR="$MOONRISE_ROOT"
RELEASE_MAC=${ASKED:-${RELEASE_MAC:-}}; PERSONAL_MACS=${PERSONAL_MACS:-}
# common.sh set MAC from MAC (or to local on a Mac, with ssh/rsync/scp run here): none of that applies.
unset -f ssh rsync scp 2>/dev/null || true
MAC=$RELEASE_MAC; FASTLANE_MAC=; TSKIT_MAC=
MAC_DIR=${RELEASE_DIR:-moonrise-release}
# release_mac_ok: RELEASE_MAC is set, is user@host, and isn't in PERSONAL_MACS. Prints why not.
host_of() { local h=${1##*@}; printf '%s\n' "$h" | tr '[:upper:]' '[:lower:]'; }
release_mac_ok() {
  local p
  if [ -z "$RELEASE_MAC" ]; then
    echo "error: no release Mac: set RELEASE_MAC=user@host (the build Mac with the distribution certificates) in scripts/mac.env or the environment; MAC and FASTLANE_MAC aren't used for releases" >&2
    return 1
  fi
  if [ "$RELEASE_MAC" = local ]; then
    echo "error: RELEASE_MAC=local isn't supported: name the Mac as user@host (Remote Login on)" >&2
    return 1
  fi
  for p in $PERSONAL_MACS; do
    if [ "$(host_of "$p")" = "$(host_of "$RELEASE_MAC")" ]; then
      echo "error: RELEASE_MAC $RELEASE_MAC is listed in PERSONAL_MACS: releases never run on a personal Mac" >&2
      return 1
    fi
  done
}
REVIEW_ENV=${REVIEW_ENV:-}

usage() { echo "usage: scripts/release-apple.sh ids|profiles|ios|tvos|mac|validate|upload|next-build|app|builds|status|check-metadata|metadata|screenshots|testers|review|submit (scripts/release-apple.sh --help)" >&2; exit 2; }
platform() { case "${1:-}" in ios|tvos|mac) printf '%s\n' "$1" ;; *) echo "error: platform ios, tvos or mac (not '${1:-}')" >&2; exit 2 ;; esac; }
platforms() { # a comma list of platforms, each checked
  local p list=${1:-ios,tvos,mac}
  for p in ${list//,/ }; do platform "$p" >/dev/null; done
  printf '%s\n' "$list"
}

# The store version: MARKETING_VERSION in project.yml.
marketing_version() {
  [ -n "${MOONRISE_VERSION:-}" ] && { printf '%s\n' "$MOONRISE_VERSION"; return; }
  sed -n 's/^ *MARKETING_VERSION: *"\{0,1\}\([0-9.]*\)"\{0,1\} *$/\1/p' "$DIR/$APPLE_DIR/project.yml" 2>/dev/null | head -1 || true
}
VERSION=$(marketing_version)

# check_metadata <dir>: the texts deliver uploads (dir = clients/apple/fastlane/metadata), against App
# Store Connect's limits, in characters. Prints each problem; exit 1 when there is one.
# (Characters need a UTF-8 locale: C.UTF-8 on Linux, en_US.UTF-8 on macOS.)
U8=C.UTF-8; [ "$(uname -s)" != Darwin ] || U8=en_US.UTF-8
check_metadata() {
  local dir=$1 bad=0 f limit n loc
  for loc in "$dir"/*/; do
    loc=${loc%/}
    case "$(basename "$loc")" in review_information|trade_representative_contact_information) continue ;; esac
    for f in name:30 subtitle:30 keywords:100 promotional_text:170 description:4000 release_notes:4000; do
      limit=${f#*:}; f=$loc/${f%%:*}.txt
      if [ ! -f "$f" ]; then
        case "$(basename "$f")" in name.txt|description.txt|keywords.txt) echo "missing: ${f#"$dir"/}"; bad=1 ;; esac
        continue
      fi
      n=$(tr -d '\n' < "$f" | sed 's/[[:space:]]*$//' | LC_ALL=$U8 wc -m)
      [ "$n" -le "$limit" ] || { echo "too long: ${f#"$dir"/} has $n characters (App Store limit $limit)"; bad=1; }
      [ "$n" -gt 0 ] || { echo "empty: ${f#"$dir"/}"; bad=1; }
    done
    for f in support_url marketing_url privacy_url; do
      [ -f "$loc/$f.txt" ] || continue
      grep -qE '^https://[^[:space:]]+$' "$loc/$f.txt" || { echo "not an https URL: $(basename "$loc")/$f.txt"; bad=1; }
    done
    if [ -f "$loc/keywords.txt" ] && grep -qE ', | ,' "$loc/keywords.txt"; then
      echo "keywords: no spaces around the commas (they count against the 100): $(basename "$loc")/keywords.txt"; bad=1
    fi
  done
  f=$dir/../review_notes.txt
  if [ ! -f "$f" ]; then echo "missing: review_notes.txt"; bad=1
  else n=$(LC_ALL=$U8 wc -m < "$f"); [ "$n" -le 4000 ] || { echo "too long: review_notes.txt has $n characters (limit 4000)"; bad=1; }; fi
  return $bad
}

# The fastlane value KEY=<n> in output with colour codes.
fastlane_value() { sed -E 's/\x1b\[[0-9;]*m//g' | grep -oE "$1=[0-9]+" | tail -1 | cut -d= -f2; }

CMD=${1:-}; shift || true
case "$CMD" in
  ""|help) usage ;;
  check-metadata)
    check_metadata "$DIR/$APPLE_DIR/fastlane/metadata" && echo "metadata: within App Store limits"
    exit ;;
esac

release_mac_ok || exit 1
need_signing scripts/release-apple.sh
need "$ASC_ENV" "no App Store Connect key: set ASC_ENV in scripts/mac.env (a file on the Mac exporting ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH, APPLE_TEAM_ID)"
[ -n "$VERSION" ] || { echo "error: no MARKETING_VERSION in $APPLE_DIR/project.yml" >&2; exit 1; }

# On the Mac: the keychain unlocked, fastlane's environment, the key, the app's id and version.
SOURCE_REVIEW=; [ -z "$REVIEW_ENV" ] || SOURCE_REVIEW=". $REVIEW_ENV;"
KEY_PATH=; [ -z "$ASC_KEY_PATH" ] || KEY_PATH="export ASC_KEY_PATH=$ASC_KEY_PATH"
PRELUDE="$MAC_UNLOCK
$MAC_PATH; export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 FASTLANE_SKIP_UPDATE_CHECK=1 FASTLANE_HIDE_TIMESTAMP=1
set -a; . $ASC_ENV; $SOURCE_REVIEW set +a
$KEY_PATH
export MOONRISE_BUNDLE_ID=$APP_ID MOONRISE_VERSION=$VERSION
set -e; cd ~/$MAC_DIR"
sync_tree() {
  rsync_retry -a --delete --exclude .git --exclude /private --exclude server/tools/local --exclude "build-*" --exclude Moonrise.xcodeproj \
    --exclude "*.log" --exclude Vendor --exclude server/target --exclude /site --exclude /art "$DIR/" "$MAC:~/$MAC_DIR/"
}
# fl <lane> [key:value...]: fastlane on the Mac, in the synced copy.
fl() {
  local args; args=$(printf '%q ' "$@")
  ssh "${MAC_SSH_OPTS[@]}" "$MAC" "$PRELUDE; cd $APPLE_DIR && fastlane $args"
}
next_build() { # <platform>: BUILD_NUMBER, else one past the latest on TestFlight
  if [ -n "${BUILD_NUMBER:-}" ]; then printf '%s\n' "$BUILD_NUMBER"; return; fi
  local out n
  out=$(fl next_build "platform:$1" 2>&1) || { echo "$out" | tail -15 >&2; echo "error: couldn't read $1's latest TestFlight build (no app record yet? BUILD_NUMBER=<n> sets one)" >&2; exit 1; }
  n=$(fastlane_value MOONRISE_LATEST_BUILD <<<"$out")
  [ -n "$n" ] || { echo "$out" | tail -15 >&2; echo "error: no build number in fastlane's answer" >&2; exit 1; }
  echo $((n + 1))
}
clean_tree() {
  [ "${ALLOW_DIRTY:-}" = 1 ] && { echo "note: ALLOW_DIRTY=1: building with uncommitted changes" >&2; return; }
  [ -z "$(git -C "$DIR" status --porcelain --untracked-files=no 2>/dev/null)" ] ||
    { echo "error: uncommitted changes: store builds come from a commit (ALLOW_DIRTY=1 to try the pipeline)" >&2; exit 1; }
}

case "$CMD" in
  ids) sync_tree; fl register_ids 2>&1 | grep -E 'MOONRISE_BUNDLE|\[!\]|rror' ;;
  app) sync_tree; fl app_record 2>&1 | grep -E 'MOONRISE_APP|\[!\]' ;;
  next-build) P=$(platform "${1:-}"); sync_tree; next_build "$P" ;;
  profiles)
    sync_tree
    for P in $( [ "${1:-all}" = all ] && echo ios tvos mac || platform "$1"); do
      fl store_profiles "platform:$P" 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g' | grep -E 'MOONRISE_PROFILE|\[!\]|rror' || { echo "error: no $P profiles" >&2; exit 1; }
    done ;;
  builds|status) sync_tree; fl "$([ "$CMD" = status ] && echo store_status || echo builds)" ;;
  metadata)
    check_metadata "$DIR/$APPLE_DIR/fastlane/metadata" || exit 1
    [ -n "$REVIEW_ENV" ] || echo "note: no REVIEW_ENV in scripts/mac.env (a file on the Mac exporting MOONRISE_REVIEW_FIRST_NAME, _LAST_NAME, _PHONE, _EMAIL): the texts go up without the App Review notes and contact" >&2
    sync_tree; fl metadata "platforms:$(platforms "${1:-}")" ;;
  screenshots) sync_tree; fl screenshots "platforms:$(platforms "${1:-}")" ;;
  testers) sync_tree; fl team_testers ;;
  review)
    need "$REVIEW_ENV" "no App Review contact: set REVIEW_ENV in scripts/mac.env"
    LIST=ios,tvos,mac; case "${1:-}" in *:*|"") ;; *) LIST=$(platforms "$1"); shift ;; esac
    sync_tree; fl review "platforms:$LIST" "$@" ;;
  submit)
    LIST=$(platforms "${1:-}"); [ "${2:-}" = --yes ] || { echo "error: submitting for App Review is the owner's decision: scripts/release-apple.sh submit $LIST --yes" >&2; exit 2; }
    sync_tree; fl submit "platforms:$LIST" confirm:yes ;;
  ios|tvos|mac)
    P=$CMD
    clean_tree
    need_mac_reachable
    sync_tree
    # Store builds include remote access: TailscaleKit in the release copy, made once.
    if ! ssh "$MAC" "test -d ~/$MAC_DIR/$APPLE_DIR/Vendor/TailscaleKit.xcframework"; then
      echo "==> TailscaleKit for the release copy (once)"
      MAC_DIR=$MAC_DIR TSKIT_MAC=$MAC "$DIR/scripts/build-tailscalekit.sh"
    fi
    BUILD=$(next_build "$P")
    case "$P" in
      ios)  DEST="generic/platform=iOS"; EXT=widgets; EXT_SETTING=MOONRISE_PROFILE_WIDGETS; SIGH=ios ;;
      tvos) DEST="generic/platform=tvOS"; EXT=topshelf; EXT_SETTING=MOONRISE_PROFILE_TOPSHELF; SIGH=tvos ;;
      mac)  DEST="generic/platform=macOS"; EXT=; EXT_SETTING=; SIGH=macos ;;
    esac
    echo "==> $APP_NAME $VERSION ($BUILD) for $P, $APP_ID $(build_stamp)"
    fl store_profiles "platform:$P" 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g' | grep -E 'MOONRISE_PROFILE' || { echo "error: no App Store profiles for $P" >&2; exit 1; }
    APP_PROFILE="$APP_ID AppStore $SIGH"
    PROFILES="<key>$APP_ID</key><string>$APP_PROFILE</string>"
    SETTINGS="'MOONRISE_PROFILE_APP=$APP_PROFILE'"
    if [ -n "$EXT" ]; then
      PROFILES="$PROFILES<key>$APP_ID.$EXT</key><string>$APP_ID.$EXT AppStore $SIGH</string>"
      SETTINGS="$SETTINGS '$EXT_SETTING=$APP_ID.$EXT AppStore $SIGH'"
    fi
    OUT=build-release-$P
    ssh "${MAC_SSH_OPTS[@]}" "$MAC" "$PRELUDE
scripts/xcodegen.sh >/dev/null
rm -rf $OUT && mkdir -p $OUT
xcodebuild archive -project $APPLE_DIR/Moonrise.xcodeproj -scheme Moonrise -configuration Release -destination '$DEST' \
  -archivePath $OUT/Moonrise.xcarchive -derivedDataPath build-release-dd-$P CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM=\$APPLE_TEAM_ID \
  'CODE_SIGN_IDENTITY=Apple Distribution' $SETTINGS CURRENT_PROJECT_VERSION=$BUILD MARKETING_VERSION=$VERSION \
  MOONRISE_BUILD_STAMP=$(build_stamp) > $OUT/archive.log 2>&1 || { grep -E 'error|BUILD FAILED' $OUT/archive.log | sort -u | head -20; echo 'archive failed: ~/$MAC_DIR/$OUT/archive.log'; exit 1; }
cat > $OUT/export.plist <<PL
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\"><dict>
<key>method</key><string>app-store-connect</string>
<key>destination</key><string>export</string>
<key>teamID</key><string>\$APPLE_TEAM_ID</string>
<key>signingStyle</key><string>manual</string>
<key>signingCertificate</key><string>Apple Distribution</string>
<key>installerSigningCertificate</key><string>3rd Party Mac Developer Installer</string>
<key>provisioningProfiles</key><dict>$PROFILES</dict>
<key>manageAppVersionAndBuildNumber</key><false/>
<key>uploadSymbols</key><true/>
</dict></plist>
PL
xcodebuild -exportArchive -archivePath $OUT/Moonrise.xcarchive -exportPath $OUT/export -exportOptionsPlist $OUT/export.plist \
  > $OUT/export.log 2>&1 || { grep -iE 'error|fail' $OUT/export.log | sort -u | head -20; echo 'export failed: ~/$MAC_DIR/$OUT/export.log'; exit 1; }
f=\$(find $OUT/export -maxdepth 1 \( -name '*.ipa' -o -name '*.pkg' \) | head -1)
[ -n \"\$f\" ] || { echo 'no .ipa or .pkg exported'; exit 1; }
echo \"==> ~/$MAC_DIR/\$f (\$(du -h \"\$f\" | cut -f1))\""
    echo "==> $P $VERSION ($BUILD) built; next: scripts/release-apple.sh upload $P" ;;
  validate|upload)
    P=$(platform "${1:-}")
    case "$P" in ios) T=ios ;; tvos) T=appletvos ;; mac) T=macos ;; esac
    sync_tree
    # altool reads the key from API_PRIVATE_KEYS_DIR, as AuthKey_<id>.p8.
    ssh "${MAC_SSH_OPTS[@]}" "$MAC" "$PRELUDE
f=\$(find build-release-$P/export -maxdepth 1 \( -name '*.ipa' -o -name '*.pkg' \) 2>/dev/null | head -1)
[ -n \"\$f\" ] || { echo 'nothing to validate: scripts/release-apple.sh $P first'; exit 1; }
out=\$(API_PRIVATE_KEYS_DIR=\$(dirname \"\$ASC_KEY_PATH\") xcrun altool --validate-app -f \"\$f\" -t $T --apiKey \"\$ASC_KEY_ID\" --apiIssuer \"\$ASC_ISSUER_ID\" 2>&1) || true
echo \"\$out\" | grep -E 'VERIFY|ERROR|rror|No errors' | head -20
echo \"\$out\" | grep -qE 'VERIFY SUCCEEDED|No errors validating' || { echo 'App Store Connect refused it (above)'; exit 1; }"
    if [ "$CMD" = upload ]; then
      F=$(ssh "$MAC" "find ~/$MAC_DIR/build-release-$P/export -maxdepth 1 \( -name '*.ipa' -o -name '*.pkg' \) | head -1")
      fl upload "platform:$P" "path:$F"
      echo "==> $P uploaded: TestFlight processes it in 10-30 min (scripts/release-apple.sh builds)"
    fi ;;
  *) usage ;;
esac
