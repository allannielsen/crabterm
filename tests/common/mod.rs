#![allow(dead_code)]

use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;

/// Return current timestamp as "YY-MM-DD HH:MM:SS.mmm.uuu"
pub fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let secs = now.as_secs();
    let micros = now.subsec_micros();
    let millis = micros / 1000;
    let remaining_micros = micros % 1000;

    // Convert to broken-down time using libc
    let time_t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&time_t, &mut tm) };

    format!(
        "{:02}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}.{:03}",
        (tm.tm_year + 1900) % 100,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        millis,
        remaining_micros,
    )
}

#[macro_export]
macro_rules! tprintln {
    ($($arg:tt)*) => {{
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        let secs = now.as_secs();
        let micros = now.subsec_micros();
        let millis = micros / 1000;
        let remaining_micros = micros % 1000;
        let time_t = secs as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe { libc::localtime_r(&time_t, &mut tm) };
        println!(
            "{:02}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}.{:03} {}",
            (tm.tm_year + 1900) % 100,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            millis,
            remaining_micros,
            format_args!($($arg)*)
        );
    }};
}

/// Helper to find an available port
pub async fn find_available_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

/// Wait for a TCP port to become available
pub async fn wait_for_port(port: u16, timeout_ms: u64) -> bool {
    let addr = format!("127.0.0.1:{}", port);
    tprintln!("wait_for_port: testing {}", addr);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let mut attempts = 0;
    while tokio::time::Instant::now() < deadline {
        attempts += 1;
        match TcpStream::connect(&addr) {
            Ok(s) => {
                tprintln!("wait_for_port: Peer: {:?} Local: {:?} ready after {} attempts", s.peer_addr(), s.local_addr(), attempts);
                return true;
            }
            Err(e) => {
                if attempts == 1 {
                    tprintln!("wait_for_port: {} attempt 1 failed: {}", addr, e);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tprintln!(
        "wait_for_port: {} TIMEOUT after {} attempts",
        addr, attempts
    );
    false
}

/// Log level for crabterm
#[derive(Debug, Clone, Copy, Default)]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

/// Builder for configuring and spawning a crabterm process
#[derive(Debug, Default)]
pub struct CrabtermBuilder {
    device_addr: Option<String>,
    listen_port: Option<u16>,
    use_echo_device: bool,
    log_level: LogLevel,
    headless: bool,
    no_announce: bool,
}

impl CrabtermBuilder {
    pub fn new() -> Self {
        Self {
            headless: true,      // Default to headless for tests
            no_announce: true,   // Default to no-announce for tests
            ..Default::default()
        }
    }

    /// Connect to a TCP device at the given address
    pub fn device(mut self, addr: &str) -> Self {
        self.device_addr = Some(addr.to_string());
        self.use_echo_device = false;
        self
    }

    /// Use the built-in echo device
    pub fn echo_device(mut self) -> Self {
        self.use_echo_device = true;
        self.device_addr = None;
        self
    }

    /// Listen for client connections on the given port
    pub fn listen(mut self, port: u16) -> Self {
        self.listen_port = Some(port);
        self
    }

    /// Set the log level
    pub fn log_level(mut self, level: LogLevel) -> Self {
        self.log_level = level;
        self
    }

    /// Run in headless mode (no terminal UI)
    pub fn headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    /// Suppress informational messages to clients
    pub fn no_announce(mut self, no_announce: bool) -> Self {
        self.no_announce = no_announce;
        self
    }

    /// Spawn the crabterm process
    pub fn spawn(self) -> CrabtermProcess {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_crabterm"));

        // Device configuration
        if self.use_echo_device {
            cmd.arg("--echo");
        } else if let Some(addr) = &self.device_addr {
            cmd.arg(addr);
        } else {
            panic!("CrabtermBuilder: must specify device() or echo_device()");
        }

        // Listen port
        if let Some(port) = self.listen_port {
            cmd.arg("-p").arg(port.to_string());
        }

        // Log file
        let log_file = std::env::temp_dir().join(format!(
            "crabterm_test_{}_{}.log",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        cmd.arg("--log-file").arg(&log_file);
        cmd.arg("--log-level").arg(self.log_level.as_str());

        // Headless mode
        if self.headless {
            cmd.arg("--headless");
        }

        // No-announce mode
        if self.no_announce {
            cmd.arg("--no-announce");
        }

        tprintln!("Spawning: {:?}", cmd);

        let child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to spawn crabterm");

        CrabtermProcess {
            child,
            log_file,
            listen_port: self.listen_port,
        }
    }
}

/// A running crabterm process with access to logs and cleanup
pub struct CrabtermProcess {
    child: Child,
    log_file: PathBuf,
    listen_port: Option<u16>,
}

impl CrabtermProcess {
    /// Create a new builder
    pub fn builder() -> CrabtermBuilder {
        CrabtermBuilder::new()
    }

    /// Get the listen port if configured
    pub fn listen_port(&self) -> Option<u16> {
        self.listen_port
    }

    /// Check if the process is still running
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Read the contents of the log file
    pub fn read_log(&self) -> String {
        std::fs::read_to_string(&self.log_file).unwrap_or_default()
    }

    /// Read and filter log lines containing any of the given patterns
    pub fn grep_log(&self, patterns: &[&str]) -> Vec<String> {
        self.read_log()
            .lines()
            .filter(|line| patterns.iter().any(|p| line.contains(p)))
            .map(String::from)
            .collect()
    }

    /// Get the path to the log file
    pub fn log_path(&self) -> &PathBuf {
        &self.log_file
    }

    /// Read stderr from the process (useful if it crashed)
    pub fn read_stderr(&mut self) -> String {
        if let Some(mut stderr) = self.child.stderr.take() {
            let mut output = String::new();
            let _ = stderr.read_to_string(&mut output);
            output
        } else {
            String::new()
        }
    }

    /// Gracefully stop the process (SIGTERM, wait 3s, then SIGKILL if needed)
    pub fn stop(&mut self) {
        // Send SIGTERM for graceful shutdown
        let pid = self.child.id() as i32;
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }

        // Wait up to 3 seconds for graceful exit
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return, // Process exited
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => return, // Error checking status, assume dead
            }
        }

        // Process didn't exit gracefully, force kill
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Force kill the process immediately (SIGKILL)
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Wait for the process to exit and return exit status
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait()
    }
}

impl Drop for CrabtermProcess {
    fn drop(&mut self) {
        // Gracefully stop the process if still running
        if self.is_running() {
            self.stop();
        }

        tprintln!("Log content:\r\n{}", self.read_log());

        // Clean up log file
        let _ = std::fs::remove_file(&self.log_file);
    }
}

// Keep the old function for backwards compatibility during migration
#[deprecated(note = "Use CrabtermProcess::builder() instead")]
pub fn spawn_crabterm(device_addr: &str, listen_port: Option<u16>) -> (Child, Option<PathBuf>) {
    let mut builder = CrabtermBuilder::new().device(device_addr);
    if let Some(port) = listen_port {
        builder = builder.listen(port);
    }
    let mut process = builder.spawn();

    // Take ownership of child, leaving a dummy that won't cause issues on drop
    let log_file = process.log_file.clone();

    // Kill and wait to prevent zombie, then extract what we need
    // Note: This is backwards compat only - new code should use CrabtermProcess directly
    let child = std::mem::replace(
        &mut process.child,
        Command::new("true").spawn().unwrap(), // Dummy process
    );

    // Prevent drop from cleaning up log file by leaking the process
    std::mem::forget(process);

    (child, Some(log_file))
}
