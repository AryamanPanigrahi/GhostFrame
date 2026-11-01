//! The relay serving the browser build, over a real socket. A raw TCP
//! request keeps this free of any HTTP client dependency.

use std::path::PathBuf;

use riposte_server::{app, app_with_web, AppState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn serve(router: axum::Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

/// A minimal HTTP/1.1 GET. Returns (status line, lowercase headers, body).
async fn get(addr: std::net::SocketAddr, path: &str) -> (String, String, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status = head.lines().next().unwrap().to_owned();
    (status, head.to_lowercase(), raw[split + 4..].to_vec())
}

fn web_folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("riposte-web-test-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<html>game</html>").unwrap();
    std::fs::write(dir.join("riposte-game.wasm"), [0, b'a', b's', b'm']).unwrap();
    std::fs::write(dir.join("mq_js_bundle.js"), "load()").unwrap();
    std::fs::write(dir.join("secret.txt"), "nope").unwrap();
    dir
}

#[tokio::test]
async fn serves_the_three_files_with_the_right_content_types() {
    let addr = serve(app_with_web(AppState::new(), Some(web_folder("files")))).await;

    let (status, headers, body) = get(addr, "/").await;
    assert!(status.contains("200"));
    assert!(headers.contains("text/html"));
    assert_eq!(body, b"<html>game</html>");

    let (_, headers, body) = get(addr, "/riposte-game.wasm").await;
    assert!(headers.contains("application/wasm"), "{headers}");
    assert_eq!(body, [0, b'a', b's', b'm']);

    let (_, headers, _) = get(addr, "/mq_js_bundle.js").await;
    assert!(headers.contains("javascript"));
}

#[tokio::test]
async fn other_files_in_the_folder_are_not_served() {
    let addr = serve(app_with_web(AppState::new(), Some(web_folder("secret")))).await;
    for path in ["/secret.txt", "/../secret.txt", "/%2e%2e/secret.txt"] {
        let (status, _, _) = get(addr, path).await;
        assert!(status.contains("404"), "{path}: {status}");
    }
}

#[tokio::test]
async fn missing_file_is_a_404_not_a_crash() {
    let empty = std::env::temp_dir().join(format!("riposte-web-empty-{}", std::process::id()));
    std::fs::create_dir_all(&empty).unwrap();
    let addr = serve(app_with_web(AppState::new(), Some(empty))).await;
    let (status, _, _) = get(addr, "/riposte-game.wasm").await;
    assert!(status.contains("404"));
}

#[tokio::test]
async fn without_a_web_folder_root_is_the_health_check() {
    let addr = serve(app(AppState::new())).await;
    let (status, _, body) = get(addr, "/").await;
    assert!(status.contains("200"));
    assert_eq!(body, b"riposte-server ok");
    let (status, _, _) = get(addr, "/riposte-game.wasm").await;
    assert!(status.contains("404"));
}
