#!/usr/bin/env bash

set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
  echo "This script must be run as root." >&2
  exit 1
fi

# Remove a previously generated Docker source before the initial update. This
# avoids failing on stale entries, e.g. an Ubuntu Docker repo with a Debian
# codename from an older version of this script.
rm -f /etc/apt/sources.list.d/docker.list

apt-get update
apt-get install -y --no-install-recommends \
  ca-certificates curl gnupg lsb-release

# shellcheck disable=SC1091
. /etc/os-release

case "${ID}" in
debian | ubuntu)
  docker_dist="${ID}"
  ;;
*)
  case "${ID_LIKE:-}" in
  *debian*) docker_dist="debian" ;;
  *ubuntu*) docker_dist="ubuntu" ;;
  *)
    echo "Unsupported distro: ${PRETTY_NAME:-${ID}}" >&2
    exit 1
    ;;
  esac
  ;;
esac

codename="${VERSION_CODENAME:-$(lsb_release -cs)}"

install -m 0755 -d /etc/apt/keyrings
curl -fsSL "https://download.docker.com/linux/${docker_dist}/gpg" -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc

echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/${docker_dist} ${codename} stable" >/etc/apt/sources.list.d/docker.list

apt-get update

DOCKER_VERSION="${DOCKER_VERSION:-28.5.2}"
DOCKER_PACKAGE_VERSION="$(apt-cache madison docker-ce | awk -v version="${DOCKER_VERSION}" '$3 ~ "^5:" version "-" { print $3; exit }')"

if [[ -z ${DOCKER_PACKAGE_VERSION} ]]; then
  echo "Could not find docker-ce version ${DOCKER_VERSION} for ${docker_dist} ${codename}." >&2
  echo "Available versions:" >&2
  apt-cache madison docker-ce >&2 || true
  exit 1
fi

apt-get install -y --allow-downgrades --no-install-recommends \
  "docker-ce=${DOCKER_PACKAGE_VERSION}" \
  "docker-ce-cli=${DOCKER_PACKAGE_VERSION}" \
  containerd.io \
  docker-buildx-plugin \
  docker-compose-plugin
