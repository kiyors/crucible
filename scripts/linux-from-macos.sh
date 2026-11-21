#!/bin/sh
# Wrap the "Linux from macOS" steps in AGENTS.md: rsync a copy of the repo,
# start the Apple container VM, enable flakes in the `nixlinux` VM, and run
# the aarch64-linux checks there. Exits with a clear error if `container` or
# `rsync` is missing.

set -eu

if ! command -v container >/dev/null 2>&1; then
    echo "error: the Apple container CLI is not installed; install it (for" >&2
    echo "error: example 'brew install --cask container') and rerun this script" >&2
    exit 1
fi

if ! command -v rsync >/dev/null 2>&1; then
    echo "error: rsync is not installed; install it and rerun this script" >&2
    exit 1
fi

# Start the system VM if it is not already running.
if ! container system status >/dev/null 2>&1; then
    container system start || exit 1
fi

# Mount a copy rather than the working tree itself, so --delete cannot touch
# anything under the directory being edited.
# One fixed copy, refreshed on every run: the VM mounts it once, so a fresh
# temporary directory per run would leave a reused VM checking a stale copy.
copy="${XDG_CACHE_HOME:-$HOME/.cache}/crucible/nixlinux-src"
mkdir -p "$copy" || exit 1
rsync -a --delete \
    --exclude fenix \
    --exclude nix-vite-plus \
    --exclude /target \
    --exclude result \
    "$PWD/" "$copy/" || exit 1

# Reuse the nixlinux VM only when it mounts that copy; one made by hand or by
# an older version of this script mounts something else, so replace it.
if container inspect nixlinux >/dev/null 2>&1 &&
    ! container inspect nixlinux | grep -q "\"source\" *: *\"$copy\""; then
    container stop nixlinux >/dev/null 2>&1
    container delete nixlinux >/dev/null || exit 1
fi
if ! container exec nixlinux true 2>/dev/null; then
    container start nixlinux 2>/dev/null ||
        container run -d --name nixlinux --memory 8G --cpus 6 \
            -v "$copy":/src docker.io/nixos/nix:latest sleep infinity ||
        exit 1
fi

# Enable flakes (an appended duplicate is harmless; the VM may have it).
container exec nixlinux sh -c 'echo "experimental-features = nix-command flakes" >> /etc/nix/nix.conf' ||
    exit 1

# Run the aarch64-linux checks on the copy.
container exec nixlinux sh -c 'cd /src && nix flake check -L --keep-going' ||
    exit 1

echo "done: aarch64-linux nix flake check -L --keep-going passed"
