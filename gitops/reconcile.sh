#!/bin/sh
set -u

: "${REEF_HOST:?REEF_HOST is required}"
: "${REEF_DIR:=$HOME/agents}"
REEF="${REEF:-$HOME/.local/bin/reef}"
MSB="${MSB_PATH:-$HOME/.microsandbox/bin/msb}"
export GIT_SSH_COMMAND="${GIT_SSH_COMMAND:-ssh -o ConnectTimeout=10 -o ServerAliveInterval=10 -o ServerAliveCountMax=3 -o ControlMaster=auto -o ControlPath=~/.ssh/%C -o ControlPersist=60}"
FLEET="$REEF_DIR/fleet/fleet/$REEF_HOST"

apply() {
  "$REEF" role apply "$REEF_DIR"/main/roles/*.toml || return 1
  mkdir -p "$FLEET"
  "$REEF" fleet apply "$FLEET" --prune
}

cause() {
  printf '%s\n' "$1" |
    { grep -E '^[^ ]+: ' | grep -v '^Error: ' || printf '%s\n' "$1"; } |
    tail -n 1 | sed 's/^[[:space:]]*//' | cut -c 1-200
}

tick() (
  set -e
  for tree in main fleet; do
    printf '%s\n' "$heads" | grep -qx "$(git -C "$REEF_DIR/$tree" rev-parse HEAD)[[:space:]]refs/heads/$tree" ||
      git -C "$REEF_DIR/$tree" pull --quiet --ff-only || true
  done

  applied="$REEF_DIR/applied"
  declared="$(git -C "$REEF_DIR/main" rev-parse HEAD) $(git -C "$REEF_DIR/fleet" rev-parse HEAD)"
  result=ok
  error=
  if [ "$declared" != "$(cat "$applied" 2>/dev/null || true)" ]; then
    if out="$(apply 2>&1)"; then
      printf '%s' "$declared" > "$applied"
    else
      result=failed
      error="$(cause "$out")"
    fi
    printf '%s\n' "$out"
  elif ! out="$("$REEF" reconcile 2>&1)"; then
    result=failed
    error="$(cause "$out")"
    printf '%s\n' "$out"
  fi

  at="$(date -u +%Y-%m-%dT%H:%M)"
  file="$REEF_DIR/status/status/$REEF_HOST.json"
  mkdir -p "${file%/*}"
  jq -n \
    --arg host "$REEF_HOST" \
    --arg reef "$("$REEF" --version | cut -d' ' -f2)" \
    --arg msb "$("$MSB" --version | cut -d' ' -f2)" \
    --arg at "${at%?}0:00Z" \
    --arg applied "$(cat "$applied" 2>/dev/null || true)" \
    --arg result "$result" \
    --arg error "$error" \
    --argjson roles "$("$REEF" role list --json)" \
    --argjson agents "$("$REEF" agent list --json)" \
    --argjson events "$("$REEF" events --json | jq '.[-100:]')" \
    '{version: 1, host: $host, reef: $reef, msb: $msb, at: $at,
      applied: ($applied | split(" ") | if length == 2 then {main: .[0], fleet: .[1]} else null end),
      result: $result, error: (if $error == "" then null else $error end),
      roles: $roles, agents: $agents, events: $events}' > "$file"

  git -C "$REEF_DIR/status" add "status/$REEF_HOST.json"
  git -C "$REEF_DIR/status" diff --cached --quiet ||
    git -C "$REEF_DIR/status" commit --quiet -m "status $REEF_HOST"
  if [ -n "$(git -C "$REEF_DIR/status" rev-list '@{u}..')" ]; then
    git -C "$REEF_DIR/status" pull --quiet --rebase
    git -C "$REEF_DIR/status" push --quiet
  fi
)

trap 'exit 0' TERM
seen=
last=0
while :; do
  heads="$(git -C "$REEF_DIR/main" ls-remote origin refs/heads/main refs/heads/fleet 2>/dev/null)"
  now="$(date +%s)"
  if [ "$heads" != "$seen" ] || [ $((now - last)) -ge 30 ]; then
    tick
    seen="$heads" last="$now"
  fi
  sleep 2
done
