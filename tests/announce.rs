#[macro_use]
mod common;

use common::{CrabtermProcess, find_available_port, wait_for_port};
use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

fn empty_config() -> PathBuf {
    let config_dir =
        std::env::temp_dir().join(format!("crabterm_test_config_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&config_dir);
    let config_path = config_dir.join(".crabterm_empty");
    std::fs::write(&config_path, "").unwrap();
    config_path
}

#[tokio::test]
async fn test_client_receives_device_not_connected_hint() {
    let crabterm_port = find_available_port().await;
    let config = empty_config();

    // Start crabterm with a non-existent device
    let mut crabterm = CrabtermProcess::builder()
        .device("/dev/non_existent_device_12345")
        .listen(crabterm_port)
        .no_announce(false)
        .config(config.clone())
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
    let _ = std::fs::remove_file(config);
}

#[tokio::test]
async fn test_client_receives_device_connected_hint() {
    let crabterm_port = find_available_port().await;
    let config = empty_config();

    let mut crabterm = CrabtermProcess::builder()
        .echo_device()
        .listen(crabterm_port)
        .no_announce(false)
        .config(config.clone())
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
    let _ = std::fs::remove_file(config);
}

#[tokio::test]
async fn test_late_connecting_client_receives_last_error() {
    let crabterm_port = find_available_port().await;
    let config = empty_config();

    let mut crabterm = CrabtermProcess::builder()
        .device("/dev/non_existent_device_late")
        .listen(crabterm_port)
        .no_announce(false)
        .config(config.clone())
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
    let _ = std::fs::remove_file(config);
}

#[tokio::test]
async fn test_custom_template() {
    let crabterm_port = find_available_port().await;
    let config = empty_config();

    let mut crabterm = CrabtermProcess::builder()
        .echo_device()
        .listen(crabterm_port)
        .no_announce(false)
        .config(config.clone())
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
    let _ = std::fs::remove_file(config);
}

#[tokio::test]
async fn test_template_without_newline() {
    let crabterm_port = find_available_port().await;

    // Use a custom config file with a template that has NO newline
    let config_dir =
        std::env::temp_dir().join(format!("crabterm_test_config_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&config_dir);
    let config_path = config_dir.join(".crabterm_no_nl");
    std::fs::write(&config_path, "set announce-template \"{%s: %m}\"").unwrap();

    // Spawn manually to use -c
    let log_file =
        std::env::temp_dir().join(format!("crabterm_test_template_{}.log", std::process::id()));
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_crabterm"));
    cmd.arg("echo")
        .arg("-p")
        .arg(crabterm_port.to_string())
        .arg("-c")
        .arg(&config_path)
        .arg("--log-file")
        .arg(&log_file)
        .arg("--headless");

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn");

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

    // We expect the first announcement: "{127.0.0.1:port: Echo: Connected}"
    let n = client.read(&mut buf).expect("Failed to read");
    let received = String::from_utf8_lossy(&buf[..n]);
    tprintln!("First received: {:?}", received);
    assert!(
        !received.contains('\n'),
        "First output should not contain newline"
    );
    assert!(
        !received.contains('\r'),
        "First output should not contain carriage return"
    );

    let _ = child.kill();
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_file(&log_file);
}
