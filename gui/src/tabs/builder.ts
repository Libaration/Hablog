// Writing a message by hand, and decoding values from the log.
import type { App, Tab } from "../app";
import { appendReadable, headerId, headerText, hexDump, hexToBytes, readable } from "../fuse";
import type { Entry } from "../store";
import type { Part } from "../types";
import { group, h } from "../ui";

const KINDS: [string, string, string][] = [
  ["i", "Number", "5"],
  ["b", "Boolean", "true"],
  ["s", "String", "text"],
  ["r", "Raw", "bytes as in the log, [2] is byte 2"],
];

const encoder = new TextEncoder();

interface Row {
  element: HTMLElement;
  kind: HTMLSelectElement;
  text: HTMLInputElement;
}

export class BuilderTab implements Tab {
  readonly title = "Builder";
  readonly element = h("section.panel.builder");
  private direction: "in" | "out" = "in";
  private directions: Record<"in" | "out", HTMLInputElement>;
  private header = h("input.header", { type: "text", placeholder: "@C or 3", spellcheck: "false" });
  private headerNote = h("span.hint");
  private parts = h("ol.parts");
  private rows: Row[] = [];
  private preview = h("pre.readable");
  private hex = h("pre.hexdump");
  private size = h("span.hint");
  private error = h("p.error");
  private built: { readable: string; hex: string } | null = null;
  private decodeInput = h("input", { type: "text", placeholder: "A value from the log: PB, IHH[1] or a number like 70", spellcheck: "false" });
  private decodeOutput = h("pre.decoded");
  private buildToken = 0;
  private timer = 0;

  constructor(private app: App) {
    const radio = (value: "in" | "out", label: string) => {
      const input = h("input", { type: "radio", name: "builder-direction", value, checked: value === "in" });
      input.addEventListener("change", () => {
        this.direction = value;
        this.changed();
      });
      return [input, h("label.toggle", {}, input, h("i", { class: `swatch ${value}` }), label)] as const;
    };
    const [inRadio, inLabel] = radio("in", "In · server to game");
    const [outRadio, outLabel] = radio("out", "Out · game to server");
    this.directions = { in: inRadio, out: outRadio };
    this.header.addEventListener("input", () => this.changed());

    const add = h(
      "div.add",
      {},
      ...KINDS.map(([kind, label]) =>
        h("button.small", { type: "button", onclick: () => this.addRow(kind, "", true) }, `+ ${label}`),
      ),
    );
    const message = group(
      "Message",
      h("div.field", {}, h("span.label", {}, "Direction"), h("div.choices", {}, inLabel, outLabel)),
      h("div.field", {}, h("span.label", {}, "Header"), this.header, this.headerNote),
      h("div.field.top", {}, h("span.label", {}, "Values"), h("div.values", {}, this.parts, add)),
      h(
        "p.hint",
        {},
        "Numbers and booleans are VL64. A string sent to the game ends with [2]; one sent to the server starts with its length. Raw bytes are added exactly as written.",
      ),
    );

    const copy = (pick: (built: { readable: string; hex: string }) => string) => () => {
      if (this.built) void navigator.clipboard?.writeText(pick(this.built));
    };
    const send = h("button", { type: "button", disabled: true }, "Send to the game");
    const result = group(
      "Result",
      this.error,
      h("h3", {}, "Message ", this.size),
      this.preview,
      h("h3", {}, "Hex"),
      this.hex,
      h(
        "div.actions",
        {},
        h("button.small", { type: "button", onclick: copy((built) => built.readable) }, "Copy"),
        h("button.small", { type: "button", onclick: copy((built) => built.hex) }, "Copy hex"),
        h("span.grow"),
        send,
      ),
      h("p.hint", {}, "Sending isn't built yet; see docs/tasks/sending-messages.md."),
    );

    let decoding = 0;
    this.decodeInput.addEventListener("input", () => {
      window.clearTimeout(decoding);
      decoding = window.setTimeout(() => this.decode(), 150);
    });
    const decode = group("Decode", this.decodeInput, this.decodeOutput);

    this.element.append(h("div.columns", {}, h("div.column", {}, message, decode), h("div.column", {}, result)));
    this.header.value = "@C";
    this.addRow("b", "true", false);
    this.addRow("i", "0", false);
    this.changed();
  }

  update() {}

  /** Fills the builder with a message from the log, as one raw value. */
  load(entry: Entry) {
    this.direction = entry.inbound ? "in" : "out";
    this.directions[this.direction].checked = true;
    const hasHeader = entry.id !== null;
    this.header.value = hasHeader ? readable(entry.bytes.subarray(0, 2)) : "";
    let body = hasHeader ? entry.bytes.subarray(2) : entry.bytes;
    // The builder ends a message for the game with [1] itself.
    if (entry.inbound && body[body.length - 1] === 1) body = body.subarray(0, -1);
    this.rows.forEach((row) => row.element.remove());
    this.rows = [];
    if (body.length) this.addRow("r", readable(body), false);
    this.changed();
  }

  private addRow(kind: string, text: string, focus: boolean) {
    const select = h("select", { "aria-label": "Kind" });
    for (const [value, label] of KINDS) select.append(h("option", { value, selected: value === kind }, label));
    const input = h("input", { type: "text", spellcheck: "false" });
    input.value = text;
    const placeholder = () => (input.placeholder = KINDS.find(([value]) => value === select.value)![2]);
    placeholder();
    const row: Row = { element: h("li"), kind: select, text: input };
    const remove = h("button.small.icon", { type: "button", title: "Remove", "aria-label": "Remove" }, "×");
    remove.addEventListener("click", () => {
      row.element.remove();
      this.rows = this.rows.filter((other) => other !== row);
      this.changed();
    });
    select.addEventListener("change", () => {
      placeholder();
      this.changed();
    });
    input.addEventListener("input", () => this.changed());
    row.element.append(select, input, remove);
    this.rows.push(row);
    this.parts.append(row.element);
    if (focus) input.focus();
    this.changed();
  }

  private changed() {
    const header = this.header.value.trim();
    const id = /^\d+$/.test(header) ? Number(header) : header.length === 2 ? headerId(encoder.encode(header)) : null;
    this.headerNote.textContent =
      id === null || id > 4095 ? "" : /^\d+$/.test(header) ? `header ${headerText(id)}` : `ID ${id}`;
    window.clearTimeout(this.timer);
    this.timer = window.setTimeout(() => void this.build(), 100);
  }

  private async build() {
    const parts: Part[] = this.rows.map((row) => [row.kind.value, row.text.value]);
    const token = ++this.buildToken;
    try {
      const built = await this.app.backend.build(this.direction, this.header.value, parts);
      if (token !== this.buildToken) return;
      const bytes = hexToBytes(built.hex);
      this.built = built;
      this.error.textContent = "";
      this.preview.replaceChildren();
      appendReadable(this.preview, bytes);
      this.hex.textContent = hexDump(bytes);
      this.size.textContent = `${bytes.length} bytes`;
    } catch (error) {
      if (token !== this.buildToken) return;
      this.built = null;
      this.error.textContent = String(error);
      this.preview.textContent = "";
      this.hex.textContent = "";
      this.size.textContent = "";
    }
  }

  private async decode() {
    const value = this.decodeInput.value;
    if (!value.trim()) {
      this.decodeOutput.textContent = "";
      return;
    }
    try {
      this.decodeOutput.textContent = (await this.app.backend.decode(value)).join("\n");
    } catch (error) {
      this.decodeOutput.textContent = String(error);
    }
  }
}
