import WebSocket from 'ws';
import { RTCPeerConnection, RTCSessionDescription, RTCIceCandidate } from 'wrtc';

const port = process.env.PORT || 8080;
const wss = new WebSocket.Server({ port });

wss.on('connection', (ws) => {
  console.log('Client connected');
  let peerConnection;

  ws.on('message', async (message) => {
    const msg = JSON.parse(message.toString());

    if (msg.type === 'offer') {
      peerConnection = new RTCPeerConnection({
        iceServers: [{ urls: 'stun:stun.l.google.com:19302' }]
      });

      await peerConnection.setRemoteDescription(new RTCSessionDescription(msg.sdp));
      const answer = await peerConnection.createAnswer();
      await peerConnection.setLocalDescription(answer);

      ws.send(JSON.stringify({ type: 'answer', sdp: peerConnection.localDescription }));

      peerConnection.onicecandidate = (event) => {
        if (event.candidate) {
          ws.send(JSON.stringify({ type: 'ice', candidate: event.candidate }));
        }
      };

      peerConnection.ondatachannel = (event) => {
        const dataChannel = event.channel;
        dataChannel.onmessage = (event) => {
          console.log('Received ad data:', event.data);
          dataChannel.send('Ad received: ' + event.data);
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

console.log(`WebSocket server running on port ${port}`);
