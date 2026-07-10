#[macro_use]
mod common;

use common::{CrabtermProcess, LogLevel, find_available_port, wait_for_port};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

/// Unique temp path for a port file (not created; crabterm writes it).
fn temp_port_file() -> PathBuf {
    std::env::temp_dir().join(format!(
        "crabterm_rwport_{}_{}.port",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Parse a `pid=..\nport=..` port file. Returns None if missing/incomplete.
fn read_rw_port_file(path: &Path) -> Option<(u32, u16)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut pid = None;
    let mut port = None;
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("pid=") {
            pid = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("port=") {
            port = v.trim().parse().ok();
        }
    }
    Some((pid?, port?))
}

/// Poll until the port file exists and parses, or time out.
async fn wait_for_rw_port_file(path: &Path, timeout_ms: u64) -> (u32, u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    while tokio::time::Instant::now() < deadline {
        if let Some(v) = read_rw_port_file(path) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("Timed out waiting for RW port file {}", path.display());
}

/// A device listener + a crabterm exposing RW (random) and RO servers.
struct ReleaseHarness {
    _device_listener: TcpListener,
    device_socket: tokio::net::TcpStream,
    rw_port: u16,
    ro_port: u16,
    port_file: PathBuf,
    crabterm: CrabtermProcess,
}

impl ReleaseHarness {
    async fn start() -> Self {
        let device_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let device_port = device_listener.local_addr().unwrap().port();

        let ro_port = find_available_port().await;
        let port_file = temp_port_file();

        let crabterm = CrabtermProcess::builder()
            .device(&format!("127.0.0.1:{}", device_port))
            .rw_port(0) // random
            .rw_port_file(port_file.clone())
            .ro_port(ro_port)
            .log_level(LogLevel::Debug)
            .spawn();

        let (device_socket, _) = timeout(Duration::from_secs(2), device_listener.accept())
            .await
            .expect("Timeout waiting for crabterm to connect to device")
            .unwrap();

        let (_pid, rw_port) = wait_for_rw_port_file(&port_file, 2000).await;
        assert!(wait_for_port(rw_port, 2000).await, "RW server should start");
        assert!(wait_for_port(ro_port, 2000).await, "RO server should start");

        Self {
            _device_listener: device_listener,
            device_socket,
            rw_port,
            ro_port,
            port_file,
            crabterm,
        }
    }
}

fn connect(port: u16) -> TcpStream {
    let client = TcpStream::connect(format!("127.0.0.1:{}", port))
        .unwrap_or_else(|e| panic!("connect to {} failed: {}", port, e));
    client.set_nonblocking(false).unwrap();
    client
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    client
}

/// R (read-only isolation): RO client input is discarded and never reaches the
/// device, while RW client input is forwarded. Device output mirrors to both.
#[tokio::test]
async fn test_ro_input_is_discarded_rw_is_forwarded() {
    let ReleaseHarness {
        mut device_socket,
        rw_port,
        ro_port,
        mut crabterm,
        ..
    } = ReleaseHarness::start().await;

    let mut ro_client = connect(ro_port);
    let mut rw_client = connect(rw_port);
    // Give crabterm a moment to register both clients.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // RO input must NOT reach the device.
    ro_client.write_all(b"IGNORE_ME").unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    // RW input MUST reach the device.
    rw_client.write_all(b"FROMRW").unwrap();

    let mut buf = [0u8; 64];
    let n = timeout(Duration::from_secs(2), device_socket.read(&mut buf))
        .await
        .expect("Timeout reading from device")
        .expect("device read error");
    let got = String::from_utf8_lossy(&buf[..n]).to_string();
    assert_eq!(got, "FROMRW", "device should only receive RW input, got {:?}", got);

    // Confirm no further (RO) bytes trickle through.
    match timeout(Duration::from_millis(300), device_socket.read(&mut buf)).await {
        Err(_) => {} // timeout: good, nothing more arrived
        Ok(Ok(0)) => {}
        Ok(Ok(m)) => {
            let extra = String::from_utf8_lossy(&buf[..m]);
            assert!(
                !extra.contains("IGNORE"),
                "RO input leaked to device: {:?}",
                extra
            );
        }
        Ok(Err(e)) => panic!("device read error: {}", e),
    }

    // Device output mirrors to BOTH clients.
    device_socket.write_all(b"HELLO_ALL").await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    let mut b = [0u8; 64];
    let n = rw_client.read(&mut b).expect("rw read");
    assert!(
        String::from_utf8_lossy(&b[..n]).contains("HELLO_ALL"),
        "RW client should mirror device output"
    );
    let n = ro_client.read(&mut b).expect("ro read");
    assert!(
        String::from_utf8_lossy(&b[..n]).contains("HELLO_ALL"),
        "RO client should mirror device output"
    );

    assert!(crabterm.is_running(), "crabterm must not crash");
    crabterm.stop();
}

/// Force-release disconnects the RW client but leaves the RO client connected
/// and still receiving device output; the process stays up.
#[tokio::test]
async fn test_force_release_drops_rw_keeps_ro() {
    let ReleaseHarness {
        mut device_socket,
        rw_port,
        ro_port,
        mut crabterm,
        ..
    } = ReleaseHarness::start().await;

    let mut ro_client = connect(ro_port);
    let mut rw_client = connect(rw_port);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Sanity: both receive device output before release.
    device_socket.write_all(b"BEFORE").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut b = [0u8; 64];
    let n = rw_client.read(&mut b).expect("rw read before");
    assert!(String::from_utf8_lossy(&b[..n]).contains("BEFORE"));
    let n = ro_client.read(&mut b).expect("ro read before");
    assert!(String::from_utf8_lossy(&b[..n]).contains("BEFORE"));

    // Force-release.
    crabterm.send_signal(libc::SIGUSR1);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // RW client connection should be closed (read returns EOF or error).
    rw_client
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    match rw_client.read(&mut b) {
        Ok(0) => {}                                        // EOF: closed
        Ok(m) => panic!("RW client still alive, read {} bytes", m),
        Err(e) if e.kind() == ErrorKind::WouldBlock => {
            panic!("RW client not disconnected by force-release")
        }
        Err(_) => {} // other error: connection gone, acceptable
    }

    // RO client is still connected and still receives device output.
    device_socket.write_all(b"AFTER").await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let n = ro_client.read(&mut b).expect("ro read after release");
    assert!(
        String::from_utf8_lossy(&b[..n]).contains("AFTER"),
        "RO client should still receive output after force-release"
    );

    assert!(crabterm.is_running(), "crabterm must stay running");
    crabterm.stop();
}

/// The RW port rotates to a new (typically different) port on force-release,
/// the pid is unchanged, and the new port accepts connections.
#[tokio::test]
async fn test_random_port_rotates_on_release() {
    let ReleaseHarness {
        rw_port,
        port_file,
        mut crabterm,
        ..
    } = ReleaseHarness::start().await;

    let (pid_a, port_a) = read_rw_port_file(&port_file).expect("port file present");
    assert_eq!(port_a, rw_port);
    assert_eq!(pid_a as i32, crabterm.pid());

    // A connection on the current port works.
    let _c = connect(port_a);

    crabterm.send_signal(libc::SIGUSR1);

    // Wait for the file to reflect the rotation (poll; port usually changes).
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2000);
    let (pid_b, port_b) = loop {
        let (pid, port) = wait_for_rw_port_file(&port_file, 2000).await;
        if port != port_a || tokio::time::Instant::now() >= deadline {
            break (pid, port);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };

    assert_eq!(pid_b, pid_a, "pid should be stable across force-release");
    assert!(
        wait_for_port(port_b, 2000).await,
        "new RW port {} should accept connections",
        port_b
    );
    let _c2 = connect(port_b);

    assert!(crabterm.is_running(), "crabterm must stay running");
    crabterm.stop();
}

/// The port file is deleted on graceful shutdown.
#[tokio::test]
async fn test_port_file_deleted_on_shutdown() {
    let ReleaseHarness {
        port_file,
        mut crabterm,
        ..
    } = ReleaseHarness::start().await;

    assert!(port_file.exists(), "port file should exist while running");

    crabterm.stop(); // SIGTERM

    // Poll for deletion.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(3000);
    while port_file.exists() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        !port_file.exists(),
        "port file should be deleted on shutdown"
    );
}

/// With a FIXED RW port, force-release still kicks the RW client, and the same
/// port accepts a fresh connection afterward.
#[tokio::test]
async fn test_fixed_port_force_release() {
    let device_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let device_port = device_listener.local_addr().unwrap().port();

    let fixed_port = find_available_port().await;
    let port_file = temp_port_file();

    let mut crabterm = CrabtermProcess::builder()
        .device(&format!("127.0.0.1:{}", device_port))
        .rw_port(fixed_port)
        .rw_port_file(port_file.clone())
        .log_level(LogLevel::Debug)
        .spawn();

    let (_device_socket, _) = timeout(Duration::from_secs(2), device_listener.accept())
        .await
        .expect("Timeout waiting for device connect")
        .unwrap();

    assert!(wait_for_port(fixed_port, 2000).await, "RW server should start");

    let mut rw_client = connect(fixed_port);
    tokio::time::sleep(Duration::from_millis(100)).await;

    crabterm.send_signal(libc::SIGUSR1);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Old RW client is disconnected.
    let mut b = [0u8; 32];
    match rw_client.read(&mut b) {
        Ok(0) => {}
        Ok(m) => panic!("RW client still alive, read {} bytes", m),
        Err(e) if e.kind() == ErrorKind::WouldBlock => panic!("RW client not disconnected"),
        Err(_) => {}
    }

    // The same fixed port is rebound and accepts a new connection.
    assert!(
        wait_for_port(fixed_port, 2000).await,
        "fixed RW port should be rebound"
    );
    let _c = connect(fixed_port);

    // File still reports the same fixed port.
    let (_pid, port) = read_rw_port_file(&port_file).expect("port file present");
    assert_eq!(port, fixed_port, "fixed port should be unchanged");

    assert!(crabterm.is_running(), "crabterm must stay running");
    crabterm.stop();
}
