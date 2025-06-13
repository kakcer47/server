import type { P2PMessage, Post } from '../types'
import { cryptoService } from './crypto'
import { useStore } from '../store'

interface RTCPeer {
  id: string
  connection: RTCPeerConnection
  dataChannel?: RTCDataChannel
}

interface SignalMessage {
  type: 'join' | 'offer' | 'answer' | 'ice_candidate' | 'broadcast' | 'leave'
  peerId?: string
  targetPeer?: string
  offer?: RTCSessionDescriptionInit
  answer?: RTCSessionDescriptionInit
  candidate?: RTCIceCandidateInit
  message?: any
}

interface ResponseMessage {
  type: 'peer_joined' | 'peer_left' | 'peer_list' | 'peer_count' | 'offer' | 'answer' | 'ice_candidate' | 'message' | 'error'
  peerId?: string
  peers?: string[]
  count?: number
  offer?: RTCSessionDescriptionInit
  answer?: RTCSessionDescriptionInit
  candidate?: RTCIceCandidateInit
  message?: any
  error?: string
}

class P2PService {
  private signalingWs: WebSocket | null = null
  private peers = new Map<string, RTCPeer>()
  private messageHandlers = new Map<string, (data: any) => void>()
  private localPeerId: string = ''
  private reconnectInterval: number | null = null
  private activePeers = new Set<string>()

  async init(): Promise<void> {
    this.localPeerId = cryptoService.generateId()
    this.connectSignaling()
  }

  private connectSignaling(): void {
    try {
      // Используй свой Render сервер  
      this.signalingWs = new WebSocket('wss://your-app-name.onrender.com/ws')
      
      this.signalingWs.onopen = () => {
        console.log('Signaling connected')
        this.sendSignaling({ type: 'join', peerId: this.localPeerId })
      }

      this.signalingWs.onclose = () => {
        this.scheduleReconnect()
        useStore.getState().setConnected(false)
      }

      this.signalingWs.onerror = () => {
        this.scheduleReconnect()
      }

      this.signalingWs.onmessage = async (event) => {
        const message: ResponseMessage = JSON.parse(event.data)
        await this.handleSignaling(message)
      }
    } catch (error) {
      this.scheduleReconnect()
    }
  }

  private scheduleReconnect(): void {
    if (this.reconnectInterval) return
    this.reconnectInterval = window.setTimeout(() => {
      this.reconnectInterval = null
      this.connectSignaling()
    }, 3000)
  }

  private async handleSignaling(message: ResponseMessage): Promise<void> {
    switch (message.type) {
      case 'peer_list':
        // Подключаемся ко всем существующим пирам
        if (message.peers) {
          for (const peerId of message.peers) {
            await this.createPeerConnection(peerId, true)
          }
        }
        break

      case 'peer_joined':
        if (message.peerId && message.peerId !== this.localPeerId) {
          this.activePeers.add(message.peerId)
          // Новый пир подключился, но мы не инициируем соединение
          // Он сам инициирует offer к нам
        }
        break

      case 'peer_left':
        if (message.peerId) {
          this.activePeers.delete(message.peerId)
          this.peers.delete(message.peerId)
          this.updatePeerCount()
        }
        break

      case 'peer_count':
        useStore.getState().setPeerCount(message.count || 0)
        if (message.count && message.count > 0) {
          useStore.getState().setConnected(true)
        }
        break

      case 'offer':
        if (message.peerId && message.offer) {
          await this.handleOffer(message.peerId, message.offer)
        }
        break

      case 'answer':
        if (message.peerId && message.answer) {
          await this.handleAnswer(message.peerId, message.answer)
        }
        break

      case 'ice_candidate':
        if (message.peerId && message.candidate) {
          await this.handleIceCandidate(message.peerId, message.candidate)
        }
        break

      case 'message':
        if (message.message) {
          await this.handlePeerMessage(message.message)
        }
        break

      case 'error':
        console.error('Signaling error:', message.error)
        break
    }
  }

  private async createPeerConnection(peerId: string, isInitiator: boolean): Promise<void> {
    const config: RTCConfiguration = {
      iceServers: [
        { urls: 'stun:stun.l.google.com:19302' },
        { urls: 'stun:stun1.l.google.com:19302' }
      ]
    }

    const connection = new RTCPeerConnection(config)
    let dataChannel: RTCDataChannel | undefined

    if (isInitiator) {
      dataChannel = connection.createDataChannel('data', { ordered: true })
      this.setupDataChannel(dataChannel, peerId)
    }

    connection.ondatachannel = (event) => {
      dataChannel = event.channel
      this.setupDataChannel(event.channel, peerId)
    }

    connection.onicecandidate = (event) => {
      if (event.candidate) {
        this.sendSignaling({
          type: 'ice_candidate',
          peerId: this.localPeerId,
          targetPeer: peerId,
          candidate: event.candidate
        })
      }
    }

    connection.onconnectionstatechange = () => {
      console.log(`Peer ${peerId} connection state:`, connection.connectionState)
      if (connection.connectionState === 'connected') {
        this.updatePeerCount()
      } else if (connection.connectionState === 'disconnected' || connection.connectionState === 'failed') {
        this.peers.delete(peerId)
        this.updatePeerCount()
      }
    }

    this.peers.set(peerId, { id: peerId, connection, dataChannel })

    if (isInitiator) {
      const offer = await connection.createOffer()
      await connection.setLocalDescription(offer)
      this.sendSignaling({
        type: 'offer',
        peerId: this.localPeerId,
        targetPeer: peerId,
        offer
      })
    }
  }

