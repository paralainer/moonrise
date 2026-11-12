#!/usr/bin/env bash
# The pinned toolchains. scripts/xcodegen.sh checks the Mac's Xcode and XcodeGen against
# clients/apple/Config/toolchain.env (another version warns and goes on; an older Xcode major or no
# XcodeGen stops), and scripts/rust-dev.sh builds the dev image from server/rust-toolchain.toml on first use,
# again when its recipe changes, and not otherwise. xcodebuild, xcodegen and docker are stubs.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
mkdir -p "$C/server/docker" "$SCRATCH/bin"
cp "$REPO/server/rust-toolchain.toml" "$C/server/"
cp "$REPO/server/docker/dev.Dockerfile" "$C/server/docker/"
want_xcode=$(sed -n 's/^XCODE_VERSION=//p' "$C/clients/apple/Config/toolchain.env")
want_xg=$(sed -n 's/^XCODEGEN_VERSION=//p' "$C/clients/apple/Config/toolchain.env")
want_rust=$(sed -n 's/^channel *= *"\(.*\)".*/\1/p' "$C/server/rust-toolchain.toml")

# ---- scripts/xcodegen.sh
cat > "$SCRATCH/bin/xcodebuild" <<STUB
#!/bin/sh
printf 'Xcode %s\nBuild version 17C529\n' "\$XB"
STUB
cat > "$SCRATCH/bin/xcodegen" <<STUB
#!/bin/sh
case "\$1" in
  --version) echo "Version: \$XG" ;;
  generate) echo generated >> "$SCRATCH/xcodegen-calls" ;;
esac
STUB
chmod +x "$SCRATCH/bin/xcodebuild" "$SCRATCH/bin/xcodegen"
xg() { rm -f "$SCRATCH/xcodegen-calls"; env PATH="$SCRATCH/bin:$PATH" XB="$1" XG="$2" bash "$C/scripts/xcodegen.sh" 2>&1; }
generated() { [ -f "$SCRATCH/xcodegen-calls" ]; }

out=$(xg "$want_xcode" "$want_xg"); rc=$?
expect_status 0 $rc "xcodegen.sh with the recorded versions" "$out"
expect_not_in "warning" "$out" "no warning with the recorded versions"
generated && ok || fail "xcodegen generate didn't run"

out=$(xg 99.1 "$want_xg"); rc=$?
expect_status 0 $rc "a newer Xcode only warns" "$out"
expect_in "warning: Xcode 99.1 here; the apps are built and tested with Xcode $want_xcode" "$out" "newer Xcode warning"
generated && ok || fail "xcodegen generate didn't run with a newer Xcode"

out=$(xg 16.4 "$want_xg"); rc=$?
expect_status 1 $rc "an older Xcode major stops" "$out"
expect_in "error: Xcode 16.4 is too old" "$out" "older Xcode error"
generated && fail "xcodegen generate ran with a too-old Xcode" || ok

out=$(xg "$want_xcode" 2.30.0); rc=$?
expect_status 0 $rc "another XcodeGen only warns" "$out"
expect_in "warning: XcodeGen 2.30.0 here; the project is generated with XcodeGen $want_xg" "$out" "XcodeGen warning"

# TailscaleKit: found in clients/apple/Vendor; one left in the old place, Vendor/ at the
# repository root, stops with the command that moves it.
cat > "$SCRATCH/bin/xcodegen" <<STUB
#!/bin/sh
case "\$1" in
  --version) echo "Version: \$XG" ;;
  generate) echo "generated tailscale=\$MOONRISE_TAILSCALE" >> "$SCRATCH/xcodegen-calls" ;;
