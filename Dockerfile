# 1. Базовый образ для сборки (официальный Rust)
FROM rust:1.78 as builder

# 2. Устанавливаем рабочую директорию
WORKDIR /usr/src/app

# 3. Копируем Cargo.toml и Cargo.lock заранее, чтобы кэшировать зависимости
COPY Cargo.toml Cargo.lock ./

# 4. Создаём пустой src, чтобы пройти фазу зависимости
RUN mkdir src && echo "fn main() {}" > src/main.rs

# 5. Ставим зависимости (кэшируется)
RUN cargo build --release && rm -rf src

# 6. Копируем весь проект
COPY . .

# 7. Сборка приложения в release-режиме
RUN cargo build --release

# 8. Финальный минимальный образ
FROM debian:buster-slim

# 9. Добавляем сертификаты TLS (если будешь использовать HTTPS)
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# 10. Копируем бинарник из builder
COPY --from=builder /usr/src/app/target/release/your_binary_name /usr/local/bin/server

# 11. Устанавливаем рабочую директорию
WORKDIR /app

# 12. Порт, который слушает warp (8080 по умолчанию)
EXPOSE 8080

# 13. Точка входа
CMD ["server"]
