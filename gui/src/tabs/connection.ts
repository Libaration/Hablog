// Starting and stopping, and what Hablog found and is doing.
import type { App, Tab } from "../app";
import { formatBytes, formatTime } from "../fuse";
import { group, h, rows } from "../ui";
import { hotelName } from "./settings";

export class ConnectionTab implements Tab {
  readonly title = "Connection";
  readonly element = h("section.panel.connection");
  private button = h("button.big", { type: "button", onclick: () => this.toggle() });
  private problem = h("div.problem");
  private session: HTMLElement[];
  private traffic: HTMLElement[];
  private observer = h("div.observer");
  private details = h("pre.details");
  private log = h("ol.log");
  private shownStatuses = -1;
  private observerKey = "";
  private installing = false;

  constructor(private app: App) {
    const sessionBox = group("Session");
    this.session = rows(sessionBox, ["Hotel", "Client", "Game server", "Relay", "Game", "Capture"]);
    sessionBox.append(h("div.actions", {}, this.button));

    const trafficBox = group("Traffic");
    const legend = (direction: "in" | "out", label: string) =>
      h("span.legend", {}, h(`i.swatch.${direction}`), label);
    this.traffic = rows(trafficBox, ["", "", "Total"]);
    const labels = trafficBox.querySelectorAll("dt");
    labels[0].append(legend("in", "Server to game"));
    labels[1].append(legend("out", "Game to server"));

    this.element.append(
      this.problem,
      h(
        "div.columns",
        {},
        h("div.column", {}, sessionBox, trafficBox),
        h(
          "div.column",
          {},
          group("Decrypted traffic", this.observer),
          group("Found on this computer", this.details),
        ),
      ),
      group("Activity", this.log),
    );
  }

  private toggle() {
    if (this.app.store.phase === "idle") void this.app.start();
    else void this.app.stop();
  }

  update() {
    const { store } = this.app;
    const overview = store.overview;
    const client = overview?.client;
    const running = store.phase !== "idle";

    this.button.textContent =
      store.phase === "idle" ? "Start" : store.phase === "stopping" ? "Stop now" : store.phase === "starting" ? "Starting…" : "Stop";
    this.button.classList.toggle("stop", running);
    this.button.disabled = store.phase === "starting" || (!running && !client);

    this.problem.replaceChildren();
    this.problem.hidden = !overview?.problem;
    if (overview?.problem) {
      const { error, hint, warnings } = overview.problem;
      this.problem.append(h("strong", {}, error), ...(hint ? [h("p", {}, hint)] : []));
      this.problem.append(...warnings.map((warning) => h("p.warning", {}, warning)));
    }

    const [hotel, version, upstream, relay, game, capture] = this.session;
    const code = client?.hotel ?? overview?.settings.hotel;
    hotel.textContent = code ? (code === "auto" ? "Automatic" : hotelName(code)) : "—";
    version.textContent = client ? `Origins ${client.version}` : "Not found";
    upstream.textContent = store.upstream || client?.upstream || "—";
    relay.textContent = store.listen || client?.listen || "—";
    game.textContent =
      store.phase === "connected"
        ? `Connected from ${store.peer}`
        : store.phase === "waiting"
          ? "Waiting for the game to connect"
          : store.phase === "idle"
            ? "Not started"
            : "…";
    game.className = store.phase === "connected" ? "good" : "";
    capture.textContent = store.capture ?? (overview?.settings.save_captures ? "Saved when the game connects" : "Off");

    const [inbound, outbound, total] = this.traffic;
    inbound.textContent = `${formatBytes(store.bytesIn)} · ${store.messagesIn} messages`;
    outbound.textContent = `${formatBytes(store.bytesOut)} · ${store.messagesOut} messages`;
    total.textContent = `${formatBytes(store.bytesIn + store.bytesOut)} · ${store.messagesIn + store.messagesOut} messages`;

    this.updateObserver();
    const details = overview?.details.join("\n") || "Nothing yet.";
    if (this.details.textContent !== details) this.details.textContent = details;

    if (store.statuses.length !== this.shownStatuses) {
      this.shownStatuses = store.statuses.length;
      const follow = this.log.scrollTop + this.log.clientHeight >= this.log.scrollHeight - 4;
      this.log.replaceChildren(
        ...store.statuses.map((line) =>
          h(`li.${line.level}`, {}, h("time", {}, formatTime(line.time).slice(0, 8)), line.text),
        ),
      );
      if (follow) this.log.scrollTop = this.log.scrollHeight;
    }
  }

  private updateObserver() {
    const { store } = this.app;
    const observer = store.overview?.observer;
    // Rebuilt only when it changes, so the button isn't replaced mid-click.
    const key = JSON.stringify([store.decrypting, observer, this.installing]);
    if (key === this.observerKey) return;
    this.observerKey = key;
    const children: (Node | string)[] = [];
    if (store.decrypting) {
      children.push(h("p.good", {}, "On. Messages are shown decrypted."));
    } else if (!observer) {
      children.push("…");
    } else if (observer.state === "ready") {
      children.push(h("p", {}, "Ready. The game is started through the observer so its traffic can be read."));
    } else if (observer.state === "off") {
      children.push(h("p", {}, "Off in the settings. Only the encrypted traffic is shown."));
    } else if (observer.state === "unavailable") {
      children.push(h("p.warning", {}, observer.reason), h("p", {}, "The game starts normally and the encrypted traffic is shown."));
    } else {
      const install = h(
        "button",
        {
          type: "button",
          onclick: async () => {
            this.installing = true;
            this.update();
            this.app.store.status("info", "Setting up the observer (npm ci)…");
            try {
              await this.app.backend.installObserver(observer.folder);
              this.app.store.status("success", "The observer is set up.");
            } catch (error) {
              this.app.store.status("error", String(error));
            }
            this.installing = false;
            await this.app.refresh();
          },
        },
        this.installing ? "Setting up…" : "Set up",
      );
      install.disabled = this.installing;
      children.push(
        h("p", {}, "Reading the decrypted traffic needs the observer's packages, installed once with npm (Node 22 or newer)."),
        h("div.actions", {}, install, h("span.hint", {}, observer.folder)),
      );
    }
    this.observer.replaceChildren(...children);
  }
}
