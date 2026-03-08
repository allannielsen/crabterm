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
    // We use a path that definitely doesn't exist
    let mut crabterm = CrabtermProcess::builder()
        .device("/dev/non_existent_device_12345")
        .listen(crabterm_port)
        .no_announce(false) // We WANT announcements
        .spawn();

    // Wait for the server port to be ready
    assert!(
        wait_for_port(crabterm_port, 2000).await,
        "Crabterm server should start"
    );

    // Give it a moment to try connecting and fail
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Connect a client to crabterm
    tprintln!("Client connecting to crabterm at port {}", crabterm_port);
    let mut client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port))
        .expect("Failed to connect to crabterm");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    // Read from client - it should receive an error message about the device
    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).expect("Failed to read from client");
    let received = String::from_utf8_lossy(&buf[..n]);

    tprintln!("Received from crabterm: {}", received);

    // Check for IP/Port prefix and error message
    let expected_prefix = format!(":{}", crabterm_port);
    assert!(
        received.contains(&expected_prefix),
        "Announcement should be prefixed with IP:PORT. Got: {}",
        received
    );
    assert!(
        received.contains("Error")
            || received.contains("Not connected")
            || received.contains("No such file"),
        "Client should receive hint that device is not connected. Got: {}",
        received
    );

    crabterm.stop();
}

#[tokio::test]
async fn test_client_receives_device_connected_hint() {
    let crabterm_port = find_available_port().await;

    // Start crabterm with the echo device (which connects immediately)
    let mut crabterm = CrabtermProcess::builder()
        .echo_device()
        .listen(crabterm_port)
        .no_announce(false)
        .spawn();

    assert!(
        wait_for_port(crabterm_port, 2000).await,
        "Crabterm server should start"
    );

    // Connect a client
    let mut client =
        TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).expect("Failed to connect");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).expect("Failed to read");
    let received = String::from_utf8_lossy(&buf[..n]);

    tprintln!("Received: {}", received);

    // Check for IP/Port prefix and Connected message
    let expected_prefix = format!(":{}", crabterm_port);
    assert!(
        received.contains(&expected_prefix),
        "Announcement should be prefixed with IP:PORT. Got: {}",
        received
    );
    assert!(
        received.contains("Connected"),
        "Client should receive hint that device is connected. Got: {}",
        received
    );

    crabterm.stop();
}

#[tokio::test]
async fn test_late_connecting_client_receives_last_error() {
    let crabterm_port = find_available_port().await;

    // Start crabterm with a non-existent device
    let mut crabterm = CrabtermProcess::builder()
        .device("/dev/non_existent_device_late")
        .listen(crabterm_port)
        .no_announce(false)
        .spawn();

    assert!(
        wait_for_port(crabterm_port, 2000).await,
        "Crabterm server should start"
    );

    // Wait long enough for the first connection attempt to fail and set the error message
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Connect a client LATE
    tprintln!(
        "Late client connecting to crabterm at port {}",
        crabterm_port
    );
    let mut client =
        TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).expect("Failed to connect");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).expect("Failed to read");
    let received = String::from_utf8_lossy(&buf[..n]);

    tprintln!("Late client received: {}", received);

    // Check for IP/Port prefix and Error message
    let expected_prefix = format!(":{}", crabterm_port);
    assert!(
        received.contains(&expected_prefix),
        "Late announcement should be prefixed with IP:PORT. Got: {}",
        received
    );
    assert!(
        received.contains("Error") && received.contains("No such file"),
        "Late client should receive the actual device error. Got: {}",
        received
    );

    crabterm.stop();
}
