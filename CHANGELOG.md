# Changelog

## [0.17.0] - 2026-09-25

### Fixed
- Upgrade to microsandbox 0.7.3, keep egress non-strict, simplify core


## [0.16.0] - 2026-09-18

### Added
- Let a role open host ports with network.host


## [0.15.2] - 2026-09-18

### Fixed
- Upgrade to microsandbox 0.7.2 and shrink reef migrate


## [0.15.1] - 2026-09-17

### Fixed
- Pin microsandbox 0.7.1 so reef migrate upgrades the msb store\


## [0.15.0] - 2026-09-17

### Added
- Serve terminals through microsandbox SSH server, no per-person enrollment
- Reef reconcile brings agents back after a reboot or crash
- Reef secret rotate pushes a changed secret into running agents live
- Preflight recreates, public-only star egress, refuse terminals on stopped agents
- Move to microsandbox 0.7, add reef migrate for 0.14 hosts


## [0.14.0] - 2026-09-09

### Added
- Major tui improvements - events tab, terminal, agent actions


## [0.11.0] - 2026-09-07

### Added
- Restructure the docs, add the clawbits role, simplify the store


## [0.10.0] - 2026-09-03

### Added
- Update to msb 0.6.16


## [0.9.0] - 2026-09-02

### Added
- Add roles viewer to tui


## [0.8.0] - 2026-09-02

### Added
- Reef ui, a console over the --json rows


## [0.7.0] - 2026-09-01

### Added
- Richer agent list and get output


## [0.6.0] - 2026-08-31

### Added
- Deny the host and loopback to every agent


## [0.5.0] - 2026-08-31

### Added
- Implement [files] for roles
- Add openclaw-browser role, add option to allow all hosts


## [0.4.0] - 2026-08-28

### Added
- Name agents at <agent>.localhost and print their URLs


## [0.3.0] - 2026-08-27

### Added
- Add serve cmd, add docs, update openclaw example


## [0.2.0] - 2026-08-20

### Added
- Add volumes support, update hermes reference
- Non-destructive env patching
- Fleet files, exposed ports, per-agent env
- Add active ports to forward cmd, fix minor bugs

### Fixed
- Package publish
- Package publish
