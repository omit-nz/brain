#!/usr/bin/env bash
# Signs a macOS release folder with omit's Developer ID and makes the two assets people install
# from: the update archive and brain.pkg. brain and its runtime library are signed with the
# hardened runtime and notarised; the archive packs those bytes; brain.pkg holds the same folder,
# signed with the Developer ID Installer identity, notarised and stapled. Every identity must be
# omit's team (crates/brain/src/keys/team_id), and both files must meet what an installed brain
# asks of an update (the_release_checks_what_an_update_checks keeps the two the same).
#
# Usage: scripts/sign-macos.sh <release-folder> <version> <out-dir>
# The release folder is named brain-<version> and holds brain and lib/libonnxruntime.dylib.
# The secrets come in the environment: MACOS_APP_CERT_P12 and MACOS_INSTALLER_CERT_P12 (base64
# .p12 files) with MACOS_APP_CERT_PASSWORD and MACOS_INSTALLER_CERT_PASSWORD, and the notary API
# key as NOTARY_KEY_P8 (PEM), NOTARY_KEY_ID and NOTARY_ISSUER_ID. A missing one fails the release.
set -euo pipefail

if [ $# -ne 3 ]; then
  echo "usage: $0 <release-folder> <version> <out-dir>" >&2
  exit 2
fi
release=$1
version=$2
out=$3
test "$(basename "$release")" = "brain-$version" || { echo "the release folder must be named brain-$version" >&2; exit 2; }
test -x "$release/brain"
test -f "$release/lib/libonnxruntime.dylib"

for name in MACOS_APP_CERT_P12 MACOS_APP_CERT_PASSWORD MACOS_INSTALLER_CERT_P12 MACOS_INSTALLER_CERT_PASSWORD \
  NOTARY_KEY_P8 NOTARY_KEY_ID NOTARY_ISSUER_ID; do
  if [ -z "${!name:-}" ]; then
    echo "$name is not set, so this release cannot be signed" >&2
    exit 1
  fi
done

here=$(cd "$(dirname "$0")/.." && pwd)
team=$(cat "$here/crates/brain/src/keys/team_id")
developer_id="anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"$team\""

umask 077
work=$(mktemp -d "${TMPDIR:-/tmp}/brain-sign.XXXXXX")
kc="$work/sign.keychain-db"
keychains="$work/keychains"
cleanup() {
  if [ -s "$keychains" ]; then
    # shellcheck disable=SC2046
    security list-keychains -d user -s $(cat "$keychains")
  fi
  security delete-keychain "$kc" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

# A keychain of this run's own, holding the two identities and nothing else.
kcpw=$(uuidgen)
security list-keychains -d user | tr -d '" ' >"$keychains"
security create-keychain -p "$kcpw" "$kc"
security set-keychain-settings -lut 3600 "$kc"
security unlock-keychain -p "$kcpw" "$kc"
printf '%s' "$MACOS_APP_CERT_P12" | base64 -d >"$work/app.p12"
printf '%s' "$MACOS_INSTALLER_CERT_P12" | base64 -d >"$work/installer.p12"
security import "$work/app.p12" -k "$kc" -P "$MACOS_APP_CERT_PASSWORD" -T /usr/bin/codesign
security import "$work/installer.p12" -k "$kc" -P "$MACOS_INSTALLER_CERT_PASSWORD" -T /usr/bin/productsign
rm -f "$work/app.p12" "$work/installer.p12"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$kcpw" "$kc" >/dev/null
# shellcheck disable=SC2046
security list-keychains -d user -s "$kc" $(cat "$keychains")

# The identity whose name is `<kind>: <anyone> (<team>)`; any other team fails the release.
identity() {
  security find-identity -v "$kc" | sed -n "s/^ *[0-9]*) [0-9A-F]* \"\($1: .* ($team)\)\"$/\1/p" | head -1
}
app=$(identity "Developer ID Application")
installer=$(identity "Developer ID Installer")
test -n "$app" || { echo "no Developer ID Application identity of team $team in MACOS_APP_CERT_P12" >&2; exit 1; }
test -n "$installer" || { echo "no Developer ID Installer identity of team $team in MACOS_INSTALLER_CERT_P12" >&2; exit 1; }

notarise() {
  local key="$work/notary.p8"
  printf '%s\n' "$NOTARY_KEY_P8" >"$key"
  xcrun notarytool submit "$1" --key "$key" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID" \
    --wait --timeout 30m --output-format json >"$work/notary.json"
  if [ "$(jq -r .status "$work/notary.json")" != Accepted ]; then
    xcrun notarytool log "$(jq -r .id "$work/notary.json")" --key "$key" --key-id "$NOTARY_KEY_ID" \
      --issuer "$NOTARY_ISSUER_ID" >&2 || true
    echo "notarisation of $(basename "$1"): $(jq -r .status "$work/notary.json")" >&2
    exit 1
  fi
}

# The library first: the hardened runtime loads only libraries signed by the binary's own team. A
# fixed identifier keeps macOS's privacy grants to brain across updates.
lib="$release/lib/libonnxruntime.dylib"
codesign --force --options runtime --timestamp --keychain "$kc" --sign "$app" "$lib"
codesign --force --options runtime --timestamp --keychain "$kc" --sign "$app" -i nz.omit.brain "$release/brain"
codesign --verify --strict "-R=$developer_id" "$lib"
codesign --verify --strict "-R=identifier \"nz.omit.brain\" and $developer_id" "$release/brain"
ditto -c -k --keepParent "$release" "$work/notarise.zip"
notarise "$work/notarise.zip"

mkdir -p "$out"
COPYFILE_DISABLE=1 tar -czf "$out/brain-$version-aarch64-apple-darwin.tar.gz" -C "$(dirname "$release")" "brain-$version"

"$here/scripts/build-pkg.sh" "$release" "$version" "$work/unsigned.pkg"
productsign --timestamp --keychain "$kc" --sign "$installer" "$work/unsigned.pkg" "$out/brain.pkg"
notarise "$out/brain.pkg"
xcrun stapler staple "$out/brain.pkg"
xcrun stapler validate "$out/brain.pkg"
pkgutil --check-signature "$out/brain.pkg" | grep -q "Developer ID Installer: .* ($team)"
