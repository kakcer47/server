# Используем официальный образ Rust как stage для сборки
FROM rust:1.78 as builder

# Устанавливаем рабочую директорию
WORKDIR /usr/src/app

# Копируем все файлы проекта (Cargo.toml, src и т.д.)
COPY . .

# Сборка проекта
RUN cargo build --release

# Финальный минимальный образ
FROM debian:buster-slim

# Устанавливаем сертификаты, если используешь HTTPS
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Копируем бинарник из предыдущего stage
COPY --from=builder /usr/src/app/target/release/p2p-server /usr/local/bin/server

# Настраиваем рабочую директорию (не обязательно)
WORKDIR /app

# Открываем порт для Render (по умолчанию 8080)
EXPOSE 8080

# Стартуем приложение
CMD ["server"]
