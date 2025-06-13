use warp::Filter;
use warp::ws::{Message, WebSocket};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use futures_util::{SinkExt, StreamExt};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[allow(non_snake_case)]
enum SignalMessage {
    #[serde(rename = "join")]
    Join { peerId: String },
    
    #[serde(rename = "offer")]
    Offer { 
        peerId: String, 
        targetPeer: String, 
        offer: Value 
    },
    
    #[serde(rename = "answer")]
    Answer { 
        peerId: String, 
        targetPeer: String, 
        answer: Value 
    },
    
    #[serde(rename = "ice_candidate")]
    IceCandidate { 
        peerId: String, 
        targetPeer: String, 
        candidate: Value 
    },
    
    #[serde(rename = "broadcast")]
    Broadcast { message: Value },
    
    #[serde(rename = "leave")]
    Leave { peerId: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
#[allow(non_snake_case)]
enum ResponseMessage {
    #[serde(rename = "peer_joined")]
    PeerJoined { peerId: String },
    
    #[serde(rename = "peer_left")]
    PeerLeft { peerId: String },
    
    #[serde(rename = "peer_list")]
    PeerList { peers: Vec<String> },
    
    #[serde(rename = "peer_count")]
    PeerCount { count: usize },
    
    #[serde(rename = "offer")]
    Offer { 
        peerId: String, 
        offer: Value 
    },
    
    #[serde(rename = "answer")]
    Answer { 
        peerId: String, 
        answer: Value 
    },
    
    #[serde(rename = "ice_candidate")]
    IceCandidate { 
        peerId: String, 
        candidate: Value 
    },
    
    #[serde(rename = "message")]
    Message { message: Value },
    
    #[serde(rename = "error")]
    Error { error: String },
}

struct Peer {
    tx: mpsc::UnboundedSender<Message>,
}

type Peers = Arc<Mutex<HashMap<String, Peer>>>;

#[derive(Clone)]
struct AppState {
    peers: Peers,
}

#[tokio::main]
async fn main() {
    let state = AppState {
        peers: Arc::new(Mutex::new(HashMap::new())),
    };

    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(with_state(state.clone()))
        .map(|ws: warp::ws::Ws, state| {
            ws.on_upgrade(move |socket| handle_socket(socket, state))
        });

    let health = warp::path("health")
        .map(|| "OK");

    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec!["content-type"])
        .allow_methods(vec!["GET", "POST", "OPTIONS"]);

    let routes = ws_route.or(health).with(cors);

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse::<u16>()
        .unwrap_or(8080);

    println!("🚀 P2P Signaling Server starting on 0.0.0.0:{}", port);
    
    warp::serve(routes)
        .bind(([0, 0, 0, 0], port))
        .await;
}

fn with_state(state: AppState) -> impl Filter<Extract = (AppState,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || state.clone())
}

async fn handle_socket(ws: WebSocket, state: AppState) {
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut peer_id: Option<String> = None;

    let peers_clone = state.peers.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    println!("New WebSocket connection");

    while let Some(Ok(msg)) = ws_rx.next().await {
        if msg.is_text() {
            if let Ok(signal) = serde_json::from_str::<SignalMessage>(msg.to_str().unwrap()) {
                match handle_signal(signal, &mut peer_id, &tx, &state).await {
                    Ok(_) => {},
                    Err(e) => {
                        let error_msg = ResponseMessage::Error { error: e };
                        send_message(&tx, error_msg).await;
                    }
                }
            }
        } else if msg.is_close() {
            break;
        }
    }

    if let Some(pid) = peer_id {
        let mut peers = state.peers.lock().await;
        peers.remove(&pid);
        
        let leave_msg = ResponseMessage::PeerLeft { peerId: pid.clone() };
        broadcast_to_others(&peers, &pid, leave_msg).await;
        
        let count_msg = ResponseMessage::PeerCount { count: peers.len() };
        broadcast_to_all(&peers, count_msg).await;
        
        println!("Peer disconnected: {}", pid);
    }
}

