use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use dotenv::dotenv;
use futures::{FutureExt, StreamExt, SinkExt};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use warp::{filters::BoxedFilter, ws::Ws, Filter, Reply};
use dashmap::DashMap;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

// === TYPES ===
type ClientId = String;
type Clients = Arc<DashMap<ClientId, Client>>;

#[derive(Debug, Clone)]
struct Client {
    sender: tokio::sync::mpsc::UnboundedSender<Message>,
    user_id: u32,
    master_key: String,
    joined_at: u64,
    last_sync: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TelegramAuth {
    id: u32,
    username: Option<String>,
    first_name: String,
    auth_date: u64,
    hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CompactPost {
    id: u32,
    author: u32,
    title_hash: u32,
    content_hash: u32,
    timestamp: u32,
    likes: u16,
    tags: u16,
    meta: u16,
}

#[derive(Debug, Serialize, Deserialize)]
struct WsMessage {
    #[serde(rename = "type")]
    msg_type: String,
    auth: Option<TelegramAuth>,
    master_key: Option<String>,
    data: Option<serde_json::Value>,
    timestamp: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PostOperation {
    op_type: String, // "create", "like", "edit", "delete"
    post: Option<CompactPost>,
    post_id: Option<u32>,
    user_id: u32,
    timestamp: u64,
    signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeltaSync {
    operations: Vec<PostOperation>,
    string_pool: Vec<(u32, String)>, // hash -> string
    since_timestamp: u64,
    merkle_root: String,
}

// === GLOBALS ===
struct AppState {
    clients: Clients,
    posts: Arc<DashMap<u32, CompactPost>>,
    string_pool: Arc<DashMap<u32, String>>,
    operations: Arc<RwLock<Vec<PostOperation>>>,
    user_keys: Arc<DashMap<u32, String>>, // telegram_id -> master_key
    bot_token: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
        .expect("TELEGRAM_BOT_TOKEN must be set");

    let state = Arc::new(AppState {
        clients: Arc::new(DashMap::new()),
        posts: Arc::new(DashMap::new()),
        string_pool: Arc::new(DashMap::new()),
        operations: Arc::new(RwLock::new(Vec::new())),
        user_keys: Arc::new(DashMap::new()),
        bot_token,
    });

    // Preload common strings
    init_string_pool(&state.string_pool).await;

    // WebSocket route
    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(with_state(state.clone()))
        .and_then(ws_handler);

    // Health check
    let health = warp::path("health")
        .map({
            let state = state.clone();
            move || {
                let stats = serde_json::json!({
                    "status": "healthy",
                    "clients": state.clients.len(),
                    "posts": state.posts.len(),
                    "uptime": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                    "memory_mb": get_memory_usage(),
                });
                warp::reply::json(&stats)
            }
        });

    // Stats endpoint
    let stats = warp::path("stats")
        .map({
            let state = state.clone();
            move || {
                let operations_count = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        state.operations.read().await.len()
                    })
                });
                
                let stats = serde_json::json!({
                    "total_clients": state.clients.len(),
                    "total_posts": state.posts.len(),
                    "total_operations": operations_count,
                    "string_pool_size": state.string_pool.len(),
                    "registered_users": state.user_keys.len(),
                    "compression_ratio": calculate_compression_ratio(&state),
                });
                warp::reply::json(&stats)
            }
        });

    // CORS
    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec!["content-type"])
        .allow_methods(vec!["GET", "POST", "OPTIONS"]);

    let routes = ws_route
        .or(health)
        .or(stats)
        .with(cors);

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse()
        .unwrap_or(8080);

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    
    info!("🚀 P2P WebSocket Server starting on {}", addr);
    info!("📊 Health: http://localhost:{}/health", port);
    info!("📈 Stats: http://localhost:{}/stats", port);

    warp::serve(routes)
        .run(addr)
        .await;

    Ok(())
}

async fn ws_handler(
    ws: Ws,
    state: Arc<AppState>,
) -> Result<impl Reply, Infallible> {
    Ok(ws.on_upgrade(move |socket| handle_client(socket, state)))
}

