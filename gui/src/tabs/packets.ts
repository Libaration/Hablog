// The packet log: one row per message, filtered, with the selected one's
// details underneath.
import type { App, Tab } from "../app";
import { appendReadable, formatTime, headerId, hexDump, readable, toHex } from "../fuse";
import type { Entry } from "../store";
import { h } from "../ui";

const ROW_H = 22;
const OVERSCAN = 8;
const BODY_LIMIT = 400;
const encoder = new TextEncoder();

interface Filter {
  inbound: boolean;
  outbound: boolean;
  raw: boolean;
  include: Set<number>;
  exclude: Set<number>;
  search: string;
}

const texts = new WeakMap<Entry, string>();
function textOf(entry: Entry): string {
  let text = texts.get(entry);
  if (text === undefined) texts.set(entry, (text = readable(entry.bytes).toLowerCase()));
  return text;
}

function matches(entry: Entry, filter: Filter): boolean {
  if (!(entry.inbound ? filter.inbound : filter.outbound)) return false;
  if (entry.kind !== "message") {
    if (!(filter.raw || entry.fallback) || filter.include.size) return false;
  } else if (entry.id !== null) {
    if (filter.include.size && !filter.include.has(entry.id)) return false;
    if (filter.exclude.has(entry.id)) return false;
  } else if (filter.include.size) {
    return false;
  }
  return !filter.search || textOf(entry).includes(filter.search);
}

/** "@C 3 !@G" → show @C and ID 3, hide @G. */
function parseHeaders(text: string) {
  const include = new Set<number>();
  const exclude = new Set<number>();
  let valid = true;
  for (const token of text.split(/[\s,]+/).filter(Boolean)) {
    const hide = token.startsWith("!");
    const value = hide ? token.slice(1) : token;
    const id = /^\d+$/.test(value) ? Number(value) : value.length === 2 ? headerId(encoder.encode(value)) : null;
    if (id === null || id > 4095) valid = false;
    else (hide ? exclude : include).add(id);
  }
  return { include, exclude, valid };
}

export class PacketsTab implements Tab {
  readonly title = "Packets";
  readonly element = h("section.panel.packets");
  private filter: Filter = { inbound: true, outbound: true, raw: false, include: new Set(), exclude: new Set(), search: "" };
  private toggles: Record<"inbound" | "outbound" | "raw", HTMLInputElement>;
  private headers = h("input.headers", { type: "text", placeholder: "Headers: @C 3 !@G", spellcheck: "false" });
  private search = h("input.search", { type: "search", placeholder: "Search", spellcheck: "false" });
  private pause = h("button", { type: "button", title: "Stop adding messages to the list" }, "Pause");
  private count = h("span.count");
  private list = h("div.list", { tabindex: "0" });
  private spacer = h("div.spacer");
  private detail = h("div.detail");
  private visible: Entry[] = [];
  private seen = 0;
  private generation = -1;
  private selected: Entry | null = null;
  private paused = false;
  private follow = true;
  private renderedKey = "";
  private configured = false;
  private decodeToken = 0;

  constructor(private app: App) {
    const toggle = (label: Node | string, title: string) => {
      const input = h("input", { type: "checkbox", checked: true, onchange: () => this.refilter() });
      return [input, h("label.toggle", { title }, input, label)] as const;
    };
    const [inbound, inLabel] = toggle(h("span", {}, h("i.swatch.in"), "In"), "Server to game");
    const [outbound, outLabel] = toggle(h("span", {}, h("i.swatch.out"), "Out"), "Game to server");
    const [raw, rawLabel] = toggle("Raw", "TCP chunks and frame headers as they were sent");
    this.toggles = { inbound, outbound, raw };

    let typing = 0;
    const later = () => {
      window.clearTimeout(typing);
      typing = window.setTimeout(() => this.refilter(), 150);
    };
    this.headers.addEventListener("input", later);
    this.search.addEventListener("input", later);
    this.pause.addEventListener("click", () => {
      this.paused = !this.paused;
      this.pause.classList.toggle("on", this.paused);
      this.pause.textContent = this.paused ? "Resume" : "Pause";
      this.update();
    });

    this.list.append(this.spacer);
    this.list.addEventListener("scroll", () => {
      this.follow = this.list.scrollTop + this.list.clientHeight >= this.list.scrollHeight - ROW_H;
      this.render();
    });
    this.list.addEventListener("click", (event) => {
      const row = (event.target as HTMLElement).closest<HTMLElement>(".row");
      if (row) this.select(this.visible[Number(row.dataset.index)]);
    });
    this.list.addEventListener("dblclick", (event) => {
      const row = (event.target as HTMLElement).closest<HTMLElement>(".row");
      if (row) this.openInBuilder(this.visible[Number(row.dataset.index)]);
    });
    this.list.addEventListener("keydown", (event) => this.key(event));
    new ResizeObserver(() => this.render()).observe(this.list);

    const splitter = h("div.splitter", { title: "Drag to resize" });
    splitter.addEventListener("pointerdown", (event) => {
      splitter.setPointerCapture(event.pointerId);
      const start = event.clientY;
      const height = this.detail.offsetHeight;
      const move = (moved: PointerEvent) => {
        const max = this.element.clientHeight - 120;
        this.detail.style.height = `${Math.max(80, Math.min(max, height - (moved.clientY - start)))}px`;
      };
      splitter.addEventListener("pointermove", move);
      splitter.addEventListener("pointerup", () => splitter.removeEventListener("pointermove", move), { once: true });
    });

    this.element.append(
      h(
        "div.toolbar",
        {},
        inLabel,
        outLabel,
        rawLabel,
        h("span.sep"),
        this.headers,
        this.search,
        h("span.grow"),
        this.count,
        this.pause,
        h("button", { type: "button", onclick: () => this.app.store.clearEntries() }, "Clear"),
      ),
      h(
        "div.columns-head",
        {},
        h("span", {}, "#"),
        h("span", {}, "Time"),
        h("span", {}, "Dir"),
        h("span", {}, "Hdr"),
        h("span", {}, "ID"),
        h("span", {}, "Message"),
      ),
      this.list,
      splitter,
      this.detail,
    );
    this.showDetail();
  }

