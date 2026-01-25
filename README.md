# crabterm

A terminal (UART) server and client written in Rust.

Inspired by [termhub](https://github.com/allannielsen/termhub) but rewritten from scratch in Rust (not compatible).

If used without the server ability, then it is very similar to picocom or
minicom.

## Features

- Serial port connections (e.g., `/dev/ttyUSB0`)
- TCP device connections (connect to remote serial servers)
- TCP server mode (expose a serial port over the network)
- Multiple simultaneous TCP clients
- Echo mode for testing without hardware
- Configurable keybindings
- Timestamp filtering on output
- Auto-reconnection on disconnect

## Installation

```bash
cargo build --release
sudo cp target/release/crabterm /usr/local/bin/
```

## Usage

```bash
# Connect to a serial device
crabterm /dev/ttyUSB0

# Connect with specific baudrate
crabterm /dev/ttyUSB0 -b 9600

# Connect to a TCP device
crabterm 192.168.1.100:4000

# Start a TCP server exposing a serial port
crabterm /dev/ttyUSB0 -p 4000

# Echo mode (for testing)
crabterm echo

# Headless mode (daemon, no local console)
crabterm /dev/ttyUSB0 -p 4000 --headless
```

## Configuration

Configuration file: `~/.crabterm`

Example:
```
# Quit with Ctrl+Q
bind ctrl+q quit

# Prefix mode: Ctrl+A followed by another key
prefix ctrl+a

# Send break signal with Ctrl+A, b
bind ctrl+a b send \x00

# Toggle timestamp filter
bind ctrl+a t toggle-timestamp
```

## License

MIT
