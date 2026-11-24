import { WidgetType } from "@codemirror/view";

// Request only widgets with a DOM view. Several widgets measured in the same frame share one IPC request.
export function createWidgetRenderer(renderFragments, hydrate) {
  const cache = new Map();
  let pending = [];
  let revision = 0;
  let destroyed = false;

  async function flush() {
    const queue = pending;
    pending = [];
    for (let start = 0; start < queue.length; start += 16) {
      const batch = queue.slice(start, start + 16);
      const html = destroyed ? [] : await renderFragments(batch.map((entry) => entry.source)).catch(() => []);
      batch.forEach((entry, index) => entry.resolve(html[index] ?? ""));
    }
  }

  return {
    get revision() { return revision; },
    hydrate,
    render(source) {
      if (cache.has(source)) return cache.get(source);
      const result = new Promise((resolve) => {
        if (!pending.length) queueMicrotask(flush);
        pending.push({ source, resolve });
      });
      if (cache.size >= 128) cache.delete(cache.keys().next().value);
      cache.set(source, result);
      return result;
    },
    invalidate() {
      revision++;
      cache.clear();
    },
    destroy() {
      destroyed = true;
      cache.clear();
    },
  };
}

export function enableSourceReveal(view, element) {
  const followLink = (event) => event.target.closest("a[href]") && (event.metaKey || event.ctrlKey);
  element.addEventListener("mousedown", (event) => {
    if (event.button === 0 && !followLink(event)) event.preventDefault();
  });
  element.addEventListener("click", (event) => {
    if (followLink(event)) return;
    event.preventDefault();
    const from = view.posAtDOM(element);
    const cursor = Math.min(view.state.doc.length, from + 1);
    view.dispatch({ selection: { anchor: cursor }, scrollIntoView: true });
    view.focus();
  });
}

export class RenderedMarkdown extends WidgetType {
  constructor(source, kind, block, renderer, label = "", tableRows = 0) {
    super();
    Object.assign(this, { source, kind, block, renderer, label, tableRows, revision: renderer.revision });
  }

  eq(other) {
    return this.kind === other.kind && this.block === other.block && this.source === other.source &&
      this.revision === other.revision;
  }

  get estimatedHeight() {
    if (!this.block) return -1;
    if (this.kind === "table") return this.tableRows * 34 + 20;
    if (this.kind === "diagram") return 160;
    return this.kind === "image" ? 140 : 76;
  }

  toDOM(view) {
    const document = view.dom.ownerDocument;
    const element = document.createElement(this.block ? "div" : "span");
    element.className = `cm-live-widget cm-live-widget-${this.kind}` + (this.block ? " preview-doc" : " cm-live-widget-inline");
    element.textContent = this.label;

    const refresh = () => {
      if (element.isConnected) view.requestMeasure();
    };
    element.addEventListener("load", refresh, true);
    element.addEventListener("error", refresh, true);
    enableSourceReveal(view, element);

    void this.renderer.render(this.source).then((html) => {
      if (!html || !element.isConnected || this.revision !== this.renderer.revision) return;
      if (this.block) {
        element.innerHTML = html;
      } else {
        const template = document.createElement("template");
        template.innerHTML = html;
        const fragment = template.content;
        const first = fragment.firstElementChild;
        if (first?.tagName === "P" && fragment.childElementCount === 1) element.replaceChildren(...first.childNodes);
        else element.replaceChildren(fragment);
      }
      for (const checkbox of element.querySelectorAll(".mt-task-cb")) checkbox.tabIndex = -1;
      void this.renderer.hydrate(element).then(refresh);
      refresh();
    });
    return element;
  }
}
