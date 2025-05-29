FROM node:18

WORKDIR /app

# Copy package.json and install dependencies
COPY package*.json ./
RUN apt-get update && apt-get install -y \
    build-essential \
    python3 \
    libvpx-dev \
    && npm install --build-from-source

# Copy the rest of the application
COPY . .

# Expose the port Render will provide
EXPOSE $PORT

# Start the server
CMD ["npm", "start"]
