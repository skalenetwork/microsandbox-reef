# Prepare a host

reef runs every agent as a microsandbox microVM on hardware you own. This page
is what that host needs before the first agent boots, and how to find out
without guessing.

```text
Give this to your agent:

Prepare a reef host by following https://reef.clawbits.ai/docs/setup/host.md.
Measure the host first and show me the results before you change anything.
Do not guess version numbers: read the msb pin from the reef release you
install. Stop and ask me if /dev/kvm is missing, if a non-KVM hypervisor module
is loaded, or if anything already listens in 19000-19999.
```

## Measure first

```sh
uname -r; . /etc/os-release && echo "$PRETTY_NAME"; ldd --version | head -1
nproc; free -g | head -2; df -h /home | tail -1; stat -f -c %T /home
systemd-detect-virt; lsmod | grep -E 'kvm|vbox|vmmon|vmnet|xen'
ls -l /dev/kvm; getent group kvm
ss -ltn | awk '{print $4}' | grep -E ':19[0-9]{3}$' || echo "19000-19999 clear"
```

## The binary

Linux releases are glibc builds made on Ubuntu 24.04 and need **glibc 2.39 or
newer**. Ubuntu 22.04, Debian 12 and RHEL 9 cannot run them: `reef --version`
dies with `GLIBC_2.39 not found`. `ldd --version` answers that before you
install anything.

## The runtime

reef drives microsandbox rather than shipping it, and each reef release pins one
exact `msb` version. The installer takes the newest release, which is not always
the pinned one, so check rather than assume:

```sh
curl -fsSL https://install.microsandbox.dev | sh
curl -fsSL "https://raw.githubusercontent.com/skalenetwork/microsandbox-reef/v$(reef --version | cut -d' ' -f2)/crates/reef/Cargo.toml" | grep microsandbox
msb --version
```

Roll back with `msb self downgrade <version> -y` when the installer ran ahead. A
mismatch does not fail at install time. It fails at the first `agent create`
with a launch-config error that names neither version.

`msb` needs `libcap-ng0`, and on aarch64 so does `reef` itself.

## Virtualization

`/dev/kvm` has to exist, and the account that runs reef has to be in the group
that owns it. Two things make this less obvious than it looks.

**A passing `msb doctor` is not proof.** Before Linux 6.13, KVM enables
virtualization lazily at the first VM creation rather than at module load. The
device opens, every doctor row passes, and creation still fails.

**Another hypervisor may hold VMX.** VirtualBox's `vboxdrv` and VMware's
`vmmon` take VMX exclusively, and KVM cannot then create any VM at all:

```
failed to start "reef-x": VM enter: build error: start: build_microvm: Internal(Vm(VmFd(Error(16))))
```

`Error(16)` is `EBUSY` from `KVM_CREATE_VM`. The kernel discards the per-CPU
reason on the way out, so read it from the log instead:

```sh
sudo dmesg | grep -iE 'enabling virtualization|VMXON'
```

Ask the kernel directly, as the account that will run reef. This is the single
most useful check on this page:

```sh
python3 -c "import fcntl,os; fd=os.open('/dev/kvm',os.O_RDWR); print('ok', fcntl.ioctl(fd,0xAE01,0))"
```

If that fails with errno 16 while no hypervisor module is loaded, a module was
loaded and removed earlier without `VMXOFF` and only a reboot clears it.

`msb doctor` is what checks the rest of the host: CPU virtualization, the KVM
device, and whether this account can open it. `reef doctor` only reports the msb
it resolved.

## The account

Run reef as its own Unix account that owns the state and the sandboxes. Give it
a **real login shell**: sshd's `ForceCommand` runs through it, so `nologin`
breaks [remote access](/docs/enterprise/access) in a way that is annoying to
diagnose later.

```sh
sudo adduser --disabled-password --gecos '' --shell /bin/bash reef
sudo usermod -aG kvm reef
sudo -i -u reef
```

`sudo -i -u reef` starts a new session, so the group applies immediately with no
logout. Administrators reach the CLI through sudo rather than by logging in as
that account; see [remote access](/docs/enterprise/access).

## Sharing a host

**Ports.** reef allocates each published port from `19000-19999`, tracked in its
own state directory. A second reef, or anything else in that range, allocates
independently and neither can see the other. microsandbox reports a failed port
bind only in its own logs, so a collision presents as an agent that looks healthy
and answers nothing. Check the range before you start and after any
`agent rm`.

**msb.** The bundle lives at `$HOME/.microsandbox`, so separate accounts get
separate versions and separate state. That is the clean way to run two reefs on
one machine.

**cloudflared.** If a tunnel already runs here, do not use `cloudflared service
install`: it writes one `cloudflared.service` and would replace it. Give yours
its own unit and its own config directory.

## Filesystem

`msb doctor` warns `copy fallback, reflink unavailable` on ext4. Agents still
work, but each one allocates its own copy of the image root instead of sharing
extents. Prefer XFS with reflink or btrfs under `$HOME/.microsandbox` if you
plan to run many agents.

## Done when

```sh
msb doctor && reef doctor
```

`msb doctor` ends with `Host setup is ready.` and every KVM row passing, and
`reef doctor` prints the pinned msb version. Then boot something small before
anything real: `reef role apply roles/echo.toml`.
