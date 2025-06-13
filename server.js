const WebSocket = require('ws');
const http = require('http');
const { v4: uuidv4 } = require('uuid');

const peers = new Map();
const port = process.env.PORT || 8080;

// Создаем HTTP сервер для health check
const server = http.createServer((req, res) => {
  // CORS headers
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');

  if (req.method === 'OPTIONS') {
    res.writeHead(200);
    res.end();
    return;
  }

  if (req.url === '/health' || req.url === '/') {
    res.writeHead(200, { 'Content-Type': 'text/plain' });
    res.end('P2P Signaling Server OK');
  } else {
    res.writeHead(404);
    res.end('Not Found');
  }
});

// WebSocket сервер на том же порту
const wss = new WebSocket.Server({ 
  server,
  perMessageDeflate: false 
});

wss.on('connection', (ws, req) => {
  let peerId = null;
  const connectionId = uuidv4();
  
  console.log(`✅ New WebSocket connection from ${req.socket.remoteAddress}`);

  ws.on('message', (data) => {
    try {
      const message = JSON.parse(data.toString());
      console.log(`📨 Received:`, message.type, message.peerId || '');
      handleMessage(message, ws);
    } catch (error) {
      console.error('❌ Message parse error:', error);
      ws.send(JSON.stringify({ type: 'error', error: 'Invalid JSON' }));
    }
  });

  ws.on('close', (code, reason) => {
    console.log(`❌ WebSocket closed: ${code} ${reason}`);
    if (peerId) {
      peers.delete(peerId);
      broadcast({ type: 'peer_left', peerId }, peerId);
      broadcast({ type: 'peer_count', count: peers.size });
      console.log(`👋 Peer left: ${peerId} (remaining: ${peers.size})`);
    }
  });

  ws.on('error', (error) => {
    console.error('❌ WebSocket error:', error);
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
        
        console.log(`🎉 Peer joined: ${peerId} (total: ${peers.size})`);
        break;

      case 'offer':
        console.log(`📞 Offer from ${peerId} to ${msg.targetPeer}`);
        sendToPeer(msg.targetPeer, { 
          type: 'offer', 
          peerId: peerId, 
          offer: msg.offer 
        });
        break;

      case 'answer':
        console.log(`📞 Answer from ${peerId} to ${msg.targetPeer}`);
        sendToPeer(msg.targetPeer, { 
          type: 'answer', 
          peerId: peerId, 
          answer: msg.answer 
        });
        break;

      case 'ice_candidate':
        console.log(`🧊 ICE candidate from ${peerId} to ${msg.targetPeer}`);
        sendToPeer(msg.targetPeer, { 
          type: 'ice_candidate', 
          peerId: peerId, 
          candidate: msg.candidate 
        });
        break;

      case 'broadcast':
        console.log(`📢 Broadcast from ${peerId}`);
        broadcast({ type: 'message', message: msg.message }, peerId);
        break;

      case 'leave':
        if (peerId) {
          peers.delete(peerId);
          broadcast({ type: 'peer_left', peerId }, peerId);
          broadcast({ type: 'peer_count', count: peers.size });
          console.log(`👋 Peer manually left: ${peerId}`);
        }
        break;

      default:
        console.log(`❓ Unknown message type: ${msg.type}`);
        socket.send(JSON.stringify({ type: 'error', error: 'Unknown message type' }));
    }
  }

  function sendToPeer(targetPeer, message) {
    const targetSocket = peers.get(targetPeer);
    if (targetSocket && targetSocket.readyState === WebSocket.OPEN) {
      targetSocket.send(JSON.stringify(message));
      console.log(`✉️  Sent to ${targetPeer}:`, message.type);
    } else {
      console.log(`❌ Target peer ${targetPeer} not found or disconnected`);
    }
  }

  function broadcast(message, excludePeer = null) {
    const data = JSON.stringify(message);
    let sentCount = 0;
    peers.forEach((socket, id) => {
      if (id !== excludePeer && socket.readyState === WebSocket.OPEN) {
        socket.send(data);
        sentCount++;
      }
    });
    console.log(`📡 Broadcast ${message.type} to ${sentCount} peers`);
  }
});

// Запускаем сервер
server.listen(port, '0.0.0.0', () => {
  console.log(`🚀 P2P Signaling Server running on port ${port}`);
  console.log(`📡 WebSocket endpoint: ws://localhost:${port}`);
  console.log(`🔗 Health check: http://localhost:${port}/health`);
});

// Логирование статистики каждые 30 секунд
setInterval(() => {
  console.log(`📊 Stats: ${peers.size} connected peers`);
}, 30000);
