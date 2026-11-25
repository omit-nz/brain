#!/usr/bin/env bash
# Tests for sign-macos.sh against stub security, codesign, xcrun, productsign and pkgutil: a run
# with every secret signs the library, then brain, notarises both, and ships a signed, notarised
# and stapled brain.pkg beside the archive; a missing secret, another team's identity or a refused
# notarisation fails the release, and the keychain never outlives the run. Needs macOS (ditto,
# pkgbuild and productbuild run for real) and jq.
#
# Usage: sign-macos.test.sh
#
# The tests are called from a list, which the linter reads as unused functions.
# shellcheck disable=SC2317,SC2329
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
version=9.9.9
team=$(cat "$here/../crates/brain/src/keys/team_id")
failed=0

# Stubs log every call to $box/log and act on files the way the real tools do, closely enough
# for the script to go on.
setup() {
  box=$(mktemp -d "${TMPDIR:-/tmp}/sign-test.XXXXXX")
  mkdir -p "$box/bin" "$box/brain-$version/lib"
  printf 'brain' >"$box/brain-$version/brain"
  chmod 755 "$box/brain-$version/brain"
  printf 'runtime' >"$box/brain-$version/lib/libonnxruntime.dylib"
  identity_team=$team
  notary=Accepted
  export MACOS_APP_CERT_P12=YXBw MACOS_APP_CERT_PASSWORD=app-pw MACOS_INSTALLER_CERT_P12=aW5z \
    MACOS_INSTALLER_CERT_PASSWORD=ins-pw NOTARY_KEY_P8=p8 NOTARY_KEY_ID=kid NOTARY_ISSUER_ID=iss
}

stubs() {
  cat >"$box/bin/security" <<EOF
#!/usr/bin/env bash
echo "security \$*" >>"$box/log"
case \$1 in
  list-keychains) [ "\${3:-}" = -s ] || echo '"/Users/runner/Library/Keychains/login.keychain-db"' ;;
  create-keychain) touch "\${@: -1}" ;;
  delete-keychain) rm -f "\$2" ;;
  find-identity)
    echo '  1) AAAA "Developer ID Application: Someone ($identity_team)"'
    echo '  2) BBBB "Developer ID Installer: Someone ($identity_team)"'
    echo '     2 valid identities found' ;;
esac
EOF
  cat >"$box/bin/codesign" <<EOF
#!/usr/bin/env bash
echo "codesign \$*" >>"$box/log"
EOF
  cat >"$box/bin/xcrun" <<EOF
#!/usr/bin/env bash
echo "xcrun \$*" >>"$box/log"
case "\$1 \$2" in
  "notarytool submit") echo '{"id":"n1","status":"$notary"}' ;;
  "stapler staple") echo stapled >>"\$3" ;;
esac
EOF
  cat >"$box/bin/productsign" <<EOF
#!/usr/bin/env bash
echo "productsign \$*" >>"$box/log"
cp "\${@: -2:1}" "\${@: -1}"
EOF
  cat >"$box/bin/pkgutil" <<EOF
#!/usr/bin/env bash
echo "pkgutil \$*" >>"$box/log"
echo "   1. Developer ID Installer: Someone ($identity_team)"
EOF
  chmod 755 "$box/bin/"*
}

sign() {
  stubs
  PATH="$box/bin:$PATH" bash "$here/sign-macos.sh" "$box/brain-$version" "$version" "$box/out" >"$box/out.txt" 2>&1
}

fail() {
  echo "FAIL $1: $2"
  cat "$box/out.txt"
  failed=1
}

# The line number of the first log line matching $1, or nothing.
at() {
  grep -n -m1 -- "$1" "$box/log" | cut -d: -f1
}

