// The shapes Hablog's Rust side sends; see src/session.rs and gui/src-tauri.

export type Direction = "server_to_client" | "client_to_server";
export type Level = "normal" | "info" | "success" | "warning" | "error";

export type SessionEvent =
  | { kind: "status"; level: Level; text: string }
  | { kind: "ready"; listen: string; upstream: string }
  | { kind: "connected"; peer: string; upstream: string; capture: string | null }
  | { kind: "disconnected"; capture: string | null; chunks: number }
  | { kind: "decrypting"; on: boolean }
  | { kind: "message"; time_ms: number; direction: Direction; bytes: string; decrypted: boolean }
  | {
      kind: "raw";
      time_ms: number;
      direction: Direction;
      raw: "wire" | "frame";
      bytes: string;
      plain: boolean;
    }
  | { kind: "connecting"; host: string; port: number }
  | { kind: "traffic"; bytes_in: number; bytes_out: number }
  | { kind: "stopped" };

export type Show = "both" | "in" | "out" | "none";

export interface Settings {
  hotel: string;
  launch: boolean;
  decrypt: boolean;
  show: Show;
  show_raw: boolean;
  save_captures: boolean;
  captures_folder: string;
  advanced: {
    upstream: string | null;
    appdata: string | null;
    local_appdata: string | null;
    observer: string | null;
  };
}

export interface Client {
  version: string;
  hotel: string | null;
  upstream: string;
  listen: string;
  routed: string;
}

export interface Problem {
  error: string;
  hint: string | null;
  warnings: string[];
}

export type ObserverStatus =
  | { state: "ready" }
  | { state: "not_installed"; folder: string }
  | { state: "unavailable"; reason: string }
  | { state: "off" };

export interface Overview {
  settings: Settings;
  settings_note: string | null;
  client: Client | null;
  problem: Problem | null;
  details: string[];
  hotels: string[];
  observer: ObserverStatus;
  running: boolean;
}

export interface Built {
  readable: string;
  hex: string;
}

/** A value in the builder: i, b, s or r, and its text. */
export type Part = [string, string];
