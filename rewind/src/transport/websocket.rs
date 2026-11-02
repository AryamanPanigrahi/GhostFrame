//! A `Transport` that talks to the relay server over a WebSocket (R6).
//!
//! Peers never connect to each other directly. Each one opens a WebSocket to
//! the relay, asks it to create or join a room, and from then on every
//! `PeerMsg` we `send` is wrapped as `ClientMsg::Relay(bytes)` and the relay
//! forwards the bytes to the other player as `ServerMsg::Relay(bytes)`.
//!
//! The socket differs by platform behind the small `ByteSocket` trait:
//! `tungstenite` on desktop, quad-net's JavaScript `WebSocket` in the
//! browser. Both are non-blocking to use: `try_recv` returns immediately,
//! which is what `Transport::poll` needs.
//!
//! The room handshake happens on the same connection, so besides the
//! `Transport` methods there is a small lobby API: `create_room`,
//! `join_room` and `lobby_events`.

use serde::{Deserialize, Serialize};

use crate::api::Transport;

/// What this transport needs from a socket. `quad_net`'s `WebSocket`
/// implements it; tests can swap in a fake, so most of the logic is tested
/// without any network.
pub trait ByteSocket {
    /// Queue one binary frame. Must only be called once `is_open` is true.
    fn send_bytes(&mut self, data: &[u8]);
    /// The next received frame, if one has arrived. Never blocks.
    fn try_recv(&mut self) -> Option<Vec<u8>>;
    /// False while a browser WebSocket is still connecting. Sending before
    /// that point throws in JavaScript, so we hold messages back.
    fn is_open(&self) -> bool;
    /// True once a connection that was open has closed or failed. Nothing
    /// sent after that arrives, so the game must be told. Fakes that never
    /// close can keep the default.
    fn is_closed(&self) -> bool {
        false
    }
}

/// The browser socket: JavaScript's `WebSocket`, through quad-net.
#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::Cell;

    use super::ByteSocket;

    pub struct BrowserSocket {
        ws: quad_net::web_socket::WebSocket,
        /// Seen open at least once. `connected()` is false both while
        /// connecting and after closing; this tells the two apart.
        was_open: Cell<bool>,
    }

    impl BrowserSocket {
        pub fn connect(url: &str) -> Result<BrowserSocket, String> {
            quad_net::web_socket::WebSocket::connect(url)
                .map(|ws| BrowserSocket {
                    ws,
                    was_open: Cell::new(false),
                })
                .map_err(|e| format!("{e:?}"))
        }
    }

    impl ByteSocket for BrowserSocket {
        fn send_bytes(&mut self, data: &[u8]) {
            self.ws.send_bytes(data);
        }
        fn try_recv(&mut self) -> Option<Vec<u8>> {
            self.ws.try_recv()
        }
        fn is_open(&self) -> bool {
            let open = self.ws.connected();
            if open {
                self.was_open.set(true);
            }
            open
        }
        fn is_closed(&self) -> bool {
            !self.is_open() && self.was_open.get()
        }
    }
}

/// The desktop socket: tungstenite, a plain blocking WebSocket client that
/// we switch to non-blocking once connected, so `try_recv` returns at once.
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::ErrorKind;

    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Error, Message, WebSocket};

    use super::ByteSocket;

    pub struct NativeSocket {
        ws: WebSocket<MaybeTlsStream<std::net::TcpStream>>,
        closed: bool,
    }

    impl NativeSocket {
        /// Blocks until the connection and the WebSocket handshake are done.
        pub fn connect(url: &str) -> Result<NativeSocket, String> {
            let (ws, _response) = tungstenite::connect(url).map_err(|e| e.to_string())?;
            // The handshake needs blocking reads; after it, never block the
            // game loop waiting for the network. For `wss://` the TCP socket
            // sits under the TLS layer, which passes `WouldBlock` through.
            let tcp = match ws.get_ref() {
                MaybeTlsStream::Plain(stream) => stream,
                MaybeTlsStream::Rustls(tls) => tls.get_ref(),
                _ => return Err("unsupported stream type".to_string()),
            };
            tcp.set_nonblocking(true).map_err(|e| e.to_string())?;
            Ok(NativeSocket { ws, closed: false })
        }
    }

    impl ByteSocket for NativeSocket {
        fn send_bytes(&mut self, data: &[u8]) {
            if self.closed {
                return;
            }
            match self.ws.send(Message::Binary(data.to_vec().into())) {
                Ok(()) => {}
                // The frame is queued inside tungstenite and is written out
                // by a later `flush` (see `try_recv`).
                Err(Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {}
                Err(_) => self.closed = true,
            }
        }

        fn try_recv(&mut self) -> Option<Vec<u8>> {
            if self.closed {
                return None;
            }
            loop {
                match self.ws.read() {
                    Ok(Message::Binary(bytes)) => return Some(bytes.to_vec()),
                    // The relay only speaks binary; pings are answered by
                    // tungstenite itself on the next read/flush.
                    Ok(Message::Close(_)) => {
                        self.closed = true;
                        return None;
                    }
                    Ok(_) => continue,
                    Err(Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                        // Nothing to read: use the moment to push out any
                        // frame that couldn't be fully written earlier.
                        if let Err(Error::Io(e)) = self.ws.flush() {
                            if e.kind() != ErrorKind::WouldBlock {
                                self.closed = true;
                            }
                        }
                        return None;
                    }
                    Err(_) => {
                        self.closed = true;
                        return None;
                    }
                }
            }
        }

        fn is_open(&self) -> bool {
            true
        }

        fn is_closed(&self) -> bool {
            self.closed
        }
    }
}

