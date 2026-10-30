import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import vm from 'node:vm';
import test from 'node:test';
import { fileOffsetForRva, signatureMaskForRva, FUNCTION_RVAS, SUPPORTED_SHA256 } from '../profile.mjs';

const require = createRequire(import.meta.url);
const { OPERATIONS, pathKey, signatureMatches, readString, readEndpoint } = require('../origins-agent.cjs');
const agentSource = readFileSync(new URL('../origins-agent.cjs', import.meta.url), 'utf8');

class Memory {
  constructor(size = 0x40000) {
    this.bytes = new Uint8Array(size);
    this.view = new DataView(this.bytes.buffer);
    this.blockByteReads = false;
  }
  ptr(address) { return new Pointer(this, address); }
  u32(address, value) { this.view.setUint32(address, value, true); }
  string(address, bytes, heap = 0x30000) {
    this.u32(address + 16, bytes.length);
    this.u32(address + 20, bytes.length <= 15 ? 15 : bytes.length + 16);
    if (bytes.length <= 15) this.bytes.set(bytes, address);
    else { this.u32(address, heap); this.bytes.set(bytes, heap); }
    return this.ptr(address);
  }
}

class Pointer {
  constructor(memory, address) { this.memory = memory; this.address = address; }
  add(count) { return new Pointer(this.memory, this.address + count); }
  isNull() { return this.address === 0; }
  equals(other) { return other.address === this.address; }
  toString() { return `0x${this.address.toString(16)}`; }
  readU8() { return this.memory.view.getUint8(this.address); }
  readU16() { return this.memory.view.getUint16(this.address, true); }
  readU32() { return this.memory.view.getUint32(this.address, true); }
  readU64() { return this.memory.view.getBigUint64(this.address, true); }
  readPointer() { return this.memory.ptr(this.readU32()); }
  readByteArray(length) {
    assert.equal(this.memory.blockByteReads, false, 'metadata mode read payload memory');
    if (this.address + length > this.memory.bytes.length) throw new Error('unmapped');
    return this.memory.bytes.buffer.slice(this.address, this.address + length);
  }
}

test('native small-string storage preserves binary bytes including NUL', () => {
  const memory = new Memory();
  const pointer = memory.string(0x100, new Uint8Array([0, 1, 0xff, 65]));
  assert.deepEqual(readString(pointer), { length: 4, captured: 4, truncated: false, hex: '0001ff41' });
  assert.equal(readString(memory.string(0x140, new Uint8Array(15).fill(0xaa))).hex, 'aa'.repeat(15));
});

test('16-byte native strings use the heap pointer and can be truncated explicitly', () => {
  const memory = new Memory();
  const pointer = memory.string(0x100, new Uint8Array(16).fill(0xbb));
  assert.equal(readString(pointer).hex, 'bb'.repeat(16));
  assert.deepEqual(readString(pointer, true, 3), { length: 16, captured: 3, truncated: true, hex: 'bbbbbb' });
});

test('metadata mode reads only string metadata', () => {
  const memory = new Memory();
  const pointer = memory.string(0x100, new Uint8Array(64));
  memory.blockByteReads = true;
  assert.deepEqual(readString(pointer, false), { length: 64, captured: 0, truncated: false, hex: null });
});

test('invalid string layout and capture limits are rejected before copying', () => {
  const memory = new Memory();
  const pointer = memory.string(0x100, new Uint8Array(1));
  memory.u32(0x110, 20);
  assert.throws(() => readString(pointer), /layout/);
  assert.throws(() => readString(memory.ptr(0)), /Null/);
  assert.throws(() => readString(pointer, true, -1), /limit/);
  assert.throws(() => readString(pointer, true, 1024 * 1024 + 1), /limit/);
});

test('empty heap string does not dereference its data pointer', () => {
  const memory = new Memory();
  memory.u32(0x114, 31);
  assert.equal(readString(memory.ptr(0x100)).hex, '');
});

test('sockaddr decoding uses network-order ports for IPv4 and IPv6', () => {
  const memory = new Memory();
  memory.bytes.set([2, 0, 0x75, 0x31, 127, 0, 0, 1], 0x100);
  assert.deepEqual(readEndpoint(memory.ptr(0x100), 16), { family: 'ipv4', host: '127.0.0.1', port: 30001 });
  memory.bytes.set([23, 0, 0x01, 0xbb], 0x200);
  memory.bytes.set([0x20, 0x01, 0x0d, 0xb8], 0x208);
  memory.bytes[0x217] = 1;
  memory.u32(0x218, 4);
  assert.deepEqual(readEndpoint(memory.ptr(0x200), 28), { family: 'ipv6', host: '2001:db8:0:0:0:0:0:1', port: 443, scope_id: 4 });
  assert.equal(readEndpoint(memory.ptr(0x100), 4), null);
});

