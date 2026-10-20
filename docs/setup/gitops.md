# Run hosts from git

One private GitHub repository drives every host: people change roles through
reviewed pull requests, a controller declares agents, and each host reports
back. Nothing connects to a host, and the controller never talks to one.

## The repository

Three branches, each with exactly one writer:

| branch | holds | written by |
| --- | --- | --- |
| `main` | `roles/*.toml` | people, through reviewed pull requests |
| `fleet` | `fleet/<host>/<name>.toml` | the controller |
| `status` | `status/<host>.json` | each host, its own file |

Roles and fleet files are reef's own formats. The controller removes an agent
by replacing its fleet file with a purge entry, `purge = ["<name>"]`, which
deletes its volumes and keeps the name taken.

A status file carries `version` (1), the host name, the `reef` and `msb`
versions, `at` (the current UTC time rounded down to ten minutes, so an idle
host still commits about every ten minutes), `applied` (the `main` and `fleet`
heads last applied in full, null until one lands), `result` (`ok` or
`failed`), `error` (the cause reef printed, else null), and the rows of
`reef role list`, `reef agent list` and the last 100 of `reef events`. Never
`agent get`, which prints env.

## Lock each branch to its writer

A deploy key has no branch scope: it reads and writes the whole repository.
GitHub rulesets apply to deploy keys, so they confine one. A bypass skips every
rule in its ruleset, which is why history rules sit in a ruleset nobody
bypasses:

| ruleset | targets | rules | bypass |
| --- | --- | --- | --- |
| main | `main` | pull request, 1 approval from someone other than the last pusher, stale approvals dismissed; no force push, no deletion | nobody |
| history | `fleet`, `status` | no force push, no deletion | nobody |
| fleet writer | `fleet` | restrict updates | the controller |
| status writers | `status` | restrict updates | deploy keys |
| other branches | every other branch | restrict creation, updates, deletion | write, maintain and admin roles |
| tags | every tag | restrict creation, updates, deletion | nobody |

Turn Actions off for the repository: where a workflow's token may approve pull
requests, its approval satisfies the `main` rule. All deploy keys count as one bypass actor, so a host can write another
host's status file, never roles or agents.

## Set up a host

Prepare it with [the host guide](/docs/setup/host): msb, KVM, the `reef`
account and its `secrets.toml`, checked with `reef doctor`. Skip the guide's
boot unit: this service runs `reef reconcile`, which brings agents back after a
reboot. Then, with `git` and `jq` installed:

```sh
curl -fsSL https://reef.clawbits.ai/gitops/bootstrap.sh |
  sudo REEF_HOST=prod-eu REEF_REPO=acme/reef-fleet sh
```

`REEF_HOST` names the host's directory under `fleet/` and its file under
`status/`: lowercase letters, digits and hyphens, starting with a letter. The
first run makes the `reef` account's ed25519 key, pins github.com's published
host key, prints the key and stops. Add it to the repository under Deploy keys
with write access, which the `status writers` ruleset confines, and run the
same line again. It clones one tree per branch into `~reef/agents`
(`REEF_DIR`), fetches `reconcile.sh` beside them, and installs and starts
`reef-reconcile.service` as that account (`REEF_USER`).

## What the host does

Every two seconds it asks the repository where `main` and `fleet` point, over
one SSH connection it keeps open. When either moved, or 30 seconds passed, it
pulls what moved. When the pair differs from what it last applied, it runs
`reef role apply` and then `reef fleet apply --prune` on its own fleet
directory, where an empty directory means no agents. Otherwise it runs
`reef reconcile`, which restarts any agent that died. An apply that fails
leaves the recorded heads alone, so the next tick retries it.

A repository it cannot reach changes nothing: it keeps the trees it last
pulled and still reconciles. It then writes its status file, commits only when
the content changed, rebases onto `status` and pushes. Each host touches only
its own file, so the rebase never conflicts.

## The controller

Its whole contract is writing fleet files and reading status files. Give it
one actor, the only `fleet writer` bypass: a team when people edit fleet files,
or a GitHub App when a system does. Never a personal token.

Every host and every reader of the repository can read every fleet file, and
git keeps them. Fleet env carries only values that are useless once spent,
such as a single-use, short-lived signup token. Shared secrets stay in each
host's `secrets.toml`. A role's `fleet-env` lists the env keys a fleet entry
may set; without it, any.

## Check it

```sh
systemctl status reef-reconcile
journalctl -u reef-reconcile -n 50
sudo -u reef git -C ~reef/agents/status log -1 --format='%cr'
```

While the service runs, the last status commit is never more than about ten
minutes old.
