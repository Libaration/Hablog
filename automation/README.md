# Origins observer companion

Hablog starts this local companion through private stdin/stdout pipes whenever
`decrypt = true` in `hablog.toml` (the default) and it is set up. It launches the routed Origins client suspended through
Frida, hooks the client's own BobbaXtra encrypt/decrypt calls, resumes it, and
reports each operation's bytes so Hablog can print the decrypted traffic. The
companion does not contact a remote Frida server, change installed game files,
export session keys, replace native functions, or send captured bytes over a
network. Frida installs temporary interception code in the client process and
removes it on detach. The client's own connections stay its own.

## Setup

It needs Node.js 22 or later on the Windows machine that runs the game. The
first time, Hablog asks whether to install the dependencies for you; to do it
yourself:

```bash
cd automation
npm ci
```

Hablog looks for `host.mjs` in `observer/` next to the `hablog` executable, then
in `./automation`, or in `observer` under `[advanced]` in `hablog.toml`. It
runs it with a `node.exe` in that folder if there is one, otherwise with `node`
from PATH.

`npm test` runs the fixtures without launching or attaching to anything.
`npm run check` checks the JavaScript syntax.

## Protocol

Hablog launches `node host.mjs` with no console window and redirected
stdin/stdout, and keeps stdin open while observing. Each control and event is
one UTF-8 JSON line. The companion writes no captured payloads to files.

The first control message is:

```json
{"command":"start","config":{"clientExecutablePath":"C:\\...\\368\\Hablog-Routed.exe","modulePath":"C:\\...\\368\\Xtras\\BobbaXtra.x32","captureMode":"full","launch":true,"argv":[]}}
```

Hablog fills in the paths from discovery. With `"launch":true` the companion
launches that client suspended through Frida, reports `client_spawned`, attaches the agent, registers the
module observer, then resumes it. A client the companion spawned is resumed even
if startup fails, so it is never left suspended. Alternatively, omit `launch`
and pass a `pid`, or neither to wait for an existing process whose executable
path exactly matches the validated client.

`captureMode` defaults to `full`. `{"command":"capture_mode","mode":"metadata"}`
switches to metadata mode, which copies no input/output bytes.
`{"command":"stop"}` or closing stdin unloads the agent and detaches, leaving
the client running. Hablog sends `stop` when it exits and gives the companion
five seconds to exit before ending the helper process (never the game).

Hablog uses the status events to follow the game: an error before
`client_spawned` means no game was started, so Hablog starts it normally
instead, and `detached` with reason `process-terminated` means the game closed,
so Hablog stops too.

An observation event has this schema (the data below is only a fixture):

```json
{"type":"crypto","index":1,"pid":1234,"timestamp_ms":0,"session_id":1,"operation":"encrypt_payload","direction":"client_to_server","kind":"payload","sequence":"0","plaintext_side":"input","input_length":2,"output_length":3,"input_hex":"0041","output_hex":"414243","truncated":false,"capture_mode":"full"}
```

`sequence` is a decimal string to preserve all 64 bits. The input/output hex
contains the raw native `std::string` bytes: for encryption the input is
plaintext and the output is unpadded Base64; for decryption the sides are
reversed. This is a crypto-operation boundary, not an asserted complete game
frame. Header and payload operations have separate streams and sequences.

`maxCaptureBytes` optionally bounds each captured side, from zero to 1 MiB,
default 1 MiB. Truncated events keep the original lengths and set
`truncated:true`. Events buffer at most 128 operations between 50 ms flushes;
overflow emits `type:dropped` with a count. Socket connect observations include
the destination and raw native result; a failed or pending connect is not
reported as established. The ordinary `connect` and `WSAConnect` exports are
covered; other socket APIs remain unverified.

Status events include `companion_ready`, `waiting_for_client`,
`client_spawned`, `waiting_for_module`, `observing`, `signature_mismatch_proceeding`,
`client_resumed`, `module_unloaded`, and `detached`. Errors are JSON events.

## Supported build

Only Windows x86 BobbaXtra with SHA-256
`72385eec7d0205c7774cc12c07e64b80df83aea4d976d1e2cccd7cbdaf036577`
is supported. Before launching anything, the companion verifies that the
selected executable is an x86 PE, that the module is the selected installation's
`Xtras/BobbaXtra.x32`, and that the complete module file matches that hash; any
other build is refused without installing hooks. Inside the client, the agent
checks the architecture, then compares the loaded module path and the four
function prologs (with PE HIGHLOW relocation bytes masked for ASLR). The
projector loads its Xtras through 8.3 short names (`HABBOL~1`), so the loaded
path is expanded with `GetLongPathNameW` before it is compared. Because the
file hash was already verified, a difference there is reported as
`signature_mismatch_proceeding` and the verified RVAs are still hooked.

The four RVAs from the static investigation are `d270` (outgoing payload),
`d110` (outgoing header), `cfd0` (incoming payload), and `ce90` (incoming
header). The profile reads the x86 MSVC 24-byte string layout, the ECX session
context, the ready flag at `+138`, and the 64-bit sequence counters at `+118`,
`+120`, `+128`, `+130`. No shared-secret memory is read. Application plaintext
can include sensitive data such as your password; Hablog only prints it.

The fixtures in `test/` exercise the agent against simulated memory and
callbacks. They do not launch or attach to a client.

## Dependencies

The pinned npm dependency tree and package integrity hashes are in
`package-lock.json`. Frida's Node bindings are
[frida-node](https://github.com/frida/frida-node) (LGPL-2.0 WITH
WxWindows-exception-3.1); see [Frida's API](https://frida.re/docs/javascript-api/).
