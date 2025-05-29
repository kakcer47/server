FROM node:18

WORKDIR /app

# Установи системные зависимости для компиляции wrtc
RUN apt-get update && apt-get install -y \
    build-essential \
    python3 \
    python3-pip \
    libvpx-dev \
    && npm install -g node-pre-gyp

# Копируй package.json и установи зависимости
COPY package*.json ./
RUN npm install --build-from-source

# Копируй остальной код
COPY . .

# Укажи порт, который предоставит Render
EXPOSE $PORT

# Запусти сервер
CMD ["npm", "start"]
