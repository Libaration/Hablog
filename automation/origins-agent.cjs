/* Read-only native-boundary observer. No game functions are called or replaced. */
(function () {
  'use strict';

  const MAX_CAPTURE_BYTES = 1024 * 1024;
  const OPERATIONS = Object.freeze([
    { name: 'encrypt_payload', rva: 0xd270, direction: 'client_to_server', kind: 'payload', counter: 0x118, plaintext: 'input' },
    { name: 'encrypt_header', rva: 0xd110, direction: 'client_to_server', kind: 'header', counter: 0x120, plaintext: 'input' },
    { name: 'decrypt_payload', rva: 0xcfd0, direction: 'server_to_client', kind: 'payload', counter: 0x128, plaintext: 'output' },
    { name: 'decrypt_header', rva: 0xce90, direction: 'server_to_client', kind: 'header', counter: 0x130, plaintext: 'output' },
  ]);

  function hex(bytes) {
    if (bytes === null) return null;
    return Array.from(new Uint8Array(bytes), x => x.toString(16).padStart(2, '0')).join('');
  }

  function pathKey(value) {
    return value.replace(/\//g, '\\').replace(/^\\\\\?\\/, '').toLowerCase();
  }

  function signatureMatches(actual, expected, mask) {
    if (typeof expected !== 'string' || expected.length !== 32 || !/^[0-9a-f]{32}$/.test(expected)) return false;
    if (mask === undefined) mask = 'ff'.repeat(16);
    if (typeof mask !== 'string' || !/^[0-9a-f]{32}$/.test(mask)) return false;
    const bytes = new Uint8Array(actual);
    if (bytes.length !== 16) return false;
    for (let i = 0; i < 16; i++) {
      const significant = parseInt(mask.slice(i * 2, i * 2 + 2), 16);
      const expectedByte = parseInt(expected.slice(i * 2, i * 2 + 2), 16);
      if ((bytes[i] & significant) !== (expectedByte & significant)) return false;
    }
    return true;
  }

  function readString(object, capture = true, limit = MAX_CAPTURE_BYTES) {
    if (!Number.isInteger(limit) || limit < 0 || limit > MAX_CAPTURE_BYTES) throw new Error('Invalid capture limit');
    if (object.isNull()) throw new Error('Null string object');
    const length = object.add(16).readU32();
    const capacity = object.add(20).readU32();
    if (length > capacity || (capacity < 16 && length > 15)) throw new Error('Invalid MSVC x86 string layout');
    const result = { length, captured: capture ? Math.min(length, limit) : 0, truncated: capture && length > limit, hex: null };
    if (!capture) return result;
    if (length === 0) {
      result.hex = '';
      return result;
    }
    const data = capacity < 16 ? object : object.readPointer();
    if (data.isNull()) throw new Error('Null string data');
    result.hex = hex(data.readByteArray(result.captured));
    return result;
  }

  function readEndpoint(address, length) {
    if (address.isNull() || length < 8) return null;
    const family = address.readU16();
    const portBytes = new Uint8Array(address.add(2).readByteArray(2));
    const port = (portBytes[0] << 8) | portBytes[1];
    if (family === 2 && length >= 16) {
      const ip = Array.from(new Uint8Array(address.add(4).readByteArray(4))).join('.');
      return { family: 'ipv4', host: ip, port };
    }
    if (family === 23 && length >= 28) {
      const bytes = new Uint8Array(address.add(8).readByteArray(16));
      const groups = [];
      for (let i = 0; i < 16; i += 2) groups.push(((bytes[i] << 8) | bytes[i + 1]).toString(16));
      return { family: 'ipv6', host: groups.join(':'), port, scope_id: address.add(24).readU32() };
    }
    return null;
  }

  // Pure helpers are exported only in the Node fixture environment.
  if (typeof module !== 'undefined') module.exports = { OPERATIONS, hex, pathKey, signatureMatches, readString, readEndpoint };
  if (typeof Process === 'undefined' || typeof rpc === 'undefined') return;

  let configured = false;
  let observer = null;
  let captureMode = 'full';
  let captureLimit = MAX_CAPTURE_BYTES;
  let modulePath = '';
  let profile = null;
  let nativeModule = null;
  let nextSession = 1;
  const sessions = new Map();
  const listeners = [];
  const hookedSockets = new Set();
  const queue = [];
  let dropped = 0;
  let packetIndex = 0;

  function publish(event) {
    event.pid = Process.id;
    event.timestamp_ms = Date.now();
    if (queue.length >= 128) {
      dropped++;
      return;
    }
    queue.push(event);
  }

  function flush() {
    if (dropped > 0) {
      send({ type: 'dropped', count: dropped, pid: Process.id });
      dropped = 0;
    }
    if (queue.length > 0) send({ type: 'batch', events: queue.splice(0) });
  }

  const flushTimer = setInterval(flush, 50);

  function sessionId(context) {
    const key = context.toString();
    if (!sessions.has(key)) sessions.set(key, nextSession++);
    return sessions.get(key);
  }

  function attachCrypto(module) {
    if (nativeModule !== null) return;
    // The client file hash was already verified as the supported build before
    // launch, so a path or in-memory signature difference (ASLR relocations,
    // loader fix-ups) is reported but does not abort hooking the verified RVAs.
    const diagnostics = [];
    if (pathKey(module.path) !== pathKey(modulePath)) {
      diagnostics.push('path=' + module.path);
    }
    for (const operation of OPERATIONS) {
      const signature = profile.signatures[operation.name];
      if (typeof signature !== 'string' || signature.length !== 32) throw new Error('Missing function signature');
      const actual = module.base.add(operation.rva).readByteArray(16);
      if (!signatureMatches(actual, signature, profile.signatureMasks?.[operation.name])) {
        diagnostics.push(operation.name + ' exp ' + signature + ' got ' + hex(actual));
      }
    }
    if (diagnostics.length > 0) {
      publish({ type: 'status', state: 'signature_mismatch_proceeding', detail: diagnostics.join(' | ') });
    }
    const additions = [];
    try {
      for (const operation of OPERATIONS) {
        additions.push(Interceptor.attach(module.base.add(operation.rva), {
          onEnter() {
            this.observation = null;
            try {
              const context = this.context.ecx;
              // x86 __thiscall: ECX=this; [ESP+4]=out; [ESP+8]=input.
              if (context.isNull()) return;
              const ready = context.add(0x138).readU8() !== 0;
              if (!ready) return;
              const out = this.context.esp.add(4).readPointer();
              const input = this.context.esp.add(8).readPointer();
              const capture = captureMode === 'full';
              this.observation = {
                index: ++packetIndex,
                session_id: sessionId(context),
                sequence: context.add(operation.counter).readU64().toString(10),
                out,
                input: readString(input, capture, captureLimit),
                capture,
              };
            } catch (_) {
              publish({ type: 'observation_error', operation: operation.name, stage: 'entry' });
            }
          },
          onLeave() {
            const observation = this.observation;
            if (observation === null || observation === undefined) return;
            try {
              const output = readString(observation.out, observation.capture, captureLimit);
              publish({
                type: 'crypto', index: observation.index, session_id: observation.session_id,
                direction: operation.direction, kind: operation.kind, operation: operation.name,
                sequence: observation.sequence, plaintext_side: operation.plaintext,
                input_length: observation.input.length, output_length: output.length,
                input_hex: observation.input.hex, output_hex: output.hex,
                truncated: observation.input.truncated || output.truncated,
                capture_mode: observation.capture ? 'full' : 'metadata',
              });
            } catch (_) {
              publish({ type: 'observation_error', operation: operation.name, stage: 'return' });
            }
          },
        }));
      }
    } catch (error) {
      additions.forEach(listener => listener.detach());
      throw error;
    }
    listeners.push(...additions);
    nativeModule = module;
    publish({ type: 'status', state: 'observing', module_path: module.path, profile_sha256: profile.sha256 });
  }

  function attachSockets(module) {
    for (const name of ['connect', 'WSAConnect']) {
      const target = module.findExportByName(name);
      if (target === null || hookedSockets.has(target.toString())) continue;
      hookedSockets.add(target.toString());
      listeners.push(Interceptor.attach(target, {
        onEnter(args) {
          this.endpoint = null;
          try { this.endpoint = readEndpoint(args[1], args[2].toInt32()); } catch (_) { /* Unreadable endpoint is ignored. */ }
        },
        onLeave(result) {
          if (this.endpoint !== null && this.endpoint !== undefined) {
            publish({ type: 'socket_connect', api: name, endpoint: this.endpoint, result: result.toInt32() });
          }
        },
      }));
    }
  }

  rpc.exports = {
    configure(value) {
      if (configured) throw new Error('Observer is already configured');
      if (Process.platform !== 'windows' || Process.arch !== 'ia32' || Process.pointerSize !== 4) throw new Error('This profile requires a Windows x86 client');
      if (Process.codeSigningPolicy === 'required') throw new Error('Process policy does not permit native interception');
      if (!value || value.sha256 !== '72385eec7d0205c7774cc12c07e64b80df83aea4d976d1e2cccd7cbdaf036577') throw new Error('Unsupported client build');
      if (!['full', 'metadata'].includes(value.captureMode)) throw new Error('Invalid capture mode');
      if (!Number.isInteger(value.maxCaptureBytes) || value.maxCaptureBytes < 0 || value.maxCaptureBytes > MAX_CAPTURE_BYTES) throw new Error('Invalid capture size');
      profile = value;
      modulePath = value.modulePath;
      captureMode = value.captureMode;
      captureLimit = value.maxCaptureBytes;
      configured = true;
      observer = Process.attachModuleObserver({
        onAdded(module) {
          try {
            if (module.name.toLowerCase() === 'bobbaxtra.x32') attachCrypto(module);
            if (module.name.toLowerCase() === 'ws2_32.dll') attachSockets(module);
          } catch (error) {
            publish({ type: 'error', code: 'unsupported_loaded_module', message: String(error.message) });
          }
        },
        onRemoved(module) {
          if (nativeModule !== null && module.base.equals(nativeModule.base)) {
            nativeModule = null;
            publish({ type: 'status', state: 'module_unloaded' });
          }
        },
      });
      flush();
      return { pid: Process.id, state: nativeModule === null ? 'waiting_for_module' : 'observing' };
    },
    setcapturemode(mode) {
      if (!['full', 'metadata'].includes(mode)) throw new Error('Invalid capture mode');
      captureMode = mode;
      return { capture_mode: captureMode };
    },
    dispose() {
      if (observer !== null) observer.detach();
      listeners.splice(0).forEach(listener => listener.detach());
      clearInterval(flushTimer);
      flush();
      return true;
    },
  };
})();
