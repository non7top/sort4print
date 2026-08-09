#!/usr/bin/env bash
# Run a command inside the disposable build container as the host user.
#
#   ./x test                 -- run the core test suite (native Linux)
#   ./x check                -- type-check the whole workspace for Windows
#   ./x build                -- release-build sort4print.exe into ./dist
#   ./x pack-cities          -- regenerate assets/cities.bin from ./data
#   ./x <anything else>      -- run it verbatim in the container
set -euo pipefail

cd "$(dirname "$0")"
DOCKER_UID="$(id -u)"
DOCKER_GID="$(id -g)"
export DOCKER_UID DOCKER_GID

run() { docker compose run --rm build "$@"; }

case "${1:-build}" in
  test)
    shift
    run cargo test -p sort4print-core "$@"
    ;;
  check)
    shift
    run cargo xwin check --target x86_64-pc-windows-msvc --workspace "$@"
    ;;
  build)
    shift
    run cargo xwin build --release --target x86_64-pc-windows-msvc -p sort4print "$@"
    mkdir -p dist
    run cp /cache/target/x86_64-pc-windows-msvc/release/sort4print.exe /work/dist/sort4print.exe
    ls -lh dist/sort4print.exe
    ;;
  pack-cities)
    shift
    run cargo run -p pack-cities -- data/cities15000.txt data/countryInfo.txt assets/cities.bin "$@"
    ;;
  *)
    run "$@"
    ;;
esac
