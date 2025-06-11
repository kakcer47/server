use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use warp::Filter;
use warp::ws::Message;

type Clients = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Message>>>>;
type Topics = Arc<Mutex<HashMap<String, VecDeque<Message>>>>;

#[tokio::main]
async fn main() {
    let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
    let topics: Topics = Arc::new(Mutex::new(HashMap::new()));

    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(with_clients(clients.clone()))
        .and(with_topics(topics.clone()))
        .map(|ws: warp::ws::Ws, clients, topics| {
            ws.on_upgrade(|websocket| handle_connection(websocket, clients, topics))
        });

    println!("Server running on ws://0.0.0.0:8080");
    warp::serve(ws_route).run(([0, 0, 0, 0], 8080)).await;
}

fn with_clients(clients: Clients) -> impl Filter<Extract = (Clients,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || clients.clone())
}

fn with_topics(topics: Topics) -> impl Filter<Extract = (Topics,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || topics.clone())
}

async fn handle_connection(ws: warp::ws::WebSocket, clients: Clients, topics: Topics) {
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let client_id = uuid::Uuid::new_v4().to_string();

    clients.lock().await.insert(client_id.clone(), tx.clone());

    let clients_clone = clients.clone();
    let client_id_clone = client_id.clone();

    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if let Err(e) = ws_tx.send(message).await {
                eprintln!("Error sending message: {}", e);
                break;
            }
        }
        clients_clone.lock().await.remove(&client_id_clone);
    });

    while let Some(result) = ws_rx.next().await {
        match result {
            Ok(msg) => {
                if msg.is_text() {
                    if let Ok(text) = msg.to_str() {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                            handle_message(json, &client_id, &clients, &topics).await;
                        }
                    }
                }
            }
            Err(e) => eprintln!("Error receiving message: {}", e),
        }
    }
}

async fn handle_message(msg: serde_json::Value, client_id: &str, clients: &Clients, topics: &Topics) {
    let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let topic = msg.get("topic").and_then(|t| t.as_str()).unwrap_or("general");

    match msg_type {
        "Subscribe" => {
            let topics_lock = topics.lock().await;
            let empty = VecDeque::new();
            let messages = topics_lock.get(topic).unwrap_or(&empty);
            let recent_messages = messages.iter().rev().take(25).cloned().collect::<Vec<_>>();
            let clients_lock = clients.lock().await;
            if let Some(client_tx) = clients_lock.get(client_id) {
                for msg in recent_messages.iter().rev() {
                    let _ = client_tx.send(msg.clone());
                }
            }
        }
        "Publish" => {
            if let Some(message) = msg.get("message") {
                let ws_message = Message::text(
                    serde_json::json!({
                        "type": "MessageReceived",
                        "topic": topic,
                        "message": message
                    })
                    .to_string(),
                );

                let mut topics_lock = topics.lock().await;
                let messages = topics_lock.entry(topic.to_string()).or_insert(VecDeque::new());
                messages.push_back(ws_message.clone());
                if messages.len() > 100 {
                    messages.pop_front();
                }

                let clients_lock = clients.lock().await;
                for (id, tx) in clients_lock.iter() {
                    if id != client_id {
                        let _ = tx.send(ws_message.clone());
                    }
                }
            }
        }
        _ => {}
    }
}
