const WebSocket = require('ws');

const wss = new WebSocket.Server({ port: process.env.PORT || 8080 });

console.log("WebSocket server starting...");

wss.on('connection', (ws) => {
  console.log('New client connected');

  // Когда клиент отправляет сообщение
  ws.on('message', (message) => {
    try {
      const msg = JSON.parse(message.toString());
      console.log('Received message:', msg);

      // Пересылаем сообщение всем подключённым клиентам, кроме отправителя
      wss.clients.forEach((client) => {
        if (client !== ws && client.readyState === WebSocket.OPEN) {
          client.send(JSON.stringify(msg));
        }
      });
    } catch (error) {
      console.error('Failed to handle message:', error);
    }
  });

  ws.on('close', () => {
    console.log('Client disconnected');
  });

  ws.on('error', (error) => {
    console.error('WebSocket error:', error);
  });
});
