// A made-up session for previewing the window in a browser. Nothing here
// talks to the game. The encoders mirror src/origins/fuse.rs closely enough
// to preview the builder; the real ones run in Rust.
import type { Backend } from "./backend";
import { readable, toHex } from "./fuse";
import type { Built, Direction, Overview, Part, SessionEvent, Settings } from "./types";

const encoder = new TextEncoder();
const hex = (text: string) => toHex(fromReadable(text));

function fromReadable(text: string): Uint8Array {
  const bytes: number[] = [];
  const pattern = /\[(\d{1,3})\]/g;
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    const value = Number(match[1]);
    if (value > 255) continue;
    bytes.push(...encoder.encode(text.slice(last, match.index)), value);
    last = (match.index ?? 0) + match[0].length;
  }
  bytes.push(...encoder.encode(text.slice(last)));
  return new Uint8Array(bytes);
}

function vl64(value: number): number[] {
  let rest = Math.abs(value);
  const bytes = [64 | (rest & 3)];
  rest = Math.floor(rest / 4);
  while (rest) {
    bytes.push(64 | (rest & 0x3f));
    rest = Math.floor(rest / 64);
  }
  bytes[0] |= (bytes.length << 3) | (value < 0 ? 4 : 0);
  return bytes;
}

/** The VL64 number at the start of `bytes` and its length. */
function readVl64(bytes: Uint8Array): [number, number] | null {
  const first = bytes[0];
  const count = (first >> 3) & 7;
  if (first < 64 || first > 127 || !count || bytes.length < count) return null;
  let value = first & 3;
  for (let i = 1; i < count; i++) {
    if (bytes[i] < 64 || bytes[i] > 127) return null;
    value += (bytes[i] & 0x3f) * 4 * 64 ** (i - 1);
  }
  return [first & 4 ? -value : value, count];
}

function b64(value: number, digits: number): number[] | null {
  if (value >= 64 ** digits) return null;
  return Array.from({ length: digits }, (_, i) => 64 | ((value >> (6 * (digits - 1 - i))) & 0x3f));
}

const SETTINGS: Settings = {
  hotel: "auto",
  launch: true,
  decrypt: true,
  show: "both",
  show_raw: false,
  save_captures: true,
  captures_folder: "logs",
  advanced: { upstream: null, appdata: null, local_appdata: null, observer: null },
};

// A session shaped like a real one; the login is made up.
const SCRIPT: [number, Direction, string, boolean][] = [
  [300, "server_to_client", "@@[1]", false],
  [80, "client_to_server", "CN", false],
  [120, "server_to_client", "DU574282476854826307567272[2][1]", false],
  [120, "client_to_server", "CJBG18388861247521449655020633283973107217938308038390616589", false],
  [120, "server_to_client", "@A2902256788312683597958095505086431570718660282792930[2][1]", false],
  [200, "client_to_server", "@E[_D@A2@vhttp://origins-gamedata.habbo.com/external_variables/1", true],
  [90, "server_to_client", "DARAHHIIKHJHPAIQAdd-MM-yyyy[2][1]", true],
  [160, "client_to_server", "@D@Ssomeone@example.com@H********@@@@", true],
  [140, "server_to_client", "@BPBfuse_login[2]fuse_buy_credits[2]fuse_trade[2]fuse_create_flat[2]fuse_ignore_room_owner[2]fuse_habbo_chooser[2]fuse_furni_chooser[2]fuse_pet_companions[2][1]", true],
  [40, "server_to_client", "@CIHH[1]", true],
  [120, "client_to_server", "@G", true],
  [30, "client_to_server", "@K", true],
  [60, "client_to_server", "@Z@Jclub_habbo", true],
];

export class DemoBackend implements Backend {
  readonly demo = true;
  private handler: (events: SessionEvent[]) => void = () => {};
  private timers: number[] = [];
  private running = false;
  private stopping = false;
  private bytesIn = 0;
  private bytesOut = 0;

  async overview(): Promise<Overview> {
    return {
      settings: SETTINGS,
      settings_note: "Browser preview: this is a made-up session, not your game.",
      client: {
        version: "371",
        hotel: "hhous",
        upstream: "game-ous.habbo.com:40001",
        listen: "127.0.0.1:40001",
        routed: "C:\\Users\\you\\AppData\\Roaming\\Habbo Launcher\\downloads\\shockwave\\371\\Hablog-Routed.exe",
      },
      problem: null,
      details: [
        "Launcher: C:\\Users\\you\\AppData\\Local\\Programs\\habbo-electron-launcher\\Habbo Launcher.exe (version 1.0.80)",
        "Launcher's selected version: 371",
        "Origins 371 at C:\\Users\\you\\AppData\\Roaming\\Habbo Launcher\\downloads\\shockwave\\371",
        "  Habbo.exe:  game game-ous.habbo.com:40001 (hhous)",
        "  hhobr     game game-obr.habbo.com:40001, MUS game-obr.habbo.com:40002  (HabboHotel-obr.exe)",
        "  hhoes     game game-oes.habbo.com:40001, MUS game-oes.habbo.com:40002  (HabboHotel-oes.exe)",
        "  hhous     game game-ous.habbo.com:40001, MUS game-ous.habbo.com:40002  (HabboHotel-ous.exe)",
      ],
      hotels: ["hhobr", "hhoes", "hhous"],
      observer: { state: "ready" },
      running: this.running,
    };
  }

