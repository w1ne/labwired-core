# Deployment Lessons Learned: LabWired Foundry to Pluto

This document summarizes the insights and challenges encountered during the deployment to Pluto (`89.167.12.41`) to improve future automation.

## 1. Server Bootstrapping & Permissions
- **Challenge**: Initial plan used `/srv/labwired`, which requires `sudo`. The `w1ne` user did not have passwordless sudo, blocking directory creation and package installation.
- **Lesson**: Standardize on a user-owned deployment path (e.g., `~/deploy/` or `~/labwired/`) for initial bootstrapping to minimize sudo dependencies.
- **Action**: Update deployment scripts to allow configurable paths that don't require root.

## 2. Authentication & Repository Access
- **Challenge**: `git clone` on the server failed because the repository is private and the server lacked SSH keys/credentials.
- **Solution**: Used `rsync` to sync the local repository state directly to the server.
- **Lesson**: For "push-style" deployments (local machine to VPS), `rsync` or `docker push/pull` is more reliable than "pull-style" git clones.

## 3. Build Environment Constraints
- **Challenge**: Attempting to build the `labwired-cli` (Rust) on Pluto failed because the server lacked a C compiler (`cc`), and `sudo` was required to install `build-essential`.
- **Solution**: Performed a local build and synced the binary (verified `x86_64` compatibility).
- **Lesson**: Do not rely on compilation on the target VPS. Use CI/CD (GitHub Actions) to build binaries/images or build locally.

## 4. Docker Permission Management
- **Challenge**: The `w1ne` user was not in the `docker` group, leading to "permission denied" errors when running `docker compose`.
- **Lesson**: Pre-flight checks should verify if the current user can communicate with the Docker daemon.

## 5. Configuration & Connectivity
- **Challenge**: Default `Caddyfile` used a domain name, but no domain was yet pointed to Pluto.
- **Solution**: Updated `Caddyfile` for direct IP-based access (`:80`).
- **Lesson**: Deployment templates should support a "Quick Start" IP-based mode.

## Proposed Strategy Improvements

- **CI-Generated Binaries**: Ensure GitHub Actions builds the `labwired` core binary as a release artifact so it can be downloaded directly by the backend Docker image.
- **Docker-Centric Setup**: Move the `labwired` binary inside a dedicated "worker" container or the backend container to eliminate host-path dependency (`/home/w1ne/bin/labwired`).
- **Pre-flight Script**: Create a `check_deps.sh` that validates Docker access, architecture, and required paths before attempting deployment.

## Security Hardening Addendum (Production)

- **Secrets Rotation First**: If any key appears in terminal history/logs, rotate it immediately before further deploys.
- **Host Baseline Script**: Use `foundry/deploy/scripts/harden_vps.sh` to enforce SSH key-only auth, fail2ban, UFW (`22/80/443`), and unattended security updates.
- **Deploy Verification Script**: Run `foundry/deploy/scripts/verify_foundry.sh https://<domain>` after each rollout.
- **Container Runtime Hardening**: Keep `read_only`, `no-new-privileges`, dropped capabilities, and tmpfs mounts in production compose.
- **Proxy Guardrails**: Keep Caddy request size limits and security headers enabled by default.
