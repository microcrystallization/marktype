(() => {
  "use strict";

  let pdfjsPromise = null;

  function loadPdfJs() {
    if (window.pdfjsLib) return Promise.resolve(window.pdfjsLib);
    if (pdfjsPromise) return pdfjsPromise;
    pdfjsPromise = new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = "/vendor/pdf.js";
      script.onload = () => {
        if (!window.pdfjsLib) {
          reject(new Error("PDF.js failed to initialize"));
          return;
        }
        window.pdfjsLib.GlobalWorkerOptions.workerSrc = "/vendor/pdf.worker.mjs";
        resolve(window.pdfjsLib);
      };
      script.onerror = () => reject(new Error("Failed to load bundled PDF.js"));
      document.head.appendChild(script);
    });
    return pdfjsPromise;
  }

  function createViewer(options) {
    const {
      viewportEl,
      pagesEl,
      pageInputEl,
      pageTotalEl,
      zoomLabelEl,
      statusEl,
      onInverseSync,
      onStateChange,
    } = options;

    let pdfDoc = null;
    let loadSerial = 0;
    let currentPath = null;
    let currentTitle = "";
    let hasSyncTex = false;
    let fitMode = true;
    let scale = 1.0;
    let currentPage = 1;
    let pageSlots = [];
    let observer = null;
    let resizeTimer = null;
    let highlightTimer = null;

    function notify() {
      if (onStateChange) {
        onStateChange({
          loaded: Boolean(pdfDoc),
          path: currentPath,
          title: currentTitle,
          hasSyncTex,
          page: currentPage,
          numPages: pdfDoc ? pdfDoc.numPages : 0,
          scale,
          fitMode,
        });
      }
    }

    function updateToolbar() {
      const total = pdfDoc ? pdfDoc.numPages : 0;
      if (pageInputEl) {
        pageInputEl.value = total ? String(currentPage) : "";
        pageInputEl.max = String(Math.max(1, total));
        pageInputEl.disabled = total === 0;
      }
      if (pageTotalEl) {
        pageTotalEl.textContent = total ? `/ ${total}` : "/ 0";
      }
      if (zoomLabelEl) {
        zoomLabelEl.textContent = `${Math.round(scale * 100)}%`;
      }
      notify();
    }

    function disconnectObserver() {
      if (observer) {
        observer.disconnect();
        observer = null;
      }
    }

    function cancelAllRenders() {
      for (const slot of pageSlots) {
        if (slot.renderTask) {
          try { slot.renderTask.cancel(); } catch {}
          slot.renderTask = null;
        }
        slot.renderedScale = 0;
      }
    }

    function computeFitScale(pageWidthPt) {
      const available = Math.max(240, viewportEl.clientWidth - 40);
      return Math.min(2.5, Math.max(0.45, available / pageWidthPt));
    }

    async function renderSlot(slot, serial) {
      if (!pdfDoc || serial !== loadSerial) return;
      const targetScale = slot.scale;
      if (slot.renderedScale === targetScale && !slot.renderTask) return;
      if (slot.renderTask) {
        try { slot.renderTask.cancel(); } catch {}
        slot.renderTask = null;
      }
      const page = await pdfDoc.getPage(slot.pageNum);
      if (serial !== loadSerial) return;
      const dpr = Math.min(window.devicePixelRatio || 1, 2.5);
      const cssViewport = page.getViewport({ scale: targetScale });
      const renderViewport = page.getViewport({ scale: targetScale * dpr });

      slot.wrap.style.width = `${cssViewport.width}px`;
      slot.wrap.style.height = `${cssViewport.height}px`;
      slot.canvas.width = Math.max(1, Math.floor(renderViewport.width));
      slot.canvas.height = Math.max(1, Math.floor(renderViewport.height));
      slot.canvas.style.width = `${cssViewport.width}px`;
      slot.canvas.style.height = `${cssViewport.height}px`;

      const ctx = slot.canvas.getContext("2d", { alpha: false });
      ctx.fillStyle = "#ffffff";
      ctx.fillRect(0, 0, slot.canvas.width, slot.canvas.height);

      const task = page.render({ canvasContext: ctx, viewport: renderViewport });
      slot.renderTask = task;
      try {
        await task.promise;
        if (serial === loadSerial) {
          slot.renderedScale = targetScale;
          slot.wrap.dataset.rendered = "true";
        }
      } catch (err) {
        if (err && err.name !== "RenderingCancelledException") {
          console.warn("PDF page render failed", slot.pageNum, err);
        }
      } finally {
        if (slot.renderTask === task) slot.renderTask = null;
      }
    }

    async function buildLayout(preserveRatio) {
      if (!pdfDoc) return;
      const serial = loadSerial;
      disconnectObserver();
      cancelAllRenders();

      const prevScrollTop = viewportEl.scrollTop;
      const prevScrollHeight = Math.max(1, viewportEl.scrollHeight);
      const ratio = preserveRatio ? prevScrollTop / prevScrollHeight : 0;

      const firstPage = await pdfDoc.getPage(1);
      if (serial !== loadSerial) return;
      const baseViewport = firstPage.getViewport({ scale: 1.0 });
      if (fitMode) {
        scale = computeFitScale(baseViewport.width);
      }

      const fragment = document.createDocumentFragment();
      pageSlots = [];

      for (let num = 1; num <= pdfDoc.numPages; num++) {
        const page = num === 1 ? firstPage : await pdfDoc.getPage(num);
        if (serial !== loadSerial) return;
        const vp = page.getViewport({ scale });
        const wrap = document.createElement("div");
        wrap.className = "pdf-page-wrap";
        wrap.dataset.page = String(num);
        wrap.style.width = `${vp.width}px`;
        wrap.style.height = `${vp.height}px`;

        const canvas = document.createElement("canvas");
        canvas.className = "pdf-page-canvas";
        canvas.width = 1;
        canvas.height = 1;

        const syncLayer = document.createElement("div");
        syncLayer.className = "pdf-synctex-layer";

        wrap.append(canvas, syncLayer);
        fragment.appendChild(wrap);
        pageSlots.push({
          pageNum: num,
          wrap,
          canvas,
          syncLayer,
          widthPt: page.getViewport({ scale: 1.0 }).width,
          heightPt: page.getViewport({ scale: 1.0 }).height,
          scale,
          renderedScale: 0,
          renderTask: null,
        });
      }

      pagesEl.replaceChildren(fragment);

      if (preserveRatio && ratio > 0) {
        viewportEl.scrollTop = Math.round(ratio * viewportEl.scrollHeight);
      }

      observer = new IntersectionObserver(
        (entries) => {
          for (const entry of entries) {
            if (!entry.isIntersecting) continue;
            const num = Number(entry.target.dataset.page);
            const slot = pageSlots[num - 1];
            if (slot) renderSlot(slot, serial);
          }
        },
        { root: viewportEl, rootMargin: "500px 0px" }
      );
      for (const slot of pageSlots) observer.observe(slot.wrap);

      // Eagerly render the current visible page immediately.
      const initialSlot = pageSlots[Math.min(currentPage - 1, pageSlots.length - 1)] || pageSlots[0];
      if (initialSlot) renderSlot(initialSlot, serial);
      updateCurrentPageFromScroll();
      updateToolbar();
    }

    function updateCurrentPageFromScroll() {
      if (!pageSlots.length) return;
      const mid = viewportEl.scrollTop + viewportEl.clientHeight * 0.35;
      let found = 1;
      for (const slot of pageSlots) {
        const top = slot.wrap.offsetTop;
        const bottom = top + slot.wrap.offsetHeight;
        if (mid >= top && mid <= bottom) {
          found = slot.pageNum;
          break;
        }
        if (top <= mid) found = slot.pageNum;
      }
      if (found !== currentPage) {
        currentPage = found;
        updateToolbar();
      }
    }

    function clearHighlights() {
      clearTimeout(highlightTimer);
      for (const slot of pageSlots) {
        slot.syncLayer.replaceChildren();
      }
    }

    function triggerInverseAtEvent(event) {
      if (!pdfDoc || !currentPath || !onInverseSync) return;
      const wrap = event.target.closest(".pdf-page-wrap");
      if (!wrap) return;
      const pageNum = Number(wrap.dataset.page);
      const slot = pageSlots[pageNum - 1];
      if (!slot) return;
      const rect = wrap.getBoundingClientRect();
      const xPt = Math.max(0, (event.clientX - rect.left) / slot.scale);
      const yPt = Math.max(0, (event.clientY - rect.top) / slot.scale);
      onInverseSync({ path: currentPath, page: pageNum, x: xPt, y: yPt });
    }

    viewportEl.addEventListener("scroll", () => {
      updateCurrentPageFromScroll();
    }, { passive: true });

    pagesEl.addEventListener("dblclick", (event) => {
      event.preventDefault();
      triggerInverseAtEvent(event);
    });

    pagesEl.addEventListener("click", (event) => {
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault();
        triggerInverseAtEvent(event);
      }
    });

    if (typeof ResizeObserver !== "undefined") {
      const resizeObserver = new ResizeObserver(() => {
        if (!pdfDoc || !fitMode) return;
        clearTimeout(resizeTimer);
        resizeTimer = setTimeout(() => {
          if (pdfDoc && fitMode) buildLayout(true);
        }, 120);
      });
      resizeObserver.observe(viewportEl);
    }

    async function open(url, meta = {}) {
      const serial = ++loadSerial;
      const preserveScroll = Boolean(meta.preserveScroll && currentPath && currentPath === meta.path);
      currentPath = meta.path || null;
      currentTitle = meta.title || "";
      hasSyncTex = Boolean(meta.hasSyncTex);
      if (statusEl) {
        statusEl.textContent = meta.loadingText || "";
        statusEl.classList.toggle("hidden", !meta.loadingText);
      }
      try {
        const pdfjsLib = await loadPdfJs();
        if (serial !== loadSerial) return;
        const response = await fetch(url, { cache: "no-store" });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const buffer = await response.arrayBuffer();
        if (serial !== loadSerial) return;
        const prevDoc = pdfDoc;
        const loadingTask = pdfjsLib.getDocument({
          data: new Uint8Array(buffer),
          isEvalSupported: false,
        });
        const nextDoc = await loadingTask.promise;
        if (serial !== loadSerial) {
          try { await nextDoc.destroy(); } catch {}
          return;
        }
        pdfDoc = nextDoc;
        if (prevDoc) {
          try { await prevDoc.destroy(); } catch {}
        }
        if (!preserveScroll) currentPage = 1;
        else currentPage = Math.min(currentPage, pdfDoc.numPages);
        if (statusEl) {
          statusEl.textContent = "";
          statusEl.classList.add("hidden");
        }
        await buildLayout(preserveScroll);
      } catch (err) {
        if (serial !== loadSerial) return;
        if (statusEl) {
          statusEl.textContent = String(err?.message || err);
          statusEl.classList.remove("hidden");
        }
        notify();
      }
    }

    function close() {
      loadSerial++;
      disconnectObserver();
      cancelAllRenders();
      clearHighlights();
      if (pdfDoc) {
        const doc = pdfDoc;
        pdfDoc = null;
        try { doc.destroy(); } catch {}
      }
      pageSlots = [];
      currentPath = null;
      currentTitle = "";
      hasSyncTex = false;
      currentPage = 1;
      pagesEl.replaceChildren();
      if (statusEl) {
        statusEl.textContent = "";
        statusEl.classList.add("hidden");
      }
      updateToolbar();
    }

    function goToPage(page) {
      if (!pdfDoc || !pageSlots.length) return;
      const clamped = Math.max(1, Math.min(pdfDoc.numPages, Math.floor(Number(page) || 1)));
      const slot = pageSlots[clamped - 1];
      if (!slot) return;
      currentPage = clamped;
      viewportEl.scrollTop = Math.max(0, slot.wrap.offsetTop - 16);
      renderSlot(slot, loadSerial);
      updateToolbar();
    }

    function setZoom(nextScale) {
      if (!pdfDoc) return;
      fitMode = false;
      scale = Math.min(3.0, Math.max(0.4, Math.round(nextScale * 100) / 100));
      buildLayout(true);
    }

    function zoomIn() {
      setZoom(scale + 0.15);
    }

    function zoomOut() {
      setZoom(scale - 0.15);
    }

    function fitWidth() {
      if (!pdfDoc) return;
      fitMode = true;
      buildLayout(true);
    }

    function highlight(pageNum, boxes) {
      if (!pdfDoc || !pageSlots.length) return false;
      const slot = pageSlots[Math.max(0, Math.min(pageSlots.length - 1, pageNum - 1))];
      if (!slot) return false;
      clearHighlights();
      renderSlot(slot, loadSerial);

      const list = Array.isArray(boxes) && boxes.length
        ? boxes
        : [{ x: 36, y: 36, width: slot.widthPt - 72, height: 18 }];
      for (const box of list) {
        const el = document.createElement("div");
        el.className = "pdf-synctex-box";
        el.style.left = `${Math.max(0, box.x * slot.scale)}px`;
        el.style.top = `${Math.max(0, box.y * slot.scale)}px`;
        el.style.width = `${Math.max(18, box.width * slot.scale)}px`;
        el.style.height = `${Math.max(12, box.height * slot.scale)}px`;
        slot.syncLayer.appendChild(el);
      }

      const first = list[0];
      const targetTop = slot.wrap.offsetTop + first.y * slot.scale - viewportEl.clientHeight * 0.35;
      viewportEl.scrollTo({ top: Math.max(0, targetTop), behavior: "smooth" });
      currentPage = slot.pageNum;
      updateToolbar();

      highlightTimer = setTimeout(() => {
        for (const s of pageSlots) s.syncLayer.replaceChildren();
      }, 2600);
      return true;
    }

    return {
      open,
      close,
      goToPage,
      prevPage: () => goToPage(currentPage - 1),
      nextPage: () => goToPage(currentPage + 1),
      zoomIn,
      zoomOut,
      fitWidth,
      highlight,
      getState: () => ({
        loaded: Boolean(pdfDoc),
        path: currentPath,
        title: currentTitle,
        hasSyncTex,
        page: currentPage,
        numPages: pdfDoc ? pdfDoc.numPages : 0,
        scale,
        fitMode,
      }),
    };
  }

  window.MTPdfViewer = { create: createViewer };
})();
