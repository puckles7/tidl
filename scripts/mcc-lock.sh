#!/usr/bin/env bash
# mcc merge lock — serialises concurrent `mcc` runs across a repo and all of
# its git worktrees.
#
# The flag lives in the repo's *common* git dir (`git rev-parse
# --git-common-dir`), so every checkout of the same repository — the main one
# and every linked worktree — sees the same lock. It is never tracked.
#
# Usage:
#   scripts/mcc-lock.sh acquire [--force] [--poll-secs N] [--stale-secs N]
#   scripts/mcc-lock.sh release
#   scripts/mcc-lock.sh status
#
#   acquire  Atomically claim the flag. If another mcc holds it, poll every
#            15s (default; --poll-secs to change) until it is released, then
#            claim it. Returns only once *this* run owns the flag.
#   release  Drop the flag. Always call this when the merge is finished.
#   status   Print who holds the flag and for how long.
#
# Race-safety: the flag is a *directory*, claimed with mkdir(2), which is
# atomic — exactly one concurrent claimer can succeed, even if many wake up
# at the same instant when a holder releases. Never check-then-create (that is
# TOCTOU-racy). Release renames the flag dir aside before deleting it, so the
# lock path is freed atomically and a mid-delete window can't hand the flag to
# two waiters.
set -euo pipefail

SELF="${0##*/}"
POLL_SECS="${MCC_LOCK_POLL_SECS:-15}"
STALE_SECS="${MCC_LOCK_STALE_SECS:-3600}"
FORCE=0
CMD=""

while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1; shift ;;
    --poll-secs) POLL_SECS="$2"; shift 2 ;;
    --stale-secs) STALE_SECS="$2"; shift 2 ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) CMD="$1"; shift ;;
  esac
done

COMMON_DIR="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || {
  echo "$SELF: not inside a git repository" >&2
  exit 1
}
LOCK_DIR="$COMMON_DIR/mcc.lock.d"

holder_summary() {
  if [ -f "$LOCK_DIR/owner" ]; then
    # shellcheck disable=SC1090
    . "$LOCK_DIR/owner"
    echo "${branch:-?}@${worktree:-?} (pid ${pid:-?})"
  else
    echo "unknown owner"
  fi
}

lock_age_secs() {
  local mtime now
  mtime="$(stat -c %Y "$LOCK_DIR" 2>/dev/null)" || return 1
  now="$(date +%s)"
  echo $((now - mtime))
}

claim() {
  if mkdir "$LOCK_DIR" 2>/dev/null; then
    {
      printf 'pid=%s\n' "$$"
      printf 'branch=%s\n' "$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')"
      printf 'worktree=%s\n' "$PWD"
      printf 'started=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    } > "$LOCK_DIR/owner"
    return 0
  fi
  return 1
}

release_flag() {
  [ -d "$LOCK_DIR" ] || return 0
  local graveyard="$LOCK_DIR.released.$$"
  # Atomic rename frees the lock path in one step, then clean up off-path.
  if ! mv "$LOCK_DIR" "$graveyard" 2>/dev/null; then
    return 0
  fi
  rm -rf "$graveyard"
}

do_acquire() {
  while ! claim; do
    if [ "$FORCE" -eq 1 ]; then
      echo "$SELF: --force: stealing lock held by $(holder_summary)" >&2
      release_flag
      continue
    fi
    local age
    if age="$(lock_age_secs)"; then
      if [ "$age" -ge "$STALE_SECS" ]; then
        echo "$SELF: WARNING: lock is ${age}s old (>= ${STALE_SECS}s); a previous mcc may have crashed." >&2
        echo "$SELF:          inspect '$LOCK_DIR/owner', then re-run with --force to steal it." >&2
      fi
    fi
    echo "$SELF: another merge is in progress — $(holder_summary); retrying in ${POLL_SECS}s" >&2
    sleep "$POLL_SECS"
  done
  echo "$SELF: lock acquired ($LOCK_DIR)"
}

do_release() {
  if [ ! -d "$LOCK_DIR" ]; then
    echo "$SELF: no lock held"
    return 0
  fi
  release_flag
  echo "$SELF: lock released"
}

do_status() {
  if [ ! -d "$LOCK_DIR" ]; then
    echo "$SELF: free (no merge in progress)"
    return 0
  fi
  local age="?"
  age="$(lock_age_secs 2>/dev/null)" || true
  echo "$SELF: held by $(holder_summary) for ${age}s"
  [ -f "$LOCK_DIR/owner" ] && cat "$LOCK_DIR/owner"
}

case "$CMD" in
  acquire) do_acquire ;;
  release) do_release ;;
  status) do_status ;;
  *) echo "usage: $SELF {acquire|release|status} [--force] [--poll-secs N] [--stale-secs N]" >&2; exit 2 ;;
esac
