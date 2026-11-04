#!/bin/sh
set -eu

distrusted="
86a1ecba089c4a8d3bbe2734c612ba341d813e043cf9e8a862cd5c57a36bbe6b
125609aa301da0a249b97a8239cb6a34216f44dcac9f3954b14292f2e8c8608f
bc4d809b15189d78db3e1d8cf4f9726a795da1643ca5f1358e1ddb0edc0d7eb3
"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

split() {
  rm -rf "$work/certs"
  mkdir "$work/certs"
  awk -v dir="$work/certs" '
    /-----BEGIN CERTIFICATE-----/ { n++; f = sprintf("%s/%06d", dir, n); print > f; next }
    f != "" { print > f }
    /-----END CERTIFICATE-----/ { close(f); f = "" }
  ' "$1"
}

fingerprint() {
  fp=$(awk '/-----END CERTIFICATE-----/ { exit } p { print } /-----BEGIN CERTIFICATE-----/ { p = 1 }' "$1" \
    | base64 -d | sha256sum | cut -d ' ' -f 1)
  case "$fp" in
    *[!0-9a-f]* | "") echo "cannot fingerprint a certificate in $1" >&2; exit 1 ;;
  esac
  [ "${#fp}" -eq 64 ] || { echo "cannot fingerprint a certificate in $1" >&2; exit 1; }
  echo "$fp"
}

is_distrusted() {
  [ -n "$1" ] || return 1
  case "$distrusted" in *"$1"*) return 0 ;; esac
  return 1
}

check_file() {
  split "$1"
  for cert in "$work"/certs/*; do
    [ -f "$cert" ] || continue
    fp=$(fingerprint "$cert") || return 1
    if is_distrusted "$fp"; then
      echo "distrusted root $fp still trusted via $2" >&2
      return 1
    fi
  done
}

filter() {
  bundle=$1
  split "$bundle"
  : > "$work/bundle"
  kept=0
  removed=0
  for cert in "$work"/certs/*; do
    [ -f "$cert" ] || continue
    fp=$(fingerprint "$cert")
    if is_distrusted "$fp"; then
      removed=$((removed + 1))
    else
      cat "$cert" >> "$work/bundle"
      kept=$((kept + 1))
    fi
  done
  [ "$kept" -gt 0 ] || { echo "no certificates left in $bundle" >&2; exit 1; }
  cat "$work/bundle" > "$bundle"
  echo "$bundle: removed $removed distrusted roots, kept $kept"
}

check() {
  for path in "$@"; do
    if [ -d "$path" ]; then
      for entry in "$path"/*; do
        [ -f "$entry" ] || continue
        check_file "$entry" "$entry" || exit 1
      done
    elif [ -f "$path" ]; then
      check_file "$path" "$path" || exit 1
    fi
  done
  echo "no distrusted roots in $*"
}

case "${1:-}" in
  filter) shift; filter "$1"; check "$1" ;;
  check) shift; check "$@" ;;
  *) echo "usage: $0 filter <bundle> | check <file-or-dir>..." >&2; exit 2 ;;
esac
