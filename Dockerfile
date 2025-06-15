# Используем актуальную версию Rust
FROM rust:1.82 as builder

# Устанавливаем рабочую директорию
WORKDIR /usr/src/app

# Копируем Cargo.toml и Cargo.lock отдельно для кэширования зависимостей
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Опционально: если есть другие файлы (например .env или миграции), добавь их:
# COPY migrations ./migrations

# Скачиваем зависимости
RUN cargo fetch

# Сборка проекта в релизе
RUN cargo build --release

# Финальный минимальный образ
FROM debian:buster-slim

# Устанавливаем SSL-сертификаты
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Копируем бинарник
COPY --from=builder /usr/src/app/target/release/p2p-server /usr/local/bin/server

# Порт сервера
EXPOSE 8080

# Запуск
CMD ["server"]
