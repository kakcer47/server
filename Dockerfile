FROM node:18

WORKDIR /app

RUN apt-get update && apt-get install -y \
    build-essential \
    python3 \
    python3-pip \
    libvpx-dev \
    libopus-dev \
    cmake \
    && rm -rf /var/lib/apt/lists/*

RUN npm install -g node-pre-gyp node-gyp

COPY package*.json ./
RUN npm install --build-from-source --verbose

COPY . .

EXPOSE $PORT

CMD ["npm", "start"]
