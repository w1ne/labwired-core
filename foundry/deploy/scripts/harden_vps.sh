#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   sudo ./harden_vps.sh
#
# This applies a baseline hardening profile for a public Docker host.

if [[ "${EUID}" -ne 0 ]]; then
  echo "Run as root: sudo $0" >&2
  exit 1
fi

echo "[1/6] Installing security packages..."
export DEBIAN_FRONTEND=noninteractive
apt-get update -y
apt-get install -y ufw fail2ban unattended-upgrades

echo "[2/6] Configuring automatic security updates..."
dpkg-reconfigure -f noninteractive unattended-upgrades || true
cat >/etc/apt/apt.conf.d/20auto-upgrades <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
EOF

echo "[3/6] Hardening SSH daemon..."
cp /etc/ssh/sshd_config "/etc/ssh/sshd_config.bak.$(date +%Y%m%d%H%M%S)"

set_or_add() {
  local key="$1"
  local value="$2"
  if grep -Eq "^[#[:space:]]*${key}[[:space:]]+" /etc/ssh/sshd_config; then
    sed -i -E "s|^[#[:space:]]*${key}[[:space:]]+.*|${key} ${value}|g" /etc/ssh/sshd_config
  else
    echo "${key} ${value}" >>/etc/ssh/sshd_config
  fi
}

set_or_add "PermitRootLogin" "no"
set_or_add "PasswordAuthentication" "no"
set_or_add "KbdInteractiveAuthentication" "no"
set_or_add "ChallengeResponseAuthentication" "no"
set_or_add "MaxAuthTries" "3"
set_or_add "X11Forwarding" "no"

sshd -t
systemctl restart ssh || systemctl restart sshd

echo "[4/6] Enabling fail2ban for SSH..."
cat >/etc/fail2ban/jail.d/sshd.local <<'EOF'
[sshd]
enabled = true
backend = systemd
maxretry = 5
findtime = 10m
bantime = 1h
EOF

systemctl enable --now fail2ban

echo "[5/6] Configuring UFW firewall..."
ufw --force default deny incoming
ufw --force default allow outgoing
ufw allow OpenSSH
ufw allow 80/tcp
ufw allow 443/tcp
ufw --force enable

echo "[6/6] Done."
echo "Harden baseline applied. Verify active rules with: ufw status verbose"