async fn handle_client(
    socket: tokio_tungstenite::WebSocketStream<warp::ws::WebSocket>,
    state: Arc<AppState>,
) {
    let client_id = uuid::Uuid::new_v4().to_string();
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    info!("🔌 New client connected: {}", client_id);

    // Handle outgoing messages
    let outgoing = async {
        while let Some(msg) = rx.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    };

    // Handle incoming messages
    let incoming = async {
        while let Some(result) = ws_receiver.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    if let Err(e) = handle_message(&state, &client_id, &text).await {
                        error!("Message handling error: {}", e);
                    }
                }
                Ok(Message::Binary(data)) => {
                    if let Err(e) = handle_binary_message(&state, &client_id, &data).await {
                        error!("Binary message handling error: {}", e);
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }
    };

    // Run both tasks
    tokio::select! {
        _ = incoming => {},
        _ = outgoing => {},
    }

    // Cleanup
    state.clients.remove(&client_id);
    info!("🔌 Client disconnected: {}", client_id);
}

async fn handle_message(
    state: &Arc<AppState>,
    client_id: &str,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let msg: WsMessage = serde_json::from_str(text)?;

    match msg.msg_type.as_str() {
        "auth" => handle_auth(state, client_id, msg).await?,
        "sync_request" => handle_sync_request(state, client_id, msg).await?,
        "create_post" => handle_create_post(state, client_id, msg).await?,
        "like_post" => handle_like_post(state, client_id, msg).await?,
        "ping" => handle_ping(state, client_id).await?,
        _ => warn!("Unknown message type: {}", msg.msg_type),
    }

    Ok(())
}

async fn handle_binary_message(
    state: &Arc<AppState>,
    client_id: &str,
    data: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    // Decompress LZ4
    let decompressed = lz4_flex::decompress_size_prepended(data)?;
    
    // Deserialize MessagePack
    let delta: DeltaSync = rmp_serde::from_slice(&decompressed)?;
    
    info!("📦 Received compressed delta: {} ops", delta.operations.len());
    
    // Apply operations
    apply_delta_operations(state, &delta.operations).await?;
    
    // Broadcast to other clients
    broadcast_delta(state, client_id, &delta).await?;
    
    Ok(())
}

async fn handle_auth(
    state: &Arc<AppState>,
    client_id: &str,
    msg: WsMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let auth = msg.auth.ok_or("Missing auth data")?;
    let master_key = msg.master_key.ok_or("Missing master key")?;

    // Verify Telegram authentication
    if !verify_telegram_auth(&auth, &state.bot_token) {
        return Err("Invalid Telegram authentication".into());
    }

    // Get or create master key for user
    let stored_key = state.user_keys
        .entry(auth.id)
        .or_insert_with(|| master_key.clone())
        .clone();

    // Verify master key matches
    if stored_key != master_key {
        return Err("Master key mismatch".into());
    }

    // Create client
    let (tx, _) = tokio::sync::mpsc::unbounded_channel();
    let client = Client {
        sender: tx,
        user_id: auth.id,
        master_key: master_key.clone(),
        joined_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        last_sync: 0,
    };

    state.clients.insert(client_id.to_string(), client);

    info!("✅ User {} authenticated with master key", auth.id);

    // Send auth success
    let response = serde_json::json!({
        "type": "auth_success",
        "user_id": auth.id,
        "master_key": stored_key,
        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    });

    send_to_client(state, client_id, Message::Text(response.to_string())).await;

    Ok(())
}

async fn handle_sync_request(
    state: &Arc<AppState>,
    client_id: &str,
    msg: WsMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let since = msg.timestamp.unwrap_or(0);
    
    // Get operations since timestamp
    let operations = state.operations.read().await;
    let filtered_ops: Vec<PostOperation> = operations
        .iter()
        .filter(|op| op.timestamp > since)
        .cloned()
        .collect();
    drop(operations);

    // Get relevant string pool entries
    let string_pool_entries: Vec<(u32, String)> = state.string_pool
        .iter()
        .map(|entry| (*entry.key(), entry.value().clone()))
        .collect();

    let delta = DeltaSync {
        operations: filtered_ops,
        string_pool: string_pool_entries,
        since_timestamp: since,
        merkle_root: calculate_merkle_root(state).await,
    };

    // Compress with MessagePack + LZ4
    let serialized = rmp_serde::to_vec(&delta)?;
    let compressed = lz4_flex::compress_prepend_size(&serialized);

    info!("📤 Sending sync: {} ops, compressed {} -> {} bytes", 
          delta.operations.len(), serialized.len(), compressed.len());

    send_to_client(state, client_id, Message::Binary(compressed)).await;

    Ok(())
}

