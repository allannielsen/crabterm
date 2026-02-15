# Crabterm Requirements

## Overview

Crabterm is a terminal server that bridges a device (serial port or TCP) with
one or more TCP clients. This document defines the requirements for reliable
operation.

## Architecture

```
                          +----------+
    +--------+            |          |   Console  +----------+
    | Device |<---------->| Crabterm |<---------->| Client 1 |
    +--------+  TCP or    |          |            +----------+
                UART dev  |          |   TCP      +----------+
                          |          |<---------->| Client 2 |
                          |          |            +----------+
                          +----------+
```

Data flows bidirectionally:
- **Device → Clients**: Device output is broadcast to all connected clients
- **Clients → Device**: Client input is forwarded to the device

## Requirements

### R1: Stability

Crabterm must never crash regardless of:
- Data rate mismatches between device and clients
- Client connection/disconnection patterns
- Network conditions

### R2: Device Connection Integrity

Crabterm must never take the initiative to close or reset a functional
connection to the device.

If the connection to the device is broken due to the device being disconnected,
end-of-file, or other issues, then crabterm shall attempt to reconnect.

The device connection is the primary resource and must be preserved.

### R3: Slow Client Handling (Device → Client Direction)

Clients that cannot consume data at the rate the device produces it shall be
disconnected. This prevents a single slow client from affecting other clients or
the device.

The client may choose to reconnect.

### R4: Backpressure (Client → Device Direction)

Crabterm shall ensure that backpressure works in the client-to-device direction.

Typically clients can send data much faster than the device can consume it (a
UART device typically has a baud rate of 115200).

Consider the following example:

    cat large-file | netcat <CRABTERM-IP> <CRABTERM-PORT>

If crabterm is connected to a UART device at 115200 baud rate, then backpressure
must work to ensure that the entire file is sent to the device without data
loss.

This backpressure must also work when multiple clients are connected.

### R5: Client Isolation

A fast client must not be negatively impacted by the presence of slow clients.
If one client can consume data quickly while another cannot, the fast client
should continue receiving data normally while the slow client is disconnected.

## Acceptance Criteria

| Requirement | Test Criteria |
|-------------|---------------|
| R1 | Crabterm process is still running after test completion |
| R2 | Device can send and receive data after slow client handling |
| R3 | Slow client connection is closed by crabterm |
| R4 | Client write to crabterm blocks when device cannot keep up |
| R5 | Fast client receives significantly more data than slow client |
