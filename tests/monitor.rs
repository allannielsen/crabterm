#[macro_use]
mod common;

use common::{find_available_port, wait_for_port};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

#[tokio::test]
async fn test_device_monitor_basic() {
    let crabterm_port = find_available_port().await;
    let monitor_port = find_available_port().await;

    let log_file =
        std::env::temp_dir().join(format!("crabterm_monitor_test_{}.log", std::process::id()));
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_crabterm"));
    cmd.arg("echo")
        .arg("-p")
        .arg(crabterm_port.to_string())
        .arg("--device-monitor-port")
        .arg(monitor_port.to_string())
        .arg("--device-monitor-template")
        .arg("%d: %m\n")
        .arg("--log-file")
        .arg(&log_file)
        .arg("--headless");

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn");

    assert!(
        wait_for_port(crabterm_port, 2000).await,
        "Terminal port should start"
    );
    assert!(
        wait_for_port(monitor_port, 2000).await,
        "Monitor port should start"
    );

    // Connect monitor
    let mut monitor = TcpStream::connect(format!("127.0.0.1:{}", monitor_port))
        .expect("Failed to connect monitor");
    monitor
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();

    // Connect client
    let mut client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port))
        .expect("Failed to connect client");
    client
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();

    // Skip announcements on client
    let mut buf = [0u8; 1024];
    let _ = client.read(&mut buf).unwrap();

    // Send "hi\n" from client to device (TX)
    client.write_all(b"hi\n").unwrap();
    client.flush().unwrap();

    let mut monitor_received = String::new();
    let start = Instant::now();

    // Using template: "%d: %m\n"
    let expected = ["TX: hi\\n\n", "RX: hi\\n\n"];

    while start.elapsed() < Duration::from_secs(2) {
        let n = match monitor.read(&mut buf) {
            Ok(n) => n,
            Err(_) => 0,
        };
        if n > 0 {
            monitor_received.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        if expected.iter().all(|s| monitor_received.contains(s)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    tprintln!("Monitor received: {:?}", monitor_received);

    for s in expected.iter() {
        assert!(
            monitor_received.contains(s),
            "Should contain {:?}. Got: {:?}",
            s,
            monitor_received
        );
    }

    let _ = child.kill();
    let _ = std::fs::remove_file(&log_file);
}

#[tokio::test]
async fn test_device_monitor_escaping() {
    let crabterm_port = find_available_port().await;
    let monitor_port = find_available_port().await;

    let log_file = std::env::temp_dir().join(format!(
        "crabterm_monitor_esc_test_{}.log",
        std::process::id()
    ));
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_crabterm"));
    cmd.arg("echo")
        .arg("-p")
        .arg(crabterm_port.to_string())
        .arg("--device-monitor-port")
        .arg(monitor_port.to_string())
        .arg("--device-monitor-template")
        .arg("%d: %m\n")
        .arg("--log-file")
        .arg(&log_file)
        .arg("--headless");

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn");

    assert!(wait_for_port(crabterm_port, 2000).await);
    assert!(wait_for_port(monitor_port, 2000).await);

    let mut monitor = TcpStream::connect(format!("127.0.0.1:{}", monitor_port)).unwrap();
    monitor
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();

    let mut client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).unwrap();

    // Send non-printable chars
    client.write_all(&[0x01, b'\r', b'\t', b'\\']).unwrap();

    let mut monitor_received = String::new();
    let mut buf = [0u8; 1024];
    let start = Instant::now();
    let expected = ["TX: \\x01\\r\\t\\\\", "RX: \\x01\\r\\t\\\\"];

    while start.elapsed() < Duration::from_secs(2) {
        let n = match monitor.read(&mut buf) {
            Ok(n) => n,
            Err(_) => 0,
        };
        if n > 0 {
            monitor_received.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        if expected.iter().all(|s| monitor_received.contains(s)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    tprintln!("Monitor received esc: {:?}", monitor_received);

    for s in expected.iter() {
        assert!(
            monitor_received.contains(s),
            "Should contain {:?}. Got: {:?}",
            s,
            monitor_received
        );
    }

    let _ = child.kill();
    let _ = std::fs::remove_file(&log_file);
}
