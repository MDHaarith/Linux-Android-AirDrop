#!/usr/bin/env bash
set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "Usage: $0 PACKAGE..." >&2
  exit 2
fi

# GitHub's Ubuntu runner uses this mirror list for its Ubuntu sources. The
# Azure mirror took 38 minutes to serve 61 MB in release run 39795828530.
# Prefer the official archive and retain the other mirrors as fallbacks.
printf '%s\n' \
  $'https://archive.ubuntu.com/ubuntu/\tpriority:1' \
  $'https://security.ubuntu.com/ubuntu/\tpriority:2' \
  $'http://azure.archive.ubuntu.com/ubuntu/\tpriority:3' \
  | sudo tee /etc/apt/apt-mirrors.txt > /dev/null

apt_options=(
  -o Acquire::Retries=2
  -o Acquire::http::Timeout=20
  -o Acquire::https::Timeout=20
)

# The calling workflow also bounds the entire step: an idle timeout alone
# cannot stop a mirror that keeps sending data at a very low rate.
sudo env DEBIAN_FRONTEND=noninteractive apt-get "${apt_options[@]}" update
sudo env DEBIAN_FRONTEND=noninteractive apt-get "${apt_options[@]}" \
  install -y --no-install-recommends "$@"
