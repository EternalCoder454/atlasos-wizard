#!/bin/bash
# Print the atlas-framework commit the crates are locked to (Cargo.lock): the
# image build checks that the tag it clones is still that commit, so a tag that
# was moved after the lock was written builds nothing.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
sha=$(sed -n 's|^source = "git+https://github.com/EternalCoder454/atlas-framework?tag=[^#"]*#\([0-9a-f]\{40\}\)"$|\1|p' Cargo.lock | sort -u)
[[ $sha =~ ^[0-9a-f]{40}$ ]] || {
    echo "framework-sha: Cargo.lock does not lock atlas-framework to one commit (got '$sha')" >&2
    exit 1
}
printf '%s\n' "$sha"