a_run_with_every_secret_signs_notarises_and_staples() {
  setup
  if ! sign; then
    fail "${FUNCNAME[0]}" "refused"
    return
  fi
  local lib brain zip pkg staple
  lib=$(at "codesign --force --options runtime --timestamp .*--sign Developer ID Application: Someone ($team) .*/lib/libonnxruntime.dylib$")
  brain=$(at "codesign --force --options runtime --timestamp .*--sign Developer ID Application: Someone ($team) -i nz.omit.brain .*/brain-$version/brain$")
  zip=$(at "xcrun notarytool submit .*notarise.zip ")
  pkg=$(at "xcrun notarytool submit $box/out/brain.pkg ")
  staple=$(at "xcrun stapler staple $box/out/brain.pkg")
  if [ -z "$lib" ] || [ -z "$brain" ] || [ -z "$zip" ] || [ -z "$pkg" ] || [ -z "$staple" ]; then
    fail "${FUNCNAME[0]}" "a step is missing (lib=$lib brain=$brain zip=$zip pkg=$pkg staple=$staple)"
  elif ! [ "$lib" -lt "$brain" ] || ! [ "$brain" -lt "$zip" ] || ! [ "$zip" -lt "$pkg" ] || ! [ "$pkg" -lt "$staple" ]; then
    fail "${FUNCNAME[0]}" "steps out of order (lib=$lib brain=$brain zip=$zip pkg=$pkg staple=$staple)"
  elif ! grep -q "^codesign --verify --strict -R=anchor apple generic and certificate leaf\[subject.OU\] = \"$team\" .*/brain-$version/brain$" "$box/log" \
    || ! grep -q "^codesign --verify --strict -R=anchor apple generic and certificate leaf\[subject.OU\] = \"$team\" .*/libonnxruntime.dylib$" "$box/log"; then
    fail "${FUNCNAME[0]}" "the signatures were not checked against the team"
  elif ! grep -q "^productsign --timestamp --keychain .* --sign Developer ID Installer: Someone ($team) " "$box/log"; then
    fail "${FUNCNAME[0]}" "brain.pkg was not signed with the Installer identity"
  elif [ "$(tar -tzf "$box/out/brain-$version-aarch64-apple-darwin.tar.gz" | sort | tr '\n' ' ')" != "brain-$version/ brain-$version/brain brain-$version/lib/ brain-$version/lib/libonnxruntime.dylib " ]; then
    fail "${FUNCNAME[0]}" "the archive holds $(tar -tzf "$box/out/brain-$version-aarch64-apple-darwin.tar.gz" | tr '\n' ' ')"
  elif ! pkgutil --expand "$box/out/brain.pkg" "$box/expanded" 2>/dev/null; then
    fail "${FUNCNAME[0]}" "brain.pkg is not a package"
  else
    echo "ok   ${FUNCNAME[0]}"
  fi
}

the_keychain_is_gone_and_the_search_list_restored() {
  setup
  sign || true
  local kc
  kc=$(sed -n 's/^security create-keychain -p [^ ]* //p' "$box/log" 2>/dev/null || true)
  if [ -z "$kc" ] || [ -e "$kc" ]; then
    fail "${FUNCNAME[0]}" "the keychain ${kc:-?} is still there"
  elif [ "$(tail -2 "$box/log" | head -1)" != "security list-keychains -d user -s /Users/runner/Library/Keychains/login.keychain-db" ]; then
    fail "${FUNCNAME[0]}" "the search list was not restored: $(tail -2 "$box/log" | head -1)"
  else
    echo "ok   ${FUNCNAME[0]}"
  fi
}

each_missing_secret_fails_before_anything_is_signed() {
  local name
  for name in MACOS_APP_CERT_P12 MACOS_APP_CERT_PASSWORD MACOS_INSTALLER_CERT_P12 MACOS_INSTALLER_CERT_PASSWORD \
    NOTARY_KEY_P8 NOTARY_KEY_ID NOTARY_ISSUER_ID; do
    setup
    unset "$name"
    if sign; then
      fail "${FUNCNAME[0]}" "signed without $name"
    elif ! grep -q "$name is not set" "$box/out.txt"; then
      fail "${FUNCNAME[0]}" "the refusal does not name $name"
    elif [ -e "$box/log" ]; then
      fail "${FUNCNAME[0]}" "a tool ran without $name"
    else
      echo "ok   ${FUNCNAME[0]} $name"
    fi
  done
}

another_teams_identity_fails_the_release() {
  setup
  identity_team=AAAAAAAAAA
  if sign; then
    fail "${FUNCNAME[0]}" "signed with team $identity_team"
  elif grep -q "^codesign --force" "$box/log"; then
    fail "${FUNCNAME[0]}" "signed something first"
  else
    echo "ok   ${FUNCNAME[0]}"
  fi
}

a_refused_notarisation_fails_the_release() {
  setup
  notary=Invalid
  if sign; then
    fail "${FUNCNAME[0]}" "an Invalid notarisation passed"
  elif ! grep -q "notarisation of notarise.zip: Invalid" "$box/out.txt"; then
    fail "${FUNCNAME[0]}" "the refusal does not say why"
  elif [ -e "$box/out/brain.pkg" ] || [ -e "$box/out/brain-$version-aarch64-apple-darwin.tar.gz" ]; then
    fail "${FUNCNAME[0]}" "an asset was made after the refusal"
  else
    echo "ok   ${FUNCNAME[0]}"
  fi
}

for t in a_run_with_every_secret_signs_notarises_and_staples the_keychain_is_gone_and_the_search_list_restored \
  each_missing_secret_fails_before_anything_is_signed another_teams_identity_fails_the_release \
  a_refused_notarisation_fails_the_release; do
  "$t"
done
if [ "$failed" -ne 0 ]; then
  exit 1
fi
echo "all passed"
