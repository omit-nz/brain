#!/usr/bin/env bash
# Stage the ONNX Runtime library ll-core loads, from NuGet's Microsoft.ML.OnnxRuntime package.
# GitHub's microsoft/onnxruntime releases have returned 404 since 2026-11-18, and ll-core's own
# download points there. One package carries every platform's library; its SHA-256 is pinned here.
#
# Usage: fetch-ort.sh <target triple> <dir>
# Prints the path of the staged library; point ORT_DYLIB_PATH at it.
set -euo pipefail

version=1.24.2
sha256=782564d3d68e87269ea1812cc94710ad06fe014faf093f01a6f89d0bd58719d3
target=${1:?usage: fetch-ort.sh <target> <dir>}
dir=${2:?usage: fetch-ort.sh <target> <dir>}

case "$target" in
  aarch64-apple-darwin) member=runtimes/osx-arm64/native/libonnxruntime.dylib ;;
  x86_64-unknown-linux-gnu) member=runtimes/linux-x64/native/libonnxruntime.so ;;
  x86_64-pc-windows-msvc) member=runtimes/win-x64/native/onnxruntime.dll ;;
  *) echo "fetch-ort.sh: no ONNX Runtime for $target" >&2; exit 2 ;;
esac

mkdir -p "$dir"
pkg=$(mktemp)
trap 'rm -f "$pkg"' EXIT
curl --retry 5 --retry-all-errors --proto '=https' -fsSL -o "$pkg" \
  "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/$version/microsoft.ml.onnxruntime.$version.nupkg"

if command -v sha256sum >/dev/null; then got=$(sha256sum "$pkg" | cut -d' ' -f1); else got=$(shasum -a 256 "$pkg" | cut -d' ' -f1); fi
if [ "$got" != "$sha256" ]; then
  echo "fetch-ort.sh: package SHA-256 $got, expected $sha256" >&2
  exit 1
fi

out="$dir/$(basename "$member")"
unzip -p "$pkg" "$member" >"$out"
[ -s "$out" ] || { echo "fetch-ort.sh: $member missing from the package" >&2; exit 1; }
echo "$out"