  async start() {
    this.running = true;
    this.stopping = false;
    let at = 0;
    const later = (delay: number, events: SessionEvent[]) => {
      at += delay;
      this.timers.push(window.setTimeout(() => this.handler(events), at));
    };
    later(50, [{ kind: "status", level: "info", text: "Starting the game with decrypted traffic..." }]);
    later(150, [{ kind: "ready", listen: "127.0.0.1:40001", upstream: "game-ous.habbo.com:40001" }]);
    later(200, [
      { kind: "decrypting", on: true },
      { kind: "status", level: "success", text: "Decrypted traffic is on." },
      { kind: "status", level: "success", text: "Started the game." },
    ]);
    later(900, [
      {
        kind: "connected",
        peer: "127.0.0.1:52144",
        upstream: "15.197.201.180:40001",
        capture: "logs/hablog-preview-1.jsonl",
      },
    ]);
    for (const [delay, direction, text, decrypted] of SCRIPT) later(delay, [this.message(direction, text, decrypted)]);
    const loop = () => {
      if (!this.running || this.stopping) return;
      const ticks = [
        this.message("server_to_client", `@C${String.fromCharCode(73 + Math.floor(Math.random() * 3))}HH[1]`, true),
        this.message("client_to_server", Math.random() < 0.5 ? "@G" : "@K", true),
      ];
      this.handler([...ticks, { kind: "traffic", bytes_in: this.bytesIn, bytes_out: this.bytesOut }]);
      this.timers.push(window.setTimeout(loop, 600 + Math.random() * 1400));
    };
    this.timers.push(window.setTimeout(loop, at + 800));
  }

  private message(direction: Direction, text: string, decrypted: boolean): SessionEvent {
    const bytes = hex(text);
    const size = bytes.length / 2 + (decrypted ? Math.ceil((bytes.length / 2) * 4 / 3) + 6 : 0);
    if (direction === "server_to_client") this.bytesIn += size;
    else this.bytesOut += size;
    return { kind: "message", time_ms: Date.now(), direction, bytes, decrypted };
  }

  async stop() {
    if (!this.running) return;
    if (this.stopping) {
      this.finish();
      return;
    }
    this.stopping = true;
    this.handler([{ kind: "status", level: "normal", text: "Stopping Hablog. The game stays open but loses its connection..." }]);
    this.timers.push(window.setTimeout(() => this.finish(), 400));
  }

  private finish() {
    this.timers.forEach((timer) => window.clearTimeout(timer));
    this.timers = [];
    this.running = false;
    this.handler([
      { kind: "disconnected", capture: "logs/hablog-preview-1.jsonl", chunks: 42 },
      { kind: "decrypting", on: false },
      { kind: "stopped" },
    ]);
  }

  async installObserver() {}

  async decode(value: string): Promise<string[]> {
    const number = Number(value.trim());
    if (value.trim() !== "" && Number.isInteger(number)) {
      const two = b64(number, 2);
      const three = b64(number, 3);
      const b64Parts = [
        two && `${readable(new Uint8Array(two))} (a header or string length)`,
        three && `${readable(new Uint8Array(three))} (a client message length)`,
      ].filter(Boolean);
      return [
        `VL64: ${readable(new Uint8Array(vl64(number)))}`,
        number < 0 || !b64Parts.length ? "B64:  doesn't fit" : `B64:  ${b64Parts.join(", ")}`,
      ];
    }
    // Numbers one after another, as src/tools.rs reads them.
    let rest = fromReadable(value.trim());
    const numbers: string[] = [];
    for (let read = readVl64(rest); read; read = readVl64(rest)) {
      const used = rest.subarray(0, read[1]);
      if (used.length >= 4 && used.every((b) => (b >= 97 && b <= 122) || b === 95)) break;
      numbers.push(`${readable(used)} = ${read[0]}`);
      rest = rest.subarray(read[1]);
    }
    if (!numbers.length) return ["VL64: not a VL64 number"];
    return [`VL64: ${numbers.join(", ")}${rest.length ? `, then ${readable(rest)}` : ""}`];
  }

  async build(direction: "in" | "out", header: string, parts: Part[]): Promise<Built> {
    const head = header.trim();
    const id = /^\d+$/.test(head) ? Number(head) : null;
    const headerBytes = id !== null ? b64(id, 2) : head.length === 2 ? Array.from(encoder.encode(head)) : null;
    if (!headerBytes) throw `${head || "The header"} is not a header; use two characters like @C, or an ID like 3.`;
    const bytes = [...headerBytes];
    for (const [kind, text] of parts) {
      if (kind === "i") {
        if (!/^-?\d+$/.test(text.trim())) throw `i:${text} is not a whole number.`;
        bytes.push(...vl64(Number(text)));
      } else if (kind === "b") {
        const flag = text.trim().toLowerCase();
        if (!["true", "false", "1", "0"].includes(flag)) throw `b:${text} is not true or false.`;
        bytes.push(...vl64(flag === "true" || flag === "1" ? 1 : 0));
      } else if (kind === "s") {
        const textBytes = fromReadable(text);
        if (direction === "in") bytes.push(...textBytes, 2);
        else bytes.push(...(b64(textBytes.length, 2) ?? []), ...textBytes);
      } else {
        bytes.push(...fromReadable(text));
      }
    }
    if (direction === "in") bytes.push(1);
    const message = new Uint8Array(bytes);
    return { readable: readable(message), hex: toHex(message) };
  }

  async saveSettings(settings: Settings) {
    Object.assign(SETTINGS, settings);
  }

  onEvents(handler: (events: SessionEvent[]) => void) {
    this.handler = handler;
  }
}