/// The socket `WebSocketTransport::connect` uses on this platform. Public
/// so the game can name `Lobby<DefaultSocket>` in its screen enum (I1).
#[cfg(target_arch = "wasm32")]
pub type DefaultSocket = browser::BrowserSocket;
#[cfg(not(target_arch = "wasm32"))]
pub type DefaultSocket = native::NativeSocket;

/// Client to relay messages. Must stay identical to `ClientMsg` in
/// `server/src/protocol.rs`: bincode encodes a variant by its position, so
/// order matters. A test in `server/tests/` checks the two agree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMsg {
    CreateRoom,
    JoinRoom { code: String },
    Relay(Vec<u8>),
}

/// Relay to client messages. Must stay identical to `ServerMsg` in
/// `server/src/protocol.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMsg {
    RoomCreated { code: String },
    MatchStart { player_index: u8, seed: u64 },
    Relay(Vec<u8>),
    PeerLeft,
}

/// What the relay told us about the room, as opposed to relayed game
/// messages. The game's menu reads these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyEvent {
    /// We created a room; show this code to the friend.
    RoomCreated { code: String },
    /// Both players are in. `player_index` is 0 or 1 and `seed` starts the
    /// game's PRNG, so both machines begin identically.
    MatchStart { player_index: u8, seed: u64 },
    /// The other player left or disconnected.
    PeerLeft,
    /// Our own connection to the relay closed (the relay stopped, or the
    /// network dropped). Reported once; nothing sent afterwards arrives.
    ConnectionLost,
}

/// Why a connection could not be opened.
#[derive(Debug)]
pub struct ConnectError(pub String);

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "could not connect to the relay: {}", self.0)
    }
}

impl std::error::Error for ConnectError {}

pub struct WebSocketTransport<S: ByteSocket = DefaultSocket> {
    socket: S,
    /// Encoded `ClientMsg`s not yet handed to the socket, because it isn't
    /// open yet. Sent in order as soon as it opens.
    unsent: Vec<Vec<u8>>,
    /// Relayed `PeerMsg` bytes waiting for `Transport::poll`.
    relayed: Vec<Vec<u8>>,
    /// Room news waiting for `lobby_events`.
    lobby: Vec<LobbyEvent>,
    /// `ConnectionLost` has been queued, so it isn't queued again.
    lost: bool,
}

impl WebSocketTransport {
    /// Open a WebSocket to the relay, e.g. `ws://localhost:8080/ws`, or
    /// `wss://example.com/ws` for a relay behind HTTPS (desktop uses rustls;
    /// the browser handles TLS itself).
    ///
    /// On native this blocks until the connection is open. In the browser it
    /// returns at once and the socket opens in the background; messages sent
    /// meanwhile are held back until then.
    pub fn connect(url: &str) -> Result<WebSocketTransport, ConnectError> {
        #[cfg(target_arch = "wasm32")]
        let socket = browser::BrowserSocket::connect(url);
        #[cfg(not(target_arch = "wasm32"))]
        let socket = native::NativeSocket::connect(url);
        socket
            .map(WebSocketTransport::with_socket)
            .map_err(ConnectError)
    }
}

impl<S: ByteSocket> WebSocketTransport<S> {
    /// Use an already-connected socket (tests pass a fake one).
    pub fn with_socket(socket: S) -> WebSocketTransport<S> {
        WebSocketTransport {
            socket,
            unsent: Vec::new(),
            relayed: Vec::new(),
            lobby: Vec::new(),
            lost: false,
        }
    }

