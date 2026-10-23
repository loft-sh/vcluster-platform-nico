#!/usr/bin/env bash
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: vcluster/rebuild.sh <nvidia-tag>   e.g. vcluster/rebuild.sh v2.4.0-rc.1" >&2
  exit 2
fi

TAG=$1
BRANCH="vcluster/${TAG}"
UPSTREAM_URL=${UPSTREAM_URL:-https://github.com/NVIDIA/infra-controller.git}
FORK_REMOTE=${FORK_REMOTE:-origin}
ROOT=$(git rev-parse --show-toplevel)
PATCHES="${ROOT}/vcluster/patches.txt"

git -C "$ROOT" config rerere.enabled true
git -C "$ROOT" fetch --no-tags "$UPSTREAM_URL" "refs/tags/${TAG}:refs/tags/${TAG}"
git -C "$ROOT" fetch "$FORK_REMOTE"

if git -C "$ROOT" rev-parse --verify --quiet "refs/heads/${BRANCH}" >/dev/null && [ "${REBUILD:-0}" != "1" ]; then
  echo "${BRANCH} already exists; set REBUILD=1 to recreate it" >&2
  exit 1
fi

WORK=$(mktemp -d)
trap 'git -C "$ROOT" worktree remove --force "$WORK" >/dev/null 2>&1 || true' EXIT
git -C "$ROOT" worktree add --force -B "$BRANCH" "$WORK" "refs/tags/${TAG}" >/dev/null

DROP=()
while read -r name upstream _; do
  case "$name" in ''|'#'*) continue ;; esac
  if commit=$(git -C "$ROOT" rev-parse --verify --quiet "refs/remotes/${FORK_REMOTE}/${name}^{commit}"); then
    :
  elif commit=$(git -C "$ROOT" rev-parse --verify --quiet "refs/heads/${name}^{commit}"); then
    echo "warning: ${name} is not on ${FORK_REMOTE}; using the local branch" >&2
  else
    echo "error: patch branch ${name} not found" >&2
    exit 1
  fi

  if git -C "$WORK" cherry "refs/tags/${TAG}" "$commit" | grep -q '^-'; then
    echo "skip ${name}: already in ${TAG} (upstream ${upstream}); remove it from vcluster/patches.txt"
    DROP+=("$name")
    continue
  fi

  if ! GIT_COMMITTER_NAME=$(git -C "$ROOT" log -1 --format=%cn "$commit") \
    GIT_COMMITTER_EMAIL=$(git -C "$ROOT" log -1 --format=%ce "$commit") \
    GIT_COMMITTER_DATE=$(git -C "$ROOT" log -1 --format=%cI "$commit") \
    git -C "$WORK" -c commit.gpgsign=false cherry-pick "$commit" >/dev/null; then
    echo "error: ${name} does not apply on ${TAG}; resolve in ${WORK}, run git cherry-pick --continue, then rerun with REBUILD=1" >&2
    trap - EXIT
    exit 1
  fi
  echo "applied ${name} (${commit:0:10}, upstream ${upstream})"
done < "$PATCHES"

SHA=$(git -C "$WORK" rev-parse HEAD)
BASE=${TAG#v}
BASE=${BASE%%-*}

echo
echo "built ${BRANCH} at ${SHA}"
if [ "${REBUILD:-0}" = "1" ]; then
  echo "push:        git push --force-with-lease ${FORK_REMOTE} ${BRANCH}"
else
  echo "push:        git push ${FORK_REMOTE} ${BRANCH}"
fi
echo "build.conf:  BASE_VERSION=${BASE}"
echo "             UPSTREAM_REF=${SHA}"
echo "             VCLUSTER_ITERATION=<next iteration for ${BASE}>"
if [ ${#DROP[@]} -gt 0 ]; then
  echo "drop from patches.txt: ${DROP[*]}"
fi
