use std::os::fd::AsRawFd;
use std::sync::OnceLock;
use termios::{TCSANOW, Termios, cfmakeraw, tcsetattr};

static ORIGINAL_TERMIOS: OnceLock<Termios> = OnceLock::new();

pub fn enable_raw_mode() -> std::io::Result<()> {
    let fd = std::io::stdin().as_raw_fd();
    let mut termios = Termios::from_fd(fd)?;
    ORIGINAL_TERMIOS.set(termios).ok(); // ignore if already set
    cfmakeraw(&mut termios);
    tcsetattr(fd, TCSANOW, &termios)?;
    Ok(())
}

pub fn disable_raw_mode() -> std::io::Result<()> {
    let fd = std::io::stdin().as_raw_fd();
    if let Some(original) = ORIGINAL_TERMIOS.get() {
        tcsetattr(fd, TCSANOW, original)?;
    }
    Ok(())
}