    /// Ask the relay for a new room. Its code arrives as
    /// `LobbyEvent::RoomCreated`.
    pub fn create_room(&mut self) {
        self.send_client_msg(&ClientMsg::CreateRoom);
    }

    /// Join the room with this code. `LobbyEvent::MatchStart` follows if the
    /// room exists and has space.
    pub fn join_room(&mut self, code: &str) {
        self.send_client_msg(&ClientMsg::JoinRoom {
            code: code.to_string(),
        });
    }

    /// Room news received since the last call. Also moves messages along, so
    /// call it every frame while waiting in the menu.
    pub fn lobby_events(&mut self) -> Vec<LobbyEvent> {
        self.pump();
        std::mem::take(&mut self.lobby)
    }

    fn send_client_msg(&mut self, msg: &ClientMsg) {
        // Serializing strings, ints and byte vectors can't fail.
        let bytes = bincode::serialize(msg).expect("ClientMsg always serializes");
        self.unsent.push(bytes);
        self.flush();
    }

    /// Hand held-back messages to the socket once it is open.
    fn flush(&mut self) {
        if !self.socket.is_open() {
            return;
        }
        for bytes in self.unsent.drain(..) {
            self.socket.send_bytes(&bytes);
        }
    }

    /// Read everything the socket has received and sort it into relayed game
    /// bytes and lobby events.
    fn pump(&mut self) {
        self.flush();
        while let Some(bytes) = self.socket.try_recv() {
            // Anything that isn't a valid ServerMsg is ignored, like a
            // garbled PeerMsg in the session: the protocol resends.
            let Ok(msg) = bincode::deserialize::<ServerMsg>(&bytes) else {
                continue;
            };
            match msg {
                ServerMsg::Relay(payload) => self.relayed.push(payload),
                ServerMsg::RoomCreated { code } => {
                    self.lobby.push(LobbyEvent::RoomCreated { code })
                }
                ServerMsg::MatchStart { player_index, seed } => self
                    .lobby
                    .push(LobbyEvent::MatchStart { player_index, seed }),
                ServerMsg::PeerLeft => self.lobby.push(LobbyEvent::PeerLeft),
            }
        }
        // After the messages that arrived before the close, so a `PeerLeft`
        // the relay sent first is still seen first.
        if !self.lost && self.socket.is_closed() {
            self.lost = true;
            self.lobby.push(LobbyEvent::ConnectionLost);
        }
    }
}

impl<S: ByteSocket> Transport for WebSocketTransport<S> {
    fn send(&mut self, bytes: &[u8]) {
        self.send_client_msg(&ClientMsg::Relay(bytes.to_vec()));
    }

