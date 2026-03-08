#[macro_use]
mod common;

use common::{CrabtermProcess, find_available_port, wait_for_port};
use std::io::Read;
use std::net::TcpStream;
use std::time::Duration;

#[tokio::test]
async fn test_client_receives_device_not_connected_hint() {
    let crabterm_port = find_available_port().await;

    // Start crabterm with a non-existent device
    let mut crabterm = CrabtermProcess::builder()
        .device("/dev/non_existent_device_12345")
        .listen(crabterm_port)
        .no_announce(false)
        .spawn();

    assert!(
        wait_for_port(crabterm_port, 2000).await,
        "Crabterm server should start"
    );

    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port))
        .expect("Failed to connect to crabterm");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).expect("Failed to read from client");
    let received = String::from_utf8_lossy(&buf[..n]);

    tprintln!("Received: {}", received);

    // Default template: MSG-%s: %t %m\r\n
    assert!(
        received.contains("MSG-127.0.0.1:"),
        "Announcement should have MSG-IP:PORT prefix. Got: {}",
        received
    );
    // Check for HH:MM:SS timestamp format
    let has_timestamp = received.chars().filter(|&c| c == ':').count() >= 4;
    assert!(
        has_timestamp,
        "Announcement should contain a timestamp (HH:MM:SS). Got: {}",
        received
    );
    assert!(
        received.contains("Not connected") || received.contains("No such file"),
        "Client should receive hint that device is not connected. Got: {}",
        received
    );
    assert!(
        received.ends_with("\r\n"),
        "Announcement should end with \\r\\n. Got: {:?}",
        received
    );

    crabterm.stop();
}

#[tokio::test]
async fn test_client_receives_device_connected_hint() {
    let crabterm_port = find_available_port().await;

    let mut crabterm = CrabtermProcess::builder()
        .echo_device()
        .listen(crabterm_port)
        .no_announce(false)
        .spawn();

    assert!(
        wait_for_port(crabterm_port, 2000).await,
        "Crabterm server should start"
    );

    let mut client =
        TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).expect("Failed to connect");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).expect("Failed to read");
    let received = String::from_utf8_lossy(&buf[..n]);

    tprintln!("Received: {}", received);

    assert!(
        received.contains("MSG-127.0.0.1:"),
        "Announcement should have MSG-IP:PORT prefix. Got: {}",
        received
    );
    assert!(
        received.to_lowercase().contains("echo: connected"),
        "Client should receive hint that device is connected. Got: {}",
        received
    );
    assert!(
        received.ends_with("\r\n"),
        "Announcement should end with \\r\\n. Got: {:?}",
        received
    );

    crabterm.stop();
}

#[tokio::test]
async fn test_late_connecting_client_receives_last_error() {
    let crabterm_port = find_available_port().await;

    let mut crabterm = CrabtermProcess::builder()
        .device("/dev/non_existent_device_late")
        .listen(crabterm_port)
        .no_announce(false)
        .spawn();

    assert!(
        wait_for_port(crabterm_port, 2000).await,
        "Crabterm server should start"
    );

    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut client =
        TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).expect("Failed to connect");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).expect("Failed to read");
    let received = String::from_utf8_lossy(&buf[..n]);

    tprintln!("Late client received: {}", received);

    assert!(
        received.contains("MSG-127.0.0.1:"),
        "Announcement should have MSG-IP:PORT prefix. Got: {}",
        received
    );
    assert!(
        received.contains("No such file"),
        "Late client should receive the actual device error. Got: {}",
        received
    );
    assert!(
        received.ends_with("\r\n"),
        "Announcement should end with \\r\\n. Got: {:?}",
        received
    );

    crabterm.stop();
}

#[tokio::test]
async fn test_custom_template() {
    let crabterm_port = find_available_port().await;

    let mut crabterm = CrabtermProcess::builder()
        .echo_device()
        .listen(crabterm_port)
        .no_announce(false)
        .spawn();

    assert!(
        wait_for_port(crabterm_port, 2000).await,
        "Crabterm server should start"
    );

    let mut client =
        TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).expect("Failed to connect");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).expect("Failed to read");
    let received = String::from_utf8_lossy(&buf[..n]);

    tprintln!("Received: {}", received);

    // Default template: MSG-%s: %t %m\r\n
    assert!(
        received.starts_with("MSG-127.0.0.1:"),
        "Should start with default template MSG- and origin. Got: {}",
        received
    );
    assert!(
        received.ends_with("\r\n"),
        "Announcement should end with \\r\\n. Got: {:?}",
        received
    );

    crabterm.stop();
}
