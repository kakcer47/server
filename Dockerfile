# Этап 1: сборка бинарника в musl
FROM clux/muslrust:stable as builder

WORKDIR /app
COPY . .

RUN cargo build --release

# Этап 2: минимальный рантайм-образ
FROM scratch

COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/server /server

ENTRYPOINT ["/server"]
