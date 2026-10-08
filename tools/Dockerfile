FROM rust:1.90.0-bookworm@sha256:3914072ca0c3b8aad871db9169a651ccfce30cf58303e5d6f2db16d1d8a7e58f

RUN apt-get update \
    && apt-get install --no-install-recommends --yes \
       clang-14=1:14.0.6-12 \
       llvm-14=1:14.0.6-12 \
       lld-14=1:14.0.6-12 \
       python3 \
    && rm -rf /var/lib/apt/lists/*

ENV LLVM_CONFIG=/usr/bin/llvm-config-14 \
    IL_CLANG=/usr/bin/clang-14 \
    IL_LLC=/usr/bin/llc-14 \
    CARGO_INCREMENTAL=0 \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    TZ=UTC

WORKDIR /workspace
