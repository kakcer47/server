const WebSocket = require('ws');
   const http = require('http');
   const server = http.createServer();
   const wss = new WebSocket.Server({ server });

   wss.on('connection', (ws) => {
     console.log('New client connected');
     ws.on('message', (message) => {
       const msg = JSON.parse(message);
       wss.clients.forEach((client) => {
         if (client !== ws && client.readyState === WebSocket.OPEN) {
           client.send(JSON.stringify({ ...msg, from: msg.from || 'unknown' }));
         }
       });
     });
     ws.on('close', () => console.log('Client disconnected'));
   });

   const PORT = process.env.PORT || 8080;
   server.listen(PORT, () => {
     console.log(`WebSocket server running on port ${PORT}`);
   });
