#!/usr/bin/env bash
# Register this VM as a self-hosted GitHub Actions runner for staging.
#
# Prerequisites:
#   - Docker installed and user in "docker" group
#   - gh CLI authenticated with admin:repo scope
#   - Admin access to the repository (create registration token)
#
# Usage:
#   chmod +x scripts/setup-runner.sh
#   ./scripts/setup-runner.sh
#
# This script is idempotent — re-running it when a runner is already
# configured will remove the old one and register a fresh instance.

set -euo pipefail

REPO="RantAI-dev/RantAI-Lakehouse"
RUNNER_DIR="$HOME/actions-runner"
RUNNER_VERSION="2.337.0"
RUNNER_LABELS="staging,rep-vm"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

say() { printf "${GREEN}==>${NC} %s\n" "$*"; }
warn() { printf "${YELLOW}WARN:${NC} %s\n" "$*" >&2; }
die() { printf "${RED}FATAL:${NC} %s\n" "$*" >&2; exit 1; }

# ── pre-checks ──────────────────────────────────────────────────────────

command -v docker >/dev/null 2>&1 || die "docker not found — install it first"
docker ps >/dev/null 2>&1 || die "docker not accessible — put this user in the docker group and re-login"

[ -d "$RUNNER_DIR" ] || die "$RUNNER_DIR not found — download runner v$RUNNER_VERSION first:
  mkdir -p $RUNNER_DIR
  cd $RUNNER_DIR
  curl -fL -o runner.tar.gz https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz
  tar xzf runner.tar.gz"

# ── stop & remove any existing runner ────────────────────────────────────

cd "$RUNNER_DIR"

if [ -f .runner ]; then
  say "Removing existing runner registration ..."
  ./svc.sh stop  2>/dev/null || true
  ./svc.sh uninstall 2>/dev/null || true
  ./config.sh remove --token "" 2>/dev/null || true
  rm -f .runner .credentials .credentials_rsaparams
  say "Old runner cleaned up."
fi

# ── fetch registration token ────────────────────────────────────────────

say "Requesting registration token from GitHub ..."
TOKEN="$(gh api -X POST "repos/${REPO}/actions/runners/registration-token" --jq .token)" \
  || die "Failed to get registration token.
  Make sure your gh token has 'admin:org' scope and you are a repo admin.
  Run: gh auth refresh -h github.com -s admin:org"

say "Registration token obtained (expires in 1 hour)."

# ── configure runner ────────────────────────────────────────────────────

say "Configuring runner with labels: $RUNNER_LABELS ..."
./config.sh \
  --url "https://github.com/${REPO}" \
  --token "$TOKEN" \
  --labels "$RUNNER_LABELS" \
  --name "$(hostname)-staging" \
  --unattended \
  --replace

say "Runner configured."

# ── install as systemd service ──────────────────────────────────────────

say "Installing runner service ..."
sudo ./svc.sh install "$(whoami)" || die "Failed to install runner service"

say "Starting runner service ..."
sudo ./svc.sh start || die "Failed to start runner service"

# ── verify ──────────────────────────────────────────────────────────────

say "Runner service status:"
sleep 2
sudo ./svc.sh status 2>/dev/null || systemctl --user status actions.runner.* 2>/dev/null || true

echo ""
say "Setup complete! The runner should appear at:
   https://github.com/${REPO}/settings/actions/runners"
say "Test it: push to main or trigger 'Deploy Staging' manually:
   https://github.com/${REPO}/actions/workflows/deploy-staging.yml"