  shown() {
    this.renderedKey = "";
    if (this.follow) requestAnimationFrame(() => (this.list.scrollTop = this.list.scrollHeight));
  }

  update() {
    const { store } = this.app;
    if (!this.configured && store.overview) {
      // Start with what the settings show on the command line.
      const { show, show_raw } = store.overview.settings;
      this.configured = true;
      this.toggles.inbound.checked = show === "both" || show === "in";
      this.toggles.outbound.checked = show === "both" || show === "out";
      this.toggles.raw.checked = show_raw;
      this.readFilter();
    }
    if (store.generation !== this.generation) {
      this.generation = store.generation;
      this.seen = 0;
      this.visible = [];
      if (this.selected && !store.entries.includes(this.selected)) this.select(null);
    }
    if (!this.paused) {
      for (let i = this.seen; i < store.entries.length; i++) {
        if (matches(store.entries[i], this.filter)) this.visible.push(store.entries[i]);
      }
      this.seen = store.entries.length;
    }
    const waiting = store.entries.length - this.seen;
    this.count.textContent =
      `${this.visible.length.toLocaleString()} of ${store.entries.length.toLocaleString()}` +
      (this.paused && waiting ? ` · ${waiting.toLocaleString()} new` : "");
    this.render();
  }

  private readFilter() {
    const headers = parseHeaders(this.headers.value);
    this.headers.classList.toggle("invalid", !headers.valid);
    this.filter = {
      inbound: this.toggles.inbound.checked,
      outbound: this.toggles.outbound.checked,
      raw: this.toggles.raw.checked,
      include: headers.include,
      exclude: headers.exclude,
      search: this.search.value.trim().toLowerCase(),
    };
  }

  private refilter() {
    this.readFilter();
    this.generation = -1;
    this.update();
  }

  private render() {
    if (this.element.hidden) return;
    const height = this.list.clientHeight;
    this.spacer.style.height = `${this.visible.length * ROW_H}px`;
    if (this.follow) this.list.scrollTop = this.list.scrollHeight;
    const first = Math.max(0, Math.floor(this.list.scrollTop / ROW_H) - OVERSCAN);
    const last = Math.min(this.visible.length, Math.ceil((this.list.scrollTop + height) / ROW_H) + OVERSCAN);
    const key = `${this.generation}:${first}:${last}:${this.visible.length}`;
    if (key === this.renderedKey) return;
    this.renderedKey = key;
    const rows: HTMLElement[] = [];
    for (let i = first; i < last; i++) rows.push(this.row(this.visible[i], i));
    this.spacer.replaceChildren(...rows);
  }

  private row(entry: Entry, index: number): HTMLElement {
    const direction = entry.inbound ? "in" : "out";
    const message = entry.kind === "message";
    const hasHeader = message && entry.id !== null;
    const body = h("span.body");
    appendReadable(body, hasHeader ? entry.bytes.subarray(2) : entry.bytes, BODY_LIMIT);
    const row = h(
      `div.row.${direction}`,
      { "data-index": index, style: `top:${index * ROW_H}px` },
      h("span.n", {}, String(entry.n + 1)),
      h("span.time", {}, formatTime(entry.time)),
      h("span.dir", {}, h(`i.swatch.${direction}`), entry.inbound ? "IN" : "OUT"),
      h("span.head", {}, hasHeader ? readable(entry.bytes.subarray(0, 2)) : message ? "" : entry.kind),
      h("span.id", {}, hasHeader ? String(entry.id) : ""),
      body,
    );
    if (!message) row.classList.add("raw");
    if (entry === this.selected) row.classList.add("selected");
    return row;
  }

