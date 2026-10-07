# agyctl

A unified CLI and background daemon for Antigravity:
- **Profile Management & Quota Tracking**: Instant switching between accounts and live AI quota tracking with zero CLI latency.
- **Background Daemon**: A native systemd user service (`agyctl.service`) that periodically updates quotas in the background.
- **History & Project Syncing**: Bidirectional sync between local machines and remote servers with automated SQLite and Protobuf wire-format path translation.

---

## Features

- **Fast CLI**: Reads directly from an atomically updated local cache (`~/.config/agyctl/cache.json`).
- **Profile Switching**: Save and switch accounts with `agyctl switch <name>`.
- **AI Quota Telemetry**: Queries both upstream APIs and the local Antigravity Language Server for quota windows and reset countdowns.
- **Subscription Tracking**: Monitor renewal and expiration dates directly in the account table.
- **Protobuf Wire Rewriter**: Safely parses and modifies conversation states and project paths across local and remote environments.
- **Systemd Integration**: User service management via `agyctl service enable|disable|status|restart|logs`.

---

## Installation

### Debian / Ubuntu (.deb)
```bash
sudo dpkg -i agyctl_<version>_amd64.deb
```
The package automatically installs the binary to `/usr/bin/agyctl` and sets up the systemd user service.

### Building from Source
```bash
cargo build --release
cp target/release/agyctl ~/.local/bin/agyctl
agyctl service enable
```

---

## Commands

### Accounts & Quotas
```bash
agyctl list                      # List accounts with cached quotas
agyctl current                   # Show active account details
agyctl switch <name> [--restart] # Switch active account (optional: restart IDE)
agyctl save <name> [--expires]   # Save active session as profile
agyctl set <name> [--expires]    # Update profile tier or expiration
agyctl usage [--all]             # Detailed model usage and reset timers
agyctl delete <name>             # Delete saved profile
```

### Remote Synchronization
```bash
agyctl status                    # Compare local and remote history
agyctl pull [--projects]         # Pull remote history and projects
agyctl push [--projects]         # Push local history and projects
agyctl project list              # List known projects
```

### Service Management
```bash
agyctl service status            # Check background service status
agyctl service enable            # Enable and start background daemon
agyctl service disable           # Disable and stop background daemon
agyctl service restart           # Restart daemon
agyctl service logs              # View background daemon logs
```

---

## Output Examples

### Account Overview
```
Accounts & Quotas  cached 12s ago  daemon active

   NAME     TIER     EMAIL                    EXPIRATION       GEMINI   5-HOUR     CLAUDE  STATUS
*  work     AI Pro   dev@example.com          Nov 20 (44d)     100.0%   100.0%     100.0%  active
   backup   pro      alt@example.com          Auto              82.8%     0.0%  exhausted  ready
```

### Usage Breakdown
```
Usage & Limits (dev@example.com, Pro)

  Gemini Models
    Weekly Limit Remaining   [██████████]  100.0% remaining  resets in 6d 4h
    Five Hour Limit Remaining [██████████]  100.0% remaining  resets in 4h 30m

  Claude and GPT models
    Weekly Limit Remaining   [██████████]  100.0% remaining  resets in 6d 23h
    Five Hour Limit Remaining [██████████]  100.0% remaining  resets in 4h 45m
```

---

## License

MIT License