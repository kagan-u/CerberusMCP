#!/usr/bin/env bash
# Sync the GUI bundle version with the release tag (v*). On non-tag runs
# (workflow_dispatch) the current tauri.conf.json version is kept.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

if [ $# -ge 1 ]; then
  v="$1"
else
  case "${GITHUB_REF_NAME:-}" in
    v*) v="${GITHUB_REF_NAME#v}" ;;
    *) v="$(jq -r .version gui/src-tauri/tauri.conf.json)" ;;
  esac
fi

echo "GUI version: $v"
jq --arg v "$v" '.version = $v' gui/src-tauri/tauri.conf.json > /tmp/tauri.conf.json
mv /tmp/tauri.conf.json gui/src-tauri/tauri.conf.json

py="$(command -v python3 || command -v python)"
"$py" - "$v" <<'PY'
import re, sys
p = "gui/src-tauri/Cargo.toml"
s = open(p).read()
s = re.sub(r'(?m)^version = ".*"$', f'version = "{sys.argv[1]}"', s, count=1)
open(p, "w").write(s)
PY
