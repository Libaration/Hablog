# Hablog
This project is a simple TCP proxy implemented in Rust using the Tokio asynchronous runtime.
It relays traffic between the Habbo client and the game server and logs the packets in both directions, with names for known headers. It also supports Habbo Origins (see below).
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

## Habbo Origins
Hablog also relays the Origins client (Windows, Shockwave build 368). It reads the connection details from the installed client itself, so you don't type in any addresses.

```bash
cargo run --release -- origins-details
```

prints the Habbo Launcher, the installed version, and for each region (`hhous`, `hhobr`, `hhoes`, `hhod`) the game and MUS host:port embedded in that region's client, plus its site and external variables URLs and any client that is already running. `--json` prints everything, including the byte offsets the addresses were read from.

```bash
cargo run --release -- origins
```

1. Discovers the client the launcher last selected (or the region given with `--server hhobr`).
2. Starts the relay on `127.0.0.1:<game port>`, forwarding to the real game host.
3. Writes `Hablog-Routed.exe` next to the installed client: a copy whose game host is replaced with `127.0.0.1`. The installed client, the launcher and the hosts file are never changed. The MUS connection is not redirected.
4. Launches that copy. Use `--no-launch` to start it yourself. On macOS and Linux it's only written, since the client runs on Windows.

Close any Origins client you already have open first, since it is connected directly. The installation is found through `%APPDATA%` and `%LOCALAPPDATA%`; `--appdata` and `--local-appdata` point it elsewhere, for example into a Wine prefix. Logging and capture work as for Habbo, but Origins traffic is encrypted, so the console shows each TCP chunk instead of packets.

### Decrypted traffic
`--observe` launches the routed client through the Frida observer in [automation/](automation/README.md). It reads the client's own encrypt/decrypt calls and Hablog prints each decrypted operation:

```
18:15:02.114 [In][payload #12][...]
```

It needs Node.js 22 or later and a one-time `npm ci` in `automation/`. It supports only the BobbaXtra build it was written for, and refuses any other build without installing hooks. Decrypted bytes are only printed, never written to disk, because they include what you type, such as your password.

### Decrypting by hand
With your own session's shared key (56 bytes as 112 hex characters, from the client's `Crypto_GetSharedKeyHex`) in a file:

```bash
cargo run --release -- origins-decrypt --key-file my.secret --direction in --stream payload --sequence 12 <base64 wire value>
```

`--direction` is `in` (server to client) or `out`, `--stream` is `header` or `payload`, and `--sequence` is that stream's counter, starting at 0 for each session. `--hex` takes hex instead of Base64. `origins-encrypt` does the reverse from hex plaintext. The key can also come from the `HABLOG_SESSION_KEY` environment variable. Files ending in `.secret` are ignored by git.

The cipher has no authentication tag, so a wrong key, stream or counter still produces output; it just isn't readable. The key derivation follows static analysis of BobbaXtra build 368 and is checked against the RFC 5869/8439 test vectors and a fixed four-stream fixture, not against a live session.

## Project Structure
* `main.rs`: The Habbo and Origins commands, the console log and per-session captures, and shutdown on Ctrl+C/SIGTERM.
* `cli.rs`: Command-line options.
* `proxy.rs`: The relay. Accepts the client, connects to the game server, forwards both directions and reports events (listening, connected, data, disconnected, errors).
* `capture.rs`: Writes the JSONL capture files.
* `model.rs`: Traffic direction and capture records.
* `packet_handler/packet_handler.rs`: Fetches packet names and reassembles each direction's TCP stream into packets.
* `packet_handler/packet.rs`: The Packet struct and its byte readers.
* `logger.rs`: Console output.
* `origins/discovery.rs`: Finds the launcher, installed versions, regional clients and their embedded endpoints, and running clients.
* `origins/routing.rs`: Writes the routed copy of the client.
* `origins/crypto.rs`: The Origins session decoder.
* `origins/observer.rs`: Runs the Frida observer in `automation/` and reads its events.

## Tests

```bash
cargo test
```

The loopback tests cover exact forwarding in both directions, half-closes, reconnects, rejecting a second client, shutdown, the loopback-only listener, the self-connection check and a full log queue. The Origins tests cover discovery against fixture installations, routing, the crypto test vectors and observer events. The observer has its own tests:

```bash
cd automation && node --test test/*.test.mjs
```

# Limited Example


https://github.com/Libaration/Hablog/assets/11550216/e01ad50f-07e5-4dbf-842e-d4c14cbec4cc

# Packet Debugging


https://github.com/Libaration/Hablog/assets/11550216/8865c60e-22a4-442c-9528-f77280a6472a
