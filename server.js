const WebSocket = require('ws');
const { RTCPeerConnection, RTCSessionDescription, RTCIceCandidate } = require('wrtc');

const wss = new WebSocket.Server({ port: 8080 });

wss.on('connection', (ws) => {
  console.log('Client connected');
  let peerConnection;

  ws.on('message', async (message) => {
    const msg = JSON.parse(message);

    if (msg.type === 'offer') {
      peerConnection = new RTCPeerConnection({
        iceServers: [{ urls: 'stun:stun.l.google.com:19302' }]
      });

      // Set the remote description with the received offer
      await peerConnection.setRemoteDescription(new RTCSessionDescription(msg.sdp));

      // Create and set the answer
      const answer = await peerConnection.createAnswer();
      await peerConnection.setLocalDescription(answer);

      // Send the answer back to the client
      ws.send(JSON.stringify({ type: 'answer', sdp: peerConnection.localDescription }));

      // Handle ICE candidates
      peerConnection.onicecandidate = (event) => {
        if (event.candidate) {
          ws.send(JSON.stringify({ type: 'ice', candidate: event.candidate }));
        }
      };

      // Handle incoming DataChannel from the client
      peerConnection.ondatachannel = (event) => {
        const dataChannel = event.channel;
        dataChannel.onmessage = (event) => {
          console.log('Received ad data:', event.data);
          // Process ad data here (e.g., store, broadcast, etc.)
          dataChannel.send('Ad received: ' + event.data); // Echo back for demo
        };
        dataChannel.onopen = () => console.log('DataChannel opened');
        dataChannel.onclose = () => console.log('DataChannel closed');
      };
    } else if (msg.type === 'ice') {
      if (peerConnection) {
        await peerConnection.addIceCandidate(new RTCIceCandidate(msg.candidate));
      }
    }
  });
});

console.log('WebSocket server running on ws://localhost:8080');
