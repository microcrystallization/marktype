(() => {
  let library;
  let serial = 0;
  const observed = new WeakSet();
  const started = new WeakMap();

  function load() {
    if (!library) {
      const script = window.mermaid ? Promise.resolve() : new Promise((resolve, reject) => {
        const element = document.createElement("script");
        element.src = "/vendor/mermaid.js";
        element.onload = resolve;
        element.onerror = () => { element.remove(); reject(Error("Mermaid could not load")); };
        document.head.append(element);
      });
      library = script.then(() => {
        window.mermaid.initialize({
          startOnLoad: false, securityLevel: "strict", theme: "neutral", htmlLabels: false,
          suppressErrorRendering: true,
          secure: ["secure", "securityLevel", "startOnLoad", "maxTextSize", "maxEdges", "htmlLabels"],
        });
        return window.mermaid;
      }).catch((err) => {
        library = null;
        throw err;
      });
    }
    return library;
  }

  function diagram(element) {
    const prior = started.get(element);
    if (prior) return prior;
    const task = (async () => {
      const source = element.textContent.trim();
      if (!source) return;
      try {
        const mermaid = await load();
        if (!element.isConnected) return;
        const { svg } = await mermaid.render(`marktype-diagram-${++serial}`, source);
        if (element.isConnected) {
          element.innerHTML = svg;
          element.classList.add("mt-diagram-rendered");
        }
      } catch {
        element.classList.add("mt-diagram-fallback");
      }
    })();
    started.set(element, task);
    return task;
  }

  let observer;
  function observe(root) {
    for (const element of root.querySelectorAll(".mt-diagram")) {
      if (observed.has(element)) continue;
      observed.add(element);
      if (!observer) observer = new IntersectionObserver((entries) => {
        for (const entry of entries) if (entry.isIntersecting) {
          observer.unobserve(entry.target);
          void diagram(entry.target);
        }
      }, { rootMargin: "160px" });
      observer.observe(element);
    }
  }

  window.MTDiagrams = { observe, render: (root) => Promise.all(Array.from(root.querySelectorAll(".mt-diagram"), diagram)) };
})();
