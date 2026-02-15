#[macro_use]
mod common;

use common::{find_available_port, wait_for_port, CrabtermProcess, LogLevel};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

/// Common test setup: starts a device listener, spawns crabterm, accepts the
/// device connection, and waits for crabterm's server port to be ready.
struct TestHarness {
    device_listener: TcpListener,
    device_socket: tokio::net::TcpStream,
    crabterm_port: u16,
    crabterm: CrabtermProcess,
}

impl TestHarness {
    async fn start(log_level: LogLevel) -> Self {
        let device_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let device_port = device_listener.local_addr().unwrap().port();

        let crabterm_port = find_available_port().await;
        let crabterm = CrabtermProcess::builder()
            .device(&format!("127.0.0.1:{}", device_port))
            .listen(crabterm_port)
            .log_level(log_level)
            .spawn();

        let (device_socket, _) = timeout(Duration::from_secs(2), device_listener.accept())
            .await
            .expect("Timeout waiting for crabterm to connect to device")
            .unwrap();

        assert!(
            wait_for_port(crabterm_port, 2000).await,
            "Crabterm server should start"
        );

        Self {
            device_listener,
            device_socket,
            crabterm_port,
            crabterm,
        }
    }
}

#[tokio::test]
async fn test_tcp_connects_to_server() {
    let TestHarness {
        mut device_socket,
        crabterm_port,
        mut crabterm,
        ..
    } = TestHarness::start(LogLevel::Debug).await;

    // Connect a client to crabterm
    tprintln!("Trying to connect");
    let mut client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).unwrap();
    client.set_nonblocking(false).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    tprintln!("Client connected: Peer: {:?}, Local: {:?}", client.peer_addr(), client.local_addr());


    // Send data from client -> crabterm -> device
    client.write_all(b"hello").unwrap();
    tprintln!("Client sent hello");

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

    crabterm.stop();
}

#[tokio::test]
async fn test_tcp_reconnects_after_server_disconnect() {
    let TestHarness {
        device_listener,
        mut device_socket,
        crabterm_port,
        mut crabterm,
    } = TestHarness::start(LogLevel::default()).await;
    let device_addr = device_listener.local_addr().unwrap().to_string();

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

    crabterm.stop();
}

#[tokio::test]
async fn test_tcp_handles_connection_refused() {
    // Pick a port with nothing listening
    let unused_port = find_available_port().await;

    // Start crabterm trying to connect to nothing
    let crabterm_port = find_available_port().await;
    let mut crabterm = CrabtermProcess::builder()
        .device(&format!("127.0.0.1:{}", unused_port))
        .listen(crabterm_port)
        .log_level(LogLevel::Debug)
        .spawn();

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

    crabterm.stop();
}

/// A slow (non-reading) client must not cause backpressure on the device connection.
/// Crabterm should accept all device data regardless of client state.
#[tokio::test]
async fn test_slow_client_does_not_backpressure_device() {
    let TestHarness {
        mut device_socket,
        crabterm_port,
        mut crabterm,
        ..
    } = TestHarness::start(LogLevel::Debug).await;

    // Connect a client that will never read
    let _slow_client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send 8MB from the device into crabterm
    let chunk = vec![b'X'; 1024];
    let total_chunks = 8192; // 8MB
    let mut chunks_sent = 0;

    tprintln!("Sending 8MB from device...");
    for i in 0..total_chunks {
        match timeout(Duration::from_millis(500), device_socket.write_all(&chunk)).await {
            Ok(Ok(())) => {
                chunks_sent = i + 1;
            }
            Ok(Err(e)) => {
                tprintln!("Device write error at chunk {}: {}", i, e);
                break;
            }
            Err(_) => {
                tprintln!("Device write timeout at chunk {} (backpressure detected)", i);
                break;
            }
        }
    }

    let total_bytes = chunks_sent * chunk.len();
    tprintln!("Device sent {} chunks ({} bytes)", chunks_sent, total_bytes);

    assert!(
        crabterm.is_running(),
        "Crabterm must not crash"
    );

    assert_eq!(
        chunks_sent, total_chunks,
        "All 8MB should be writable without backpressure (only sent {}/{} chunks)",
        chunks_sent, total_chunks
    );

    // Verify crabterm closed the slow client's socket.
    // Read everything available — we should hit EOF well before 8MB.
    let mut slow_client = _slow_client;
    slow_client.set_nonblocking(false).unwrap();
    slow_client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let mut total_read = 0usize;
    let mut buf = [0u8; 8192];
    loop {
        match slow_client.read(&mut buf) {
            Ok(0) => {
                tprintln!("Slow client: EOF after reading {} bytes", total_read);
                break;
            }
            Ok(n) => {
                total_read += n;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut
                || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                panic!(
                    "Slow client: read timed out after {} bytes — socket not closed by crabterm",
                    total_read
                );
            }
            Err(e) => {
                tprintln!("Slow client: read error after {} bytes: {} (treating as closed)", total_read, e);
                break;
            }
        }
    }

    assert!(
        total_read < 8 * 1024 * 1024,
        "Slow client should have been disconnected before receiving all 8MB (got {} bytes)",
        total_read
    );
    tprintln!("Slow client received {} bytes before EOF (< 8MB) — confirmed disconnected", total_read);

    crabterm.stop();
}

