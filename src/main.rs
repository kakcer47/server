//! main.rs — Production-ready autonomous P2P WebSocket server
//! - In-memory & on-disk persistence (Sled)
//! - Create/Edit/Delete/Like operations
//! - Telegram WebApp auth
//! - UUID post IDs, HMAC signatures
//! - Memory metrics via sysinfo
//! - Parallel Tokio tasks
//! - Optional TLS via Rustls if CERT_PATH & KEY_PATH provided

use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::{SystemTime, UNIX_EPOCH}};
use futures::{StreamExt, SinkExt};
use tokio::sync::{mpsc::UnboundedSender, RwLock};
use warp::{Filter, Reply};
use warp::ws::{Message, Ws};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use hmac::{Hmac, Mac}; use sha2::Sha256;
use dashmap::DashMap;
use sled::Db;
use sysinfo::{System, SystemExt};
use tracing::{info, error};

// Type aliases
type HmacSha256 = Hmac<Sha256>;
type ClientId = String;

// === Data Structures ===
#[derive(Clone)]
struct AppState {
    clients: Arc<DashMap<ClientId, Client>>,
    posts: Arc<DashMap<String, CompactPost>>,
    operations: Arc<RwLock<Vec<PostOperation>>> ,
    user_keys: Arc<DashMap<u32, String>>,
    db: Db,
    bot_token: String,
}

#[derive(Clone)]
struct Client { sender: UnboundedSender<Message>, user_id: u32, master_key: String }

#[derive(Serialize, Deserialize, Clone)]
struct TelegramAuth { id: u32, username: Option<String>, first_name: String, auth_date: u64, hash: String }

#[derive(Serialize, Deserialize, Clone)]
struct CompactPost {
    id: String,
    author: u32,
    title: String,
    content: String,
    timestamp: u64,
    likes: u16,
}

#[derive(Serialize, Deserialize, Clone)]
struct PostOperation {
    op_type: String,
    post: Option<CompactPost>,
    post_id: Option<String>,
    user_id: u32,
    timestamp: u64,
    signature: String,
}

#[derive(Serialize, Deserialize)]
struct WsMessageIn {
    #[serde(rename="type")] msg_type: String,
    auth: Option<TelegramAuth>,
    master_key: Option<String>,
    data: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct WsMessageOut<T: Serialize> { #[serde(rename="type")] msg_type: String, #[serde(flatten)] inner: T }

#[derive(Serialize)]
struct AuthSuccess { user_id: u32, master_key: String, timestamp: u64 }

#[derive(Serialize)]
struct Health { status: &'static str, memory_mb: u64 }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt().init();

    let bot_token = std::env::var("TELEGRAM_BOT_TOKEN").expect("set TELEGRAM_BOT_TOKEN");
    let db = sled::open("p2p_ws.db")?;

    // Initialize state
    let state = Arc::new(AppState {
        clients: Arc::new(DashMap::new()),
        posts: Arc::new(DashMap::new()),
        operations: Arc::new(RwLock::new(Vec::new())),
        user_keys: Arc::new(DashMap::new()),
        db,
        bot_token,
    });

    // Load persisted user_keys
    let uk = state.db.open_tree("user_keys")?;
    for item in uk.iter() {
        let (k, v) = item?;
        let id = u32::from_le_bytes(k.as_ref().try_into().unwrap());
        let key = String::from_utf8(v.to_vec())?;
        state.user_keys.insert(id, key);
    }

    // Load persisted posts and operations
    let pt = state.db.open_tree("posts")?;
    for item in pt.iter() {
        let (k, v) = item?;
        let post: CompactPost = serde_json::from_slice(&v)?;
        state.posts.insert(String::from_utf8(k.to_vec())?, post);
    }
    let ot = state.db.open_tree("operations")?;
    for item in ot.iter() {
        let (_, v) = item?;
        let op: PostOperation = serde_json::from_slice(&v)?;
        state.operations.write().await.push(op);
    }

    // Routes
    let state_filter = warp::any().map(move || state.clone());
    let ws_route = warp::path("ws").and(warp::ws()).and(state_filter.clone()).and_then(ws_handler);
    let health = warp::path("health").map(|| warp::reply::json(&Health { status: "ok", memory_mb: current_memory_mb() }));
    let routes = ws_route.or(health).with(warp::cors().allow_any_origin());

    // Start server (with optional TLS)
    let port: u16 = std::env::var("PORT").unwrap_or_else(|_| "8080".into()).parse()?;
    let addr = SocketAddr::from(([0,0,0,0], port));
    info!("🚀 Server listening on {}", addr);

    if let (Ok(cert), Ok(key)) = (std::env::var("CERT_PATH"), std::env::var("KEY_PATH")) {
        warp::serve(routes).tls().cert_path(cert).key_path(key).run(addr).await;
    } else {
        warp::serve(routes).run(addr).await;
    }

    Ok(())
}

// WebSocket handshake
async fn ws_handler(ws: Ws, state: Arc<AppState>) -> Result<impl Reply, Infallible> {
    Ok(ws.on_upgrade(move |socket| handle_client(socket, state)))
}

// Client lifecycle
async fn handle_client(socket: warp::ws::WebSocket, state: Arc<AppState>) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let client_id = Uuid::new_v4().to_string();

    // Outgoing loop
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(msg).await.is_err() { break; }
        }
    });

    // Incoming loop
    let state_clone = state.clone();
    tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            if let Message::Text(text) = msg {
                if let Err(e) = handle_message(&state_clone, &client_id, &tx_clone, &text).await {
                    error!("Error handling message: {}", e);
                }
            }
        }
        // Cleanup
        state_clone.clients.remove(&client_id);
    });
}

