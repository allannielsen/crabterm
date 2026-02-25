#[macro_use]
mod common;

use common::{find_available_port, LogLevel};
use std::os::unix::io::FromRawFd;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Create a PTY pair using openpty
fn create_pty() -> Result<(i32, i32), String> {
    let mut master: i32 = -1;
    let mut slave: i32 = -1;

    let ret = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };

    if ret != 0 {
        return Err(format!("openpty failed: {}", ret));
    }

    Ok((master, slave))
}

/// Helper to get the slave PTY device path
fn get_pty_path(fd: i32) -> Result<String, String> {
    let name_ptr = unsafe { libc::ttyname(fd) };
    if name_ptr.is_null() {
        return Err(format!("ttyname failed: {}", std::io::Error::last_os_error()));
    }

    let c_str = unsafe { std::ffi::CStr::from_ptr(name_ptr) };
    Ok(c_str.to_string_lossy().to_string())
}

/// Write to a file descriptor
fn write_fd(fd: i32, data: &[u8]) -> Result<usize, String> {
    let ret = unsafe { libc::write(fd, data.as_ptr() as *const libc::c_void, data.len()) };
    if ret < 0 {
        Err(format!("write failed: {}", std::io::Error::last_os_error()))
    } else {
        Ok(ret as usize)
    }
}

/// Read from a file descriptor (non-blocking)
fn read_fd(fd: i32, buf: &mut [u8]) -> Result<usize, String> {
    let ret = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
    if ret < 0 {
        Err(format!("read failed: {}", std::io::Error::last_os_error()))
    } else {
        Ok(ret as usize)
    }
}

/// Test harness for console testing
struct ConsoleTestHarness {
    device_master: i32,
    device_slave: i32,
    console_master: i32,
    #[allow(dead_code)]
    console_slave: i32,
    crabterm: Child,
    #[allow(dead_code)]
    crabterm_port: u16,
    log_file: PathBuf,
}

impl ConsoleTestHarness {
    async fn start(log_level: LogLevel) -> Self {
        Self::start_with_args(log_level, &[]).await
    }

