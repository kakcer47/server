use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};
use futures::{StreamExt, SinkExt};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use dashmap::DashMap;
use warp::{Filter, Reply};
use warp::ws::{Message, Ws};
use uuid::Uuid;
use tracing::{info, error, warn};
use serde::{Deserialize, Serialize};
use tokio::time::{interval, Instant};

// Максимальные лимиты
const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
const MAX_CLIENTS: usize = 10000;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(90);

// Структура клиента
#[derive(Debug)]
struct Client {
    sender: UnboundedSender<Message>,
    last_pong: Instant,
    user_id: Option<u32>,
}

// Shared map of client_id -> Client
type Clients = Arc<DashMap<String, Client>>;

// Структура сообщения
#[derive(Debug, Serialize, Deserialize)]
struct P2PMessage {
    #[serde(rename = "type")]
    msg_type: String,
    data: Option<serde_json::Value>,
    timestamp: u64,
    sender_id: Option<u32>,
}

#[tokio::main]
async fn main() {
    // Инициализация логирования
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    // Shared state
    let clients: Clients = Arc::new(DashMap::new());

    // Запускаем cleanup задачу
    let clients_cleanup = clients.clone();
    tokio::spawn(async move {
        cleanup_disconnected_clients(clients_cleanup).await;
    });

    // Запускаем heartbeat задачу
    let clients_heartbeat = clients.clone();
    tokio::spawn(async move {
        heartbeat_task(clients_heartbeat).await;
    });

    // CORS настройки
    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec!["content-type"])
        .allow_methods(vec!["GET", "POST", "OPTIONS"]);

    // WebSocket route с ограничениями
    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(warp::header::optional::<String>("user-agent"))
        .and(with_clients(clients.clone()))
        .and_then(|ws: Ws, user_agent: Option<String>, clients: Clients| async move {
            // Проверяем количество клиентов
            if clients.len() >= MAX_CLIENTS {
                warn!("🚫 Max clients limit reached: {}", MAX_CLIENTS);
                return Err(warp::reject::custom(TooManyClients));
            }

            info!("🔌 New WebSocket connection from: {:?}", user_agent);
            Ok(ws.on_upgrade(move |socket| handle_ws(socket, clients)))
        });

    // Health check
    let health = warp::path("health")
        .map(|| {
            warp::reply::json(&serde_json::json!({
                "status": "ok",
                "timestamp": chrono::Utc::now().timestamp()
            }))
        });

    // Stats endpoint
    let stats = warp::path("stats")
        .and(with_clients(clients.clone()))
        .map(|clients: Clients| {
            warp::reply::json(&serde_json::json!({
                "connected_clients": clients.len(),
                "max_clients": MAX_CLIENTS,
                "uptime": "TODO", // TODO: добавить uptime
            }))
        });

    let routes = ws_route
        .or(health)
        .or(stats)
        .with(cors)
        .recover(handle_rejection);

    // Запуск сервера
    let addr: SocketAddr = ([0, 0, 0, 0], 8080).into();
    info!("🚀 P2P Signaling server running at {}", addr);
    info!("📊 Max clients: {}, Max message size: {}KB", MAX_CLIENTS, MAX_MESSAGE_SIZE / 1024);

    warp::serve(routes).run(addr).await;
}

fn with_clients(clients: Clients) -> impl Filter<Extract = (Clients,), Error = Infallible> + Clone {
    warp::any().map(move || clients.clone())
}

async fn handle_ws(ws: warp::ws::WebSocket, clients: Clients) {
    let (mut tx_ws, mut rx_ws) = ws.split();
    let client_id = Uuid::new_v4().to_string();
    let (tx, mut rx) = unbounded_channel::<Message>();

    // Создаем нового клиента
    let client = Client {
        sender: tx,
        last_pong: Instant::now(),
        user_id: None,
    };

    // Вставляем клиента
    clients.insert(client_id.clone(), client);
    info!("✅ Client connected: {} (total: {})", client_id, clients.len());

    // Отправляем приветственное сообщение
    let welcome_msg = P2PMessage {
        msg_type: "welcome".to_string(),
        data: Some(serde_json::json!({
            "client_id": client_id,
            "server_time": chrono::Utc::now().timestamp()
        })),
        timestamp: chrono::Utc::now().timestamp() as u64,
        sender_id: None,
    };

    if let Ok(welcome_json) = serde_json::to_string(&welcome_msg) {
        let _ = clients.get(&client_id).unwrap().sender.send(Message::text(welcome_json));
    }

    // Задача отправки сообщений клиенту
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if tx_ws.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Получение сообщений от клиента
    while let Some(result) = rx_ws.next().await {
        match result {
            Ok(msg) => {
                if msg.is_close() {
                    break;
                }

                if msg.is_pong() {
                    // Обновляем время последнего pong
                    if let Some(mut client) = clients.get_mut(&client_id) {
                        client.last_pong = Instant::now();
                    }
                    continue;
                }

                if msg.is_text() {
                    let text = msg.to_str().unwrap_or("");
                    
                    // Проверяем размер сообщения
                    if text.len() > MAX_MESSAGE_SIZE {
                        warn!("🚫 Message too large from {}: {} bytes", client_id, text.len());
                        continue;
                    }

                    // Парсим сообщение
                    match serde_json::from_str::<P2PMessage>(text) {
                        Ok(p2p_msg) => {
                            handle_p2p_message(&client_id, p2p_msg, &clients).await;
                        }
                        Err(e) => {
                            warn!("🚫 Invalid JSON from {}: {}", client_id, e);
                        }
                    }
                } else if msg.is_binary() {
                    // Обрабатываем бинарные данные (сжатые посты)
                    let data = msg.as_bytes();
                    if data.len() <= MAX_MESSAGE_SIZE {
                        broadcast_binary(&client_id, data, &clients).await;
                    } else {
                        warn!("🚫 Binary message too large from {}: {} bytes", client_id, data.len());
                    }
                }
            }
            Err(e) => {
                error!("❌ WebSocket error from {}: {}", client_id, e);
                break;
            }
        }
    }

    // Cleanup при отключении
    clients.remove(&client_id);
    info!("❌ Client disconnected: {} (total: {})", client_id, clients.len());
}

