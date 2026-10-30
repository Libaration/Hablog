# Task: send messages to the game

**Status: to do.** `hablog build` makes exact messages, but Hablog can't send them yet.

## Goal

During a live session, send a message built with `hablog build` to the game as if the server had sent it, to see what it does. Only the game on your own computer receives it; nothing reaches Habbo's server.

Sending messages to the server is out of scope. The server acts on what it receives, including against your account, and it would need the same work in the other direction.

## What we know

The observer's decrypted messages from a real Origins 371 session line up byte for byte with the TCP chunks the relay saw. After the unencrypted handshake (`@@`, `CN`, `DU`, `CJ`, `@A`), every message travels as one encrypted frame:

| Part | Plaintext | On the wire |
|---|---|---|
| Frame header | 4 bytes: one byte of unknown purpose, then the length of the payload's Base64 as 3 B64 characters | Ciphertext as unpadded Base64: always 6 characters |
| Payload | The message itself, e.g. `DA…[2][1]` | Ciphertext as unpadded Base64: 4n/3 characters, rounded up |

From that session:

| Message | Bytes | Frame header | Wire chunk |
|---|---|---|---|
| `OUT @E` (game data URL) | 64 | `[8]@AV`: `@AV` = 86 | 92 bytes = 6 + 86 |
| `IN DA` (session parameters) | 29 | `Q@@g`: `@@g` = 39 | 45 bytes = 6 + 39 |
| `OUT @Z` (`club_habbo`) | 14 | `Z@@S`: `@@S` = 19 | 25 bytes = 6 + 19 |

A frame can span TCP chunks: one large server message arrived as 1448 + 1448 + 290 bytes.

Each direction encrypts frame headers and payloads as two separate streams. Each stream has its own counter, which goes up by one per frame. `src/origins/crypto.rs` implements the cipher: HKDF-SHA256 from the session's 56-byte shared key, then ChaCha20 with the stream's base nonce plus the counter. It comes from static analysis of BobbaXtra and passes the standard test vectors, but hasn't been checked against a live session yet.

## The problem: counters

The game decrypts the next server frame with the next counter on each server stream. An added frame uses up a counter value, so every real frame after it would arrive one counter behind. The game would then fail to decrypt everything that follows, and the session would break.

## Approaches

### A. Re-encrypting relay (recommended)

From the first injected frame on, Hablog decrypts each server frame with the server's counter and re-encrypts it with the game's counter. The game's counter is ahead by the number of frames Hablog has added. This needs:

1. **The session's shared key.** The client has a `Crypto_GetSharedKeyHex` method, so the observer could call it once the session is ready and pass the key to Hablog over its existing pipe. The observer is read-only today, and its README promises it reads no secrets. This change has to be deliberate, documented, and keep the key in memory only: never printed, logged or saved.
2. **A frame parser in the relay.** It reads the 6-character header, decrypts it, takes the payload length, and gathers the payload across TCP chunks.
3. **Counters for each of the four streams**, kept on both sides of the relay, with `crypto.rs` doing the work.
4. **A way to send.** The relay needs a command channel into the live session, like the original Axed relay's `ProxyHandle::send`. A `send` prompt in Hablog would use the same syntax as `hablog build`, and later the GUI would use it too.

Once the key works, Hablog can decrypt the wire itself and no longer depends on the observer's per-operation events, which can be dropped under load.

### B. Splice into the game's own decrypt

The observer already intercepts the game's `decrypt_payload`. It could append the new message to the decrypted text of the next real server message, since one payload can hold several messages, each ending in `[1]`. This needs no key and no counters. But it means growing a C++ string inside the game, which needs the game's own memory allocator, and a mistake crashes the game. The message also only arrives with the next server message. This is the fallback if the key can't be obtained.

## Steps

- [ ] Get the shared key from a live session, held in memory only.
- [ ] Prove the key and `crypto.rs`: decrypt captured frames with `crypto.rs` and compare them with the observer's plaintext for the same counters.
- [ ] Parse frames in the relay, in both directions and across TCP chunks.
- [ ] Find out what the first frame-header byte is (random? a checksum?) and whether the game checks it. Try reusing the byte from a real frame first.
- [ ] Re-encrypt the server-to-game streams with the offset counter after the first injected frame.
- [ ] Add the `send` prompt, using `hablog build` syntax, which queues the message into the relay.
- [ ] Add a test session with a known key, proving that frames after an injected one still decrypt in the game's order.

## Open questions

- What is the first frame-header byte, and does the game reject a frame when it's wrong?
- Does `Crypto_GetSharedKeyHex` return the key the derivation in `crypto.rs` expects? The first proof step answers this.
- Does the game need anything beyond the frame itself, such as messages arriving in an order the server would use?
