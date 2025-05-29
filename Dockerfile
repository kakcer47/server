FROM node:18

WORKDIR /app

RUN apt-get update && apt-get install -y \
    build-essential \
    python3 \
    python3-pip \
    libvpx-dev \
    && npm install -g node-pre-gyp

COPY package*.json ./
RUN npm install --build-from-source

COPY . .

EXPOSE $PORT

CMD ["npm", "start"]
