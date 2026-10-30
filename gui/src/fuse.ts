// Showing FUSE messages. The authoritative code is src/origins/fuse.rs; this
// only formats bytes the way the CLI does.
import type { Direction } from "./types";

export function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.substr(i * 2, 2), 16);
  return bytes;
}

export function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

const printable = (byte: number) => byte >= 32 && byte < 127;

/** Printable ASCII as is, any other byte as [n]. */
export function readable(bytes: Uint8Array, limit = Infinity): string {
  let text = "";
  const end = Math.min(bytes.length, limit);
  for (let i = 0; i < end; i++) {
    text += printable(bytes[i]) ? String.fromCharCode(bytes[i]) : `[${bytes[i]}]`;
  }
  if (bytes.length > limit) text += ` … ${bytes.length - limit} more bytes`;
  return text;
}

/** Appends `readable(bytes)` to `parent`, with the [n] bytes dimmed. */
export function appendReadable(parent: HTMLElement, bytes: Uint8Array, limit = Infinity): void {
  const end = Math.min(bytes.length, limit);
  let run = "";
  const flush = () => {
    if (run) parent.append(run);
    run = "";
  };
  for (let i = 0; i < end; i++) {
    if (printable(bytes[i])) {
      run += String.fromCharCode(bytes[i]);
    } else {
      flush();
      const escape = document.createElement("span");
      escape.className = "esc";
      escape.textContent = `[${bytes[i]}]`;
      parent.append(escape);
    }
  }
  flush();
  if (bytes.length > limit) {
    const more = document.createElement("span");
    more.className = "esc";
    more.textContent = ` … ${bytes.length - limit} more bytes`;
    parent.append(more);
  }
}

/** The numeric ID of a two-character header, e.g. @C = 3. */
export function headerId(bytes: Uint8Array): number | null {
  if (bytes.length < 2) return null;
  const [a, b] = [bytes[0], bytes[1]];
  if (a < 64 || a > 127 || b < 64 || b > 127) return null;
  return ((a & 0x3f) << 6) | (b & 0x3f);
}

export function hexDump(bytes: Uint8Array): string {
  const lines: string[] = [];
  for (let offset = 0; offset < bytes.length; offset += 16) {
    const row = bytes.subarray(offset, offset + 16);
    const hex = Array.from(row, (b) => b.toString(16).padStart(2, "0")).join(" ");
    const text = Array.from(row, (b) => (printable(b) ? String.fromCharCode(b) : "·")).join("");
    lines.push(`${offset.toString(16).padStart(4, "0")}  ${hex.padEnd(47)}  ${text}`);
  }
  return lines.join("\n");
}

export const isIn = (direction: Direction) => direction === "server_to_client";

export function formatBytes(count: number): string {
  if (count >= 1048576) return `${(count / 1048576).toFixed(1)} MiB`;
  if (count >= 1024) return `${(count / 1024).toFixed(1)} KiB`;
  return `${count} B`;
}

export function formatTime(ms: number): string {
  const time = new Date(ms);
  const pad = (n: number, width = 2) => String(n).padStart(width, "0");
  return `${pad(time.getHours())}:${pad(time.getMinutes())}:${pad(time.getSeconds())}.${pad(time.getMilliseconds(), 3)}`;
}

/** The two characters of header `id`, e.g. 3 = @C. */
export function headerText(id: number): string {
  return String.fromCharCode(64 | ((id >> 6) & 0x3f), 64 | (id & 0x3f));
}
