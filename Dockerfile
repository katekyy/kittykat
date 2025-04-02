FROM rust:alpine AS builder

WORKDIR /app

RUN apk add musl-dev pkgconf openssl-libs-static openssl-dev

COPY Cargo.* /app

COPY ./src /app/src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo fetch

RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install --path .


FROM alpine:3

WORKDIR /app

COPY --from=builder /usr/local/cargo/bin/kittykat /usr/local/bin/kittykat

ENV KITTYKAT_LISTEN_ADDRESS="0.0.0.0:1234"

EXPOSE 1234/tcp

CMD ["kittykat"]
