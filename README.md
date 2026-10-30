# Hablog
This project is a simple TCP proxy implemented in Rust using the Tokio asynchronous runtime.
It relays traffic between the Habbo client and the game server and logs the packets in both directions, with names for known headers.
This was kinda just an excuse to learn more Rust, but it'd be pretty easy to use as a starting point to add packet headers / packet sending.

## Running

```bash
cargo run --release
```

Hablog doesn't need root and doesn't change any system settings. It listens on `127.0.0.1:38101` and forwards to `game-us.habbo.com:38101`. Both can be changed:

| Option | Default | |
|---|---|---|
| `--listen ADDR` | `127.0.0.1:38101` | Address the client connects to. Must be loopback. |
| `--upstream ADDR` | `game-us.habbo.com:38101` | The real game server. |
| `--capture-dir DIR` | `logs` | Where per-session capture files are written. |
| `--no-capture` | | Don't write capture files. |
| `--no-client-log` | | Don't print client to server packets (still captured). |
| `--no-server-log` | | Don't print server to client packets (still captured). |

Ctrl+C (or SIGTERM) closes the connection and the listener cleanly. Press it twice to stop immediately.

## Routing the client
Hablog no longer edits `/etc/hosts`. You decide how the client's connection reaches the listener, and undo it when you're done.

The simplest way is a hosts entry that you add and remove yourself. Look up the server's real address first, because once the entry exists the game host resolves to `127.0.0.1`:

```bash
dig +short game-us.habbo.com
```

Add `127.0.0.1 game-us.habbo.com` to `/etc/hosts`, then give Hablog the address you looked up:

```bash
cargo run --release -- --upstream <real IP>:38101
```

If you forget, Hablog refuses to start rather than connecting to itself. While the entry is in place the client only works with Hablog running, so remove it afterwards. Any other redirect (a firewall rule, a client you can point at another host) works the same way.

## Connections
* One client is relayed at a time. A second client is closed while one is active, and a new client is accepted after the current one disconnects, so you can relog without restarting Hablog.
* Half-closes are forwarded after pending bytes, so the last packets in either direction aren't lost.
* Connecting to the game server times out after 5 seconds per address, and errors are logged instead of crashing the proxy.
* Logging never slows forwarding down. If the log falls behind, chunks are left out of it and the count is reported.

## Logs
Every line on the console is timestamped. Packets look like `[In][header][name][body]` for server to client and `[Out][header][name][body]` for client to server. Packets are reassembled from the TCP stream, so packets split across reads or sharing one read are logged correctly. Habbo encrypts client to server traffic after the handshake. When that direction stops parsing, Hablog says so once and keeps forwarding and capturing it.

Each client session is also captured to its own file, for example `logs/hablog-20261030-181502-1.jsonl`. Existing files are never overwritten. The first line is a header, then one line per TCP chunk with the exact bytes as hex:

```json
{"format":"hablog-capture","version":1,"record_kind":"tcp_chunk","client":"127.0.0.1:52144","upstream":"1.2.3.4:38101","started_ms":1793358835100}
{"index":1,"timestamp_ms":1793358835100,"direction":"client_to_server","bytes":"0000000607980000..."}
```

A row is a TCP read, not necessarily one complete packet.

## Project Structure
* `main.rs`: Command-line options, the console log and per-session captures, and shutdown on Ctrl+C/SIGTERM.
* `proxy.rs`: The relay. Accepts the client, connects to the game server, forwards both directions and reports events (listening, connected, data, disconnected, errors).
* `capture.rs`: Writes the JSONL capture files.
* `model.rs`: Traffic direction and capture records.
* `packet_handler/packet_handler.rs`: Fetches packet names and reassembles each direction's TCP stream into packets.
* `packet_handler/packet.rs`: The Packet struct and its byte readers.
* `logger.rs`: Console output.

## Tests

```bash
cargo test
```

The loopback tests cover exact forwarding in both directions, half-closes, reconnects, rejecting a second client, shutdown, the loopback-only listener, the self-connection check and a full log queue.

# Limited Example


https://github.com/Libaration/Hablog/assets/11550216/e01ad50f-07e5-4dbf-842e-d4c14cbec4cc

# Packet Debugging


https://github.com/Libaration/Hablog/assets/11550216/8865c60e-22a4-442c-9528-f77280a6472a
