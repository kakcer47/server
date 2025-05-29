import { WebSocketServer } from 'ws';

const port = process.env.PORT || 8080;
const wss = new WebSocketServer({ port });

const clients = new Map();

wss.on('connection', (ws) => {
  const clientId = Math.random().toString(36).substr(2, 9);
  clients.set(clientId, ws);
  console.log(`Client ${clientId} connected`);

  ws.on('message', (message) => {
    const msg = JSON.parse(message.toString());
    console.log(`Received: ${msg.type} from ${clientId}`);

    // Пересылаем сообщение всем остальным клиентам
    for (const [id, client] of clients) {
      if (id !== clientId && client.readyState === 1) { // 1 = OPEN
        client.send(message);
      }
    }
  });

  ws.on('close', () => {
    clients.delete(clientId);
    console.log(`Client ${clientId} disconnected`);
  });
});

console.log(`WebSocket server running on port ${port}`);
