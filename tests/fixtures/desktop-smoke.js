// Runs only with --smoke-test, against the real native WebView and IPC.
(() => {
  let editor;
  Object.defineProperty(window, "MTEditor", {
    configurable: true,
    set(module) {
      Object.defineProperty(window, "MTEditor", { configurable: true, value: {
        create(...args) { editor = module.create(...args); return editor; },
      } });
    },
  });
  async function waitFor(label, condition) {
    const deadline = Date.now() + 12000;
    while (!condition()) {
      if (Date.now() >= deadline) throw new Error(label);
      await new Promise((resolve) => setTimeout(resolve, 40));
    }
  }

  window.addEventListener("DOMContentLoaded", async () => {
    let error;
    try {
      const preview = document.getElementById("preview-doc");
      await waitFor("initial document did not arrive over IPC", () =>
        document.getElementById("about-version").textContent !== "" &&
        editor?.text.includes("MarkType") &&
        preview.querySelector("h1"));
      await waitFor("KaTeX did not hydrate the document", () => preview.querySelector(".katex"));
      await waitFor("the custom protocol did not serve the titlebar icon", () =>
        document.querySelector(".brand-mark").naturalWidth > 0);
      await waitFor("the wallpaper did not load", () =>
        document.getElementById("hero-curr").naturalWidth > 0);
      if (getComputedStyle(document.querySelector(".app-shell")).display !== "flex") {
        throw new Error("the application stylesheet did not load");
      }

      editor.replace(0, editor.text.length, "\u{1f9ea} Smoke\n\n" + editor.text + "\n\n**Desktop smoke edit**\n");
      await waitFor("the edited Markdown was not rendered", () =>
        Array.from(preview.querySelectorAll("strong")).some((node) =>
          node.textContent === "Desktop smoke edit"));

      editor.select(0, 2);
      const shared = editor;
      document.querySelector('.mode-btn[data-mode="live"]').click();
      const live = document.getElementById("editor-host");
      await waitFor("CodeMirror did not mount with rendered math", () => live.querySelector(".katex"));
      document.querySelector('[data-fmt="bold"]').click();
      await waitFor("the live edit did not reach CodeMirror", () =>
        live.querySelector(".cm-line").textContent.startsWith("\u{1f9ea} **bold text**Smoke"));
      document.querySelector('.mode-btn[data-mode="split"]').click();
      await waitFor("split did not render the live edit", () =>
        preview.querySelector("strong")?.textContent === "bold text" &&
        editor.text.startsWith("\u{1f9ea} **bold text**Smoke"));
      document.querySelector('.mode-btn[data-mode="source"]').click();
      await waitFor("the editor was recreated across modes", () =>
        editor === shared && document.querySelector(".panes-wrap.mode-source"));
      const mac = /Mac/.test(navigator.platform);
      live.querySelector(".cm-content").dispatchEvent(new KeyboardEvent("keydown", {
        key: "z", ctrlKey: !mac, metaKey: mac, bubbles: true, cancelable: true,
      }));
      await waitFor("the Source view could not undo a Live edit", () => !editor.text.includes("**bold text**"));
      document.querySelector('.mode-btn[data-mode="split"]').click();
      await waitFor("Split view did not load after undo", () => document.querySelector(".panes-wrap.mode-split"));

      const languages = [
        ["python", "def square(value): return value ** 2", "def"],
        ["cpp", "int main() { return 0; }", "return"],
        ["go", "package sample", "package"],
        ["bash", "export READY=1", "export"],
        ["json", '{"visible": true}', "true"],
        ["yml", 'status: "ready"', '"ready"'],
      ];
      const svg = btoa('<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="black"/></svg>');
      const sample = [
        "Live widget verification", "", "Pair: $x+1$ and $y+2$.", "",
        "An [example link](https://example.com/live-widget-url)", "",
        "![Smoke image](data:image/svg+xml;base64," + svg + ")", "",
        "| Name | Score |", "| --- | ---: |", "| Alpha | 2 |", "",
        "> [!NOTE]", "> Rendered note with **bold content**.", "",
        "~~~mermaid", "flowchart LR", " A[Source] --> B[Preview]", "~~~", "",
        ...languages.flatMap(([name, code]) => ["~~~" + name, code, "~~~", ""]),
      ].join("\n");
      editor.replace(0, editor.text.length, sample);
      await waitFor("sample document was not updated", () => preview.querySelector("table td")?.textContent === "Alpha");
      await waitFor("Mermaid did not render in Split", () => preview.querySelector(".mt-diagram svg"));
      editor.select(0, 0);
      document.querySelector('.mode-btn[data-mode="live"]').click();
      await waitFor("Live rich widgets did not render", () =>
        live.querySelector(".cm-live-widget-table td")?.textContent === "Alpha" &&
        live.querySelector(".cm-live-widget-callout .mt-callout-note strong")?.textContent === "bold content" &&
        live.querySelector(".cm-live-widget-image img")?.naturalWidth === 8 &&
        live.querySelector(".cm-live-widget-link a")?.textContent === "example link" &&
        live.querySelector(".cm-live-widget-diagram svg") &&
        !live.textContent.includes("https://example.com/live-widget-url"));
      await waitFor("a Live code language did not highlight", () => languages.every(([, code, token]) =>
        Array.from(live.querySelectorAll(".cm-line")).some((line) => line.textContent === code &&
          Array.from(line.querySelectorAll('[class*="hl-"]')).some((span) => span.textContent === token))));

      const pair = () => Array.from(live.querySelectorAll(".cm-line")).find((line) => line.textContent.startsWith("Pair:"));
      await waitFor("both inline formulas must initially render", () => pair()?.querySelectorAll(".cm-live-math-inline").length === 2);
      pair().querySelector(".cm-live-math-inline").click();
      await waitFor("editing one formula expanded another formula on the same line", () =>
        pair()?.textContent.includes("$x+1$") && pair().querySelectorAll(".cm-live-math-inline").length === 1 &&
        pair().querySelector("annotation")?.textContent === "y+2");
      live.querySelector(".cm-live-widget-table td").click();
      await waitFor("clicking a table did not reveal its Markdown", () =>
        !live.querySelector(".cm-live-widget-table") && live.textContent.includes("| Alpha | 2 |"));
    } catch (failure) {
      error = String(failure);
    }
    window.ipc.postMessage(JSON.stringify({ cmd: "smokeResult", error }));
  });
})();