// Route incoming messages
async fn handle_message(
    state: &Arc<AppState>,
    client_id: &str,
    tx: &UnboundedSender<Message>,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let msg: WsMessageIn = serde_json::from_str(text)?;
    match msg.msg_type.as_str() {
        "auth"   => handle_auth(state, client_id, tx, msg).await?,
        "create" => handle_create(state, client_id, msg).await?,
        "edit"   => handle_edit(state, client_id, msg).await?,
        "delete" => handle_delete(state, client_id, msg).await?,
        "like"   => handle_like(state, client_id, msg).await?,
        _ => {}
    }
    Ok(())
}

async fn handle_auth(
    state: &Arc<AppState>,
    client_id: &str,
    tx: &UnboundedSender<Message>,
    msg: WsMessageIn,
) -> Result<(), Box<dyn std::error::Error>> {
    let auth = msg.auth.ok_or("Missing auth")?;
    let mk = msg.master_key.ok_or("Missing master key")?;
    // verify Telegram hash
    let data_str = format!("auth_date={}&first_name={}&id={}&username={}", auth.auth_date, auth.first_name, auth.id, auth.username.clone().unwrap_or_default());
    let mut mac = HmacSha256::new_from_slice(format!("WebAppData{}", state.bot_token).as_bytes())?;
    mac.update(data_str.as_bytes());
    if hex::encode(mac.finalize().into_bytes()) != auth.hash { return Err("Invalid auth".into()); }
    // store master key
    let mk_stored = state.user_keys.entry(auth.id).or_insert(mk.clone()).clone();
    // persist
    let tree = state.db.open_tree("user_keys")?;
    tree.insert(&auth.id.to_le_bytes(), mk_stored.as_bytes())?;

    // add client
    state.clients.insert(client_id.to_string(), Client { sender: tx.clone(), user_id: auth.id, master_key: mk_stored.clone() });
    info!("User {} authenticated", auth.id);

    let out = WsMessageOut { msg_type: "auth_success".into(), inner: AuthSuccess { user_id: auth.id, master_key: mk_stored, timestamp: now_secs() }};
    let txt = serde_json::to_string(&out)?;
    tx.send(Message::Text(txt))?;
    Ok(())
}

async fn handle_create(
    state: &Arc<AppState>,
    client_id: &str,
    msg: WsMessageIn,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = msg.data.ok_or("Missing data")?;
    let client = state.clients.get(client_id).ok_or("Not authed")?;
    let title = data["title"].as_str().unwrap_or_default().to_string();
    let content = data["content"].as_str().unwrap_or_default().to_string();
    let id = Uuid::new_v4().to_string();
    let ts = now_secs();

    let post = CompactPost { id: id.clone(), author: client.user_id, title: title.clone(), content: content.clone(), timestamp: ts, likes: 0 };
    state.posts.insert(id.clone(), post.clone());

    let sig = { let mut m=HmacSha256::new_from_slice(client.master_key.as_bytes())?; m.update(id.as_bytes()); hex::encode(m.finalize().into_bytes()) };
    let op = PostOperation { op_type: "create".into(), post: Some(post.clone()), post_id: Some(id.clone()), user_id: client.user_id, timestamp: ts, signature: sig };
    state.operations.write().await.push(op.clone());
    let tree = state.db.open_tree("operations")?;
    tree.insert(&ts.to_le_bytes(), serde_json::to_vec(&op)?)?;
    let tree2 = state.db.open_tree("posts")?;
    tree2.insert(id.as_bytes(), serde_json::to_vec(&post)?)?;

    broadcast(state, client_id, &op).await;
    Ok(())
}

