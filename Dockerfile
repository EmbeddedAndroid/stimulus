# syntax=docker/dockerfile:1
FROM rust:1.98-slim-trixie AS rust-builder
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends pkg-config git ca-certificates && rm -rf /var/lib/apt/lists/* && rustup component add rustfmt clippy
ENV CARGO_HOME=/usr/local/cargo CARGO_TERM_COLOR=always
WORKDIR /work
COPY . .
RUN --mount=type=cache,id=logicport-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=logicport-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=logicport-release-target,target=/work/target \
    cargo build --release -p analyzerd \
    && cp /work/target/release/analyzerd /tmp/analyzerd

FROM node:24-trixie-slim AS web-builder
WORKDIR /web
COPY web ./
RUN npm ci && npm run build

FROM debian:trixie-slim AS runtime
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates tini curl && rm -rf /var/lib/apt/lists/*
COPY --from=rust-builder /tmp/analyzerd /usr/local/bin/analyzerd
COPY --from=web-builder /web/dist /usr/local/share/logicport/web
COPY fixtures/vendor/LogicPort.ccf /usr/local/share/logicport/LogicPort.ccf
COPY fixtures/vendor/examples /usr/local/share/logicport/examples
ENV LP_CCF=/usr/local/share/logicport/LogicPort.ccf LP_WEB=/usr/local/share/logicport/web RUST_LOG=info
EXPOSE 8471
ENTRYPOINT ["/usr/bin/tini","--","/usr/local/bin/analyzerd"]
CMD ["serve","--bind","0.0.0.0:8471"]

FROM rust-builder AS dev
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends nodejs npm python3 curl jq procps usbutils && rm -rf /var/lib/apt/lists/* \
 && cargo install cargo-nextest --locked --version "0.9.*" \
 && cargo install cargo-watch --locked --version "8.5.*"
ENV CARGO_TARGET_DIR=/work/target-docker
CMD ["cargo","nextest","run","--workspace"]

FROM mcr.microsoft.com/playwright:v1.62.0-noble AS e2e
WORKDIR /work/web
CMD ["npx","playwright","test","-c","../tests/e2e/playwright.config.ts"]

FROM debian:trixie-slim AS tools
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends p7zip-full python3 python3-pip usbutils && rm -rf /var/lib/apt/lists/*
WORKDIR /work

FROM ghcr.io/zephyrproject-rtos/zephyr-build:v0.26-branch AS stimulus-build
USER root
RUN pip3 install --no-cache-dir pyocd==0.36.* && apt-get update && apt-get install -y --no-install-recommends openocd usbutils && rm -rf /var/lib/apt/lists/*
WORKDIR /work/stimulus/zephyr
CMD ["sh","-c","west init -l . 2>/dev/null || true; west update --narrow; west build -p auto -b ${LP_STIM_BOARD:-nrf52840dk/nrf52840} -d build-${LP_STIM_BOARD_SLUG:-nrf52840dk} app && sha256sum build-*/zephyr/zephyr.hex"]

FROM e2e AS screenshot-runtime
WORKDIR /work/tools/screenshot
CMD ["node","server.mjs","--port","9223"]
