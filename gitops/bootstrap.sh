#!/bin/sh
set -eu

: "${REEF_HOST:?REEF_HOST is required, for example prod-eu}"
: "${REEF_REPO:?REEF_REPO is required, for example acme/reef-fleet}"
ACCOUNT="${REEF_USER:-reef}"
SRC="${REEF_SRC:-https://reef.clawbits.ai/gitops}"

[ "$(id -u)" = 0 ] || { echo "run as root, through sudo" >&2; exit 1; }
echo "$REEF_HOST" | grep -Eq '^[a-z]([a-z0-9-]{0,38}[a-z0-9])?$' ||
  { echo "REEF_HOST must be lowercase letters, digits and hyphens, starting with a letter" >&2; exit 1; }
HOME_DIR="$(getent passwd "$ACCOUNT" | cut -d: -f6)"
[ -n "$HOME_DIR" ] || { echo "no $ACCOUNT account: prepare the host first, https://reef.clawbits.ai/docs/setup/host" >&2; exit 1; }
for cmd in git jq curl ssh-keygen; do
  command -v "$cmd" >/dev/null || { echo "$cmd is required" >&2; exit 1; }
done

DIR="${REEF_DIR:-$HOME_DIR/agents}"
KEY="$HOME_DIR/.ssh/id_ed25519"
as_reef() { runuser -u "$ACCOUNT" -- "$@"; }

as_reef mkdir -p -m 700 "$HOME_DIR/.ssh"
[ -f "$KEY" ] || as_reef ssh-keygen -q -t ed25519 -N '' -f "$KEY" -C "reef@$REEF_HOST"
grep -qs '^github.com ssh-ed25519' "$HOME_DIR/.ssh/known_hosts" ||
  echo 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl' |
  as_reef tee -a "$HOME_DIR/.ssh/known_hosts" >/dev/null

if ! as_reef ssh -o BatchMode=yes -T git@github.com 2>&1 | grep -q 'successfully authenticated'; then
  echo "Add this key to github.com/$REEF_REPO under Settings > Deploy keys, with write access, then run this again:"
  cat "$KEY.pub"
  exit 1
fi

as_reef mkdir -p "$DIR"
for branch in main fleet status; do
  [ -d "$DIR/$branch/.git" ] ||
    as_reef git clone --quiet --branch "$branch" --single-branch "git@github.com:$REEF_REPO.git" "$DIR/$branch"
done
as_reef git -C "$DIR/status" config user.name reef
as_reef git -C "$DIR/status" config user.email "reef@$REEF_HOST"
as_reef curl -fsSL "$SRC/reconcile.sh" -o "$DIR/reconcile.sh"
chmod 755 "$DIR/reconcile.sh"

cat > /etc/systemd/system/reef-reconcile.service <<UNIT
[Unit]
Description=Reconcile this host against its reef repository
After=network-online.target
Wants=network-online.target

[Service]
User=$ACCOUNT
Environment=REEF_HOST=$REEF_HOST
Environment=REEF_DIR=$DIR
ExecStart=$DIR/reconcile.sh
KillMode=process
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable reef-reconcile
systemctl restart reef-reconcile

echo "$REEF_HOST reconciles against $REEF_REPO."
