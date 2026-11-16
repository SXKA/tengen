#!/bin/sh
# Downloads Rapfi's NNUE weights into public/engines/rapfi/weights, so the
# site serves them itself; pages fall back to CDNs when they are missing.
# The commit is the one the Rapfi build's configs come from. Failing to
# download is not an error: the build goes on without them.
set -u
cd "$(dirname "$0")/.."
dir=public/engines/rapfi/weights
commit=$(cat public/engines/rapfi/networks-commit.txt)
mkdir -p "$dir"
for name in mix9svqfreestyle_bsmix.bin.lz4 mix9svqstandard_bs15.bin.lz4 \
  mix9svqrenju_bs15_black.bin.lz4 mix9svqrenju_bs15_white.bin.lz4; do
  [ -s "$dir/$name" ] && continue
  if ! curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$dir/$name.part" \
    "https://raw.githubusercontent.com/dhbloo/rapfi-networks/$commit/mix9svq/$name"; then
    echo "fetch-rapfi-weights: could not download $name; pages will use the CDNs" >&2
    rm -f "$dir/$name.part"
    continue
  fi
  mv "$dir/$name.part" "$dir/$name"
done
ls -la "$dir"
