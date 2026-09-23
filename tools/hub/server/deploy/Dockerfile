# biwa_hub_server のビルド用 Dockerfile。
#
# `biwa_hub_domain` / `biwa_hub_infrastructure` が `compiler/src/biwac_base` を
# 相対パスで参照しているため、ビルドコンテキストはリポジトリルートでなければならない
# (`docker compose -f tools/hub/server/deploy/compose.yaml build` を想定。
# compose.yaml 側で context をリポジトリルートに設定してある)。

FROM rust:1-slim-bookworm AS builder

# `ring` など一部の依存が C コンパイラを要求する。
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

WORKDIR /src/tools/hub/server
RUN cargo build --release -p biwa_hub_server

FROM debian:bookworm-slim AS runtime

# rustls が実行時にシステムの root CA を読むことがあるため同梱する。
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/tools/hub/server/target/release/biwa-hub-server /usr/local/bin/biwa-hub-server

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/biwa-hub-server"]