    async fn start_with_args(log_level: LogLevel, extra_args: &[&str]) -> Self {
        // Create PTY for device
        let (device_master, device_slave) = create_pty().expect("Failed to create device PTY");
        let device_path = get_pty_path(device_slave).expect("Failed to get device path");
        tprintln!("Device PTY: {}", device_path);

        // Create PTY for console
        let (console_master, console_slave) = create_pty().expect("Failed to create console PTY");
        tprintln!("Console PTY master={}, slave={}", console_master, console_slave);

        // Find port for TCP server
        let crabterm_port = find_available_port().await;

        // Create log file
        let log_file = std::env::temp_dir().join(format!(
            "crabterm_console_test_{}_{}.log",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        // Spawn crabterm with console enabled (no --headless)
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_crabterm"));
        cmd.arg("-d")
            .arg(&device_path)
            .arg("-p")
            .arg(crabterm_port.to_string())
            .arg("--log-file")
            .arg(&log_file)
            .arg("--log-level")
            .arg(log_level.as_str())
            .arg("--no-announce");

        // Add extra arguments if provided
        for arg in extra_args {
            cmd.arg(arg);
        }

        // Redirect stdin/stdout/stderr to console slave PTY
        // Need to dup() the fd for each use since from_raw_fd takes ownership
        unsafe {
            let stdin_fd = libc::dup(console_slave);
            let stdout_fd = libc::dup(console_slave);
            let stderr_fd = libc::dup(console_slave);

            cmd.stdin(Stdio::from_raw_fd(stdin_fd));
            cmd.stdout(Stdio::from_raw_fd(stdout_fd));
            cmd.stderr(Stdio::from_raw_fd(stderr_fd));
        }

        tprintln!("Spawning crabterm: {:?}", cmd);
        let crabterm = cmd.spawn().expect("Failed to spawn crabterm");

        // Give crabterm time to initialize
        tokio::time::sleep(Duration::from_millis(500)).await;

        Self {
            device_master,
            device_slave,
            console_master,
            console_slave,
            crabterm,
            crabterm_port,
            log_file,
        }
    }

    fn is_running(&mut self) -> bool {
        matches!(self.crabterm.try_wait(), Ok(None))
    }

    fn read_log(&self) -> String {
        std::fs::read_to_string(&self.log_file).unwrap_or_default()
    }

    fn stop(&mut self) {
        if self.is_running() {
            let pid = self.crabterm.id() as i32;
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(100));
            let _ = self.crabterm.wait();
        }
    }
}

impl Drop for ConsoleTestHarness {
    fn drop(&mut self) {
        self.stop();

        tprintln!("Console test log:\n{}", self.read_log());

        // Clean up PTY file descriptors
        unsafe {
            libc::close(self.device_master);
            libc::close(self.device_slave);
            libc::close(self.console_master);
            // console_slave is already owned by child process
        }

        // Clean up log file
        let _ = std::fs::remove_file(&self.log_file);
    }
}

trait LogLevelExt {
    fn as_str(&self) -> &'static str;
}

impl LogLevelExt for LogLevel {
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

#[tokio::test]
#[serial_test::serial]
async fn test_console_ctrl_q_exits() {
    let mut harness = ConsoleTestHarness::start(LogLevel::Debug).await;

    tprintln!("Crabterm started with PID {}", harness.crabterm.id());

    // Verify crabterm is running
    assert!(harness.is_running(), "Crabterm should be running initially");

    // Send Ctrl+Q to console (0x11 = Ctrl+Q)
    tprintln!("Sending Ctrl+Q to console...");
    let ctrl_q = [0x11u8];
    match write_fd(harness.console_master, &ctrl_q) {
        Ok(n) => tprintln!("Wrote {} bytes (Ctrl+Q) to console", n),
        Err(e) => panic!("Failed to write Ctrl+Q: {}", e),
    }

    // Give crabterm time to process the keybind and exit
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check if crabterm has exited
    let running_after_ctrl_q = harness.is_running();
    tprintln!("After Ctrl+Q: running={}", running_after_ctrl_q);

    if running_after_ctrl_q {
        // Read console output to see what happened
        let mut buf = [0u8; 4096];
        match read_fd(harness.console_master, &mut buf) {
            Ok(n) => {
                tprintln!("Console output ({} bytes): {:?}", n, String::from_utf8_lossy(&buf[..n]));
            }
            Err(e) => {
                tprintln!("Could not read console output: {}", e);
            }
        }

        // Print log for debugging
        tprintln!("Log contents:\n{}", harness.read_log());
    }

    assert!(
        !running_after_ctrl_q,
        "Crabterm should exit after Ctrl+Q"
    );

    tprintln!("Test passed: Ctrl+Q successfully exited crabterm");
}

#[tokio::test]
#[serial_test::serial]
async fn test_verbose_flag_enables_console_logging() {
    // Test different verbose levels
    let test_cases = vec![
        (3, "INFO", "-vvv"),   // Start with INFO level since lower levels produce no output
        (4, "DEBUG", "-vvvv"),
    ];

    for (level, expected_level, flag) in test_cases {
        tprintln!("Testing verbose level {}: {}", level, flag);

        // Start harness with verbose flag
        let mut harness = ConsoleTestHarness::start_with_args(LogLevel::Debug, &[flag]).await;

        tprintln!("Crabterm started with verbose {}", flag);

        // Give it a moment to generate some log output
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Set console_master to non-blocking mode
        unsafe {
            let flags = libc::fcntl(harness.console_master, libc::F_GETFL);
            libc::fcntl(harness.console_master, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        // Read console output
        let mut buf = [0u8; 8192];
        let mut output = String::new();

        // Try to read what's available (non-blocking)
        loop {
            match read_fd(harness.console_master, &mut buf) {
                Ok(n) if n > 0 => {
                    output.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                _ => break,
            }
        }

        tprintln!("Console output for {}:\n{}", flag, output);

        // Verify log output appears on console with proper line endings
        assert!(
            output.contains("Starting crabterm") || output.contains(expected_level),
            "Verbose flag {} should show {} level logs on console",
            flag,
            expected_level
        );

        // Verify proper line endings - should contain \r for raw mode compatibility
        if !output.is_empty() {
            assert!(
                output.contains("\r"),
                "Verbose console output should use \\r\\n line endings for raw mode compatibility"
            );
        }

        // Cleanup
        harness.stop();
    }

    tprintln!("Test passed: Verbose flags work correctly with proper line endings");
}