esac
STUB
out=$(xg "$want_xcode" "$want_xg"); rc=$?
expect_in "Vendor/TailscaleKit.xcframework not found" "$out" "no TailscaleKit: a note"
expect_in "tailscale=NO" "$(cat "$SCRATCH/xcodegen-calls")" "no TailscaleKit: generated without it"
mkdir -p "$C/Vendor/TailscaleKit.xcframework"
out=$(xg "$want_xcode" "$want_xg"); rc=$?
expect_status 1 $rc "TailscaleKit in the old place stops" "$out"
expect_in "mv '$C/Vendor' '$C/clients/apple/Vendor'" "$out" "says how to move it"
generated && fail "xcodegen generate ran with TailscaleKit in the old place" || ok
mv "$C/Vendor" "$C/clients/apple/Vendor"
out=$(xg "$want_xcode" "$want_xg"); rc=$?
expect_status 0 $rc "TailscaleKit in clients/apple/Vendor" "$out"
expect_in "tailscale=YES" "$(cat "$SCRATCH/xcodegen-calls")" "generated with TailscaleKit"
rm -rf "$C/clients/apple/Vendor"

rm "$SCRATCH/bin/xcodegen"
out=$(env PATH="$SCRATCH/bin:/usr/bin:/bin" XB="$want_xcode" bash "$C/scripts/xcodegen.sh" 2>&1); rc=$?
expect_status 1 $rc "no XcodeGen stops" "$out"
expect_in "error: xcodegen not found: install XcodeGen $want_xg" "$out" "says what to install"

# ---- scripts/rust-dev.sh (docker stub: images are label files in $SCRATCH/images)
mkdir -p "$SCRATCH/images"
cat > "$SCRATCH/bin/docker" <<STUB
#!/usr/bin/env bash
echo "docker \$*" >> "$SCRATCH/docker-calls"
case "\$1 \$2" in
  "image inspect") f="$SCRATCH/images/\$(echo "\${@: -1}" | tr '/:' '__')"; [ -f "\$f" ] && cat "\$f" || exit 1 ;;
  build*) tag=; label=
     while [ \$# -gt 0 ]; do case "\$1" in -t) tag=\$2; shift ;; --label) label=\${2#*=}; shift ;; esac; shift; done
     echo "\$label" > "$SCRATCH/images/\$(echo "\$tag" | tr '/:' '__')" ;;
  run*) echo ran ;;
esac
STUB
chmod +x "$SCRATCH/bin/docker"
rd() { rm -f "$SCRATCH/docker-calls"; env -u RUST_DEV_IMAGE -u CARGO_VOLUME -u CARGO_TARGET_VOLUME PATH="$SCRATCH/bin:$PATH" "$@" 2>&1; }
builds() { grep -c '^docker build' "$SCRATCH/docker-calls" 2>/dev/null || true; }

out=$(rd bash "$C/scripts/rust-dev.sh" cargo test --locked); rc=$?
expect_status 0 $rc "rust-dev.sh on a fresh machine" "$out"
calls=$(cat "$SCRATCH/docker-calls")
expect_in "--build-arg RUST_VERSION=$want_rust" "$calls" "builds from the pinned version"
expect_in "-t moonrise-rust-dev:$want_rust" "$calls" "names the image after the version"
expect_in "-v moonrise-cargo:/usr/local/cargo/registry -v moonrise-test-target:/target" "$calls" "default volumes"
expect_in "moonrise-rust-dev:$want_rust cargo test --locked" "$calls" "runs the command in the image"

out=$(rd bash "$C/scripts/rust-dev.sh" cargo clippy); rc=$?
expect_status 0 $rc "rust-dev.sh again" "$out"
[ "$(builds)" = 0 ] && ok || fail "rebuilt an up-to-date image" "$(cat "$SCRATCH/docker-calls")"

out=$(rd env RUST_DEV_IMAGE=other-dev:x CARGO_VOLUME=c1 CARGO_TARGET_VOLUME=t1 bash "$C/scripts/rust-dev.sh" --image); rc=$?
expect_status 0 $rc "rust-dev.sh --image with overrides" "$out"
expect_in "other-dev:x" "$out" "prints the image"
expect_in "-t other-dev:x" "$(cat "$SCRATCH/docker-calls")" "builds the overridden image"
expect_not_in "docker run" "$(cat "$SCRATCH/docker-calls")" "--image runs nothing"

echo "RUN true" >> "$C/server/docker/dev.Dockerfile"
out=$(rd bash "$C/scripts/rust-dev.sh" cargo test); rc=$?
[ "$(builds)" = 1 ] && ok || fail "didn't rebuild after the Dockerfile changed" "$(cat "$SCRATCH/docker-calls")"
done_testing
