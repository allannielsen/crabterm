# Reservation & Force-Release

## Overview

Crabterm is often used as the access point to a shared hardware setup in a
test-automation lab: bots or humans **reserve** a setup, interact with the
device, and then **release** it so the next user can take over.

The problem this feature solves: sometimes a bot hangs or a user forgets to
release, leaving a stale connection that blocks everyone else. We need a way to
**force-release** that connection — cleanly kicking the stuck reserver — while
*not* disturbing people who are only **listening in**.

The solution is two separate server ports plus a signal:

- a **read-write (RW)** port for the reserving client, and
- a **read-only (RO)** port for observers.

A `SIGUSR1` signal force-releases the RW side without touching the RO side.

## Concepts

### Read-write socket (`--rw-port`)

Bidirectional, exactly like a normal crabterm client: device output is broadcast
to the client, and the client's input is forwarded to the device. This is the
socket a reserver connects to.

- `--rw-port 0` binds an **OS-assigned random port**. Because the port is not
  known in advance, its value is published via `--rw-port-file` (below).
- `--rw-port <PORT>` binds a fixed port.
- `-p` / `--port` is a backward-compatible alias for `--rw-port`.

The RW socket is the **target of force-release**.

### Read-only socket (`--ro-port`)

A raw mirror of the device output. Clients receive exactly the same bytes the
device emits, but **any input they send is discarded** and never reaches the
device. RO clients are:

- **never** disconnected by force-release, and
- never able to interfere with the reserver or the device.

> This is different from `--device-monitor-port`, which emits *escaped and
> annotated* output (RX/TX framing, hex escapes) intended for debugging. The RO
> socket is a byte-for-byte raw mirror suitable for a human to "watch along".

### Force-release (`SIGUSR1`)

On `SIGUSR1`, crabterm:

1. Binds a fresh RW listener — a **new random port** if `--rw-port 0`, or the
   **same port** if a fixed port was configured.
2. Tears down the old RW listener and **disconnects all connected RW clients**.
3. Rewrites `--rw-port-file` with the new port.
4. Leaves the RO clients, the local console, the device connection, and the
   device monitor completely untouched.

If the new listener fails to bind, the existing server is kept and an error is
logged (no clients are dropped).

> Why the RW port rotates: the port number acts as a lightweight capability.
> A hung bot is holding a socket on the *old* listener; closing that listener
> and rotating the port means only a client that reads the *new* port from the
> port file can reconnect — the stuck bot cannot silently reclaim the setup.

`SIGINT` / `SIGTERM` perform a graceful shutdown and **remove** the port file,
so the reservation system can tell the server is gone.

## Port file format

`--rw-port-file` is written atomically (temp file + `rename`) on startup and
after every force-release, and removed on graceful shutdown. Format:

```
pid=12345
port=54321
```

It is deliberately trivial to parse and is directly `source`-able in a POSIX
shell:

```bash
. /run/crabterm/rw.port   # sets $pid and $port
echo "pid=$pid port=$port"
```

`pid` is stable for the lifetime of the process; only `port` changes across a
force-release.

## Workflow

```
                                   +-----------------------------+
   reservation system              |          crabterm           |
   -----------------               |                             |
   1. read rw.port  <--------------|  --rw-port-file  pid,port    |
   2. connect $port  ------------->|  --rw-port 0   (RW server)   |<--> device
   3. ...work...                   |                             |
   4a. release: SIGTERM ---------->|  (shutdown, file removed)    |
   4b. force:   SIGUSR1 ---------->|  (kick RW, rotate port,      |
                                   |   rewrite file)              |
                                   |  --ro-port     (RO server)   |<--- observers
                                   +-----------------------------+
```

1. **Reserve** — the reservation system reads `pid` and `port` from the port
   file and hands the port to the bot/user, who connects to the RW port.
2. **Use** — the reserver interacts with the device normally.
3. **Release** — normally the reserver disconnects. To reclaim a hung or
   forgotten session, the reservation system sends `SIGUSR1` to `pid`; the RW
   client is dropped and the port is rotated. The system re-reads the file to
   get the new port for the next reservation.
4. **Observe** — anyone may connect to the stable RO port at any time to watch;
   they are unaffected by reservations and releases.

## Example

Start a reservable server with a random RW port, a port file, and a stable RO
listen-in port:

```bash
crabterm /dev/ttyUSB0 --headless \
    --rw-port 0 --rw-port-file /run/crabterm/rw.port \
    --ro-port 7000
```

Connect as the reserver:

```bash
. /run/crabterm/rw.port
nc localhost "$port"        # bidirectional
```

Listen in (observer):

```bash
nc localhost 7000           # receives device output; input is ignored
```

Force-release the current reserver:

```bash
. /run/crabterm/rw.port
kill -USR1 "$pid"           # RW client is disconnected, port rotates
. /run/crabterm/rw.port     # re-read to get the new $port
```

## Notes and guarantees

- **Random vs fixed RW port.** With `--rw-port 0`, each force-release yields a
  new random port (the same number may occasionally be reused by the OS). With a
  fixed `--rw-port`, force-release re-binds the same port but still disconnects
  all RW clients — a plain "kick everyone off" that needs no port file.
- **The port file requires an RW server.** Passing `--rw-port-file` without
  `--rw-port` (or `-p`) is an error.
- **Headless.** `--headless` requires at least one of `--rw-port`/`-p` or
  `--ro-port`.
- **Stability.** Force-release upholds the flow-control requirements in
  [requirements.md](requirements.md): the device connection is never touched
  (R2), and slow RO/RW clients are still disconnected on write failure (R3/R5).
- **Closing the listener is not enough on its own.** Already-accepted RW
  connections are separate sockets from the listener, so force-release
  explicitly closes each RW client in addition to rotating the listener.

## See also

- `crabterm(1)` — full option and `SIGNALS` reference (`crabterm.1`).
- [requirements.md](requirements.md) — flow-control requirements (R1–R5).
