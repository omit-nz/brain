#!/usr/bin/env bash
# Make or rotate brain's release signing key.
#
#   release-key.sh init     make the first key
#   release-key.sh rotate   make a new key; the old one signs the next release too, so installs
#                           that trust only the old key can still take the release that moves
#                           them to the new one
#
# The private key is written once, into a private temporary folder that is removed on exit. It
# reaches 1Password (item "brain release signing key", vault Omit-Infra) and the repo secret
# RELEASE_SIGNING_KEY only on stdin, base64-encoded, and never appears in an argument, on stdout
# or on stderr. The public key is written to crates/brain/src/keys/release.pub for you to commit.
#
# Needs zipsign 0.2.2 (cargo install zipsign --version 0.2.2 --locked), op signed in to the
# account holding Omit-Infra, and gh signed in with admin on omit-nz/brain.
set -euo pipefail
umask 077

repo=omit-nz/brain
vault=Omit-Infra
item="brain release signing key"
root=$(cd "$(dirname "$0")/.." && pwd)
pub="$root/crates/brain/src/keys/release.pub"

die() { echo "release-key.sh: $*" >&2; exit 1; }

placeholder() { [ "$(od -An -v -tx1 "$pub" | tr -d ' \n')" = "$(printf '%064d' 0)" ]; }

mode=${1:-}
case "$mode" in
  init) placeholder || die "a release key already exists in $pub; use rotate to replace it" ;;
  rotate) placeholder && die "there is no release key yet; use init" ;;
  *) die "usage: release-key.sh init | rotate" ;;
esac
for tool in zipsign op gh; do
  command -v "$tool" >/dev/null || die "$tool is not on PATH"
done

dir=$(mktemp -d "${TMPDIR:-/tmp}/brain-key.XXXXXX")
trap 'rm -rf "$dir"' EXIT
zipsign gen-key "$dir/private" "$dir/public" >/dev/null
[ "$(wc -c <"$dir/public" | tr -d ' ')" = 32 ] || die "zipsign wrote a public key that is not 32 bytes"

if [ "$mode" = rotate ]; then
  op read "op://$vault/$item/password" | gh secret set RELEASE_SIGNING_KEY_PREVIOUS --repo "$repo" >/dev/null
  op item edit "$item" --vault "$vault" --title "$item (retired $(date +%Y-%m-%d))" >/dev/null
fi

{
  printf '{"title":"%s","category":"PASSWORD","fields":[{"id":"password","type":"CONCEALED","purpose":"PASSWORD","label":"password","value":"' "$item"
  base64 <"$dir/private" | tr -d '\n'
  printf '"}]}'
} | op item create --vault "$vault" - >/dev/null

base64 <"$dir/private" | tr -d '\n' | gh secret set RELEASE_SIGNING_KEY --repo "$repo" >/dev/null

cp "$dir/public" "$pub"
echo "Release key made. Commit $pub; 1Password holds the private key as \"$item\" in $vault."
if [ "$mode" = rotate ]; then
  echo "The next release is signed by both keys. After it is out, delete RELEASE_SIGNING_KEY_PREVIOUS:"
  echo "  gh secret delete RELEASE_SIGNING_KEY_PREVIOUS --repo $repo"
fi
