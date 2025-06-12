use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use warp::Filter;
use warp::ws::Message;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    Init { user_id: String, username: String },
    Subscribe { topic: String },
    Unsubscribe { topic: String },
    Publish { topic: String, message: String },
    Like { post_id: String },
    Filter { topics: Vec<String> },
    Search { query: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Post {
    post_id: String,
    topic: String,
    message: String,
    author: Author,
    timestamp: String,
    likes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Author {
    id: String,
    username: String,
}

type Clients = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Message>>>>;
type Topics = Arc<Mutex<HashMap<String, VecDeque<Post>>>>;
type Likes = Arc<Mutex<HashMap<String, HashSet<String>>>>;
type Subscriptions = Arc<Mutex<HashMap<String, HashSet<String>>>>;
type Authors = Arc<Mutex<HashMap<String, Author>>>;

#[tokio::main]
async fn main() {
    let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
    let topics: Topics = Arc::new(Mutex::new(HashMap::new()));
    let likes: Likes = Arc::new(Mutex::new(HashMap::new()));
    let subs: Subscriptions = Arc::new(Mutex::new(HashMap::new()));
    let authors: Authors = Arc::new(Mutex::new(HashMap::new()));

    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(with_state(clients.clone()))
        .and(with_state(topics.clone()))
        .and(with_state(likes.clone()))
        .and(with_state(subs.clone()))
        .and(with_state(authors.clone()))
        .map(|ws: warp::ws::Ws, clients, topics, likes, subs, authors| {
            ws.on_upgrade(move |socket| {
                handle_connection(socket, clients, topics, likes, subs, authors)
            })
        });

    println!("Server running on ws://0.0.0.0:8080");
    warp::serve(ws_route).run(([0, 0, 0, 0], 8080)).await;
}

fn with_state<T: Clone + Send + Sync + 'static>(state: T) -> impl Filter<Extract = (T,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || state.clone())
}

async fn handle_connection(
    ws: warp::ws::WebSocket,
    clients: Clients,
    topics: Topics,
    likes: Likes,
    subs: Subscriptions,
    authors: Authors,
) {
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let client_id = uuid::Uuid::new_v4().to_string();

    clients.lock().await.insert(client_id.clone(), tx.clone());

    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        if let Ok(text) = msg.to_str() {
            if let Ok(parsed) = serde_json::from_str::<ClientMessage>(text) {
                handle_message(
                    parsed,
                    &client_id,
                    &clients,
                    &topics,
                    &likes,
                    &subs,
                    &authors,
                )
                .await;
            }
        }
    }

    clients.lock().await.remove(&client_id);
    subs.lock().await.remove(&client_id);
}

async fn handle_message(
    msg: ClientMessage,
    client_id: &str,
    clients: &Clients,
    topics: &Topics,
    likes: &Likes,
    subs: &Subscriptions,
    authors: &Authors,
) {
    match msg {
        ClientMessage::Init { user_id, username } => {
            authors.lock().await.insert(client_id.to_string(), Author { id: user_id, username });
        }
        ClientMessage::Subscribe { topic } => {
            subs.lock().await.entry(client_id.to_string()).or_default().insert(topic.clone());
            let messages = topics.lock().await.get(&topic).cloned().unwrap_or_default();
            if let Some(tx) = clients.lock().await.get(client_id) {
                for post in messages.iter().rev().take(25).rev() {
                    let msg = Message::text(serde_json::to_string(post).unwrap());
                    let _ = tx.send(msg);
                }
            }
        }
        ClientMessage::Unsubscribe { topic } => {
            subs.lock().await.entry(client_id.to_string()).or_default().remove(&topic);
        }
        ClientMessage::Publish { topic, message } => {
            let post_id = uuid::Uuid::new_v4().to_string();
            let timestamp = chrono::Utc::now().to_rfc3339();
            let author = authors.lock().await.get(client_id).cloned().unwrap_or(Author { id: "anon".into(), username: "anon".into() });
            let post = Post { post_id: post_id.clone(), topic: topic.clone(), message, author, timestamp, likes: 0 };

            let mut topics_lock = topics.lock().await;
            let queue = topics_lock.entry(topic.clone()).or_default();
            queue.push_back(post.clone());
            if queue.len() > 100 { queue.pop_front(); }

            let post_json = Message::text(serde_json::to_string(&post).unwrap());
            let subs_map = subs.lock().await;
            let clients_map = clients.lock().await;
            for (id, topics) in subs_map.iter() {
                if topics.contains(&topic) && id != client_id {
                    if let Some(tx) = clients_map.get(id) {
                        let _ = tx.send(post_json.clone());
                    }
                }
            }
        }
        ClientMessage::Like { post_id } => {
            let mut likes_map = likes.lock().await;
            let entry = likes_map.entry(post_id.clone()).or_default();
            entry.insert(client_id.to_string());
        }
        ClientMessage::Filter { topics: filter_topics } => {
            subs.lock().await.insert(client_id.to_string(), filter_topics.iter().cloned().collect());
        }
        ClientMessage::Search { query } => {
            let topics_lock = topics.lock().await;
            let mut results = Vec::new();
            for messages in topics_lock.values() {
                for post in messages.iter() {
                    if post.message.contains(&query) {
                        results.push(post.clone());
                    }
                }
            }
            if let Some(tx) = clients.lock().await.get(client_id) {
                for post in results.iter().take(25) {
                    let _ = tx.send(Message::text(serde_json::to_string(post).unwrap()));
                }
            }
        }
    }
}
