const WebSocket = require('ws');
const express = require('express');
const app = express();
const server = require('http').createServer(app);
const wss = new WebSocket.Server({ server });

app.get('/health', (req, res) => res.send('OK')); // Для проверки Render

const nodes = new Map(); // Хранит узлы: { id: ws }

wss.on('connection', (ws, req) => {
  const nodeId = req.url.slice(1) || Math.random().toString(36).slice(2); // ID из URL или случайный
  nodes.set(nodeId, ws);
  console.log(`Node connected: ${nodeId}, Total: ${nodes.size}`);

  ws.on('message', (message) => {
    const data = JSON.parse(message);
    if (data.type === 'offer' || data.type === 'answer' || data.type === 'candidate') {
      const targetWs = nodes.get(data.target);
      if (targetWs) {
        targetWs.send(JSON.stringify({ ...data, sender: nodeId }));
      }
    }
  });

  ws.on('close', () => {
    nodes.delete(nodeId);
    console.log(`Node disconnected: ${nodeId}, Total: ${nodes.size}`);
  });
});

const PORT = process.env.PORT || 8080;
server.listen(PORT, () => console.log(`Server running on port ${PORT}`));