async fn handle_edit(
    state: &Arc<AppState>,
    client_id: &str,
    msg: WsMessageIn,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = msg.data.ok_or("Missing data")?;
    let id = data["post_id"].as_str().unwrap_or_default();
    let new_title = data["title"].as_str().unwrap_or_default().to_string();
    let new_content = data["content"].as_str().unwrap_or_default().to_string();
    if let Some(mut post) = state.posts.get_mut(id) {
        post.title = new_title.clone();
        post.content = new_content.clone();
        // persist
        let tree = state.db.open_tree("posts")?;
        tree.insert(id.as_bytes(), serde_json::to_vec(&*post)?)?;

        // op
        let client = state.clients.get(client_id).unwrap();
        let ts = now_secs();
        let sig = { let mut m=HmacSha256::new_from_slice(client.master_key.as_bytes())?; m.update(id.as_bytes()); hex::encode(m.finalize().into_bytes()) };
        let op = PostOperation { op_type:"edit".into(), post: Some(post.clone()), post_id: Some(id.into()), user_id: client.user_id, timestamp: ts, signature: sig };
        state.operations.write().await.push(op.clone());
        state.db.open_tree("operations")?.insert(ts.to_le_bytes(), serde_json::to_vec(&op)?)?;
        broadcast(state, client_id, &op).await;
    }
    Ok(())
}

async fn handle_delete(
    state: &Arc<AppState>,
    client_id: &str,
    msg: WsMessageIn,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = msg.data.ok_or("Missing data")?;
    let id = data["post_id"].as_str().unwrap_or_default();
    state.posts.remove(id);
    state.db.open_tree("posts")?.remove(id.as_bytes())?;
    let client = state.clients.get(client_id).unwrap();
    let ts = now_secs();
    let sig = { let mut m=HmacSha256::new_from_slice(client.master_key.as_bytes())?; m.update(id.as_bytes()); hex::encode(m.finalize().into_bytes()) };
    let op = PostOperation { op_type:"delete".into(), post: None, post_id: Some(id.into()), user_id: client.user_id, timestamp: ts, signature: sig };
    state.operations.write().await.push(op.clone());
    state.db.open_tree("operations")?.insert(ts.to_le_bytes(), serde_json::to_vec(&op)?)?;
    broadcast(state, client_id, &op).await;
    Ok(())
}

async fn handle_like(
    state: &Arc<AppState>,
    client_id: &str,
    msg: WsMessageIn,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = msg.data.ok_or("Missing data")?;
    let id = data["post_id"].as_str().unwrap_or_default();
    if let Some(mut post) = state.posts.get_mut(id) {
        post.likes = post.likes.saturating_add(1);
        state.db.open_tree("posts")?.insert(id.as_bytes(), serde_json::to_vec(&*post)?)?;
        let client = state.clients.get(client_id).unwrap();
        let ts = now_secs();
        let sig = { let mut m=HmacSha256::new_from_slice(client.master_key.as_bytes())?; m.update(id.as_bytes()); hex::encode(m.finalize().into_bytes()) };
        let op = PostOperation { op_type:"like".into(), post: None, post_id: Some(id.into()), user_id: client.user_id, timestamp: ts, signature: sig };
        state.operations.write().await.push(op.clone());
        state.db.open_tree("operations")?.insert(ts.to_le_bytes(), serde_json::to_vec(&op)?)?;
        broadcast(state, client_id, &op).await;
    }
    Ok(())
}

// Broadcast op to all other clients
async fn broadcast(state: &Arc<AppState>, exclude: &str, op: &PostOperation) {
    let out = WsMessageOut { msg_type: "operation".into(), inner: op };
    if let Ok(txt) = serde_json::to_string(&out) {
        for kv in state.clients.iter() {
            if kv.key() != exclude {
                let _ = kv.value().sender.send(Message::Text(txt.clone()));
            }
        }
    }
}

fn now_secs() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() }

fn current_memory_mb() -> u64 {
    let mut sys = System::new_all(); sys.refresh_process(sysinfo::get_current_pid().unwrap());
    sys.process(sysinfo::get_current_pid().unwrap()).map(|p| p.memory() / 1024).unwrap_or(0)
}
