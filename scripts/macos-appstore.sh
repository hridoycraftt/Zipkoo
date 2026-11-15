#!/bin/bash
# Build, sign and package Zipkoo for the Mac App Store. Run on a Mac with Xcode installed.
# It stops before uploading: you upload the .pkg yourself with Transporter (or altool).
#
# Needs (see docs/mac-app-store.md):
#   APPLE_TEAM_ID         10-character Team ID from developer.apple.com → Membership
#   APP_IDENTITY          "Apple Distribution: Your Name (TEAMID)"
#   INSTALLER_IDENTITY    "3rd Party Mac Developer Installer: Your Name (TEAMID)"
#   PROVISION_PROFILE     path to the Mac App Store provisioning profile for app.zipkoo.desktop
set -euo pipefail
: "${APPLE_TEAM_ID:?set APPLE_TEAM_ID}" "${APP_IDENTITY:?set APP_IDENTITY}"
: "${INSTALLER_IDENTITY:?set INSTALLER_IDENTITY}" "${PROVISION_PROFILE:?set PROVISION_PROFILE}"

ROOT=$(cd "$(dirname "$0")/.." && pwd)
MAC=$ROOT/app/src-tauri/macos
cp "$PROVISION_PROFILE" "$MAC/embedded.provisionprofile"
sed "s/TEAM_ID/$APPLE_TEAM_ID/g" "$MAC/Zipkoo.appstore.entitlements" > "$MAC/Zipkoo.generated.entitlements"
# Same overlay, pointing at the entitlements with the real Team ID.
sed 's#Zipkoo.appstore.entitlements#Zipkoo.generated.entitlements#' "$ROOT/app/src-tauri/tauri.appstore.conf.json" > "$MAC/tauri.generated.json"

rustup target add aarch64-apple-darwin x86_64-apple-darwin
cd "$ROOT/app"
npm ci
APPLE_SIGNING_IDENTITY="$APP_IDENTITY" npx tauri build \
  --target universal-apple-darwin --bundles app \
  --config "$MAC/tauri.generated.json" \
  --features appstore -- --no-default-features

APP="$ROOT/target/universal-apple-darwin/release/bundle/macos/Zipkoo.app"
echo "Checking the signature and sandbox…"
codesign --verify --deep --strict --verbose=2 "$APP"
codesign -d --entitlements - "$APP" | grep -q "com.apple.security.app-sandbox" || { echo "Sandbox entitlement missing"; exit 1; }

PKG="$ROOT/Zipkoo-$(node -p "require('./src-tauri/tauri.conf.json').version").pkg"
xcrun productbuild --sign "$INSTALLER_IDENTITY" --component "$APP" /Applications "$PKG"
echo "Done: $PKG"
echo "Next: open Transporter, sign in, drag this .pkg in and press Deliver."
