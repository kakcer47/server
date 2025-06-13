const WebSocket = require('ws');
const { v4: uuidv4 } = require('uuid');

const peers = new Map();

const wss = new WebSocket.Server({ 
  port: process.env.PORT || 8080,
  perMessageDeflate: false 
});

wss.on('connection', (ws) => {
  let peerId = null;
  const connectionId = uuidv4();
  
  console.log(`New connection: ${connectionId}`);

  ws.on('message', (data) => {
    try {
      const message = JSON.parse(data.toString());
      handleMessage(message, ws, connectionId);
    } catch (error) {
      console.error('Message parse error:', error);
    }
  });

  ws.on('close', () => {
    if (peerId) {
      peers.delete(peerId);
      broadcast({ type: 'peer_left', peerId }, peerId);
      broadcast({ type: 'peer_count', count: peers.size });
      console.log(`Peer disconnected: ${peerId}`);
    }
  });

  function handleMessage(msg, socket) {
    switch (msg.type) {
      case 'join':
        if (peers.has(msg.peerId)) {
          socket.send(JSON.stringify({ type: 'error', error: 'Peer ID already exists' }));
          return;
        }
        
        peerId = msg.peerId;
        peers.set(peerId, socket);
        
        // Отправляем список существующих пиров
        const existingPeers = Array.from(peers.keys()).filter(id => id !== peerId);
        socket.send(JSON.stringify({ type: 'peer_list', peers: existingPeers }));
        
        // Уведомляем других о новом пире
        broadcast({ type: 'peer_joined', peerId }, peerId);
        broadcast({ type: 'peer_count', count: peers.size });
        
        console.log(`Peer joined: ${peerId} (total: ${peers.size})`);
        break;

      case 'offer':
        sendToPeer(msg.targetPeer, { 
          type: 'offer', 
          peerId: peerId, 
          offer: msg.offer 
        });
        break;

      case 'answer':
        sendToPeer(msg.targetPeer, { 
          type: 'answer', 
          peerId: peerId, 
          answer: msg.answer 
        });
        break;

      case 'ice_candidate':
        sendToPeer(msg.targetPeer, { 
          type: 'ice_candidate', 
          peerId: peerId, 
          candidate: msg.candidate 
        });
        break;

      case 'broadcast':
        broadcast({ type: 'message', message: msg.message }, peerId);
        break;

      case 'leave':
        if (peerId) {
          peers.delete(peerId);
          broadcast({ type: 'peer_left', peerId }, peerId);
          broadcast({ type: 'peer_count', count: peers.size });
        }
        break;
    }
  }

  function sendToPeer(targetPeer, message) {
    const targetSocket = peers.get(targetPeer);
    if (targetSocket && targetSocket.readyState === WebSocket.OPEN) {
      targetSocket.send(JSON.stringify(message));
    }
  }

  function broadcast(message, excludePeer = null) {
    const data = JSON.stringify(message);
    peers.forEach((socket, id) => {
      if (id !== excludePeer && socket.readyState === WebSocket.OPEN) {
        socket.send(data);
      }
    });
  }
});

console.log(`🚀 P2P Signaling Server running on port ${process.env.PORT || 8080}`);

// Health check endpoint для Render
const http = require('http');
const server = http.createServer((req, res) => {
  if (req.url === '/health') {
    res.writeHead(200, { 'Content-Type': 'text/plain' });
    res.end('OK');
  } else {
    res.writeHead(404);
    res.end('Not Found');
  }
});

// Если WebSocket сервер не на том же порту
if (!process.env.PORT) {
  server.listen(3000);
}