async fn handle_create_post(
    state: &Arc<AppState>,
    client_id: &str,
    msg: WsMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = msg.data.ok_or("Missing post data")?;
    let client = state.clients.get(client_id).ok_or("Client not authenticated")?;

    // Parse post data
    let title: String = data["title"].as_str().ok_or("Missing title")?.to_string();
    let content: String = data["content"].as_str().ok_or("Missing content")?.to_string();
    let tags: u16 = data["tags"].as_u64().unwrap_or(0) as u16;

    // Add strings to pool
    let title_hash = add_to_string_pool(&state.string_pool, title);
    let content_hash = add_to_string_pool(&state.string_pool, content);

    // Create post
    let post_id = generate_post_id();
    let post = CompactPost {
        id: post_id,
        author: client.user_id,
        title_hash,
        content_hash,
        timestamp: (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - 1_700_000_000) as u32,
        likes: 0,
        tags,
        meta: 0,
    };

    // Store post
    state.posts.insert(post_id, post.clone());

    // Create operation
    let operation = PostOperation {
        op_type: "create".to_string(),
        post: Some(post),
        post_id: Some(post_id),
        user_id: client.user_id,
        timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        signature: generate_signature(&client.master_key, post_id),
    };

    // Store operation
    state.operations.write().await.push(operation.clone());

    info!("📝 Post {} created by user {}", post_id, client.user_id);

    // Broadcast to all clients
    broadcast_operation(state, client_id, &operation).await;

    Ok(())
}

async fn handle_like_post(
    state: &Arc<AppState>,
    client_id: &str,
    msg: WsMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = msg.data.ok_or("Missing like data")?;
    let client = state.clients.get(client_id).ok_or("Client not authenticated")?;
    let post_id = data["post_id"].as_u64().ok_or("Missing post_id")? as u32;
    let liked = data["liked"].as_bool().unwrap_or(true);

    // Update post likes
    if let Some(mut post) = state.posts.get_mut(&post_id) {
        if liked {
            post.likes = post.likes.saturating_add(1);
        } else {
            post.likes = post.likes.saturating_sub(1);
        }
    }

    // Create operation
    let operation = PostOperation {
        op_type: "like".to_string(),
        post: None,
        post_id: Some(post_id),
        user_id: client.user_id,
        timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        signature: generate_signature(&client.master_key, post_id),
    };

    // Store operation
    state.operations.write().await.push(operation.clone());

    // Broadcast to all clients
    broadcast_operation(state, client_id, &operation).await;

    Ok(())
}

async fn handle_ping(
    state: &Arc<AppState>,
    client_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = serde_json::json!({
        "type": "pong",
        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    });

    send_to_client(state, client_id, Message::Text(response.to_string())).await;
    Ok(())
}

// === HELPER FUNCTIONS ===

fn verify_telegram_auth(auth: &TelegramAuth, bot_token: &str) -> bool {
    // Create data string for HMAC verification
    let data_string = format!(
        "auth_date={}&first_name={}&id={}&username={}",
        auth.auth_date,
        auth.first_name,
        auth.id,
        auth.username.as_deref().unwrap_or("")
    );

    // Calculate HMAC
    let secret_key = format!("WebAppData{}", bot_token);
    let mut mac = HmacSha256::new(secret_key.as_bytes());
    mac.update(data_string.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    // Verify hash
    expected == auth.hash
}

async fn init_string_pool(pool: &Arc<DashMap<u32, String>>) {
    let common_strings = vec![
        "спорт", "технологии", "путешествия", "еда", "музыка",
        "искусство", "новости", "игры", "крипто", "учеба",
        "работа", "здоровье", "фото", "видео", "мемы", "другое"
    ];

    for s in common_strings {
        let hash = calculate_string_hash(s);
        pool.insert(hash, s.to_string());
    }

    info!("🎯 String pool initialized with {} entries", pool.len());
}

fn add_to_string_pool(pool: &Arc<DashMap<u32, String>>, text: String) -> u32 {
    let hash = calculate_string_hash(&text);
    pool.entry(hash).or_insert(text);
    hash
}

fn calculate_string_hash(s: &str) -> u32 {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let result = hasher.finalize();
    u32::from_le_bytes([result[0], result[1], result[2], result[3]])
}

fn generate_post_id() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u32
}