function simulatedAgent({ architecture = 'ia32', loadedPath = 'C:\\client\\Xtras\\BobbaXtra.x32', longPaths = {} } = {}) {
  const memory = new Memory();
  const hooks = new Map();
  const messages = [];
  let tick = () => {};
  const nativeModule = { name: 'BobbaXtra.x32', path: loadedPath, base: memory.ptr(0x1000) };
  const sandbox = {
    rpc: { exports: {} }, Uint8Array, Map, Set, Array, Object, Number, String, Date,
    Process: {
      id: 987, platform: 'windows', arch: architecture, pointerSize: architecture === 'ia32' ? 4 : 8,
      codeSigningPolicy: 'optional',
      attachModuleObserver(observer) { observer.onAdded(nativeModule); return { detach() {} }; },
      getModuleByName(name) { return { getExportByName: exportName => `${name}!${exportName}` }; },
    },
    // GetLongPathNameW, answering from `longPaths` (0 when there is no long form).
    NativeFunction: function (address, returnType, argumentTypes, abi) {
      assert.equal(address, 'kernel32.dll!GetLongPathNameW');
      assert.equal(abi, 'stdcall');
      return (input, output) => { output.value = longPaths[input.value] ?? ''; return output.value.length; };
    },
    Memory: {
      allocUtf16String(value) { return { value }; },
      alloc() { return { value: '', readUtf16String(length) { return this.value.slice(0, length); } }; },
    },
    Interceptor: { attach(address, callbacks) { hooks.set(address.address - 0x1000, callbacks); return { detach() {} }; } },
    send(message) { messages.push(message); },
    setInterval(callback) { tick = callback; return 1; }, clearInterval() {},
  };
  vm.runInNewContext(agentSource, sandbox);
  const configure = mode => sandbox.rpc.exports.configure({
    sha256: SUPPORTED_SHA256, modulePath: 'C:\\client\\Xtras\\BobbaXtra.x32',
    signatures: Object.fromEntries(OPERATIONS.map(operation => [operation.name, '00'.repeat(16)])),
    captureMode: mode, maxCaptureBytes: 1024 * 1024,
  });
  return { memory, hooks, messages, configure, rpc: sandbox.rpc.exports, tick: () => tick() };
}

test('four supported thiscall hooks capture both native boundaries and exact u64 sequence', () => {
  const agent = simulatedAgent();
  agent.configure('full');
  assert.equal(agent.hooks.size, 4);
  assert.deepEqual([...agent.hooks.keys()].sort(), Object.values(FUNCTION_RVAS).sort());
  const { memory } = agent;
  memory.bytes[0x21138] = 1;
  memory.view.setBigUint64(0x21118, 9007199254740993n, true);
  memory.u32(0x20004, 0x22100);
  memory.u32(0x20008, 0x22000);
  memory.string(0x22000, new TextEncoder().encode('dummy plaintext'));
  memory.string(0x22100, new TextEncoder().encode('dummy wire'));
  const invocation = { context: { ecx: memory.ptr(0x21000), esp: memory.ptr(0x20000) } };
  const callbacks = agent.hooks.get(0xd270);
  callbacks.onEnter.call(invocation);
  callbacks.onLeave.call(invocation);
  agent.tick();
  const event = agent.messages.flatMap(message => message.events ?? []).find(event => event.type === 'crypto');
  assert.equal(event.sequence, '9007199254740993');
  assert.equal(event.direction, 'client_to_server');
  assert.equal(event.kind, 'payload');
  assert.equal(event.input_hex, Buffer.from('dummy plaintext').toString('hex'));
  assert.equal(event.output_hex, Buffer.from('dummy wire').toString('hex'));
  assert.equal(event.session_id, 1);
});

test('runtime metadata control omits both payloads and still reports sizes', () => {
  const agent = simulatedAgent();
  agent.configure('full');
  agent.rpc.setcapturemode('metadata');
  const { memory } = agent;
  memory.bytes[0x21138] = 1;
  memory.u32(0x20004, 0x22100);
  memory.u32(0x20008, 0x22000);
  memory.string(0x22000, new Uint8Array(32));
  memory.string(0x22100, new Uint8Array(24), 0x31000);
  memory.blockByteReads = true;
  const invocation = { context: { ecx: memory.ptr(0x21000), esp: memory.ptr(0x20000) } };
  agent.hooks.get(0xcfd0).onEnter.call(invocation);
  agent.hooks.get(0xcfd0).onLeave.call(invocation);
  agent.tick();
  const event = agent.messages.flatMap(message => message.events ?? []).find(event => event.type === 'crypto');
  assert.equal(event.input_length, 32);
  assert.equal(event.output_length, 24);
  assert.equal(event.input_hex, null);
  assert.equal(event.output_hex, null);
  assert.equal(event.plaintext_side, 'output');
});

