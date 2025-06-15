# 📦 Этап 1: Сборка на musl для полной совместимости (без glibc)
FROM clux/muslrust:stable as builder

# Установка зависимостей
WORKDIR /app
COPY . .

# Собираем бинарник как статически линкованный
RUN cargo build --release

# 🧼 Этап 2: Финальный минимальный образ (только бинарник, без зависимостей)
FROM scratch

# Копируем только бинарник из предыдущего этапа
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/server /server

# Запускаем
ENTRYPOINT ["/server"]
