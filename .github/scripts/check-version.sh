#!/usr/bin/env bash
# Release : lancée sur un tag vX.Y.Z identique à la version de l'application (tauri.conf.json
# et Cargo.toml). Appelé par les jobs test et build de release.yml
set -euo pipefail
if [ "$GITHUB_REF_TYPE" != tag ]; then
  echo "::error::À lancer sur un tag vX.Y.Z (--ref vX.Y.Z), pas sur $GITHUB_REF_NAME"
  exit 1
fi
tag="${GITHUB_REF_NAME#v}"
conf=$(jq -r .version src-tauri/tauri.conf.json)
cargo=$(grep -m1 '^version' src-tauri/Cargo.toml | cut -d'"' -f2)
echo "tag $tag, tauri.conf.json $conf, Cargo.toml $cargo"
if [ "$tag" != "$conf" ] || [ "$tag" != "$cargo" ]; then
  echo "::error::Le tag $GITHUB_REF_NAME ne correspond pas à la version de l'application ($conf / $cargo)"
  exit 1
fi