/// TCP backpressure must propagate from the device back through crabterm to the
/// client.  When the device stops reading, the client's writes must eventually
/// block.  Once the device drains some data the client must be able to resume.
/// The test loops until the full 32 MB has been transmitted end-to-end,
/// verifying that backpressure kicks in (and is relieved) multiple times.
#[tokio::test]
async fn test_client_to_device_backpressure() {
    let TestHarness {
        device_socket,
        crabterm_port,
        mut crabterm,
        ..
    } = TestHarness::start(LogLevel::Debug).await;

    // Convert to std so the tokio reactor does not touch the idle socket.
    let mut device_socket = device_socket.into_std().unwrap();
    device_socket.set_nonblocking(true).unwrap();

    // Connect a client that will flood data toward the device
    let mut client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).unwrap();
    client.set_nonblocking(false).unwrap();
    client
        .set_write_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Build the full send buffer with a counting pattern:
    // each byte = (global_offset % 256) so we can spot duplicates/gaps.
    let chunk_size: usize = 1024;
    let total_target: usize = 32 * 1024 * 1024; // 32 MB
    let send_buf: Vec<u8> = (0..total_target).map(|i| (i % 256) as u8).collect();

    let mut total_sent: usize = 0;
    let mut received_buf: Vec<u8> = Vec::with_capacity(total_target);
    let mut backpressure_count: usize = 0;

    tprintln!("Sending 32 MB from client through crabterm to device...");

    // Loop: send until blocked, then drain device, repeat until all data sent
    // and received.
    loop {
        // Phase 1: Send from client until backpressure blocks or target reached.
        // Use write() (not write_all) so we can track partial writes: write_all
        // may internally write some bytes before timing out, returning Err while
        // some data was already delivered to the kernel.
        if total_sent < total_target {
            let mut blocked = false;
            while total_sent < total_target {
                let end = std::cmp::min(total_sent + chunk_size, total_target);
                match client.write(&send_buf[total_sent..end]) {
                    Ok(n) => {
                        total_sent += n;
                    }
                    Err(_) => {
                        blocked = true;
                        break;
                    }
                }
            }
            if blocked {
                backpressure_count += 1;
                tprintln!(
                    "Backpressure #{}: client blocked after sending {} bytes total",
                    backpressure_count,
                    total_sent
                );
            }
        }

        // Phase 2: Read from the device to relieve backpressure
        let mut drained = 0usize;
        let mut buf = [0u8; 65536];
        loop {
            match device_socket.read(&mut buf) {
                Ok(0) => {
                    panic!("Device socket EOF — crabterm closed the connection unexpectedly");
                }
                Ok(n) => {
                    received_buf.extend_from_slice(&buf[..n]);
                    drained += n;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(e) => {
                    panic!("Device read error: {}", e);
                }
            }
        }

        tprintln!(
            "Drained {} bytes from device (total received: {} / {})",
            drained,
            received_buf.len(),
            total_target
        );

        // Done when we have sent AND received all data
        if total_sent >= total_target && received_buf.len() >= total_target {
            break;
        }

        // If we could not drain anything and haven't sent everything yet,
        // give crabterm time to forward data before retrying.
        if drained == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    tprintln!(
        "Complete: sent={}, received={}, backpressure_events={}",
        total_sent,
        received_buf.len(),
        backpressure_count
    );

    assert!(crabterm.is_running(), "Crabterm must not crash");

    assert!(
        backpressure_count >= 2,
        "Backpressure must kick in multiple times (got {} events)",
        backpressure_count
    );

    // Compare sent vs received byte-by-byte
    if received_buf.len() != send_buf.len() || received_buf[..] != send_buf[..] {
        // Find the first mismatch to aid debugging
        let cmp_len = std::cmp::min(send_buf.len(), received_buf.len());
        let mut first_diff = None;
        for i in 0..cmp_len {
            if send_buf[i] != received_buf[i] {
                first_diff = Some(i);
                break;
            }
        }
        if let Some(pos) = first_diff {
            panic!(
                "Data mismatch at byte offset {}: sent 0x{:02x}, got 0x{:02x} \
                 (sent={} bytes, received={} bytes)",
                pos, send_buf[pos], received_buf[pos],
                send_buf.len(), received_buf.len()
            );
        } else {
            panic!(
                "Length mismatch: sent={} bytes, received={} bytes \
                 (first {} bytes match)",
                send_buf.len(), received_buf.len(), cmp_len
            );
        }
    }

    tprintln!("All {} bytes match", total_target);

    crabterm.stop();
}

#[tokio::test]
async fn test_slow_client_does_not_block_fast_client() {
    let TestHarness {
        mut device_socket,
        crabterm_port,
        mut crabterm,
        ..
    } = TestHarness::start(LogLevel::Debug).await;

    // Connect a "fast client" FIRST
    let fast_client = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", crabterm_port))
        .await
        .unwrap();
    tprintln!("Fast client connected from {:?}", fast_client.local_addr());
    // IMPORTANT: Keep both halves alive - dropping write half causes EOF on server side
    let (mut fast_reader, fast_writer) = fast_client.into_split();

    // Give crabterm time to register the fast client
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect a "slow client" that will NOT read any data
    let slow_client = TcpStream::connect(format!("127.0.0.1:{}", crabterm_port)).unwrap();
    tprintln!("Slow client connected from {:?}", slow_client.local_addr());
    slow_client.set_nonblocking(true).unwrap();

    // Give crabterm time to register the slow client
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Spawn a task to have the fast client consume data as fast as possible
    let fast_client_handle = tokio::spawn(async move {
        let mut total_received = 0usize;
        let mut connection_closed = false;
        let mut buf = [0u8; 8192];
        loop {
            match timeout(Duration::from_secs(1), fast_reader.read(&mut buf)).await {
                Ok(Ok(0)) => {
                    connection_closed = true;
                    break;
                }
                Ok(Ok(n)) => {
                    total_received += n;
                }
                Ok(Err(_)) => {
                    connection_closed = true;
                    break;
                }
                Err(_) => {
                    // Timeout - no more data coming (this is expected when device finishes)
                    break;
                }
            }
        }
        (total_received, connection_closed)
    });

    // Flood data from the device
    // Send 8MB to ensure we overflow OS buffers (which can be 2-4MB) and trigger crabterm's buffering
    let chunk = vec![b'X'; 1024]; // 1KB chunks
    let total_chunks = 8000; // 8MB total
    let mut chunks_sent = 0;
    let mut device_write_failed = false;

    tprintln!("Starting device send loop...");
    for i in 0..total_chunks {
        match timeout(Duration::from_millis(100), device_socket.write_all(&chunk)).await {
            Ok(Ok(())) => {
                chunks_sent = i + 1;
                if chunks_sent % 100 == 0 {
                    tprintln!("Device sent {} chunks", chunks_sent);
                }
            }
            Ok(Err(e)) => {
                tprintln!("Device write error at chunk {}: {}", i, e);
                device_write_failed = true;
                break;
            }
            Err(_) => {
                // Timeout means backpressure is working - device can't write because crabterm isn't reading
                // This is acceptable, not a failure
                tprintln!("Device write timeout at chunk {} (backpressure working)", i);
                break;
            }
        }
    }
    let total_bytes_sent = chunks_sent * chunk.len();
    tprintln!("Device send loop done. Sent {} chunks ({} bytes)", chunks_sent, total_bytes_sent);

    // Wait for fast client to finish receiving
    tprintln!("Waiting for fast client...");
    let (fast_received, fast_client_closed) = fast_client_handle.await.unwrap();
    tprintln!("Fast client done. Received {} bytes, closed={}", fast_received, fast_client_closed);

    // Give crabterm ample time to process remaining data and disconnect slow clients
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Check slow client status - keep checking until closed or timeout
    tprintln!("Checking slow client status...");
    let mut slow_client = slow_client;
    let mut slow_received = 0usize;
    let mut slow_client_closed = false;
    let mut buf = [0u8; 4096];
    let check_deadline = std::time::Instant::now() + Duration::from_secs(10);

    slow_client.set_nonblocking(false).unwrap();
    slow_client
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();

    while std::time::Instant::now() < check_deadline {
        match slow_client.read(&mut buf) {
            Ok(0) => {
                tprintln!("Slow client: EOF - connection closed by crabterm");
                slow_client_closed = true;
                break;
            }
            Ok(n) => {
                slow_received += n;
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                tprintln!("Slow client: error {} (treating as closed)", e);
                slow_client_closed = true;
                break;
            }
        }
    }

    if !slow_client_closed {
        tprintln!(
            "Slow client: still connected after 10s (received {} bytes)",
            slow_received
        );
    }
    tprintln!(
        "Slow client check done. Received {} bytes, closed={}",
        slow_received,
        slow_client_closed
    );

    // Crabterm must never crash
    let crabterm_running = crabterm.is_running();

    // Device connection must be preserved - verify by sending data through
    let device_connection_alive = if !device_write_failed {
        device_socket.write_all(b"PROBE").await.is_ok()
    } else {
        false
    };

    tprintln!("\n=== SUMMARY ===");
    tprintln!("Total sent by device:     {} bytes", total_bytes_sent);
    tprintln!(
        "Fast client received:     {} bytes ({:.1}%)",
        fast_received,
        if total_bytes_sent > 0 {
            100.0 * fast_received as f64 / total_bytes_sent as f64
        } else {
            0.0
        }
    );
    tprintln!("Fast client closed:       {}", fast_client_closed);
    tprintln!("Slow client received:     {} bytes", slow_received);
    tprintln!("Slow client closed:       {}", slow_client_closed);
    tprintln!("Crabterm running:         {}", crabterm_running);
    tprintln!("Device connection alive:  {}", device_connection_alive);
    tprintln!("Device write failed:      {}", device_write_failed);

    if !crabterm_running {
        tprintln!("CRABTERM STDERR:\n{}", crabterm.read_stderr());
    }

    crabterm.stop();

    // === ASSERTIONS ===

    // Crabterm must never crash
    assert!(crabterm_running, "FAILED: Crabterm crashed");

    // Device connection must be preserved
    assert!(
        !device_write_failed,
        "FAILED: Device connection was closed/reset"
    );
    assert!(
        device_connection_alive,
        "FAILED: Device connection is not alive after test"
    );

    // Slow client shall be disconnected (when it can't keep up)
    assert!(
        slow_client_closed,
        "FAILED: Slow client was not disconnected"
    );

    // Fast client must not be blocked by slow client
    assert!(
        !fast_client_closed,
        "FAILED: Fast client was incorrectly disconnected"
    );
    assert!(
        fast_received > total_bytes_sent / 2,
        "FAILED: Fast client only received {}% of data (expected >50%)",
        if total_bytes_sent > 0 {
            100 * fast_received / total_bytes_sent
        } else {
            0
        }
    );

    // Keep fast_writer alive until end of test (dropping it causes EOF on server)
    drop(fast_writer);
}

