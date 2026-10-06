#!/usr/bin/env bash
set -euo pipefail

conf="${CONF:-vcluster/build.conf}"
[ -f "$conf" ] && . "$conf"

: "${BASE_VERSION:?BASE_VERSION required}"
PRERELEASE="${PRERELEASE:-vcluster.${VCLUSTER_ITERATION:?VCLUSTER_ITERATION or PRERELEASE required}}"

if [ -z "${SHORT_SHA:-}" ]; then
  : "${UPSTREAM_REF:?UPSTREAM_REF or SHORT_SHA required}"
  SHORT_SHA="$(git rev-parse --short=8 "$UPSTREAM_REF")"
fi

printf 'v%s-%s.g%s\n' "$BASE_VERSION" "$PRERELEASE" "$SHORT_SHA"