test('architecture mismatch installs no hooks; a loaded-module path mismatch is reported', () => {
  const wrongArch = simulatedAgent({ architecture: 'x64' });
  assert.throws(() => wrongArch.configure('full'), /Windows x86/);
  assert.equal(wrongArch.hooks.size, 0);
  // The module file hash is verified before launch, so a differing loaded path
  // is reported and the verified RVAs are still hooked.
  const wrongPath = simulatedAgent({ loadedPath: 'C:\\unrelated\\BobbaXtra.x32' });
  wrongPath.configure('full');
  assert.equal(wrongPath.hooks.size, 4);
  assert.ok(wrongPath.messages.flatMap(message => message.events ?? []).some(event =>
    event.state === 'signature_mismatch_proceeding' && event.detail.includes('path=C:\\unrelated\\BobbaXtra.x32')));
  assert.equal(pathKey('C:/CLIENT/Xtras/BobbaXtra.x32'), pathKey('c:\\client\\xtras\\bobbaxtra.x32'));
});

test('8.3 short module paths are expanded before comparing', () => {
  const shortPath = 'C:\\Users\\me\\HABBOL~1\\DOWNLO~1\\SHOCKW~1\\371\\xtras\\BobbaXtra.x32';
  const agent = simulatedAgent({ loadedPath: shortPath, longPaths: { [shortPath]: 'C:\\client\\Xtras\\BobbaXtra.x32' } });
  agent.configure('full');
  assert.equal(agent.hooks.size, 4);
  const events = agent.messages.flatMap(message => message.events ?? []);
  assert.ok(!events.some(event => event.state === 'signature_mismatch_proceeding'), JSON.stringify(events));
  assert.ok(events.some(event => event.state === 'observing'));
});

function peFixture() {
  const bytes = Buffer.alloc(0x400);
  bytes.writeUInt16LE(0x5a4d, 0);
  bytes.writeUInt32LE(0x80, 0x3c);
  bytes.writeUInt32LE(0x4550, 0x80);
  bytes.writeUInt16LE(0x14c, 0x84);
  bytes.writeUInt16LE(1, 0x86);
  bytes.writeUInt16LE(0xe0, 0x94);
  bytes.writeUInt16LE(0x10b, 0x98);
  bytes.writeUInt32LE(0x1000, 0x178 + 12);
  bytes.writeUInt32LE(0x200, 0x178 + 16);
  bytes.writeUInt32LE(0x200, 0x178 + 20);
  return bytes;
}

test('PE signature mapping is based on file-backed RVAs with bounds and architecture checks', () => {
  assert.equal(fileOffsetForRva(peFixture(), 0x1010, 16), 0x210);
  assert.throws(() => fileOffsetForRva(peFixture(), 0x11ff, 16), /file-backed/);
  assert.throws(() => fileOffsetForRva(Buffer.alloc(8), 0), /DOS/);
  const wrongCpu = peFixture();
  wrongCpu.writeUInt16LE(0x8664, 0x84);
  assert.throws(() => fileOffsetForRva(wrongCpu, 0x1000), /x86/);
});

test('ASLR masks only HIGHLOW relocation bytes and still rejects patched instructions', () => {
  const bytes = peFixture();
  bytes.writeUInt32LE(0x1100, 0x98 + 96 + 5 * 8);
  bytes.writeUInt32LE(12, 0x98 + 96 + 5 * 8 + 4);
  bytes.writeUInt32LE(0x1000, 0x300);
  bytes.writeUInt32LE(12, 0x304);
  bytes.writeUInt16LE(0x3016, 0x308);
  const mask = signatureMaskForRva(bytes, 0x1010).toString('hex');
  assert.equal(mask, 'ff'.repeat(6) + '00'.repeat(4) + 'ff'.repeat(6));
  const loaded = new Uint8Array(16);
  loaded.set([0xaa, 0xbb, 0xcc, 0xdd], 6);
  assert.equal(signatureMatches(loaded.buffer, '00'.repeat(16), mask), true);
  loaded[0] = 0xe9;
  assert.equal(signatureMatches(loaded.buffer, '00'.repeat(16), mask), false);
});
