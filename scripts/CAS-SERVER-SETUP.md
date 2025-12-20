# CAS Server Setup - rrsync

Simple setup using standard `rrsync` (no custom gateway needed).

## Server Setup

```bash
# 1. Install rrsync (comes with rsync package on most systems)
# Debian/Ubuntu: /usr/bin/rrsync or /usr/share/doc/rsync/scripts/rrsync
# If not found, download from rsync source

# 2. Create CAS user and directory
sudo useradd -r -s /usr/sbin/nologin -d /srv/cas cas
sudo mkdir -p /srv/cas/cas/{sha256,pointers}
sudo chown -R cas:cas /srv/cas

# 3. Set up SSH key (get public key from CAS_SSH_KEY secret)
sudo mkdir -p /home/cas/.ssh
sudo chmod 700 /home/cas/.ssh

# 4. Add to /home/cas/.ssh/authorized_keys:
command="rrsync -wo /srv/cas/cas",no-pty,no-agent-forwarding,no-port-forwarding,no-X11-forwarding ssh-ed25519 AAAA... your-key-here

# Flags explained:
# -w: write-only (no downloads) - remove to allow downloads
# -o: write-once (prevent overwrites) - remove to allow overwrites
# /srv/cas/cas: chroot to this directory

sudo chmod 600 /home/cas/.ssh/authorized_keys
sudo chown -R cas:cas /home/cas

# 5. Test connection from client
ssh cas@golem.bearcove.cloud
# Should print: Please use rsync to access this directory.
```

## Directory Structure

```
/srv/cas/cas/
  sha256/
    XX/                  # First 2 chars of hash
      <hash>             # Full SHA256 hash as filename
  pointers/
    ci/
      <run_id>/
        <name>           # Pointer file containing hash
```

## Properties

- **Content-addressed**: Same bytes = same hash = stored once
- **Race-safe**: Multiple uploads of same hash write identical bytes
- **Simple**: Just standard rrsync, no custom scripts
- **Fast**: SSH ControlMaster reuses connections

## Debugging

```bash
# On server, check what cas user can do:
sudo -u cas ls -la /srv/cas/cas/

# Check disk usage:
sudo du -sh /srv/cas/cas/sha256/

# Find recent uploads:
sudo find /srv/cas/cas/sha256 -type f -mmin -60 | head

# Clean up old pointers (optional):
sudo find /srv/cas/cas/pointers -type f -mtime +30 -delete
```
