#!/bin/bash
# Run a command in the dev container (the image CI uses, plus tools for
# headless GUI runs), with the repo at /src and the cargo cache in named
# podman volumes.
#   scripts/dev.sh <command...>     e.g. scripts/dev.sh cargo test --workspace
#   scripts/dev.sh                  an interactive shell
# The image is built from ci/Containerfile on first use, and again whenever the
# Containerfile, the spec's BuildRequires or the atlas-framework tag in
# Cargo.toml change (ci/image-tag.sh). It needs network access: it builds
# telamon-ui from atlas-framework's tag. TELAMON_FRAMEWORK_REF=<tag or branch>
# builds from another ref (the image is rebuilt when it changes).
# Set CARGO_TARGET_DIR to /src/target/<name> to keep one target dir per task.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
image=localhost/telamon-wizard-dev:44
pinned=$("$repo/ci/framework-ref.sh")
ref=${TELAMON_FRAMEWORK_REF:-$pinned}
want=$("$repo/ci/image-tag.sh")
[ "$ref" = "$pinned" ] || want=$want-$ref

have=$(podman image inspect --format '{{ index .Labels "net.eterneon.telamon.wizard.image-tag" }}' "$image" 2>/dev/null || true)
if [ "$have" != "$want" ]; then
    podman build -f "$repo/ci/Containerfile" --target dev \
        --build-arg TELAMON_FRAMEWORK_REF="$ref" --build-arg IMAGE_TAG="$want" \
        -t "$image" "$repo" >&2
fi

tty=()
[ -t 0 ] && tty=(-it)
exec podman run --rm --security-opt label=disable "${tty[@]}" \
    -v "$repo":/src -w /src \
    -v telamon-cargo:/root/.cargo/registry \
    -v telamon-cargo-git:/root/.cargo/git \
    -e CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/src/target/dev}" \
    "$image" "${@:-bash}"
