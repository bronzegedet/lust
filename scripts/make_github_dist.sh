#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${1:-full}"
OUT_DIR="${ROOT_DIR}/dist/github-upload"

if [[ "${PROFILE}" != "full" && "${PROFILE}" != "lean" ]]; then
  echo "Usage: $0 [full|lean]"
  exit 1
fi

if [[ "${PROFILE}" == "lean" ]]; then
  OUT_DIR="${ROOT_DIR}/dist/github-upload-lean"
  INCLUDED_SUMMARY="- \`examples\` and dist script"
  INCLUDE_PATHS=(
    "Cargo.toml"
    "Cargo.lock"
    "README.md"
    "LICENSE"
    "CHANGELOG.md"
    "BEGINNER_TUTORIAL.md"
    "LANGUAGE_GUIDE.md"
    "ROADMAP.md"
    "src"
    "lust_src"
    "examples"
    "scripts/make_github_dist.sh"
  )
else
  INCLUDED_SUMMARY="- \`examples/tests/scripts\` plus benchmark/bootstrap source"
  INCLUDE_PATHS=(
    "Cargo.toml"
    "Cargo.lock"
    "README.md"
    "LICENSE"
    "CHANGELOG.md"
    "BEGINNER_TUTORIAL.md"
    "LANGUAGE_GUIDE.md"
    "ROADMAP.md"
    "WHERE_LUST_IS_NOW.md"
    "REPO_CLEANUP_STATUS.md"
    "IDE_VM_PROGRESS.md"
    "src"
    "lust_src"
    "examples"
    "tests"
    "scripts"
    "benchmarks"
    "bootstrap"
  )
fi

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"

for rel in "${INCLUDE_PATHS[@]}"; do
  src="${ROOT_DIR}/${rel}"
  if [[ -e "${src}" ]]; then
    mkdir -p "${OUT_DIR}/$(dirname "${rel}")"
    cp -a "${src}" "${OUT_DIR}/${rel}"
  fi
done

cat > "${OUT_DIR}/DIST_CONTENTS.md" <<EOF
# GitHub Upload Dist

This folder is a curated upload bundle created by `scripts/make_github_dist.sh`.
Profile: ${PROFILE}

Included:
- Core Rust project files and metadata (`Cargo.toml`, `Cargo.lock`)
- Primary docs (`README`, guides, roadmap/changelog)
- Source trees (`src`, `lust_src`)
${INCLUDED_SUMMARY}

Intentionally excluded:
- `.git/`
- `target/`
- `archive/`
- Existing binary/upload bundles under `dist/`
- Local/editor/system artifacts not required for source upload
EOF

echo "Created ${OUT_DIR}"