async fn handle_signal(
    signal: SignalMessage,
    peer_id: &mut Option<String>,
    tx: &mpsc::UnboundedSender<Message>,
    state: &AppState,
) -> Result<(), String> {
    match signal {
        SignalMessage::Join { peerId } => {
            let mut peers = state.peers.lock().await;
            
            if peers.contains_key(&peerId) {
                return Err("Peer ID already exists".to_string());
            }

            let peer = Peer {
                tx: tx.clone(),
            };
            peers.insert(peerId.clone(), peer);
            *peer_id = Some(peerId.clone());

            let existing_peers: Vec<String> = peers.keys()
                .filter(|&id| id != &peerId)
                .cloned()
                .collect();
            
            let peer_list_msg = ResponseMessage::PeerList { peers: existing_peers };
            send_message(tx, peer_list_msg).await;

            let join_msg = ResponseMessage::PeerJoined { peerId: peerId.clone() };
            broadcast_to_others(&peers, &peerId, join_msg).await;

            let count_msg = ResponseMessage::PeerCount { count: peers.len() };
            broadcast_to_all(&peers, count_msg).await;
            
            println!("Peer joined: {} (total: {})", peerId, peers.len());
        },

        SignalMessage::Offer { peerId: _, targetPeer, offer } => {
            let peers = state.peers.lock().await;
            if let Some(target) = peers.get(&targetPeer) {
                let offer_msg = ResponseMessage::Offer { 
                    peerId: peer_id.as_ref().unwrap_or(&"unknown".to_string()).clone(),
                    offer 
                };
                send_message(&target.tx, offer_msg).await;
            }
        },

        SignalMessage::Answer { peerId: _, targetPeer, answer } => {
            let peers = state.peers.lock().await;
            if let Some(target) = peers.get(&targetPeer) {
                let answer_msg = ResponseMessage::Answer { 
                    peerId: peer_id.as_ref().unwrap_or(&"unknown".to_string()).clone(),
                    answer 
                };
                send_message(&target.tx, answer_msg).await;
            }
        },

        SignalMessage::IceCandidate { peerId: _, targetPeer, candidate } => {
            let peers = state.peers.lock().await;
            if let Some(target) = peers.get(&targetPeer) {
                let ice_msg = ResponseMessage::IceCandidate { 
                    peerId: peer_id.as_ref().unwrap_or(&"unknown".to_string()).clone(),
                    candidate 
                };
                send_message(&target.tx, ice_msg).await;
            }
        },

        SignalMessage::Broadcast { message } => {
            let peers = state.peers.lock().await;
            let broadcast_msg = ResponseMessage::Message { message };
            if let Some(sender_id) = peer_id {
                broadcast_to_others(&peers, sender_id, broadcast_msg).await;
            }
        },

        SignalMessage::Leave { peerId: _ } => {
            if let Some(pid) = peer_id.take() {
                let mut peers = state.peers.lock().await;
                peers.remove(&pid);
                
                let leave_msg = ResponseMessage::PeerLeft { peerId: pid.clone() };
                broadcast_to_others(&peers, &pid, leave_msg).await;
                
                let count_msg = ResponseMessage::PeerCount { count: peers.len() };
                broadcast_to_all(&peers, count_msg).await;
            }
        },
    }
    
    Ok(())
}

async fn send_message(tx: &mpsc::UnboundedSender<Message>, msg: ResponseMessage) {
    if let Ok(json) = serde_json::to_string(&msg) {
        let _ = tx.send(Message::text(json));
    }
}

async fn broadcast_to_others(peers: &HashMap<String, Peer>, sender_id: &str, msg: ResponseMessage) {
    if let Ok(json) = serde_json::to_string(&msg) {
        for (id, peer) in peers.iter() {
            if id != sender_id {
                let _ = peer.tx.send(Message::text(json.clone()));
            }
        }
    }
}

async fn broadcast_to_all(peers: &HashMap<String, Peer>, msg: ResponseMessage) {
    if let Ok(json) = serde_json::to_string(&msg) {
        for peer in peers.values() {
            let _ = peer.tx.send(Message::text(json.clone()));
        }
    }
}
