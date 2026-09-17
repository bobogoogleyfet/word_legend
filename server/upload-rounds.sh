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

echo "uploading ${#packs[@]} default packs to the ROUNDS namespace..."
for pack in "${packs[@]}"; do
  key="pack:$(basename "$pack" .json | sed 's/^pack-//')"
  echo "  $key"
  npx wrangler kv key put --binding=ROUNDS --remote "$key" --path "$pack"
done

# Each month's own rounds, favouring its seasonal theme, if they were built
# (export_rounds --all-months). The server falls back to the defaults without them.
for month_dir in "$DIR"/month-*; do
  [ -d "$month_dir" ] || continue
  month="$(basename "$month_dir" | sed 's/^month-//')"
  echo "uploading month $month..."
  for pack in "$month_dir"/pack-*.json; do
    key="pack:m$month:$(basename "$pack" .json | sed 's/^pack-//')"
    echo "  $key"
    npx wrangler kv key put --binding=ROUNDS --remote "$key" --path "$pack"
  done
done

echo
echo "Done. Set PACK_COUNT=${#packs[@]} in wrangler.toml if it has changed."
