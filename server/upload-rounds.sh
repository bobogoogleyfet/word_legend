#!/usr/bin/env bash
# Uploads precomputed round packs to Workers KV.
#
# Credentials are never passed here. wrangler uses your own `wrangler login`
# session, or CLOUDFLARE_API_TOKEN from the environment in CI -- which comes from
# a GitHub Actions secret, never from a file in this repository.
set -euo pipefail

DIR="${1:-../rounds}"
[ -d "$DIR" ] || { echo "no packs in $DIR. Build them first:" >&2
  echo "  cargo run --release --bin export_rounds -- rounds 1000" >&2; exit 1; }

shopt -s nullglob
packs=("$DIR"/pack-*.json)
(( ${#packs[@]} )) || { echo "no pack-*.json in $DIR" >&2; exit 1; }

echo "uploading ${#packs[@]} packs to the ROUNDS namespace..."
for pack in "${packs[@]}"; do
  key="pack:$(basename "$pack" .json | sed 's/^pack-//')"
  echo "  $key"
  npx wrangler kv key put --binding=ROUNDS --remote "$key" --path "$pack"
done

echo
echo "Done. Set PACK_COUNT=${#packs[@]} in wrangler.toml if it has changed."
