# Disposable build image: cross-compiles the Windows exe from Linux.
# Nothing in here is a pet container -- every invocation is `run --rm`.
FROM rust:1-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
        clang \
        lld \
        unzip \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# cargo-xwin pulls the MSVC CRT + Windows SDK headers so we can target
# *-pc-windows-msvc without Windows or mingw. Installed into /usr/local so it is
# on PATH for whatever uid the container is run as.
RUN CARGO_INSTALL_ROOT=/usr/local cargo install cargo-xwin --locked

RUN rustup target add x86_64-pc-windows-msvc

# Caches live on named volumes. The mode-1777 directories baked into the image
# are what let an arbitrary (non-root, host-matching) uid write to those volumes,
# since a named volume inherits ownership from the image content at its path.
ENV CARGO_HOME=/cache/cargo \
    CARGO_TARGET_DIR=/cache/target \
    XWIN_CACHE_DIR=/cache/xwin \
    HOME=/cache
RUN mkdir -p /cache/cargo /cache/target /cache/xwin && chmod -R 1777 /cache

WORKDIR /work
