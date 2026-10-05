#!/usr/bin/env bash
set -euo pipefail

conf="${CONF:-vcluster/build.conf}"
[ -f "$conf" ] && . "$conf"

: "${BASE_VERSION:?BASE_VERSION required}"
: "${VCLUSTER_ITERATION:?VCLUSTER_ITERATION required}"

if [ -z "${SHORT_SHA:-}" ]; then
  : "${UPSTREAM_REF:?UPSTREAM_REF or SHORT_SHA required}"
  SHORT_SHA="$(git rev-parse --short=8 "$UPSTREAM_REF")"
fi

printf 'v%s-vcluster.%s.g%s\n' "$BASE_VERSION" "$VCLUSTER_ITERATION" "$SHORT_SHA"
