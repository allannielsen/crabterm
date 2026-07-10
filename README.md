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
- Separate read-write and read-only ("listen-in") server ports
- Force-release (`SIGUSR1`) for reservation systems — reclaim a hung session
  without disturbing observers
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

## Reservable server (reserve / release / listen-in)

For test-automation setups where bots or users *reserve* a device, do their
work, and *release* it, crabterm can expose two separate server ports:

- a **read-write** port (`--rw-port`) for the reserving client — supports a
  random port (`--rw-port 0`) whose value is published to a file
  (`--rw-port-file`), and
- a **read-only** port (`--ro-port`) for observers who only want to listen in.

Sending **`SIGUSR1`** *force-releases* the current session: all read-write
clients are disconnected, the read-write port is rotated, and the new port is
written to the port file. Read-only observers are left untouched.

```bash
# Random RW port + port file + stable RO listen-in port
crabterm /dev/ttyUSB0 --headless \
    --rw-port 0 --rw-port-file /run/crabterm/rw.port \
    --ro-port 7000

# Force-release a hung/forgotten session
. /run/crabterm/rw.port   # sets $pid and $port
kill -USR1 "$pid"         # disconnect the RW client, rotate the port
```

See [docs/reservation.md](docs/reservation.md) for the full use case and
integration details.

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
bind ctrl+a t filter-toggle timestamp
```

## License

MIT
