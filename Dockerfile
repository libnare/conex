FROM rust:latest AS base
WORKDIR /app

RUN cargo install --config net.git-fetch-with-cli=true cargo-chef

FROM base AS planner

COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM base AS builder

ARG TARGETPLATFORM
ARG RUSTFLAGS='-C target-feature=+crt-static'

COPY --from=planner /app/recipe.json ./
RUN if [ "$TARGETPLATFORM" = "linux/amd64" ]; then TARGET=x86_64-unknown-linux-gnu; elif [ "$TARGETPLATFORM" = "linux/arm64" ]; then TARGET=aarch64-unknown-linux-gnu; fi \
    && cargo chef cook --release --target $TARGET --recipe-path recipe.json

COPY . .
RUN if [ "$TARGETPLATFORM" = "linux/amd64" ]; then TARGET=x86_64-unknown-linux-gnu; elif [ "$TARGETPLATFORM" = "linux/arm64" ]; then TARGET=aarch64-unknown-linux-gnu; fi \
    && cargo build --release --target $TARGET \
    && cp -r target/$TARGET/release/conex target/release/conex

FROM alpine:latest AS packer

RUN apk add --no-cache upx

COPY --from=builder /app/target/release/conex /app/conex
RUN upx --brute /app/conex

FROM gcr.io/distroless/static-debian12:nonroot AS runtime
WORKDIR /app

COPY --from=packer /app/conex ./
CMD ["./conex"]