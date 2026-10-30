// What Hablog is and how to read the log.
import type { App, Tab } from "../app";
import { group, h } from "../ui";

export class AboutTab implements Tab {
  readonly title = "About";
  readonly element = h("section.panel.about");

  constructor(_app: App) {
    const legend = (swatch: string, label: string, text: string) =>
      h("li", {}, h(`i.swatch.${swatch}`), h("strong", {}, label), text);
    this.element.append(
      h(
        "div.columns",
        {},
        h(
          "div.column",
          {},
          group(
            "Hablog",
            h("p.title", {}, "HABLOG"),
            h("p", {}, "A packet logger for Habbo Origins, by Libaration."),
            h(
              "p",
              {},
              "Hablog makes a copy of the game that connects to it instead of the hotel, and passes everything on unchanged. Your game files and the Habbo Launcher are left alone.",
            ),
          ),
          group(
            "How it works",
            h(
              "ol.steps",
              {},
              h("li", {}, "Finds the Origins client the Habbo Launcher installed."),
              h("li", {}, "Writes Hablog-Routed.exe, a copy that connects to 127.0.0.1."),
              h("li", {}, "Relays the game's connection to the hotel and logs both directions."),
              h("li", {}, "With the observer, reads the messages before encryption and after decryption."),
            ),
          ),
        ),
        h(
          "div.column",
          {},
          group(
            "Reading the log",
            h(
              "ul.legend-list",
              {},
              legend("in", "In", "the server to the game"),
              legend("out", "Out", "the game to the server"),
            ),
            h(
              "p",
              {},
              "Each message starts with a two-character header such as @C; the ID is its number. Bytes that aren't printable are written [n], so a message to the game ends with [1] and its strings with [2].",
            ),
            h("p", {}, "Numbers are VL64: one to six characters, the first saying how many. Select a message to decode its values."),
          ),
          group(
            "Keys",
            h(
              "dl.rows",
              {},
              h("dt", {}, "↑ ↓  PgUp PgDn"),
              h("dd", {}, "Move through the log"),
              h("dt", {}, "Enter"),
              h("dd", {}, "Open the message in the builder"),
              h("dt", {}, "Esc"),
              h("dd", {}, "Clear the selection"),
            ),
          ),
        ),
      ),
    );
  }

  update() {}
}
