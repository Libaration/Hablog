// What the window knows about the session, built from Hablog's events.
import { headerId, hexToBytes, isIn } from "./fuse";
import type { Level, Overview, SessionEvent } from "./types";

export type Phase = "idle" | "starting" | "waiting" | "connected" | "stopping";

export interface Entry {
  n: number;
  time: number;
  inbound: boolean;
  kind: "message" | "wire" | "frame";
  bytes: Uint8Array;
  id: number | null;
  decrypted: boolean;
  /** An encrypted chunk while nothing was decrypting: worth showing anyway. */
  fallback: boolean;
}

export interface StatusLine {
  time: number;
  level: Level;
  text: string;
}

const MAX_ENTRIES = 50_000;
const MAX_STATUSES = 200;

export class Store {
  overview: Overview | null = null;
  phase: Phase = "idle";
  decrypting = false;
  listen = "";
  upstream = "";
  peer: string | null = null;
  capture: string | null = null;
  bytesIn = 0;
  bytesOut = 0;
  messagesIn = 0;
  messagesOut = 0;
  statuses: StatusLine[] = [];
  entries: Entry[] = [];
  /** Bumped when entries are removed, so views rebuild instead of appending. */
  generation = 0;
  private next = 0;
  private listeners = new Set<() => void>();
  private queued = false;

  subscribe(listener: () => void) {
    this.listeners.add(listener);
  }

  /** Tells the views, at most once per frame. */
  changed() {
    if (this.queued) return;
    this.queued = true;
    requestAnimationFrame(() => {
      this.queued = false;
      this.listeners.forEach((listener) => listener());
    });
  }

  status(level: Level, text: string) {
    this.statuses.push({ time: Date.now(), level, text });
    if (this.statuses.length > MAX_STATUSES) this.statuses.splice(0, this.statuses.length - MAX_STATUSES);
    this.changed();
  }

  clearEntries() {
    this.entries = [];
    this.generation++;
    this.changed();
  }

  apply(events: SessionEvent[]) {
    for (const event of events) {
      switch (event.kind) {
        case "status":
          this.status(event.level, event.text);
          break;
        case "ready":
          this.listen = event.listen;
          this.upstream = event.upstream;
          if (this.phase === "starting") this.phase = "waiting";
          break;
        case "connected":
          this.peer = event.peer;
          this.capture = event.capture;
          this.phase = "connected";
          this.status("success", `The game connected (${event.peer}), relaying to ${event.upstream}`);
          break;
        case "disconnected":
          this.peer = null;
          if (this.phase === "connected") this.phase = "waiting";
          this.status(
            "normal",
            event.capture
              ? `The game disconnected. Saved ${event.chunks} TCP chunks to ${event.capture}`
              : "The game disconnected.",
          );
          break;
        case "decrypting":
          this.decrypting = event.on;
          break;
        case "message": {
          const bytes = hexToBytes(event.bytes);
          const inbound = isIn(event.direction);
          if (inbound) this.messagesIn++;
          else this.messagesOut++;
          this.add({ time: event.time_ms, inbound, kind: "message", bytes, id: headerId(bytes), decrypted: event.decrypted, fallback: false });
          break;
        }
        case "raw":
          this.add({
            time: event.time_ms,
            inbound: isIn(event.direction),
            kind: event.raw,
            bytes: hexToBytes(event.bytes),
            id: null,
            decrypted: false,
            fallback: event.raw === "wire" && !event.plain && !this.decrypting,
          });
          break;
        case "connecting":
          this.status("normal", `The game is connecting to ${event.host}:${event.port}.`);
          break;
        case "traffic":
          this.bytesIn = event.bytes_in;
          this.bytesOut = event.bytes_out;
          break;
        case "stopped":
          this.phase = "idle";
          this.decrypting = false;
          this.peer = null;
          this.status("normal", "Stopped.");
          break;
      }
    }
    this.changed();
  }

  resetCounters() {
    this.bytesIn = this.bytesOut = this.messagesIn = this.messagesOut = 0;
    this.capture = null;
  }

  private add(entry: Omit<Entry, "n">) {
    this.entries.push({ n: this.next++, ...entry });
    if (this.entries.length > MAX_ENTRIES) {
      this.entries.splice(0, this.entries.length - MAX_ENTRIES);
      this.generation++;
    }
  }
}
