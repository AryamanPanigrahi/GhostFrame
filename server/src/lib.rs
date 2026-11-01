//! Riposte relay server: pairs two players by room code and forwards their
//! bytes without understanding them (DESIGN.md section 8).

pub mod protocol;
pub mod rooms;
pub mod web;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use protocol::{ClientMsg, ServerMsg};
use rooms::{ConnId, Outbox, Rooms};

/// Rooms with no traffic for this long are deleted.
pub const ROOM_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How often the background task looks for idle rooms.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(30);
/// Biggest WebSocket message we accept. A `PeerMsg::Inputs` is a few dozen
/// bytes, so this is generous, and it stops a client from making us buffer
/// huge messages.
const MAX_MESSAGE_BYTES: usize = 16 * 1024;

/// Shared by every connection. Cloning clones the `Arc`, not the data.
#[derive(Clone, Default)]
pub struct AppState {
    inner: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    rooms: Mutex<Rooms>,
    next_conn: AtomicU64,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Never held across an `.await`, so a std `Mutex` is enough. If a panic
    /// ever poisoned it we keep serving: the room maps stay consistent
    /// because every method finishes its edits before it can panic.
    fn rooms(&self) -> MutexGuard<'_, Rooms> {
        self.inner.rooms.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn handle_msg(&self, conn: ConnId, outbox: &Outbox, msg: ClientMsg) -> Result<(), String> {
        let now = Instant::now();
        let mut rooms = self.rooms();
        match msg {
            ClientMsg::CreateRoom => {
                let code = rooms
                    .create_room(conn, outbox.clone(), now)
                    .map_err(|e| format!("CreateRoom: {e:?}"))?;
                info!(conn, %code, "room created");
            }
            ClientMsg::JoinRoom { code } => {
                rooms
                    .join_room(conn, outbox.clone(), &code, now)
                    .map_err(|e| format!("JoinRoom {code}: {e:?}"))?;
                info!(conn, %code, "match started");
            }
            ClientMsg::Relay(bytes) => rooms.relay(conn, bytes, now),
        }
        Ok(())
    }
}

pub fn app(state: AppState) -> Router {
    app_with_web(state, None)
}

/// The relay, plus the browser build if `web_dir` is given (see `web.rs`).
pub fn app_with_web(state: AppState, web_dir: Option<PathBuf>) -> Router {
    let router = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);
    match web_dir {
        // "/" is the game page, which also answers a host's health check.
        Some(dir) => router.merge(web::routes(dir)),
        // Hosting platforms poll a plain HTTP route to see the server is up.
        None => router.route("/", get(|| async { "riposte-server ok" })),
    }
}

/// Deletes idle rooms every `interval` until the process exits.
pub fn spawn_idle_sweeper(state: AppState, interval: Duration, max_idle: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            let removed = state.rooms().sweep_idle(Instant::now(), max_idle);
            if removed > 0 {
                info!(removed, "deleted idle rooms");
            }
        }
    });
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, state))
}

/// One task per connection: reads client messages and writes whatever the
/// room logic puts in this connection's outbox.
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let conn = state.inner.next_conn.fetch_add(1, Ordering::Relaxed);
    let (outbox, mut inbox) = mpsc::unbounded_channel::<ServerMsg>();
    info!(conn, "connected");

    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Binary(data))) => match bincode::deserialize::<ClientMsg>(&data) {
                    Ok(msg) => {
                        if let Err(why) = state.handle_msg(conn, &outbox, msg) {
                            warn!(conn, %why, "protocol error, closing");
                            break;
                        }
                    }
                    Err(err) => {
                        warn!(conn, %err, "undecodable message, closing");
                        break;
                    }
                },
                Some(Ok(Message::Close(_))) | None => break,
                // Text is not part of the protocol. Pings are answered by
                // axum itself, so they and pongs need nothing here.
                Some(Ok(_)) => {}
                Some(Err(err)) => {
                    debug!(conn, %err, "socket error");
                    break;
                }
            },
            outgoing = inbox.recv() => {
                // `outbox` lives in this function, so the channel never closes.
                let Some(msg) = outgoing else { break };
                let Ok(bytes) = bincode::serialize(&msg) else { break };
                if socket.send(Message::Binary(bytes.into())).await.is_err() {
                    break;
                }
                // The room is gone after PeerLeft; this connection is done.
                if msg == ServerMsg::PeerLeft {
                    break;
                }
            }
        }
    }

    state.rooms().leave(conn);
    let _ = socket.send(Message::Close(None)).await;
    info!(conn, "disconnected");
}