  private setupDataChannel(channel: RTCDataChannel, peerId: string): void {
    channel.onopen = () => {
      console.log(`DataChannel opened with ${peerId}`)
      const peer = this.peers.get(peerId)
      if (peer) {
        peer.dataChannel = channel
        this.updatePeerCount()
      }
    }

    channel.onmessage = (event) => {
      this.handlePeerMessage(JSON.parse(event.data))
    }

    channel.onerror = (error) => {
      console.error(`DataChannel error with ${peerId}:`, error)
    }
  }

  private updatePeerCount(): void {
    const connectedPeers = Array.from(this.peers.values())
      .filter(peer => peer.dataChannel?.readyState === 'open').length
    
    useStore.getState().setPeerCount(connectedPeers)
    useStore.getState().setConnected(connectedPeers > 0 || this.signalingWs?.readyState === WebSocket.OPEN)
  }

  private async handleOffer(peerId: string, offer: RTCSessionDescriptionInit): Promise<void> {
    await this.createPeerConnection(peerId, false)
    const peer = this.peers.get(peerId)
    if (!peer) return

    await peer.connection.setRemoteDescription(offer)
    const answer = await peer.connection.createAnswer()
    await peer.connection.setLocalDescription(answer)

    this.sendSignaling({
      type: 'answer',
      peerId: this.localPeerId,
      targetPeer: peerId,
      answer
    })
  }

  private async handleAnswer(peerId: string, answer: RTCSessionDescriptionInit): Promise<void> {
    const peer = this.peers.get(peerId)
    if (peer) {
      await peer.connection.setRemoteDescription(answer)
    }
  }

  private async handleIceCandidate(peerId: string, candidate: RTCIceCandidateInit): Promise<void> {
    const peer = this.peers.get(peerId)
    if (peer) {
      await peer.connection.addIceCandidate(candidate)
    }
  }

  private sendSignaling(message: SignalMessage): void {
    if (this.signalingWs?.readyState === WebSocket.OPEN) {
      this.signalingWs.send(JSON.stringify(message))
    }
  }

  private async handlePeerMessage(envelope: any): Promise<void> {
    try {
      const message: P2PMessage = envelope.message || envelope
      if (!message) return

      const isValid = await cryptoService.verifySignature(
        JSON.stringify(message.data),
        message.signature,
        message.author
      )

      if (!isValid) return

      const handler = this.messageHandlers.get(message.type)
      if (handler) {
        handler(message.data)
      }
    } catch (error) {
      console.error('Peer message error:', error)
    }
  }

  async publishPost(post: Post): Promise<void> {
    await this.publishMessage('post', post)
  }

  async publishLike(postId: string, userId: string): Promise<void> {
    await this.publishMessage('like', { postId, userId })
  }

  async publishUnlike(postId: string, userId: string): Promise<void> {
    await this.publishMessage('unlike', { postId, userId })
  }

  private async publishMessage(type: string, data: any): Promise<void> {
    const user = useStore.getState().user
    if (!user) return

    const message: P2PMessage = {
      type: type as any,
      data,
      timestamp: Date.now(),
      author: user.publicKey,
      signature: await cryptoService.signMessage(JSON.stringify(data), user.privateKey)
    }

    // Отправляем через WebRTC DataChannels
    let sentViaPeers = false
    this.peers.forEach((peer) => {
      if (peer.dataChannel?.readyState === 'open') {
        peer.dataChannel.send(JSON.stringify({ message }))
        sentViaPeers = true
      }
    })

    // Fallback через WebSocket если нет P2P соединений
    if (!sentViaPeers && this.signalingWs?.readyState === WebSocket.OPEN) {
      this.sendSignaling({
        type: 'broadcast',
        message: { message }
      })
    }
  }

  subscribe(messageType: string, handler: (data: any) => void): void {
    this.messageHandlers.set(messageType, handler)
  }

  async connectToPeer(peerId: string): Promise<void> {
    if (!this.peers.has(peerId) && this.activePeers.has(peerId)) {
      await this.createPeerConnection(peerId, true)
    }
  }

  getPeerCount(): number {
    return Array.from(this.peers.values())
      .filter(peer => peer.dataChannel?.readyState === 'open').length
  }

  async stop(): Promise<void> {
    if (this.reconnectInterval) {
      clearTimeout(this.reconnectInterval)
      this.reconnectInterval = null
    }

    // Уведомляем о disconnection
    if (this.signalingWs?.readyState === WebSocket.OPEN) {
      this.sendSignaling({ type: 'leave', peerId: this.localPeerId })
    }

    this.peers.forEach((peer) => {
      peer.connection.close()
    })
    this.peers.clear()

    if (this.signalingWs) {
      this.signalingWs.close()
      this.signalingWs = null
    }

    useStore.getState().setConnected(false)
    useStore.getState().setPeerCount(0)
  }
}

export const p2pService = new P2PService()
