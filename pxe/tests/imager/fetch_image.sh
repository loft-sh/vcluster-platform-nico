#!/bin/bash
set -euo pipefail

MODE=${1:-}
shift || true
IMAGE_NAME=
DEST_DIR=
ADHOC_URL=
ADHOC_SHA=
IMAGES_JSON="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/images.json"

usage() {
	cat >&2 <<EOF
usage: $0 checksum|image --image NAME --dest DIR [--images-json FILE]
       $0 adhoc --image NAME --url URL [--sha ALGO:HEX] --dest DIR
  checksum  download the catalog checksum file to DIR/checksum.txt and print checksum_hash= and image_path=
  image     download DIR/image/<file> unless a cached copy already matches the checksum file, then verify it
  adhoc     download URL to DIR/image/NAME.img and verify it against ALGO:HEX when given
EOF
	exit 2
}

while [ $# -gt 0 ]; do
	case "$1" in
		--image) IMAGE_NAME=$2; shift 2 ;;
		--dest) DEST_DIR=$2; shift 2 ;;
		--images-json) IMAGES_JSON=$2; shift 2 ;;
		--url) ADHOC_URL=$2; shift 2 ;;
		--sha) ADHOC_SHA=$2; shift 2 ;;
		*) usage ;;
	esac
done
[ -n "$MODE" ] && [ -n "$IMAGE_NAME" ] && [ -n "$DEST_DIR" ] || usage
mkdir -p "$DEST_DIR/image"

emit_output() {
	echo "$1=$2"
	if [ -n "${GITHUB_OUTPUT:-}" ]; then
		echo "$1=$2" >>"$GITHUB_OUTPUT"
	fi
}

fetch_adhoc() {
	local image_path="$DEST_DIR/image/$IMAGE_NAME.img" algo digest actual
	[ -n "$ADHOC_URL" ] || usage
	echo "downloading $ADHOC_URL" >&2
	rm -f "$image_path"
	curl -fL --retry 5 --retry-all-errors -o "$image_path" "$ADHOC_URL"
	if [ -n "$ADHOC_SHA" ]; then
		algo=${ADHOC_SHA%%:*}
		digest=${ADHOC_SHA#*:}
		case "$algo" in
			sha1|sha256|sha384|sha512) ;;
			*) echo "unsupported digest algorithm $algo" >&2; exit 1 ;;
		esac
		actual=$("${algo}sum" "$image_path" | awk '{ print $1 }')
		[ "$actual" = "${digest,,}" ] || { echo "$image_path $algo is $actual, expected $digest" >&2; exit 1; }
		echo "$image_path matches $ADHOC_SHA" >&2
	else
		echo "no image_sha given: $image_path is unverified" >&2
	fi
	qemu-img info "$image_path" >&2 || true
	emit_output image_path "$image_path"
}

if [ "$MODE" = adhoc ]; then
	fetch_adhoc
	exit 0
fi

[ -f "$IMAGES_JSON" ] || { echo "images.json not found: $IMAGES_JSON" >&2; exit 1; }

image_field() {
	jq -er --arg name "$IMAGE_NAME" --arg field "$1" '.images[] | select(.name == $name) | .[$field] // empty' "$IMAGES_JSON"
}

IMAGE_URL=$(image_field url) || { echo "image $IMAGE_NAME not in $IMAGES_JSON" >&2; exit 1; }
CHECKSUM_URL=$(image_field checksum_url)
CHECKSUM_FORMAT=$(image_field checksum_format)
NAME_PATTERN=$(image_field checksum_name_pattern)
CHECKSUM_FILE="$DEST_DIR/checksum.txt"
IMAGE_PATH="$DEST_DIR/image/$(basename "$IMAGE_URL")"

checksum_entries() {
	case "$CHECKSUM_FORMAT" in
		coreutils)
			awk 'NF >= 2 && $1 ~ /^[0-9a-fA-F]+$/ { name = $2; sub(/^\*/, "", name); print tolower($1), name }' "$CHECKSUM_FILE"
			;;
		bsd)
			sed -nE 's/^SHA(256|512) \((.+)\) = ([0-9a-fA-F]+)$/\3 \2/p' "$CHECKSUM_FILE" | awk '{ print tolower($1), $2 }'
			;;
		*)
			echo "unknown checksum_format $CHECKSUM_FORMAT" >&2
			exit 1
			;;
	esac
}

candidate_digests() {
	checksum_entries | awk -v pattern="$NAME_PATTERN" '$2 ~ pattern { print $1 }'
}

image_matches_checksum() {
	local sha256 sha512 digest
	[ -s "$IMAGE_PATH" ] || return 1
	sha256=$(sha256sum "$IMAGE_PATH" | awk '{ print $1 }')
	sha512=$(sha512sum "$IMAGE_PATH" | awk '{ print $1 }')
	for digest in $(candidate_digests); do
		if [ "$digest" = "$sha256" ] || [ "$digest" = "$sha512" ]; then
			echo "$IMAGE_PATH matches $digest" >&2
			return 0
		fi
	done
	return 1
}

case "$MODE" in
	checksum)
		curl -fsSL --retry 5 --retry-all-errors -o "$CHECKSUM_FILE" "$CHECKSUM_URL"
		COUNT=$(candidate_digests | wc -l | tr -d ' ')
		[ "$COUNT" -gt 0 ] || { echo "no entry in $CHECKSUM_URL matches $NAME_PATTERN" >&2; exit 1; }
		emit_output checksum_hash "$(sha256sum "$CHECKSUM_FILE" | awk '{ print $1 }')"
		emit_output image_path "$IMAGE_PATH"
		;;
	image)
		[ -s "$CHECKSUM_FILE" ] || { echo "run '$0 checksum' first: $CHECKSUM_FILE missing" >&2; exit 1; }
		if image_matches_checksum; then
			echo "cached image is current" >&2
		else
			echo "downloading $IMAGE_URL" >&2
			rm -f "$IMAGE_PATH"
			curl -fL --retry 5 --retry-all-errors -o "$IMAGE_PATH" "$IMAGE_URL"
			image_matches_checksum || { echo "downloaded $(basename "$IMAGE_PATH") does not match any digest in $CHECKSUM_URL" >&2; exit 1; }
		fi
		qemu-img info "$IMAGE_PATH" >&2 || true
		emit_output image_path "$IMAGE_PATH"
		;;
	*)
		usage
		;;
esac
