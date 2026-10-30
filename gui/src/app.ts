// The window: banner, tabs and status bar around the tab panels.
import type { Backend } from "./backend";
import { formatBytes } from "./fuse";
import { Store, type Phase } from "./store";
import { AboutTab } from "./tabs/about";
import { BuilderTab } from "./tabs/builder";
import { ConnectionTab } from "./tabs/connection";
import { PacketsTab } from "./tabs/packets";
import { SettingsTab } from "./tabs/settings";
import { h } from "./ui";

export interface Tab {
  readonly title: string;
  readonly element: HTMLElement;
  update(): void;
  shown?(): void;
}

const PHASES: Record<Phase, string> = {
  idle: "Not running",
  starting: "Starting",
  waiting: "Waiting for the game",
  connected: "Connected",
  stopping: "Stopping",
};

export class App {
  readonly store = new Store();
  readonly builder: BuilderTab;
  private tabs: Tab[];
  private current = 0;
  private tabButtons: HTMLButtonElement[] = [];
  private pill = h("span.pill");
  private statusText = h("span.status-text");
  private statusCounts = h("span.status-counts");
  private notes = new Set<string>();

  constructor(
    private root: HTMLElement,
    readonly backend: Backend,
  ) {
    this.builder = new BuilderTab(this);
    this.tabs = [new ConnectionTab(this), new PacketsTab(this), this.builder, new SettingsTab(this), new AboutTab(this)];
  }

  async init() {
    const banner = h(
      "header.banner",
      {},
      h(
        "div.brand",
        {},
        h("div.logo", { "aria-label": "Hablog" }, "HABLOG"),
        h("div.tagline", {}, "Packet logger for Habbo Origins"),
      ),
      h(
        "div.banner-side",
        {},
        this.pill,
        h("div.badge", {}, h("span.badge-label", {}, "Origins"), h("span.badge-version", {}, "0.1")),
      ),
    );
    const nav = h("nav.tabs", { role: "tablist" });
    this.tabs.forEach((tab, index) => {
      const button = h("button.tab", { role: "tab", type: "button", onclick: () => this.select(index) }, tab.title);
      this.tabButtons.push(button);
      nav.append(button);
    });
    const panels = h("main.panels");
    this.tabs.forEach((tab) => panels.append(tab.element));
    const footer = h("footer.statusbar", {}, h("span.dot"), this.statusText, this.statusCounts);
    this.root.append(banner, nav, panels, footer);
    if (this.backend.demo) document.body.classList.add("demo");

    this.store.subscribe(() => this.update());
    this.backend.onEvents((events) => {
      this.store.apply(events);
      if (events.some((event) => event.kind === "stopped")) void this.refresh();
    });
    this.select(0);
    await this.refresh();
  }

  async refresh() {
    try {
      const overview = await this.backend.overview();
      this.store.overview = overview;
      const note = overview.settings_note;
      if (note && !this.notes.has(note)) {
        this.notes.add(note);
        this.store.status("info", note);
      }
      // A session started before the window was reloaded.
      if (overview.running && this.store.phase === "idle") this.store.phase = "waiting";
    } catch (error) {
      this.store.status("error", String(error));
    }
    this.store.changed();
  }

  async start() {
    if (this.store.phase !== "idle") return;
    this.store.resetCounters();
    this.store.phase = "starting";
    this.store.changed();
    try {
      await this.backend.start();
    } catch (error) {
      this.store.phase = "idle";
      this.store.status("error", String(error));
    }
  }

  async stop() {
    if (this.store.phase === "idle") return;
    this.store.phase = "stopping";
    this.store.changed();
    await this.backend.stop();
  }

  select(index: number) {
    this.current = index;
    this.tabs.forEach((tab, i) => {
      tab.element.hidden = i !== index;
      this.tabButtons[i].classList.toggle("active", i === index);
      this.tabButtons[i].setAttribute("aria-selected", String(i === index));
    });
    this.tabs[index].shown?.();
    this.tabs[index].update();
  }

  selectTab(tab: Tab) {
    this.select(this.tabs.indexOf(tab));
  }

  private update() {
    const store = this.store;
    this.pill.textContent = PHASES[store.phase];
    this.pill.dataset.phase = store.phase;
    document.body.dataset.phase = store.phase;
    this.statusText.textContent =
      PHASES[store.phase] + (store.phase !== "idle" && store.decrypting ? " · decrypting" : "");
    this.statusCounts.textContent = `in ${formatBytes(store.bytesIn)} · out ${formatBytes(store.bytesOut)} · ${
      store.messagesIn + store.messagesOut
    } messages`;
    this.tabs[this.current].update();
  }
}
