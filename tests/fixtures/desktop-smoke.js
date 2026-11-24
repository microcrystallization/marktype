// Runs only with --smoke-test, against the real native WebView and IPC.
(() => {
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
      const editor = document.getElementById("source-editor");
      await waitFor("initial document did not arrive over IPC", () =>
        document.getElementById("about-version").textContent !== "" &&
        editor.value.includes("MarkType") &&
        preview.querySelector("h1"));
      await waitFor("KaTeX did not hydrate the document", () => preview.querySelector(".katex"));
      await waitFor("the custom protocol did not serve the titlebar icon", () =>
        document.querySelector(".brand-mark").naturalWidth > 0);
      await waitFor("the wallpaper did not load", () =>
        document.getElementById("hero-curr").naturalWidth > 0);
      if (getComputedStyle(document.querySelector(".app-shell")).display !== "flex") {
        throw new Error("the application stylesheet did not load");
      }

      editor.value = "\u{1f9ea} Smoke\n\n" + editor.value + "\n\n**Desktop smoke edit**\n";
      editor.dispatchEvent(new Event("input", { bubbles: true }));
      await waitFor("the edited Markdown was not rendered", () =>
        Array.from(preview.querySelectorAll("strong")).some((node) =>
          node.textContent === "Desktop smoke edit"));

      editor.setSelectionRange(3, 3);
      document.querySelector('.mode-btn[data-mode="live"]').click();
      const live = document.getElementById("live-editor-host");
      await waitFor("CodeMirror did not mount with rendered math", () => live.querySelector(".katex"));
      document.querySelector('[data-fmt="bold"]').click();
      await waitFor("the live edit did not reach CodeMirror", () =>
        live.querySelector(".cm-line").textContent.startsWith("\u{1f9ea} **bold text**Smoke"));
      document.querySelector('.mode-btn[data-mode="split"]').click();
      await waitFor("split did not render the live edit", () =>
        preview.querySelector("strong")?.textContent === "bold text" &&
        editor.value.startsWith("\u{1f9ea} **bold text**Smoke"));
    } catch (failure) {
      error = String(failure);
    }
    window.ipc.postMessage(JSON.stringify({ cmd: "smokeResult", error }));
  });
})();
