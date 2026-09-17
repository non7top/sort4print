# Everything runs in a disposable container as the invoking user; nothing is
# installed on this machine. `./x` remains as a thin shorthand.
#
# The image, the volumes and the container are all rebuildable from the
# committed Dockerfile and compose file: a pruned Docker is a slow first run,
# never a broken one.

DOCKER_UID := $(shell id -u)
DOCKER_GID := $(shell id -g)
COMPOSE := DOCKER_UID=$(DOCKER_UID) DOCKER_GID=$(DOCKER_GID) docker compose
RUN := $(COMPOSE) run --rm build

.PHONY: help image test check build run lint fmt pack-cities shell stop destroy

help:
	@echo 'make test         core test suite, native Linux (fast: the usual loop)'
	@echo 'make check        type-check the workspace against the Windows target'
	@echo 'make build        release build -> dist/sort4print.exe'
	@echo 'make lint         clippy over the workspace'
	@echo 'make fmt          rustfmt the workspace'
	@echo 'make pack-cities  regenerate assets/cities.bin from ./data'
	@echo 'make shell        a prompt inside the build container'
	@echo 'make image        rebuild the build image'
	@echo 'make stop         stop anything still running'
	@echo 'make destroy      remove the container and its cache volumes'
	@echo
	@echo 'CI is the intended way to get a Windows binary; check and build'
	@echo 'compile the whole dependency tree for Windows and are CPU-hungry.'

image:
	$(COMPOSE) build build

test:
	$(RUN) cargo test -p sort4print-core --locked

check:
	$(RUN) cargo xwin check --target x86_64-pc-windows-msvc --workspace

build:
	$(RUN) cargo xwin build --release --target x86_64-pc-windows-msvc -p sort4print
	mkdir -p dist
	$(RUN) cp /cache/target/x86_64-pc-windows-msvc/release/sort4print.exe /work/dist/sort4print.exe
	@ls -lh dist/sort4print.exe

# There is nothing to run here: the product is a Windows GUI and this is Linux.
# Closest useful thing is the test suite, which covers everything but widgets.
run: test

lint:
	$(RUN) cargo clippy -p sort4print-core --all-targets

fmt:
	$(RUN) cargo fmt --all

pack-cities:
	$(RUN) cargo run -p pack-cities -- \
		data/cities15000.txt data/countryInfo.txt assets/cities.bin

shell:
	$(RUN) bash

stop:
	$(COMPOSE) down --remove-orphans

destroy:
	$(COMPOSE) down --remove-orphans --volumes
