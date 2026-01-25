use serial_test::serial;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

/// Helper to find an available port
async fn find_available_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

/// Spawn crabterm connecting to a device and optionally listening on a port
fn spawn_crabterm(device_addr: &str, listen_port: Option<u16>) -> Child {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_crabterm"));
    cmd.arg(device_addr);

    if let Some(port) = listen_port {
        cmd.arg("-p").arg(port.to_string());
    }

    cmd.arg("--headless")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn crabterm")
}

/// Wait for a TCP port to become available
async fn wait_for_port(port: u16, timeout_ms: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    while tokio::time::Instant::now() < deadline {
        if TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
#[serial]
async fn test_tcp_connects_to_server() {
    // Start a simple TCP server (the "device")
    let device_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let device_port = device_listener.local_addr().unwrap().port();

    // Start crabterm connecting to our server, with its own listening port
    let crabterm_port = find_available_port().await;
    let mut crabterm = spawn_crabterm(&format!("127.0.0.1:{}", device_port), Some(crabterm_port));

    // Accept crabterm's connection to our "device"
    let accept_result = timeout(Duration::from_secs(2), device_listener.accept()).await;
    assert!(accept_result.is_ok(), "Crabterm should connect to device");

    let (mut device_socket, _) = accept_result.unwrap().unwrap();

    // Wait for crabterm's server to be ready
    assert!(
        wait_for_port(crabterm_port, 2000).await,
        "Crabterm server should start"
    );

    // Connect a client to crabterm
    let mut client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).unwrap();
    client.set_nonblocking(false).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    // Send data from client -> crabterm -> device
    client.write_all(b"hello").unwrap();

    // Read on device side
    let mut buf = [0u8; 32];
    let n = timeout(Duration::from_secs(2), device_socket.read(&mut buf))
        .await
        .expect("Timeout reading from device")
        .expect("Read error");
    assert_eq!(&buf[..n], b"hello", "Device should receive client data");

    // Send data from device -> crabterm -> client
    device_socket.write_all(b"world").await.unwrap();

    // Read on client side
    let n = client.read(&mut buf).expect("Client read failed");
    // Note: client output may include connection info messages
    let received = String::from_utf8_lossy(&buf[..n]);
    assert!(
        received.contains("world"),
        "Client should receive device data, got: {}",
        received
    );

    crabterm.kill().ok();
}

#[tokio::test]
#[serial]
async fn test_tcp_reconnects_after_server_disconnect() {
    // Start initial TCP server (the "device")
    let device_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let device_port = device_listener.local_addr().unwrap().port();
    let device_addr = format!("127.0.0.1:{}", device_port);

    // Start crabterm
    let crabterm_port = find_available_port().await;
    let mut crabterm = spawn_crabterm(&device_addr, Some(crabterm_port));

    // Accept first connection
    let (mut device_socket, _) = timeout(Duration::from_secs(2), device_listener.accept())
        .await
        .expect("Timeout waiting for crabterm connection")
        .expect("Accept failed");

    // Wait for crabterm's server
    assert!(
        wait_for_port(crabterm_port, 2000).await,
        "Crabterm server should start"
    );

    // Verify initial connection works
    let mut client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).unwrap();
    client.set_nonblocking(false).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    client.write_all(b"test1").unwrap();
    let mut buf = [0u8; 32];
    let n = timeout(Duration::from_secs(2), device_socket.read(&mut buf))
        .await
        .expect("Timeout")
        .expect("Read error");
    assert_eq!(&buf[..n], b"test1");

    // Now disconnect the device (close the socket)
    drop(device_socket);
    // Also drop the listener to simulate server going away
    drop(device_listener);

    // Give crabterm time to detect disconnection (needs to attempt read/write)
    // The hub polls every 100ms, so we need at least a couple of ticks
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Trigger crabterm to notice disconnect by sending data through client
    // This causes crabterm to try writing to the dead device socket
    let _ = client.write_all(b"trigger");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Start a new server on the SAME port
    let device_listener2 = TcpListener::bind(&device_addr).await.unwrap();

    // Crabterm should reconnect (give it more time - reconnect happens on 100ms ticks)
    let reconnect_result = timeout(Duration::from_secs(10), device_listener2.accept()).await;

    assert!(
        reconnect_result.is_ok(),
        "Crabterm should reconnect after server restart"
    );

    let (mut device_socket2, _) = reconnect_result.unwrap().unwrap();

    // Give crabterm a moment to stabilize after reconnection
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect the client too (old connection may be stale)
    drop(client);
    let mut client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).unwrap();
    client.set_nonblocking(false).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    // Verify data flows again
    client.write_all(b"test2").unwrap();
    let n = timeout(Duration::from_secs(2), device_socket2.read(&mut buf))
        .await
        .expect("Timeout on reconnected socket")
        .expect("Read error");
    assert_eq!(&buf[..n], b"test2", "Data should flow after reconnection");

    crabterm.kill().ok();
}

#[tokio::test]
#[serial]
async fn test_tcp_handles_connection_refused() {
    // Pick a port with nothing listening
    let unused_port = find_available_port().await;

    // Start crabterm trying to connect to nothing
    let crabterm_port = find_available_port().await;
    let mut crabterm = spawn_crabterm(&format!("127.0.0.1:{}", unused_port), Some(crabterm_port));

    // Crabterm's server should still start even if device connection fails
    assert!(
        wait_for_port(crabterm_port, 3000).await,
        "Crabterm server should start even without device"
    );

    // Now start a server on that port - crabterm should connect
    let device_listener = TcpListener::bind(format!("127.0.0.1:{}", unused_port))
        .await
        .unwrap();

    let accept_result = timeout(Duration::from_secs(5), device_listener.accept()).await;
    assert!(
        accept_result.is_ok(),
        "Crabterm should eventually connect when server becomes available"
    );

    crabterm.kill().ok();
}
