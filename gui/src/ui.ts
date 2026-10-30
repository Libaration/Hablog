// A small helper for building the DOM: h("div.class", { attrs }, children).

type Child = Node | string | null | undefined | false;
type Attrs = Record<string, string | number | boolean | ((event: Event) => void) | undefined>;

export function h<K extends keyof HTMLElementTagNameMap>(
  spec: K | `${K}.${string}`,
  attrs: Attrs = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const [tag, ...classes] = spec.split(".") as [K, ...string[]];
  const element = document.createElement(tag);
  if (classes.length) element.className = classes.join(" ");
  for (const [name, value] of Object.entries(attrs)) {
    if (value === undefined || value === false) continue;
    if (typeof value === "function") element.addEventListener(name.replace(/^on/, ""), value);
    else if (name === "class") element.className += ` ${value}`;
    else if (value === true) element.setAttribute(name, "");
    else element.setAttribute(name, String(value));
  }
  for (const child of children) if (child) element.append(child);
  return element;
}

/** A titled group box, in the spirit of the old tools' panels. */
export function group(title: string, ...children: Child[]): HTMLFieldSetElement {
  return h("fieldset.group", {}, h("legend", {}, title), ...children);
}

/** Label/value rows; returns the value cells so they can be updated. */
export function rows(parent: HTMLElement, labels: string[]): HTMLElement[] {
  const list = h("dl.rows");
  parent.append(list);
  return labels.map((label) => {
    const value = h("dd");
    list.append(h("dt", {}, label), value);
    return value;
  });
}
