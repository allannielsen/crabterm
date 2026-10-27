#[macro_use]
mod common;

use common::{
    CrabtermProcess, find_available_port, read_rw_port_file, temp_port_file, wait_for_rw_port_file,
};
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

struct TempConfig(PathBuf);

impl TempConfig {
    fn new(name: &str, content: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("crabterm_bind_{}_{}.cfg", name, std::process::id()));
        std::fs::write(&path, content).unwrap();
        TempConfig(path)
    }

    fn path(&self) -> PathBuf {
        self.0.clone()
    }
}

impl Drop for TempConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn wait_for_rw_port_change(path: &Path, old: u16, timeout_ms: u64) -> u16 {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    while tokio::time::Instant::now() < deadline {
        if let Some((_, port)) = read_rw_port_file(path)
            && port != old
        {
            return port;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("RW port in {} did not change from {}", path.display(), old);
}

fn try_connect(addr: SocketAddr) -> std::io::Result<TcpStream> {
    TcpStream::connect_timeout(&addr, Duration::from_millis(500))
}

async fn wait_for_addr(addr: SocketAddr, timeout_ms: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    while tokio::time::Instant::now() < deadline {
        if try_connect(addr).is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn assert_refused(addr: SocketAddr) {
    match try_connect(addr) {
        Ok(_) => panic!("{} should not accept connections", addr),
        Err(e) => assert_eq!(
            e.kind(),
            ErrorKind::ConnectionRefused,
            "{}: unexpected error {}",
            addr,
            e
        ),
    }
}

fn sock(ip: &str, port: u16) -> SocketAddr {
    SocketAddr::new(ip.parse().unwrap(), port)
}

async fn wait_for_log(crabterm: &CrabtermProcess, pattern: &str, timeout_ms: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    while tokio::time::Instant::now() < deadline {
        if !crabterm.grep_log(&[pattern]).is_empty() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

async fn wait_for_exit(crabterm: &mut CrabtermProcess, timeout_ms: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    while tokio::time::Instant::now() < deadline {
        if !crabterm.is_running() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[serial_test::serial]
async fn test_bind_address_limits_rw_ro_and_monitor() {
    let config = TempConfig::new("limits", "");
    let port_file = temp_port_file();
    let ro_port = find_available_port().await;
    let mon_port = find_available_port().await;

    let mut crabterm = CrabtermProcess::builder()
        .config(config.path())
        .echo_device()
        .bind_address("127.0.0.2")
        .rw_port(0)
        .rw_port_file(port_file.clone())
        .ro_port(ro_port)
        .device_monitor_port(mon_port)
        .spawn();

    let (_, rw_port) = wait_for_rw_port_file(&port_file, 2000).await;
    for port in [rw_port, ro_port, mon_port] {
        assert!(
            wait_for_addr(sock("127.0.0.2", port), 2000).await,
            "port {} should listen on 127.0.0.2",
            port
        );
        assert_refused(sock("127.0.0.1", port));
    }

    crabterm.send_signal(libc::SIGUSR1);
    let new_rw_port = wait_for_rw_port_change(&port_file, rw_port, 2000).await;
    assert!(
        wait_for_addr(sock("127.0.0.2", new_rw_port), 2000).await,
        "rotated RW port should listen on 127.0.0.2"
    );
    assert_refused(sock("127.0.0.1", new_rw_port));

    assert!(crabterm.is_running(), "crabterm must not crash");
    crabterm.stop();
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[serial_test::serial]
async fn test_bind_address_fixed_rw_port_rebind() {
    let config = TempConfig::new("fixed", "");
    let rw_port = find_available_port().await;

    let mut crabterm = CrabtermProcess::builder()
        .config(config.path())
        .echo_device()
        .bind_address("127.0.0.2")
        .rw_port(rw_port)
        .spawn();

    assert!(
        wait_for_addr(sock("127.0.0.2", rw_port), 2000).await,
        "RW port should listen on 127.0.0.2"
    );

    crabterm.send_signal(libc::SIGUSR1);
    let pattern = format!("Listening on 127.0.0.2:{}", rw_port);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2000);
    while crabterm.grep_log(&[&pattern]).len() < 2 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        crabterm.grep_log(&[&pattern]).len(),
        2,
        "fixed RW port should be re-bound on 127.0.0.2"
    );
    assert!(wait_for_addr(sock("127.0.0.2", rw_port), 2000).await);
    assert_refused(sock("127.0.0.1", rw_port));

    assert!(crabterm.is_running(), "crabterm must not crash");
    crabterm.stop();
}

#[tokio::test]
#[serial_test::serial]
async fn test_bind_address_ipv6_loopback() {
    if std::net::TcpListener::bind("[::1]:0").is_err() {
        tprintln!("IPv6 loopback not available, skipping");
        return;
    }

    let config = TempConfig::new("ipv6", "");
    let ro_port = find_available_port().await;

    let mut crabterm = CrabtermProcess::builder()
        .config(config.path())
        .echo_device()
        .bind_address("::1")
        .ro_port(ro_port)
        .spawn();

    assert!(
        wait_for_addr(sock("::1", ro_port), 2000).await,
        "RO port should listen on ::1"
    );
    assert_refused(sock("127.0.0.1", ro_port));

    crabterm.stop();
}

#[tokio::test]
#[serial_test::serial]
async fn test_bind_address_default_is_unspecified() {
    let config = TempConfig::new("default", "");
    let ro_port = find_available_port().await;

    let mut crabterm = CrabtermProcess::builder()
        .config(config.path())
        .echo_device()
        .ro_port(ro_port)
        .spawn();

    assert!(wait_for_addr(sock("127.0.0.1", ro_port), 2000).await);
    let pattern = format!("Listening on 0.0.0.0:{}", ro_port);
    assert!(
        wait_for_log(&crabterm, &pattern, 2000).await,
        "expected {:?} in log",
        pattern
    );

    crabterm.stop();
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[serial_test::serial]
async fn test_bind_address_from_config() {
    let config = TempConfig::new("from_cfg", "set bind-address 127.0.0.2\n");
    let ro_port = find_available_port().await;

    let mut crabterm = CrabtermProcess::builder()
        .config(config.path())
        .echo_device()
        .ro_port(ro_port)
        .spawn();

    assert!(
        wait_for_addr(sock("127.0.0.2", ro_port), 2000).await,
        "RO port should listen on 127.0.0.2"
    );
    assert_refused(sock("127.0.0.1", ro_port));

    crabterm.stop();
}

#[tokio::test]
#[serial_test::serial]
async fn test_bind_address_cli_overrides_config() {
    let config = TempConfig::new("override", "set bind-address 127.0.0.2\n");
    let ro_port = find_available_port().await;

    let mut crabterm = CrabtermProcess::builder()
        .config(config.path())
        .echo_device()
        .bind_address("127.0.0.1")
        .ro_port(ro_port)
        .spawn();

    assert!(wait_for_addr(sock("127.0.0.1", ro_port), 2000).await);
    let pattern = format!("Listening on 127.0.0.1:{}", ro_port);
    assert!(
        wait_for_log(&crabterm, &pattern, 2000).await,
        "expected {:?} in log",
        pattern
    );

    crabterm.stop();
}

#[tokio::test]
async fn test_bind_address_invalid_config_exits() {
    let cases = [
        ("not-an-ip", "not-an-ip"),
        ("on", "on (read as a boolean)"),
        ("1", "on (read as a boolean)"),
        ("off", "off (read as a boolean)"),
    ];

    for (i, (value, shown)) in cases.iter().enumerate() {
        let config = TempConfig::new(
            &format!("invalid_{}", i),
            &format!("set bind-address {}\n", value),
        );
        let ro_port = find_available_port().await;

        let mut crabterm = CrabtermProcess::builder()
            .config(config.path())
            .echo_device()
            .ro_port(ro_port)
            .spawn();

        assert!(
            wait_for_exit(&mut crabterm, 3000).await,
            "crabterm should exit on bind-address {:?}",
            value
        );
        assert!(!crabterm.wait().unwrap().success());
        let stdout = crabterm.read_stdout();
        let expected = format!("Error: invalid bind-address in config: {}", shown);
        assert!(
            stdout.contains(&expected),
            "expected {:?} in stdout, got {:?}",
            expected,
            stdout
        );
    }
}

#[tokio::test]
async fn test_bind_address_invalid_cli_rejected() {
    let config = TempConfig::new("invalid_cli", "");
    let ro_port = find_available_port().await;

    let mut crabterm = CrabtermProcess::builder()
        .config(config.path())
        .echo_device()
        .bind_address("not-an-ip")
        .ro_port(ro_port)
        .spawn();

    assert!(
        wait_for_exit(&mut crabterm, 3000).await,
        "crabterm should reject an invalid --bind-address"
    );
    assert!(!crabterm.wait().unwrap().success());
    let stderr = crabterm.read_stderr();
    assert!(
        stderr.contains("--bind-address"),
        "expected --bind-address in stderr, got {:?}",
        stderr
    );
}
