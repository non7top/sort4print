#!/usr/bin/env bash
# Compresses the flag SVGs for packing into the binary.
#
# Gzip because usvg reads a gzipped SVG directly — the `svgz` feature — so the
# program needs no decompressor of its own. The whole set is 4.1 MB of XML and
# 1.2 MB compressed, which is less than rasterising them would cost and scales
# to any print size instead of one.
set -euo pipefail

src=${1:?usage: gzip-flags.sh <svg dir> <svgz dir>}
out=${2:?}

mkdir -p "$out"
packed=0
for svg in "$src"/*.svg; do
  name=$(basename "$svg" .svg)
  # Two-letter ISO codes only. The collection also carries six-letter codes for
  # the constituent countries of the United Kingdom, which no country column in
  # the city database ever produces.
  if [ "${#name}" -ne 2 ]; then
    continue
  fi
  gzip -9 -c "$svg" > "$out/$name.svgz"
  packed=$((packed + 1))
done
echo "compressed $packed flags into $out"