  private key(event: KeyboardEvent) {
    const index = this.selected ? this.visible.indexOf(this.selected) : -1;
    const page = Math.max(1, Math.floor(this.list.clientHeight / ROW_H) - 1);
    const moves: Record<string, number> = {
      ArrowDown: index + 1,
      ArrowUp: index < 0 ? this.visible.length - 1 : index - 1,
      PageDown: index + page,
      PageUp: index - page,
      Home: 0,
      End: this.visible.length - 1,
    };
    if (event.key === "Enter" && this.selected) {
      this.openInBuilder(this.selected);
    } else if (event.key === "Escape") {
      this.select(null);
    } else if (event.key in moves && this.visible.length) {
      const next = Math.max(0, Math.min(this.visible.length - 1, moves[event.key]));
      this.select(this.visible[next]);
      const top = next * ROW_H;
      if (top < this.list.scrollTop) this.list.scrollTop = top;
      else if (top + ROW_H > this.list.scrollTop + this.list.clientHeight)
        this.list.scrollTop = top + ROW_H - this.list.clientHeight;
    } else {
      return;
    }
    event.preventDefault();
  }

  private select(entry: Entry | null) {
    this.selected = entry;
    // Only the class changes, so the rows stay put for a double-click.
    for (const row of this.spacer.children as HTMLCollectionOf<HTMLElement>) {
      row.classList.toggle("selected", this.visible[Number(row.dataset.index)] === entry);
    }
    this.showDetail();
  }

  private showDetail() {
    const entry = this.selected;
    if (!entry) {
      this.detail.replaceChildren(
        h("p.empty", {}, "Select a message to see it in full. Double-click or press Enter to open it in the builder."),
      );
      return;
    }
    const message = entry.kind === "message";
    const hasHeader = message && entry.id !== null;
    const header = hasHeader ? readable(entry.bytes.subarray(0, 2)) : "";
    const full = h("pre.readable");
    appendReadable(full, entry.bytes);
    const decoded = h("pre.decoded", {}, "…");
    const facts = [
      hasHeader ? `${header} · ID ${entry.id}` : message ? "No header" : entry.kind === "wire" ? "TCP chunk" : "Frame header",
      entry.inbound ? "server to game" : "game to server",
      formatTime(entry.time),
      `${entry.bytes.length} bytes`,
      message ? (entry.decrypted ? "decrypted" : "plain") : entry.fallback ? "encrypted" : "",
    ].filter(Boolean);
    const copy = (text: string) => () => void navigator.clipboard?.writeText(text);
    const addFilter = (token: string) => () => {
      const tokens = this.headers.value.split(/[\s,]+/).filter((t) => t && t !== token.replace(/^!/, "") && t !== `!${token.replace(/^!/, "")}`);
      this.headers.value = [...tokens, token].join(" ");
      this.refilter();
    };
    this.detail.replaceChildren(
      h(
        "div.detail-head",
        {},
        h(`i.swatch.${entry.inbound ? "in" : "out"}`),
        h("strong", {}, facts.join(" · ")),
        h("span.grow"),
        hasHeader ? h("button.small", { type: "button", onclick: addFilter(`!${header}`), title: `Hide ${header} messages` }, `Hide ${header}`) : null,
        hasHeader ? h("button.small", { type: "button", onclick: addFilter(header), title: `Only show ${header} messages` }, `Only ${header}`) : null,
        h("button.small", { type: "button", onclick: copy(readable(entry.bytes)) }, "Copy"),
        h("button.small", { type: "button", onclick: copy(toHex(entry.bytes)) }, "Copy hex"),
        message ? h("button.small", { type: "button", onclick: () => this.openInBuilder(entry) }, "Open in builder") : null,
      ),
      h(
        "div.detail-body",
        {},
        h("div.detail-col", {}, h("h3", {}, "Message"), full, h("h3", {}, "Values"), decoded),
        h("div.detail-col", {}, h("h3", {}, "Hex"), h("pre.hexdump", {}, hexDump(entry.bytes))),
      ),
    );
    const token = ++this.decodeToken;
    const values = hasHeader ? entry.bytes.subarray(2) : entry.bytes;
    if (!message || !values.length) {
      decoded.textContent = message ? "No values." : "Encrypted or framing bytes; nothing to decode.";
      return;
    }
    this.app.backend
      .decode(readable(values))
      .then((lines) => token === this.decodeToken && (decoded.textContent = lines.join("\n")))
      .catch((error) => token === this.decodeToken && (decoded.textContent = String(error)));
  }

  private openInBuilder(entry: Entry | undefined) {
    if (!entry || entry.kind !== "message") return;
    this.app.builder.load(entry);
    this.app.selectTab(this.app.builder);
  }
}