async fn handle_p2p_message(sender_id: &str, msg: P2PMessage, clients: &Clients) {
    info!("📨 Handling {} message from {}", msg.msg_type, sender_id);

    match msg.msg_type.as_str() {
        "ping" => {
            // Отвечаем pong
            let pong_msg = P2PMessage {
                msg_type: "pong".to_string(),
                data: None,
                timestamp: chrono::Utc::now().timestamp() as u64,
                sender_id: None,
            };

            if let Ok(pong_json) = serde_json::to_string(&pong_msg) {
                if let Some(client) = clients.get(sender_id) {
                    let _ = client.sender.send(Message::text(pong_json));
                }
            }
        }
        
        "user_info" => {
            // Сохраняем информацию о пользователе
            if let Some(data) = &msg.data {
                if let Some(user_id) = data.get("user_id").and_then(|v| v.as_u64()) {
                    if let Some(mut client) = clients.get_mut(sender_id) {
                        client.user_id = Some(user_id as u32);
                        info!("👤 Client {} identified as user {}", sender_id, user_id);
                    }
                }
            }
        }

        _ => {
            // Пересылаем сообщение всем остальным клиентам
            broadcast_message(sender_id, &msg, clients).await;
        }
    }
}

async fn broadcast_message(sender_id: &str, msg: &P2PMessage, clients: &Clients) {
    if let Ok(json) = serde_json::to_string(msg) {
        let message = Message::text(json);
        let mut sent_count = 0;

        for entry in clients.iter() {
            if entry.key() != sender_id {
                if entry.value().sender.send(message.clone()).is_ok() {
                    sent_count += 1;
                }
            }
        }

        if sent_count > 0 {
            info!("📡 Broadcasted {} to {} clients", msg.msg_type, sent_count);
        }
    }
}

async fn broadcast_binary(sender_id: &str, data: &[u8], clients: &Clients) {
    let message = Message::binary(data);
    let mut sent_count = 0;

    for entry in clients.iter() {
        if entry.key() != sender_id {
            if entry.value().sender.send(message.clone()).is_ok() {
                sent_count += 1;
            }
        }
    }

    if sent_count > 0 {
        info!("📡 Broadcasted binary data ({} bytes) to {} clients", data.len(), sent_count);
    }
}

// Задача очистки отключенных клиентов
async fn cleanup_disconnected_clients(clients: Clients) {
    let mut interval = interval(Duration::from_secs(60)); // Каждую минуту

    loop {
        interval.tick().await;
        
        let mut to_remove = Vec::new();
        let now = Instant::now();

        for entry in clients.iter() {
            if now.duration_since(entry.value().last_pong) > CLIENT_TIMEOUT {
                to_remove.push(entry.key().clone());
            }
        }

        for client_id in to_remove {
            clients.remove(&client_id);
            info!("🧹 Removed inactive client: {}", client_id);
        }
    }
}

// Задача heartbeat
async fn heartbeat_task(clients: Clients) {
    let mut interval = interval(HEARTBEAT_INTERVAL);

    loop {
        interval.tick().await;

        let ping_msg = P2PMessage {
            msg_type: "ping".to_string(),
            data: None,
            timestamp: chrono::Utc::now().timestamp() as u64,
            sender_id: None,
        };

        if let Ok(ping_json) = serde_json::to_string(&ping_msg) {
            let ping_message = Message::text(ping_json);
            
            for entry in clients.iter() {
                let _ = entry.value().sender.send(ping_message.clone());
            }
            
            info!("💓 Sent heartbeat to {} clients", clients.len());
        }
    }
}

// Кастомные ошибки
#[derive(Debug)]
struct TooManyClients;
impl warp::reject::Reject for TooManyClients {}

async fn handle_rejection(err: warp::Rejection) -> Result<impl Reply, Infallible> {
    let code;
    let message;

    if err.is_not_found() {
        code = warp::http::StatusCode::NOT_FOUND;
        message = "Not Found";
    } else if let Some(_) = err.find::<TooManyClients>() {
        code = warp::http::StatusCode::SERVICE_UNAVAILABLE;
        message = "Too Many Clients";
    } else {
        code = warp::http::StatusCode::INTERNAL_SERVER_ERROR;
        message = "Internal Server Error";
    }

    Ok(warp::reply::with_status(message, code))
}
