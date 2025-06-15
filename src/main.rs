//! main.rs — Minimal WebSocket Signaling Server in Rust
//! - Pure in-memory relay (no persistence)
//! - Broadcasts incoming messages to all other connected clients
//! - Zero external dependencies beyond Warp and Tokio

use std::{convert::Infallible, net::SocketAddr, sync::Arc};
use futures::{StreamExt, SinkExt};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use dashmap::DashMap;
use warp::{Filter, Reply};
use warp::ws::{Message, Ws};
use uuid::Uuid;
use tracing::{info, error};

// Shared map of client_id -> sender
type Clients = Arc<DashMap<String, UnboundedSender<Message>>>;

#[tokio::main]
async fn main() {
    // init logging
    tracing_subscriber::fmt().init();

    // Shared state
    let clients: Clients = Arc::new(DashMap::new());

    // WebSocket route
    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(with_clients(clients.clone()))
        .map(|ws: Ws, clients| {
            ws.on_upgrade(move |socket| handle_ws(socket, clients))
        });

    // Health check
    let health = warp::path("health").map(|| "OK");

    let routes = ws_route.or(health)
        .with(warp::cors().allow_any_origin());

    // Start server
    let addr: SocketAddr = ([0,0,0,0], 8080).into();
    info!("🚀 Signaling server running at {}", addr);
    warp::serve(routes).run(addr).await;
}

fn with_clients(clients: Clients) -> impl Filter<Extract = (Clients,), Error = Infallible> + Clone {
    warp::any().map(move || clients.clone())
}

async fn handle_ws(ws: warp::ws::WebSocket, clients: Clients) {
    let (mut tx_ws, mut rx_ws) = ws.split();
    let client_id = Uuid::new_v4().to_string();
    let (tx, mut rx) = unbounded_channel::<Message>();

    // Insert new client sender
    clients.insert(client_id.clone(), tx);
    info!("Client connected: {}", client_id);

    // Task to forward messages from channel to WebSocket
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if tx_ws.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Receive from WebSocket and broadcast
    while let Some(Ok(msg)) = rx_ws.next().await {
        if msg.is_close() { break; }
        // Broadcast to others
        for entry in clients.iter() {
            if entry.key() != &client_id {
                let _ = entry.value().send(msg.clone());
            }
        }
    }

    // Cleanup
    clients.remove(&client_id);
    info!("Client disconnected: {}", client_id);
}
