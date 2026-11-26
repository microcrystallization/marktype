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
      if (Date.now() >= deadline) throw new Error(typeof label === "function" ? label() : label);
      await new Promise((resolve) => setTimeout(resolve, 40));
    }
  }

  window.addEventListener("DOMContentLoaded", async () => {
    let error;
    try {
      const preview = document.getElementById("preview-doc");
      await waitFor("initial document did not arrive over IPC", () => editor && document.getElementById("about-version").textContent);
      if (document.querySelector(".panes-wrap.tex-document")) {
        await waitFor("LaTeX command was not highlighted", () => Array.from(document.querySelectorAll(".cm-content [class*='hl-']"))
          .some((token) => token.textContent.includes("section")));
        editor.replace(editor.text.length, editor.text.length, "\nEdited in MarkType desktop smoke.\n");
        const docSaved = () => !document.querySelector(".doc-tab.active .doc-tab-dirty");
        await waitFor("The edited source was not marked dirty", () => !docSaved());
        document.querySelector(".cm-content").dispatchEvent(new KeyboardEvent("keydown", {
          key: "Enter", ctrlKey: true, bubbles: true, cancelable: true,
        }));
        await waitFor("LaTeX project main could not be detected", () =>
          /[/\\]main\.tex$/i.test(document.getElementById("latex-main").value));
        document.getElementById("latex-build").click();
        await waitFor("LaTeX did not produce an openable PDF: " + document.getElementById("latex-status").textContent,
          () => !document.getElementById("latex-open").disabled ||
            (!document.getElementById("latex-build").disabled && document.getElementById("latex-log").textContent));
        if (document.getElementById("latex-open").disabled || !docSaved()) {
          throw new Error(document.getElementById("latex-status").textContent + "\n" + document.getElementById("latex-log").textContent.slice(-3500));
        }
        document.getElementById("latex-open").click();
        await waitFor("Built-in PDF viewer did not render a PDF page canvas", () =>
          document.querySelector(".panes-wrap.tex-pdf-open") &&
          document.querySelector(".pdf-page-wrap[data-rendered='true'] canvas"));
        await waitFor("SyncTeX forward button was not enabled", () =>
          !document.getElementById("btn-tex-sync").disabled);
        const events = [];
        const errors = [];
        window.addEventListener("error", (ev) => errors.push(String(ev.message || ev.error)));
        const origRecv = window.__mtReceive;
        window.__mtReceive = (msg) => {
          if (msg?.event?.startsWith("synctex") || msg?.event?.startsWith("pdf") || msg?.event === "tabSwitched") events.push(msg);
          try { return origRecv(msg); } catch (err) { errors.push(String(err)); throw err; }
        };
        document.getElementById("btn-tex-sync").click();
        await waitFor(
          () => "Forward SyncTeX did not highlight the PDF (" + JSON.stringify({
            toast: document.getElementById("toast")?.textContent,
            events,
            errors,
            pages: document.querySelectorAll(".pdf-page-wrap").length,
          }) + ")",
          () => document.querySelector(".pdf-synctex-box")
        );

        const pages = document.getElementById("pdf-pages");
        const viewport = document.getElementById("pdf-viewport");
        const firstPage = pages.firstElementChild;
        for (let i = 0; i < 10; i++) document.getElementById("pdf-zoom-in").click();
        if (pages.firstElementChild !== firstPage || !pages.querySelector(".pdf-synctex-box")) {
          throw new Error("Zoom recreated the PDF layout or lost the current highlight");
        }
        viewport.scrollLeft = 0;
        if (firstPage.getBoundingClientRect().left < viewport.getBoundingClientRect().left) {
          throw new Error("The PDF page is clipped at the horizontal scroll limit");
        }
        document.getElementById("pdf-fit").click();

        const source = (name) => document.getElementById("latex-main").value.replace(/main\.tex$/i, `sections/${name}.tex`);
        const open = (name) => window.ipc.postMessage(JSON.stringify({ cmd: "openPath", path: source(name) }));
        const active = (name) => document.querySelector(".doc-tab.active")?.textContent.includes(`${name}.tex`);
        open("results");
        await waitFor("results chapter did not open", () => active("results"));
        editor.select(4, 5);
        const forwarded = () => events.filter((entry) => entry.event === "synctexForwardResult").length;
        const count = forwarded();
        document.getElementById("btn-tex-sync").click();
        await waitFor("Results did not locate its PDF page", () => forwarded() > count &&
          pages.querySelector('.pdf-page-wrap[data-page="2"] .pdf-synctex-box'));
        open("introduction");
        await waitFor("introduction chapter did not open", () => active("introduction"));
        const highlight = pages.querySelector('.pdf-page-wrap[data-page="2"] .pdf-synctex-box');
        if (!highlight) throw new Error("Switching to a chapter of the same project lost its PDF");
        const rect = highlight.getBoundingClientRect();
        highlight.dispatchEvent(new MouseEvent("dblclick", {
          bubbles: true, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2,
        }));
        await waitFor(() => "Inverse SyncTeX did not return to the results chapter (" + JSON.stringify(events.filter((event) => event.event?.startsWith("synctex"))) + ")",
          () => active("results") && events.some((event) => event.event === "synctexInverseResult" && /[/\\]results\.tex$/i.test(event.path)));
        window.ipc.postMessage(JSON.stringify({ cmd: "smokeResult" }));
        return;
      }
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
