#!/usr/bin/env bash
# Builds brain.pkg, unsigned, from a release folder (brain and lib/, as in a release archive).
# The package installs into the current person's home: the folder becomes brain-current in brain's
# data folder, and pkg/scripts/postinstall links ~/.local/bin/brain to it. scripts/sign-macos.sh
# builds it this way for a release, then signs, notarises and staples it.
#
# Usage: scripts/build-pkg.sh <release-folder> <version> <out.pkg>
set -euo pipefail

if [ $# -ne 3 ]; then
  echo "usage: $0 <release-folder> <version> <out.pkg>" >&2
  exit 2
fi
release=$1
version=$2
out=$3
case $version in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "a version looks like 1.2.3, got $version" >&2; exit 2 ;;
esac
test -x "$release/brain"

here=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/brain-pkg.XXXXXX")
trap 'rm -rf "$work"' EXIT

# Paths in the payload are relative to the home the package installs into.
payload="$work/payload/Library/Application Support/nz.omit.brain/bin/brain-current"
mkdir -p "$payload"
# No extended attributes, so the payload carries no ._ AppleDouble twins.
ditto --noextattr --noqtn "$release" "$payload"
export COPYFILE_DISABLE=1
chmod 755 "$here/pkg/scripts/postinstall"

pkgbuild --quiet --root "$work/payload" --identifier nz.omit.brain --version "$version" \
  --install-location / --scripts "$here/pkg/scripts" "$work/brain-component.pkg"
sed "s/version=\"0.0.0\"/version=\"$version\"/" "$here/pkg/distribution.xml" >"$work/distribution.xml"
productbuild --quiet --distribution "$work/distribution.xml" --package-path "$work" "$out"
