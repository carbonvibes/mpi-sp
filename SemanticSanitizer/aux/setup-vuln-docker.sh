#!/usr/bin/env bash

set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
  echo "This script must be run as root." >&2
  exit 1
fi

DIAGNOSTIC_PORT="${DIAGNOSTIC_PORT:-8080}"
STACKDUMP_DIR="${STACKDUMP_DIR:-/tmp/semsan-docker-tmp}"
TARGET="${TARGET:-${STACKDUMP_DIR}/example}"
STACKDUMP_OWNER_UID="${STACKDUMP_OWNER_UID:-${SUDO_UID:-1000}}"
STACKDUMP_OWNER_GID="${STACKDUMP_OWNER_GID:-${SUDO_GID:-1000}}"

echo "Disabling protected links.."
sysctl fs.protected_symlinks=0
sysctl fs.protected_regular=0

echo "Enabling diagnostic server on port ${DIAGNOSTIC_PORT}.."
install -m 0755 -d /etc/docker

python3 - "${DIAGNOSTIC_PORT}" <<'PY'
import json
import os
import sys

port = int(sys.argv[1])
path = "/etc/docker/daemon.json"

if os.path.exists(path) and os.path.getsize(path) > 0:
    with open(path, encoding="utf-8") as f:
        config = json.load(f)
else:
    config = {}

config["network-diagnostic-port"] = port

with open(path, "w", encoding="utf-8") as f:
    json.dump(config, f, indent=2, sort_keys=True)
    f.write("\n")
PY

echo "Creating non-root-owned Docker temp directory at ${STACKDUMP_DIR}.."
install -m 0755 -d "${STACKDUMP_DIR}"
chown "${STACKDUMP_OWNER_UID}:${STACKDUMP_OWNER_GID}" "${STACKDUMP_DIR}"

if command -v systemctl >/dev/null 2>&1; then
  echo "Configuring Docker to use ${STACKDUMP_DIR} as TMPDIR.."
  install -m 0755 -d /etc/systemd/system/docker.service.d
  cat >/etc/systemd/system/docker.service.d/10-semsan-vuln-docker.conf <<EOF
[Service]
Environment=TMPDIR=${STACKDUMP_DIR}
EOF

  echo "Restarting Docker to apply TMPDIR.."
  systemctl daemon-reload
  systemctl restart docker

  echo "Reloading Docker to enable the diagnostic server.."
  systemctl reload docker
else
  echo "systemd is required to configure dockerd's TMPDIR automatically." >&2
  echo "Restart dockerd manually with TMPDIR=${STACKDUMP_DIR} and rerun this script." >&2
  exit 1
fi

echo "Creating root-owned example file at ${TARGET}.."
echo "foo" >"${TARGET}"
chown root:root "${TARGET}"

if ! timeout 10 bash -c "until curl -fsS http://localhost:${DIAGNOSTIC_PORT}/help >/dev/null; do sleep 0.5; done"; then
  echo "Docker diagnostic server did not become reachable on port ${DIAGNOSTIC_PORT}." >&2
  echo "Check Docker logs with:" >&2
  echo "  journalctl -u docker -n 200 --no-pager" >&2
  echo "and verify the daemon config with:" >&2
  echo "  cat /etc/docker/daemon.json" >&2
  exit 1
fi

echo "Diagnostic server enabled on port ${DIAGNOSTIC_PORT}."
echo "Docker stack dumps should now be written under ${STACKDUMP_DIR}."