fn generate_signature(master_key: &str, post_id: u32) -> String {
    let mut mac = HmacSha256::new(master_key.as_bytes());
    mac.update(&post_id.to_le_bytes());
    hex::encode(mac.finalize().into_bytes())
}

async fn apply_delta_operations(
    state: &Arc<AppState>,
    operations: &[PostOperation],
) -> Result<(), Box<dyn std::error::Error>> {
    for op in operations {
        match op.op_type.as_str() {
            "create" => {
                if let Some(post) = &op.post {
                    state.posts.insert(post.id, post.clone());
                }
            }
            "like" => {
                if let Some(post_id) = op.post_id {
                    if let Some(mut post) = state.posts.get_mut(&post_id) {
                        post.likes = post.likes.saturating_add(1);
                    }
                }
            }
            _ => {}
        }
    }

    // Store operations
    let mut ops = state.operations.write().await;
    ops.extend(operations.iter().cloned());
    
    // Keep only last 10000 operations
    if ops.len() > 10000 {
        ops.drain(0..ops.len() - 10000);
    }

    Ok(())
}

async fn broadcast_operation(state: &Arc<AppState>, exclude_client: &str, operation: &PostOperation) {
    let msg = serde_json::json!({
        "type": "operation",
        "operation": operation
    });

    broadcast_message(state, exclude_client, Message::Text(msg.to_string())).await;
}

async fn broadcast_delta(state: &Arc<AppState>, exclude_client: &str, delta: &DeltaSync) {
    // Compress delta
    if let Ok(serialized) = rmp_serde::to_vec(delta) {
        let compressed = lz4_flex::compress_prepend_size(&serialized);
        broadcast_message(state, exclude_client, Message::Binary(compressed)).await;
    }
}

async fn broadcast_message(state: &Arc<AppState>, exclude_client: &str, message: Message) {
    let mut failed_clients = Vec::new();

    for entry in state.clients.iter() {
        let client_id = entry.key();
        let client = entry.value();

        if client_id == exclude_client {
            continue;
        }

        if client.sender.send(message.clone()).is_err() {
            failed_clients.push(client_id.clone());
        }
    }

    // Remove failed clients
    for client_id in failed_clients {
        state.clients.remove(&client_id);
    }
}

async fn send_to_client(state: &Arc<AppState>, client_id: &str, message: Message) {
    if let Some(client) = state.clients.get(client_id) {
        let _ = client.sender.send(message);
    }
}

async fn calculate_merkle_root(state: &Arc<AppState>) -> String {
    let operations = state.operations.read().await;
    let mut hasher = sha2::Sha256::new();
    
    for op in operations.iter() {
        hasher.update(&op.timestamp.to_le_bytes());
        hasher.update(&op.user_id.to_le_bytes());
        if let Some(post_id) = op.post_id {
            hasher.update(&post_id.to_le_bytes());
        }
    }
    
    hex::encode(hasher.finalize())
}

fn calculate_compression_ratio(state: &Arc<AppState>) -> f64 {
    let post_count = state.posts.len();
    if post_count == 0 {
        return 0.0;
    }

    let compressed_size = post_count * std::mem::size_of::<CompactPost>();
    let uncompressed_size = post_count * 2500; // Estimated JSON size
    
    1.0 - (compressed_size as f64 / uncompressed_size as f64)
}

fn get_memory_usage() -> u64 {
    // Simple memory estimation
    std::process::id() as u64 // Placeholder
}

fn with_state(
    state: Arc<AppState>,
) -> BoxedFilter<(Arc<AppState>,)> {
    warp::any().map(move || state.clone()).boxed()
}
