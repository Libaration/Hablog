// hablog.toml as a form. Saving keeps the file's comments.
import type { App, Tab } from "../app";
import type { Settings, Show } from "../types";
import { group, h } from "../ui";

const HOTELS: Record<string, string> = {
  hhous: "English",
  hhobr: "Portuguese",
  hhoes: "Spanish",
};

/** "English (hhous)", or the code for a hotel without a known name. */
export const hotelName = (code: string) => (HOTELS[code] ? `${HOTELS[code]} (${code})` : code);

type Control = HTMLInputElement | HTMLSelectElement;

export class SettingsTab implements Tab {
  readonly title = "Settings";
  readonly element = h("section.panel.settings");
  private hotel = h("select");
  private launch = h("input", { type: "checkbox" });
  private decrypt = h("input", { type: "checkbox" });
  private show = h("select");
  private showRaw = h("input", { type: "checkbox" });
  private saveCaptures = h("input", { type: "checkbox" });
  private capturesFolder = h("input", { type: "text", spellcheck: "false" });
  private upstream = h("input", { type: "text", placeholder: "game-ous.habbo.com:40001", spellcheck: "false" });
  private appdata = h("input", { type: "text", placeholder: "C:\\Users\\you\\AppData\\Roaming", spellcheck: "false" });
  private localAppdata = h("input", { type: "text", placeholder: "C:\\Users\\you\\AppData\\Local", spellcheck: "false" });
  private observer = h("input", { type: "text", placeholder: "automation", spellcheck: "false" });
  private save = h("button.primary", { type: "button" }, "Save");
  private revert = h("button", { type: "button" }, "Undo changes");
  private note = h("span.hint");
  private saved: Settings | null = null;
  private hotelsKey = "";

  constructor(private app: App) {
    for (const [value, label] of [
      ["both", "Both directions"],
      ["in", "Server to game only"],
      ["out", "Game to server only"],
      ["none", "Nothing"],
    ]) {
      this.show.append(h("option", { value }, label));
    }
    const field = (label: string, control: Control, help: string) =>
      h("label.setting", {}, h("span.label", {}, label), control, h("span.help", {}, help));
    const check = (control: HTMLInputElement, label: string, help: string) =>
      h("label.setting.check", {}, control, h("span.label", {}, label), h("span.help", {}, help));

    const game = group(
      "Game",
      field("Hotel", this.hotel, "Which Origins hotel to play on. Automatic uses the one the Habbo Launcher last opened."),
      check(this.launch, "Start the game", "Start the game once Hablog is ready. Otherwise, start the routed copy yourself."),
      check(
        this.decrypt,
        "Decrypted traffic",
        "Start the game through the observer so its decrypted messages can be shown. Without it, only the encrypted traffic is shown.",
      ),
    );
    const log = group(
      "Log",
      field("Show", this.show, "Which traffic the Packets tab and the command line show at first. Captures always get both."),
      check(this.showRaw, "Raw traffic", "Also show every TCP chunk as sent, and the frame header before each decrypted message."),
      check(this.saveCaptures, "Save captures", "Save each session's raw traffic to its own file."),
      field("Captures folder", this.capturesFolder, "Where captures are saved, relative to Hablog's folder."),
    );
    const advanced = h(
      "details.advanced",
      {},
      h("summary", {}, "Advanced"),
      h("p.hint", {}, "Settings you normally don't need. Leave a box empty to find it automatically."),
      field("Game server", this.upstream, "Connect to this server instead of the one built into the client."),
      field("AppData", this.appdata, "Look for the Habbo Launcher in another Roaming AppData folder, such as a Wine prefix."),
      field("Local AppData", this.localAppdata, "The matching Local AppData folder."),
      field("Observer", this.observer, "The folder containing the observer (host.mjs)."),
    );

    this.save.addEventListener("click", () => void this.store());
    this.revert.addEventListener("click", () => this.saved && this.fill(this.saved));
    this.element.append(
      h("div.columns", {}, h("div.column", {}, game, group("More", advanced)), h("div.column", {}, log)),
      h("div.savebar", {}, this.save, this.revert, this.note),
    );
    this.element.addEventListener("input", () => this.dirty());
    this.element.addEventListener("change", () => this.dirty());
    this.dirty();
  }

  update() {
    const overview = this.app.store.overview;
    if (!overview) return;
    const hotels = JSON.stringify([overview.hotels, overview.settings.hotel]);
    if (hotels !== this.hotelsKey) {
      this.hotelsKey = hotels;
      const codes = [...new Set([...overview.hotels, ...Object.keys(HOTELS), overview.settings.hotel])].filter(
        (code) => code !== "auto",
      );
      const current = this.hotel.value;
      this.hotel.replaceChildren(
        h("option", { value: "auto" }, "Automatic"),
        ...codes.map((code) =>
          h(
            "option",
            { value: code },
            `${hotelName(code)}${overview.hotels.length && !overview.hotels.includes(code) ? " — not installed" : ""}`,
          ),
        ),
      );
      this.hotel.value = current || overview.settings.hotel;
    }
    // Picks up the file as saved, unless there are unsaved changes.
    if (!this.saved || (!this.isDirty() && JSON.stringify(this.saved) !== JSON.stringify(overview.settings))) {
      this.saved = structuredClone(overview.settings);
      this.fill(this.saved);
    }
  }

  private fill(settings: Settings) {
    this.hotel.value = settings.hotel;
    this.launch.checked = settings.launch;
    this.decrypt.checked = settings.decrypt;
    this.show.value = settings.show;
    this.showRaw.checked = settings.show_raw;
    this.saveCaptures.checked = settings.save_captures;
    this.capturesFolder.value = settings.captures_folder;
    this.upstream.value = settings.advanced.upstream ?? "";
    this.appdata.value = settings.advanced.appdata ?? "";
    this.localAppdata.value = settings.advanced.local_appdata ?? "";
    this.observer.value = settings.advanced.observer ?? "";
    this.dirty();
  }

  private read(): Settings {
    const optional = (input: HTMLInputElement) => input.value.trim() || null;
    return {
      hotel: this.hotel.value || "auto",
      launch: this.launch.checked,
      decrypt: this.decrypt.checked,
      show: this.show.value as Show,
      show_raw: this.showRaw.checked,
      save_captures: this.saveCaptures.checked,
      captures_folder: this.capturesFolder.value.trim() || "logs",
      advanced: {
        upstream: optional(this.upstream),
        appdata: optional(this.appdata),
        local_appdata: optional(this.localAppdata),
        observer: optional(this.observer),
      },
    };
  }

  private isDirty() {
    return !!this.saved && JSON.stringify(this.read()) !== JSON.stringify(this.saved);
  }

  private dirty() {
    const dirty = this.isDirty();
    this.save.disabled = this.revert.disabled = !dirty;
    this.capturesFolder.disabled = !this.saveCaptures.checked;
    this.note.textContent = dirty
      ? this.app.store.phase === "idle"
        ? "Unsaved changes."
        : "Unsaved changes. They apply to the next session."
      : "Saved in hablog.toml.";
  }

  private async store() {
    const settings = this.read();
    try {
      await this.app.backend.saveSettings(settings);
      this.saved = settings;
      this.app.store.status("success", "Saved the settings.");
      this.dirty();
      await this.app.refresh();
    } catch (error) {
      this.note.textContent = String(error);
    }
  }
}
