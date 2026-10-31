//! End-to-end: a real server on a real socket, driven by real WebSocket
//! clients. Checks the wire format and the connection lifecycle that the
//! unit tests in `rooms.rs` cannot see.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use riposte_server::protocol::{ClientMsg, ServerMsg};
use riposte_server::{app, spawn_idle_sweeper, AppState};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Starts a server on a free port and returns its address.
async fn start_server(idle: Option<(Duration, Duration)>) -> String {
    let state = AppState::new();
    if let Some((interval, max_idle)) = idle {
        spawn_idle_sweeper(state.clone(), interval, max_idle);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app(state)).await.unwrap() });
    format!("ws://{addr}/ws")
}

async fn connect(url: &str) -> Client {
    tokio_tungstenite::connect_async(url).await.unwrap().0
}

async fn send(client: &mut Client, msg: ClientMsg) {
    let bytes = bincode::serialize(&msg).unwrap();
    client.send(Message::Binary(bytes.into())).await.unwrap();
}

/// Next server message, or panics if nothing arrives or the socket ended.
async fn recv(client: &mut Client) -> ServerMsg {
    let next = tokio::time::timeout(Duration::from_secs(5), client.next())
        .await
        .expect("timed out waiting for the server");
    match next {
        Some(Ok(Message::Binary(data))) => bincode::deserialize(&data).unwrap(),
        other => panic!("expected a binary message, got {other:?}"),
    }
}

/// True once the server has closed this connection.
async fn is_closed(client: &mut Client) -> bool {
    loop {
        let next = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .expect("timed out waiting for the close");
        match next {
            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return true,
            Some(Ok(_)) => {}
        }
    }
}

async fn paired_clients(url: &str) -> (Client, Client) {
    let mut host = connect(url).await;
    send(&mut host, ClientMsg::CreateRoom).await;
    let ServerMsg::RoomCreated { code } = recv(&mut host).await else {
        panic!("expected RoomCreated");
    };
    let mut guest = connect(url).await;
    send(&mut guest, ClientMsg::JoinRoom { code }).await;
    assert!(matches!(
        recv(&mut host).await,
        ServerMsg::MatchStart {
            player_index: 0,
            ..
        }
    ));
    assert!(matches!(
        recv(&mut guest).await,
        ServerMsg::MatchStart {
            player_index: 1,
            ..
        }
    ));
    (host, guest)
}

#[tokio::test]
async fn two_players_pair_and_exchange_bytes() {
    let url = start_server(None).await;
    let (mut host, mut guest) = paired_clients(&url).await;

    send(&mut host, ClientMsg::Relay(vec![1, 2, 3])).await;
    assert_eq!(recv(&mut guest).await, ServerMsg::Relay(vec![1, 2, 3]));
    send(&mut guest, ClientMsg::Relay(vec![4, 5])).await;
    assert_eq!(recv(&mut host).await, ServerMsg::Relay(vec![4, 5]));
}

#[tokio::test]
async fn disconnect_sends_peer_left_and_closes_the_other_connection() {
    let url = start_server(None).await;
    let (mut host, mut guest) = paired_clients(&url).await;

    drop(host.close(None).await);
    assert_eq!(recv(&mut guest).await, ServerMsg::PeerLeft);
    assert!(is_closed(&mut guest).await);
}

#[tokio::test]
async fn joining_an_unknown_room_closes_the_connection() {
    let url = start_server(None).await;
    let mut client = connect(&url).await;
    send(
        &mut client,
        ClientMsg::JoinRoom {
            code: "ZZZZ".into(),
        },
    )
    .await;
    assert!(is_closed(&mut client).await);
}

#[tokio::test]
async fn garbage_bytes_close_the_connection() {
    let url = start_server(None).await;
    let mut client = connect(&url).await;
    client
        .send(Message::Binary(vec![0xFF; 32].into()))
        .await
        .unwrap();
    assert!(is_closed(&mut client).await);
}

#[tokio::test]
async fn oversized_messages_close_the_connection() {
    let url = start_server(None).await;
    let (mut host, _guest) = paired_clients(&url).await;
    send(&mut host, ClientMsg::Relay(vec![0; 64 * 1024])).await;
    assert!(is_closed(&mut host).await);
}

#[tokio::test]
async fn idle_rooms_are_deleted() {
    let url = start_server(Some((
        Duration::from_millis(50),
        Duration::from_millis(200),
    )))
    .await;
    let mut host = connect(&url).await;
    send(&mut host, ClientMsg::CreateRoom).await;
    let ServerMsg::RoomCreated { code } = recv(&mut host).await else {
        panic!("expected RoomCreated");
    };

    // Nobody joins and nothing is sent, so the sweeper closes the room.
    assert_eq!(recv(&mut host).await, ServerMsg::PeerLeft);
    assert!(is_closed(&mut host).await);

    let mut late = connect(&url).await;
    send(&mut late, ClientMsg::JoinRoom { code }).await;
    assert!(is_closed(&mut late).await);
}
