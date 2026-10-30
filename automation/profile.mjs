import { createHash } from 'node:crypto';
import { readFile, realpath } from 'node:fs/promises';
import path from 'node:path';

export const SUPPORTED_SHA256 = '72385eec7d0205c7774cc12c07e64b80df83aea4d976d1e2cccd7cbdaf036577';
export const FUNCTION_RVAS = Object.freeze({ encrypt_payload: 0xd270, encrypt_header: 0xd110, decrypt_payload: 0xcfd0, decrypt_header: 0xce90 });

export function normalizedPath(value) {
  return path.resolve(value).replace(/^\\\\\?\\/, '').replace(/\//g, '\\').toLowerCase();
}

export function fileOffsetForRva(pe, rva, count = 1) {
  if (!Buffer.isBuffer(pe) || pe.length < 64 || pe.readUInt16LE(0) !== 0x5a4d) throw new Error('Not a DOS/PE image');
  const header = pe.readUInt32LE(0x3c);
  if (header + 24 > pe.length || pe.readUInt32LE(header) !== 0x4550) throw new Error('Invalid PE header');
  if (pe.readUInt16LE(header + 4) !== 0x14c) throw new Error('This profile requires an x86 PE image');
  const sections = pe.readUInt16LE(header + 6);
  const optionalSize = pe.readUInt16LE(header + 20);
  if (header + 24 + optionalSize > pe.length || pe.readUInt16LE(header + 24) !== 0x10b) throw new Error('Invalid PE32 optional header');
  const table = header + 24 + optionalSize;
  if (table + sections * 40 > pe.length) throw new Error('Truncated PE section table');
  for (let i = 0; i < sections; i++) {
    const section = table + i * 40;
    const start = pe.readUInt32LE(section + 12);
    const rawSize = pe.readUInt32LE(section + 16);
    const rawStart = pe.readUInt32LE(section + 20);
    if (rva >= start && rva + count <= start + rawSize) {
      const offset = rawStart + rva - start;
      if (offset + count > pe.length) throw new Error('Truncated PE section bytes');
      return offset;
    }
  }
  throw new Error('Function RVA is not present in the file-backed PE sections');
}

export function signatureMaskForRva(pe, rva, count = 16) {
  const mask = Buffer.alloc(count, 0xff);
  const optional = pe.readUInt32LE(0x3c) + 24;
  const relocationRva = pe.readUInt32LE(optional + 96 + 5 * 8);
  const relocationSize = pe.readUInt32LE(optional + 96 + 5 * 8 + 4);
  if (relocationRva === 0 || relocationSize === 0) return mask;
  const start = fileOffsetForRva(pe, relocationRva, relocationSize);
  const end = start + relocationSize;
  for (let block = start; block < end;) {
    if (block + 8 > end) throw new Error('Truncated PE relocation block');
    const pageRva = pe.readUInt32LE(block);
    const blockSize = pe.readUInt32LE(block + 4);
    if (blockSize < 8 || blockSize % 2 !== 0 || block + blockSize > end) throw new Error('Invalid PE relocation block');
    for (let entry = block + 8; entry < block + blockSize; entry += 2) {
      const value = pe.readUInt16LE(entry);
      const type = value >>> 12;
      const target = pageRva + (value & 0xfff);
      if (type === 0 || target >= rva + count || target + 4 <= rva) continue;
      if (type !== 3) throw new Error('Unsupported relocation in native function signature');
      // HIGHLOW relocations change four absolute-address bytes under ASLR.
      mask.fill(0, Math.max(0, target - rva), Math.min(count, target + 4 - rva));
    }
    block += blockSize;
  }
  return mask;
}

export async function verifyProfile(config) {
  if (!config || typeof config.clientExecutablePath !== 'string' || typeof config.modulePath !== 'string') throw new Error('An automatically discovered executable and module path are required');
  const executable = await realpath(config.clientExecutablePath);
  const modulePath = await realpath(config.modulePath);
  const expectedModule = await realpath(path.join(path.dirname(executable), 'Xtras', 'BobbaXtra.x32'));
  if (normalizedPath(modulePath) !== normalizedPath(expectedModule)) throw new Error('Module is outside the selected client installation');
  const executableBytes = await readFile(executable);
  fileOffsetForRva(executableBytes, 0x1000, 1); // Reject non-x86/non-PE executables before attaching.
  const bytes = await readFile(modulePath);
  const digest = createHash('sha256').update(bytes).digest('hex');
  if (digest !== SUPPORTED_SHA256) throw new Error(`Unsupported BobbaXtra build (${digest}); no hooks were installed`);
  const signatures = Object.fromEntries(Object.entries(FUNCTION_RVAS).map(([name, rva]) => {
    const offset = fileOffsetForRva(bytes, rva, 16);
    return [name, bytes.subarray(offset, offset + 16).toString('hex')];
  }));
  const signatureMasks = Object.fromEntries(Object.entries(FUNCTION_RVAS).map(([name, rva]) => [name, signatureMaskForRva(bytes, rva).toString('hex')]));
  const captureMode = config.captureMode ?? 'full';
  if (!['full', 'metadata'].includes(captureMode)) throw new Error('Invalid capture mode');
  const maxCaptureBytes = config.maxCaptureBytes ?? 1024 * 1024;
  if (!Number.isInteger(maxCaptureBytes) || maxCaptureBytes < 0 || maxCaptureBytes > 1024 * 1024) throw new Error('Invalid capture limit');
  return { executable, modulePath, sha256: digest, signatures, signatureMasks, captureMode, maxCaptureBytes };
}
