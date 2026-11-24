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
# or on stderr: op and gh run with their output discarded, since either may echo what it was
# given. The public key is written to crates/brain/src/keys/release.pub for you to commit.
#
# A new key is stored as "brain release signing key (pending)" and takes the plain title only
# once every other step has worked, so a run that stops part way leaves that item behind, and
# every later run refuses until it is repaired (docs/release.md, "A run that stopped").
#
# Needs zipsign 0.2.2 (cargo install zipsign --version 0.2.2 --locked), jq, op signed in to the
# account holding Omit-Infra, and gh signed in with admin on omit-nz/brain.
set -euo pipefail
umask 077

repo=omit-nz/brain
vault=Omit-Infra
item="brain release signing key"
pending="$item (pending)"
root=$(cd "$(dirname "$0")/.." && pwd)
pub="$root/crates/brain/src/keys/release.pub"

die() { echo "release-key.sh: $*" >&2; exit 1; }
quiet() { "$@" >/dev/null 2>&1; }

[ -f "$pub" ] || die "no $pub; run this from a checkout of omit-nz/brain"
placeholder() { [ "$(od -An -v -tx1 "$pub" | tr -d ' \n')" = "$(printf '%064d' 0)" ]; }

mode=${1:-}
case "$mode" in
  init) placeholder || die "a release key already exists in $pub; use rotate to replace it" ;;
  rotate) placeholder && die "there is no release key yet; use init" ;;
  *) die "usage: release-key.sh init | rotate" ;;
esac
for tool in zipsign op gh jq; do
  command -v "$tool" >/dev/null || die "$tool is not on PATH"
done

# How many items in the vault carry this exact title.
items_titled() {
  local list
  list=$(op item list --vault "$vault" --format json 2>/dev/null) ||
    die "op cannot list $vault; sign in with op signin"
  jq --arg t "$1" '[.[] | select(.title == $t)] | length' <<<"$list"
}
repo_has_secret() {
  local list
  list=$(gh secret list --repo "$repo" 2>/dev/null) || die "gh cannot list the secrets of $repo"
  awk '{ print $1 }' <<<"$list" | grep -qx "$1"
}

[ "$(items_titled "$pending")" = 0 ] ||
  die "a run stopped part way: $vault holds \"$pending\"; see \"A run that stopped\" in docs/release.md"
if [ "$mode" = init ]; then
  [ "$(items_titled "$item")" = 0 ] || die "1Password already holds \"$item\" in $vault; use rotate"
  repo_has_secret RELEASE_SIGNING_KEY && die "$repo already has RELEASE_SIGNING_KEY; use rotate"
else
  [ "$(items_titled "$item")" = 1 ] || die "rotate needs exactly one \"$item\" in $vault"
fi

dir=$(mktemp -d "${TMPDIR:-/tmp}/brain-key.XXXXXX")
trap 'rm -rf "$dir"' EXIT
quiet zipsign gen-key "$dir/private" "$dir/public" || die "zipsign could not make a key"
[ "$(wc -c <"$dir/public" | tr -d ' ')" = 32 ] || die "zipsign wrote a public key that is not 32 bytes"

{
  printf '{"title":"%s","category":"PASSWORD","fields":[{"id":"password","type":"CONCEALED","purpose":"PASSWORD","label":"password","value":"' "$pending"
  base64 <"$dir/private" | tr -d '\n'
  printf '"}]}'
} | quiet op item create --vault "$vault" - || die "op could not store the new key"

if [ "$mode" = rotate ]; then
  op read "op://$vault/$item/password" 2>/dev/null |
    quiet gh secret set RELEASE_SIGNING_KEY_PREVIOUS --repo "$repo" ||
    die "could not copy the old key to RELEASE_SIGNING_KEY_PREVIOUS"
fi
base64 <"$dir/private" | tr -d '\n' | quiet gh secret set RELEASE_SIGNING_KEY --repo "$repo" ||
  die "gh could not set RELEASE_SIGNING_KEY"
cp "$dir/public" "$pub"
if [ "$mode" = rotate ]; then
  quiet op item edit "$item" --vault "$vault" --title "$item (retired $(date +%Y-%m-%d))" ||
    die "could not retire the old \"$item\""
fi
quiet op item edit "$pending" --vault "$vault" --title "$item" || die "could not rename \"$pending\""

echo "Release key made. Commit $pub; 1Password holds the private key as \"$item\" in $vault."
if [ "$mode" = rotate ]; then
  echo "The next release is signed by both keys. After it is out, delete RELEASE_SIGNING_KEY_PREVIOUS:"
  echo "  gh secret delete RELEASE_SIGNING_KEY_PREVIOUS --repo $repo"
fi
