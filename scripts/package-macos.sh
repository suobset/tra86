#!/usr/bin/env bash
#
# Builds, signs, and (optionally) notarizes a distributable macOS package of the
# `bind` CLI. Produces a universal (arm64 + x86_64) binary, code-signs it with
# the Hardened Runtime and Bind's debugger entitlements, wraps it in a
# component .pkg (so it can be stapled), then notarizes and staples the .pkg.
#
# This script performs irreversible, outward-facing actions (uploading to
# Apple's notary service) only when you ask for it. Signing/notarization
# require your own Apple Developer credentials; nothing here is baked in.
#
# Required environment for signing:
#   SIGN_IDENTITY        "Developer ID Application: Your Name (TEAMID)"
#   INSTALLER_IDENTITY   "Developer ID Installer: Your Name (TEAMID)"
# For notarization, either:
#   NOTARY_PROFILE       a `notarytool store-credentials` keychain profile name
#   -- or --
#   APPLE_ID, TEAM_ID, APP_SPECIFIC_PASSWORD
#
# Flags:
#   --skip-notarize   sign + package only (useful for a local signed build)
#   --no-sign         build the universal binary only (no signing/packaging)
#
# Usage:
#   SIGN_IDENTITY=... INSTALLER_IDENTITY=... NOTARY_PROFILE=... \
#     scripts/package-macos.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="${VERSION:-$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
BUNDLE_ID="${BUNDLE_ID:-com.bind.cli}"
DIST="$REPO_ROOT/dist"
BIN_NAME="bind"
ENTITLEMENTS="$REPO_ROOT/packaging/bind.entitlements"

SKIP_NOTARIZE=0
NO_SIGN=0
for arg in "$@"; do
  case "$arg" in
    --skip-notarize) SKIP_NOTARIZE=1 ;;
    --no-sign) NO_SIGN=1 ;;
    *) echo "unknown flag: $arg" >&2; exit 2 ;;
  esac
done

echo "==> Bind $VERSION — macOS packaging"
mkdir -p "$DIST"

# 1. Build a universal release binary.
echo "==> building universal release binary"
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true
cargo build --release -p bind-cli --target aarch64-apple-darwin
cargo build --release -p bind-cli --target x86_64-apple-darwin
UNIVERSAL="$DIST/$BIN_NAME"
lipo -create -output "$UNIVERSAL" \
  "target/aarch64-apple-darwin/release/$BIN_NAME" \
  "target/x86_64-apple-darwin/release/$BIN_NAME"
echo "    $(lipo -archs "$UNIVERSAL") -> $UNIVERSAL"

if [ "$NO_SIGN" -eq 1 ]; then
  echo "==> --no-sign: stopping after the universal binary"
  exit 0
fi

: "${SIGN_IDENTITY:?set SIGN_IDENTITY to your 'Developer ID Application: ...' identity}"

# 2. Code-sign with Hardened Runtime + entitlements + secure timestamp.
echo "==> code-signing (hardened runtime + entitlements)"
codesign --force --timestamp --options runtime \
  --entitlements "$ENTITLEMENTS" \
  --sign "$SIGN_IDENTITY" \
  "$UNIVERSAL"
codesign --verify --strict --verbose=2 "$UNIVERSAL"

# 3. Build a component .pkg installing to /usr/local/bin (stapling needs a
#    container format; a bare binary cannot be stapled).
: "${INSTALLER_IDENTITY:?set INSTALLER_IDENTITY to your 'Developer ID Installer: ...' identity}"
echo "==> building installer package"
PKG_ROOT="$DIST/pkgroot"
rm -rf "$PKG_ROOT"
install -d "$PKG_ROOT/usr/local/bin"
install -m 0755 "$UNIVERSAL" "$PKG_ROOT/usr/local/bin/$BIN_NAME"
PKG="$DIST/bind-$VERSION.pkg"
pkgbuild --root "$PKG_ROOT" \
  --identifier "$BUNDLE_ID" \
  --version "$VERSION" \
  --install-location "/" \
  --sign "$INSTALLER_IDENTITY" \
  "$PKG"
echo "    $PKG"

if [ "$SKIP_NOTARIZE" -eq 1 ]; then
  echo "==> --skip-notarize: signed package ready (not notarized)"
  exit 0
fi

# 4. Notarize and staple.
echo "==> submitting to Apple notary service"
if [ -n "${NOTARY_PROFILE:-}" ]; then
  xcrun notarytool submit "$PKG" --keychain-profile "$NOTARY_PROFILE" --wait
else
  : "${APPLE_ID:?set NOTARY_PROFILE or APPLE_ID/TEAM_ID/APP_SPECIFIC_PASSWORD}"
  : "${TEAM_ID:?set TEAM_ID}"
  : "${APP_SPECIFIC_PASSWORD:?set APP_SPECIFIC_PASSWORD}"
  xcrun notarytool submit "$PKG" \
    --apple-id "$APPLE_ID" --team-id "$TEAM_ID" \
    --password "$APP_SPECIFIC_PASSWORD" --wait
fi

echo "==> stapling"
xcrun stapler staple "$PKG"
xcrun stapler validate "$PKG"
spctl --assess --type install --verbose=2 "$PKG" || true

echo "==> done: $PKG (signed, notarized, stapled)"
