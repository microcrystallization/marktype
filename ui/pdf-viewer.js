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
          pdfjsPromise = null;
          script.remove();
          reject(new Error("PDF.js failed to initialize"));
          return;
        }
        window.pdfjsLib.GlobalWorkerOptions.workerSrc = "/vendor/pdf.worker.mjs";
        resolve(window.pdfjsLib);
      };
      script.onerror = () => {
        pdfjsPromise = null;
        script.remove();
        reject(new Error("Failed to load bundled PDF.js"));
      };
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
    let layoutSerial = 0;
    let currentPath = null;
    let currentTitle = "";
    let hasSyncTex = false;
    let fitMode = true;
    let scale = 1.0;
    let currentPage = 1;
    let pageMeta = []; // cached { baseViewport, widthPt, heightPt } per page
    let pageSlots = [];
    let observer = null;
    let resizeTimer = null;
    let highlightTimer = null;
    let lastInverseAt = 0;

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
        if (document.activeElement !== pageInputEl) {
          pageInputEl.value = total ? String(currentPage) : "";
        }
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
        slot.renderGen++;
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

    async function renderSlot(slot, docSerial, laySerial) {
      if (!pdfDoc || docSerial !== loadSerial || laySerial !== layoutSerial) return;
      const targetScale = slot.scale;
      if (slot.renderedScale === targetScale && !slot.renderTask) return;

      const gen = ++slot.renderGen;
      if (slot.renderTask) {
        const prev = slot.renderTask;
        slot.renderTask = null;
        try { prev.cancel(); } catch {}
        try { await prev.promise; } catch {}
      }
      if (!pdfDoc || docSerial !== loadSerial || laySerial !== layoutSerial || gen !== slot.renderGen) {
        return;
      }

      const page = await pdfDoc.getPage(slot.pageNum);
      if (!pdfDoc || docSerial !== loadSerial || laySerial !== layoutSerial || gen !== slot.renderGen) {
        return;
      }

      const dpr = Math.min(window.devicePixelRatio || 1, 2.5);
      const cssViewport = page.getViewport({ scale: targetScale });
      const renderViewport = page.getViewport({ scale: targetScale * dpr });

      slot.viewport = cssViewport;
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
        if (docSerial === loadSerial && laySerial === layoutSerial && gen === slot.renderGen) {
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

    async function ensurePageMeta(docSerial) {
      if (pageMeta.length === pdfDoc.numPages) return true;
      const list = [];
      for (let num = 1; num <= pdfDoc.numPages; num++) {
        const page = await pdfDoc.getPage(num);
        if (docSerial !== loadSerial) return false;
        const baseViewport = page.getViewport({ scale: 1.0 });
        list.push({
          baseViewport,
          widthPt: baseViewport.width,
          heightPt: baseViewport.height,
        });
      }
      if (docSerial !== loadSerial) return false;
      pageMeta = list;
      return true;
    }

    async function buildLayout(preserveRatio) {
      if (!pdfDoc) return;
      const docSerial = loadSerial;
      const laySerial = ++layoutSerial;
      disconnectObserver();
      cancelAllRenders();

      const prevScrollTop = viewportEl.scrollTop;
      const prevScrollHeight = Math.max(1, viewportEl.scrollHeight);
      const ratio = preserveRatio ? prevScrollTop / prevScrollHeight : 0;

      const ready = await ensurePageMeta(docSerial);
      if (!ready || !pdfDoc || docSerial !== loadSerial || laySerial !== layoutSerial) return;
      if (!pageMeta.length) return;

      if (fitMode) {
        scale = computeFitScale(pageMeta[0].widthPt);
      }

      const fragment = document.createDocumentFragment();
      const nextSlots = [];

      for (let i = 0; i < pageMeta.length; i++) {
        const num = i + 1;
        const meta = pageMeta[i];
        const vp = meta.baseViewport.clone({ scale });
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
        nextSlots.push({
          pageNum: num,
          wrap,
          canvas,
          syncLayer,
          viewport: vp,
          widthPt: meta.widthPt,
          heightPt: meta.heightPt,
          scale,
          renderedScale: 0,
          renderTask: null,
          renderGen: 0,
        });
      }

      pageSlots = nextSlots;
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
            if (slot) renderSlot(slot, docSerial, laySerial);
          }
        },
        { root: viewportEl, rootMargin: "500px 0px" }
      );
      for (const slot of pageSlots) observer.observe(slot.wrap);

      const initialSlot = pageSlots[Math.min(currentPage - 1, pageSlots.length - 1)] || pageSlots[0];
      if (initialSlot) renderSlot(initialSlot, docSerial, laySerial);
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
      const now = Date.now();
      if (now - lastInverseAt < 180) return;
      const wrap = event.target.closest(".pdf-page-wrap");
      if (!wrap) return;
      const pageNum = Number(wrap.dataset.page);
      const slot = pageSlots[pageNum - 1];
      if (!slot) return;
      lastInverseAt = now;
      const rect = wrap.getBoundingClientRect();
      const cssX = Math.max(0, event.clientX - rect.left);
      const cssY = Math.max(0, event.clientY - rect.top);
      let xPt = cssX / slot.scale;
      let yPt = cssY / slot.scale;
      if (slot.viewport && typeof slot.viewport.convertToPdfPoint === "function") {
        const [pdfX, pdfY] = slot.viewport.convertToPdfPoint(cssX, cssY);
        const vb = slot.viewport.viewBox;
        xPt = Math.max(0, pdfX - vb[0]);
        yPt = Math.max(0, vb[3] - pdfY);
      }
      onInverseSync({ path: currentPath, page: pageNum, x: xPt, y: yPt });
    }

    viewportEl.addEventListener("scroll", () => {
      updateCurrentPageFromScroll();
    }, { passive: true });

    viewportEl.addEventListener("wheel", (event) => {
      if (!(event.ctrlKey || event.metaKey) || !pdfDoc) return;
      event.preventDefault();
      if (event.deltaY < 0) zoomIn();
      else if (event.deltaY > 0) zoomOut();
    }, { passive: false });

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

    let resizeObserver = null;
    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(() => {
        if (!pdfDoc || !fitMode || viewportEl.clientWidth < 100) return;
        clearTimeout(resizeTimer);
        resizeTimer = setTimeout(() => {
          if (pdfDoc && fitMode && viewportEl.clientWidth >= 100) buildLayout(true);
        }, 120);
      });
      resizeObserver.observe(viewportEl);
    }

    function resetDoc() {
      disconnectObserver();
      cancelAllRenders();
      clearHighlights();
      clearTimeout(resizeTimer);
      if (pdfDoc) {
        const doc = pdfDoc;
        pdfDoc = null;
        try { doc.destroy(); } catch {}
      }
      pageMeta = [];
      pageSlots = [];
      currentPath = null;
      currentTitle = "";
      hasSyncTex = false;
      currentPage = 1;
      pagesEl.replaceChildren();
    }

    async function open(url, meta = {}) {
      const serial = ++loadSerial;
      const preserveScroll = Boolean(meta.preserveScroll && currentPath && currentPath === meta.path);
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
        pageMeta = [];
        currentPath = meta.path || null;
        currentTitle = meta.title || "";
        hasSyncTex = Boolean(meta.hasSyncTex);
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
        resetDoc();
        if (statusEl) {
          statusEl.textContent = String(err?.message || err);
          statusEl.classList.remove("hidden");
        }
        updateToolbar();
      }
    }

    function close() {
      loadSerial++;
      layoutSerial++;
      resetDoc();
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
      renderSlot(slot, loadSerial, layoutSerial);
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
      renderSlot(slot, loadSerial, layoutSerial);

      const list = Array.isArray(boxes) && boxes.length
        ? boxes
        : [{ x: 36, y: 36, width: slot.widthPt - 72, height: 18 }];
      let firstCssTop = 0;
      const vp = slot.viewport;
      const hasVpTransform = vp && typeof vp.convertToViewportRectangle === "function" && Array.isArray(vp.viewBox);

      list.forEach((box, idx) => {
        let left = box.x * slot.scale;
        let top = box.y * slot.scale;
        let width = box.width * slot.scale;
        let height = box.height * slot.scale;
        if (hasVpTransform) {
          const vb = vp.viewBox;
          const x1 = vb[0] + box.x;
          const y1 = vb[3] - (box.y + box.height);
          const x2 = vb[0] + box.x + box.width;
          const y2 = vb[3] - box.y;
          const rect = vp.convertToViewportRectangle([x1, y1, x2, y2]);
          left = Math.min(rect[0], rect[2]);
          top = Math.min(rect[1], rect[3]);
          width = Math.abs(rect[2] - rect[0]);
          height = Math.abs(rect[3] - rect[1]);
        }
        if (idx === 0) firstCssTop = top;
        const el = document.createElement("div");
        el.className = "pdf-synctex-box";
        el.style.left = `${Math.max(0, left)}px`;
        el.style.top = `${Math.max(0, top)}px`;
        el.style.width = `${Math.max(18, width)}px`;
        el.style.height = `${Math.max(12, height)}px`;
        slot.syncLayer.appendChild(el);
      });

      const targetTop = slot.wrap.offsetTop + firstCssTop - viewportEl.clientHeight * 0.35;
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
      destroy: () => {
        close();
        if (resizeObserver) {
          resizeObserver.disconnect();
          resizeObserver = null;
        }
      },
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