    fn poll(&mut self) -> Vec<Vec<u8>> {
        self.pump();
        std::mem::take(&mut self.relayed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    /// A socket that records what was sent and plays back what it's given.
    #[derive(Default)]
    struct FakeState {
        open: bool,
        closed: bool,
        sent: Vec<Vec<u8>>,
        incoming: VecDeque<Vec<u8>>,
    }

    #[derive(Clone, Default)]
    struct FakeSocket(Rc<RefCell<FakeState>>);

    impl ByteSocket for FakeSocket {
        fn send_bytes(&mut self, data: &[u8]) {
            assert!(self.0.borrow().open, "sent on a socket that isn't open");
            self.0.borrow_mut().sent.push(data.to_vec());
        }
        fn try_recv(&mut self) -> Option<Vec<u8>> {
            self.0.borrow_mut().incoming.pop_front()
        }
        fn is_open(&self) -> bool {
            self.0.borrow().open
        }
        fn is_closed(&self) -> bool {
            self.0.borrow().closed
        }
    }

    fn open_socket() -> FakeSocket {
        let socket = FakeSocket::default();
        socket.0.borrow_mut().open = true;
        socket
    }

    fn deliver(socket: &FakeSocket, msg: &ServerMsg) {
        let bytes = bincode::serialize(msg).unwrap();
        socket.0.borrow_mut().incoming.push_back(bytes);
    }

    fn sent_msgs(socket: &FakeSocket) -> Vec<ClientMsg> {
        socket
            .0
            .borrow()
            .sent
            .iter()
            .map(|b| bincode::deserialize(b).unwrap())
            .collect()
    }

    #[test]
    fn send_wraps_bytes_for_the_relay() {
        let socket = open_socket();
        let mut t = WebSocketTransport::with_socket(socket.clone());
        t.send(&[1, 2, 3]);
        assert_eq!(sent_msgs(&socket), vec![ClientMsg::Relay(vec![1, 2, 3])]);
    }

    #[test]
    fn poll_returns_relayed_bytes_in_order_and_drains() {
        let socket = open_socket();
        let mut t = WebSocketTransport::with_socket(socket.clone());
        deliver(&socket, &ServerMsg::Relay(vec![1]));
        deliver(&socket, &ServerMsg::Relay(vec![2, 2]));
        assert_eq!(t.poll(), vec![vec![1], vec![2, 2]]);
        assert!(t.poll().is_empty());
    }

    #[test]
    fn lobby_messages_stay_out_of_the_game_stream() {
        let socket = open_socket();
        let mut t = WebSocketTransport::with_socket(socket.clone());
        deliver(
            &socket,
            &ServerMsg::RoomCreated {
                code: "ABCD".into(),
            },
        );
        deliver(
            &socket,
            &ServerMsg::MatchStart {
                player_index: 1,
                seed: 9,
            },
        );
        deliver(&socket, &ServerMsg::Relay(vec![7]));
        deliver(&socket, &ServerMsg::PeerLeft);

        // The game polls first: it sees only its own bytes.
        assert_eq!(t.poll(), vec![vec![7]]);
        // The menu still gets the room news that arrived meanwhile.
        assert_eq!(
            t.lobby_events(),
            vec![
                LobbyEvent::RoomCreated {
                    code: "ABCD".into()
                },
                LobbyEvent::MatchStart {
                    player_index: 1,
                    seed: 9
                },
                LobbyEvent::PeerLeft,
            ]
        );
        assert!(t.lobby_events().is_empty());
    }

    #[test]
    fn a_closed_socket_is_reported_once_after_pending_messages() {
        let socket = open_socket();
        let mut t = WebSocketTransport::with_socket(socket.clone());
        assert!(t.lobby_events().is_empty());
        deliver(&socket, &ServerMsg::PeerLeft);
        socket.0.borrow_mut().closed = true;
        assert_eq!(
            t.lobby_events(),
            vec![LobbyEvent::PeerLeft, LobbyEvent::ConnectionLost]
        );
        assert!(t.lobby_events().is_empty());
        assert!(t.poll().is_empty());
        assert!(
            t.lobby_events().is_empty(),
            "polling doesn't report it again"
        );
    }

    #[test]
    fn create_and_join_send_the_right_messages() {
        let socket = open_socket();
        let mut t = WebSocketTransport::with_socket(socket.clone());
        t.create_room();
        t.join_room("WXYZ");
        assert_eq!(
            sent_msgs(&socket),
            vec![
                ClientMsg::CreateRoom,
                ClientMsg::JoinRoom {
                    code: "WXYZ".into()
                }
            ]
        );
    }

    #[test]
    fn messages_wait_until_the_socket_opens() {
        // A browser WebSocket is "connecting" for a moment after connect().
        let socket = FakeSocket::default();
        let mut t = WebSocketTransport::with_socket(socket.clone());
        t.create_room();
        t.send(&[5]);
        assert!(socket.0.borrow().sent.is_empty());

        socket.0.borrow_mut().open = true;
        t.poll();
        assert_eq!(
            sent_msgs(&socket),
            vec![ClientMsg::CreateRoom, ClientMsg::Relay(vec![5])]
        );
    }

    #[test]
    fn undecodable_frames_are_ignored() {
        let socket = open_socket();
        let mut t = WebSocketTransport::with_socket(socket.clone());
        socket.0.borrow_mut().incoming.push_back(vec![0xff, 0xfe]);
        deliver(&socket, &ServerMsg::Relay(vec![3]));
        assert_eq!(t.poll(), vec![vec![3]]);
    }

    #[test]
    fn connecting_to_nothing_is_an_error_not_a_panic() {
        // Port 1 on localhost: nothing listens there.
        assert!(WebSocketTransport::connect("ws://127.0.0.1:1/ws").is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn wss_urls_are_supported_on_desktop() {
        // Without a TLS feature tungstenite rejects `wss://` before even
        // connecting. With it, the attempt gets as far as the network and
        // fails only because nothing listens on port 1.
        let err = match WebSocketTransport::connect("wss://127.0.0.1:1/ws") {
            Ok(_) => panic!("nothing listens on port 1"),
            Err(e) => e.to_string(),
        };
        assert!(!err.contains("TLS support not compiled in"), "{err}");
    }
}
