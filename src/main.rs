// ✅ Полноценный продакшн WebSocket сервер с Supabase интеграцией

use warp::Filter;
use warp::ws::{Message, WebSocket};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use uuid::Uuid;
use reqwest::Client;

// ========== Типы ==========
type Clients = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Message>>>>;

#[derive(Clone)]
struct AppState {
    clients: Clients,
    http: Client,
    supabase_url: String,
    supabase_key: String,
}

// ========== Точка входа ==========
#[tokio::main]
async fn main() {
    let state = AppState {
        clients: Arc::new(Mutex::new(HashMap::new())),
        http: Client::new(),
        supabase_url: std::env::var("SUPABASE_URL").expect("SUPABASE_URL not set"),
        supabase_key: std::env::var("SUPABASE_KEY").expect("SUPABASE_KEY not set"),
    };

    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(with_state(state.clone()))
        .map(|ws: warp::ws::Ws, state| ws.on_upgrade(move |socket| handle_socket(socket, state)));

    println!("🚀 WebSocket server running on ws://0.0.0.0:8080");
    warp::serve(ws_route).run(([0, 0, 0, 0], 8080)).await;
}

fn with_state(state: AppState) -> impl Filter<Extract = (AppState,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || state.clone())
}

// ========== Обработка соединения ==========
async fn handle_socket(ws: WebSocket, state: AppState) {
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let client_id = Uuid::new_v4().to_string();

    state.clients.lock().await.insert(client_id.clone(), tx);
    let clients = state.clients.clone();

    // Отправка клиенту
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
        clients.lock().await.remove(&client_id);
    });

    // Приём от клиента
    while let Some(Ok(msg)) = ws_rx.next().await {
        if msg.is_text() {
            if let Ok(value) = serde_json::from_str::<Value>(msg.to_str().unwrap()) {
                handle_message(value, &client_id, &state).await;
            }
        }
    }
}

// ========== Обработка сообщений ==========
async fn handle_message(msg: Value, client_id: &str, state: &AppState) {
    match msg.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "Publish" => {
            if let Some(entry) = msg.get("entry") {
                let _ = save_entry_to_supabase(entry.clone(), state).await;
                broadcast_to_others(client_id, entry.clone(), state).await;
            }
        }
        "Like" => {
            if let (Some(entry_id), Some(telegram_id)) = (msg.get("entry_id"), msg.get("telegram_id")) {
                let _ = toggle_like(entry_id.as_i64().unwrap(), telegram_id.as_i64().unwrap(), state).await;
                // можно также разослать лайк
            }
        }
        _ => {}
    }
}

// ========== Сохранение поста ==========
async fn save_entry_to_supabase(entry: Value, state: &AppState) -> Result<(), reqwest::Error> {
    state.http
        .post(format!("{}/rest/v1/entries", state.supabase_url))
        .bearer_auth(&state.supabase_key)
        .json(&entry)
        .send()
        .await?;
    Ok(())
}

// ========== Переключение лайка ==========
async fn toggle_like(entry_id: i64, telegram_id: i64, state: &AppState) -> Result<(), reqwest::Error> {
    state.http
        .post(format!("{}/rest/v1/rpc/toggle_like", state.supabase_url))
        .bearer_auth(&state.supabase_key)
        .json(&serde_json::json!({ "entry_id": entry_id, "user_telegram_id": telegram_id }))
        .send()
        .await?;
    Ok(())
}

// ========== Рассылка ==========
async fn broadcast_to_others(sender_id: &str, entry: Value, state: &AppState) {
    let msg = Message::text(
        serde_json::json!({ "type": "NewPost", "entry": entry }).to_string(),
    );

    for (id, tx) in state.clients.lock().await.iter() {
        if id != sender_id {
            let _ = tx.send(msg.clone());
        }
    }
}
