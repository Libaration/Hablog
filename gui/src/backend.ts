// The window's link to Hablog: Tauri commands and the `session` event, or a
// demo session when the page runs in an ordinary browser (`npm run dev`).
import type { Built, Overview, Part, SessionEvent, Settings } from "./types";

export interface Backend {
  readonly demo: boolean;
  overview(): Promise<Overview>;
  start(): Promise<void>;
  stop(): Promise<void>;
  installObserver(folder: string): Promise<void>;
  decode(value: string): Promise<string[]>;
  build(direction: "in" | "out", header: string, parts: Part[]): Promise<Built>;
  saveSettings(settings: Settings): Promise<void>;
  onEvents(handler: (events: SessionEvent[]) => void): void;
}

class TauriBackend implements Backend {
  readonly demo = false;
  private core = import("@tauri-apps/api/core");

  private async call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    return (await this.core).invoke<T>(command, args);
  }

  overview() {
    return this.call<Overview>("overview");
  }
  start() {
    return this.call<void>("start");
  }
  stop() {
    return this.call<void>("stop");
  }
  installObserver(folder: string) {
    return this.call<void>("install_observer", { folder });
  }
  decode(value: string) {
    return this.call<string[]>("decode", { value });
  }
  build(direction: "in" | "out", header: string, parts: Part[]) {
    return this.call<Built>("build", { direction, header, parts });
  }
  saveSettings(settings: Settings) {
    return this.call<void>("save_settings", { settings });
  }
  onEvents(handler: (events: SessionEvent[]) => void) {
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen<SessionEvent[]>("session", (event) => handler(event.payload)),
    );
  }
}

export async function connect(): Promise<Backend> {
  if ("__TAURI_INTERNALS__" in window) return new TauriBackend();
  const { DemoBackend } = await import("./demo");
  return new DemoBackend();
}
