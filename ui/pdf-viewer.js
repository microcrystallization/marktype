(() => {
  "use strict";

  const RENDER_MARGIN = 350;
  const MAX_PAGE_PIXELS = 8 * 1024 * 1024;
  const MAX_CACHED_PIXELS = 24 * 1024 * 1024;
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
    let currentUrl = null;
    let pendingLoad = null;
    let currentPath = null;
    let currentTitle = "";
    let hasSyncTex = false;
    let fitMode = true;
    let scale = 1.0;
    let currentPage = 1;
    let pageMeta = [];
    let pageSlots = [];
    const rasters = new Set();
    let rasterPixels = 0;
    let observer = null;
    let resizeTimer = null;
    let highlightTimer = null;
    let activeHighlight = null;
    let highlightSlot = null;
    let lastLayoutWidth = 0;
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

    function releaseRaster(slot, keepPageData = false) {
      rasters.delete(slot);
      slot.renderGen++;
      slot.rendering = false;
      slot.renderTask?.cancel();
      slot.renderTask = null;
      if (!keepPageData) {
        slot.page?.cleanup();
        slot.page = null;
      }
      rasterPixels -= slot.pixels;
      slot.pixels = 0;
      if (slot.canvas) {
        slot.canvas.width = slot.canvas.height = 1;
        slot.canvas.remove();
        slot.canvas = null;
      }
      delete slot.wrap.dataset.rendered;
    }

    function pruneRasters() {
      for (const slot of rasters) {
        if (!isNearViewport(slot)) releaseRaster(slot);
      }
    }

    function isNearViewport(slot) {
      const top = viewportEl.scrollTop;
      return slot.wrap.offsetTop + slot.wrap.offsetHeight >= top - RENDER_MARGIN
        && slot.wrap.offsetTop <= top + viewportEl.clientHeight + RENDER_MARGIN;
    }

    function setPageSize(wrap, base) {
      wrap.style.setProperty("--pdf-page-width", `${base.width}px`);
      wrap.style.setProperty("--pdf-page-height", `${base.height}px`);
    }

    function computeFitScale(pageWidthPt) {
      const available = Math.max(240, viewportEl.clientWidth - 40);
      return Math.min(2.5, Math.max(0.45, available / pageWidthPt));
    }

    async function renderSlot(slot, docSerial, laySerial) {
      if (!pdfDoc || docSerial !== loadSerial || laySerial !== layoutSerial) return;
      if (slot.rendering || slot.canvas) return;
      const gen = ++slot.renderGen;
      slot.rendering = true;
      rasters.add(slot);
      try {
        const page = await pdfDoc.getPage(slot.pageNum);
        if (docSerial !== loadSerial || laySerial !== layoutSerial || gen !== slot.renderGen) return;
        slot.page = page;
        let base = pageMeta[slot.pageNum - 1];
        if (!base) {
          base = pageMeta[slot.pageNum - 1] = page.getViewport({ scale: 1 });
          setPageSize(slot.wrap, base);
          if (highlightSlot === slot && activeHighlight) {
            const result = renderHighlightBoxes(activeHighlight.pageNum, activeHighlight.boxes);
            if (result) revealHighlight(result, "instant");
          }
        }
        const cssViewport = base.clone({ scale });

        const dpr = Math.min(window.devicePixelRatio || 1, 2.5,
          Math.sqrt(MAX_PAGE_PIXELS / (cssViewport.width * cssViewport.height)),
          8192 / Math.max(cssViewport.width, cssViewport.height));
        const renderViewport = base.clone({ scale: scale * dpr });
        const width = Math.max(1, Math.floor(renderViewport.width));
        const height = Math.max(1, Math.floor(renderViewport.height));
        const pixels = width * height;
        while (rasterPixels + pixels > MAX_CACHED_PIXELS) {
          let farthest = null;
          for (const candidate of rasters) {
            if (!candidate.pixels) continue;
            if (!farthest || Math.abs(candidate.pageNum - currentPage) > Math.abs(farthest.pageNum - currentPage)) {
              farthest = candidate;
            }
          }
          if (!farthest) break;
          if (Math.abs(slot.pageNum - currentPage) > Math.abs(farthest.pageNum - currentPage)) {
            releaseRaster(slot);
            return;
          }
          releaseRaster(farthest);
        }
        const canvas = document.createElement("canvas");
        canvas.className = "pdf-page-canvas";
        canvas.width = width;
        canvas.height = height;
        canvas.style.width = `${cssViewport.width}px`;
        canvas.style.height = `${cssViewport.height}px`;
        slot.canvas = canvas;
        slot.pixels = pixels;
        rasterPixels += pixels;
        slot.wrap.prepend(canvas);
        slot.renderTask = page.render({ canvasContext: canvas.getContext("2d", { alpha: false }), viewport: renderViewport });
        await slot.renderTask.promise;
        if (gen === slot.renderGen) slot.wrap.dataset.rendered = "true";
      } catch (err) {
        if (gen === slot.renderGen) releaseRaster(slot);
        if (err && err.name !== "RenderingCancelledException") {
          console.warn("PDF page render failed", slot.pageNum, err);
        }
      } finally {
        if (gen === slot.renderGen) {
          slot.renderTask = null;
          slot.rendering = false;
        }
      }
    }

    function scrollAnchor() {
      const slot = pageSlots[currentPage - 1];
      return slot && { pageNum: slot.pageNum, offset: (viewportEl.scrollTop - slot.wrap.offsetTop) / scale };
    }

    function buildLayout(anchor) {
      const first = pageMeta[0];
      if (!pdfDoc || !first) return;
      const docSerial = loadSerial;
      const laySerial = ++layoutSerial;
      const previouslyRendered = [...rasters];
      for (const slot of previouslyRendered) releaseRaster(slot, true);

      if (fitMode) {
        scale = computeFitScale(first.width);
      }
      pagesEl.style.setProperty("--pdf-scale", scale);

      if (!pageSlots.length) {
        pagesEl.style.setProperty("--pdf-default-width", `${first.width}px`);
        pagesEl.style.setProperty("--pdf-default-height", `${first.height}px`);
        const fragment = document.createDocumentFragment();
        for (let pageNum = 1; pageNum <= pdfDoc.numPages; pageNum++) {
          const wrap = document.createElement("div");
          wrap.className = "pdf-page-wrap";
          wrap.dataset.page = String(pageNum);
          fragment.appendChild(wrap);
          pageSlots.push({ pageNum, wrap, page: null, canvas: null, syncLayer: null,
            pixels: 0, rendering: false, renderTask: null, renderGen: 0 });
        }
        pagesEl.replaceChildren(fragment);
        observer = new IntersectionObserver((entries) => {
          if (docSerial !== loadSerial) return;
          for (const entry of entries) {
            const slot = pageSlots[Number(entry.target.dataset.page) - 1];
            if (isNearViewport(slot) && entry.isIntersecting) renderSlot(slot, docSerial, layoutSerial);
            else if (rasters.has(slot)) releaseRaster(slot);
          }
        }, { root: viewportEl, rootMargin: `${RENDER_MARGIN}px 0px` });
        for (const slot of pageSlots) observer.observe(slot.wrap);
      }

      lastLayoutWidth = viewportEl.clientWidth;

      const anchored = anchor && pageSlots[Math.min(anchor.pageNum, pageSlots.length) - 1];
      viewportEl.scrollTop = anchored ? anchored.wrap.offsetTop + anchor.offset * scale : 0;
      if (activeHighlight && Date.now() < activeHighlight.expiresAt) {
        renderHighlightBoxes(activeHighlight.pageNum, activeHighlight.boxes);
      } else {
        activeHighlight = null;
      }

      updateCurrentPageFromScroll();
      const initialSlot = pageSlots[currentPage - 1];
      if (initialSlot) renderSlot(initialSlot, docSerial, laySerial);
      for (const slot of previouslyRendered) {
        if (isNearViewport(slot)) renderSlot(slot, docSerial, laySerial);
        else releaseRaster(slot);
      }
      updateToolbar();
    }

    function updateCurrentPageFromScroll() {
      if (!pageSlots.length) return;
      const mid = viewportEl.scrollTop + viewportEl.clientHeight * 0.35;
      let lo = 0;
      let hi = pageSlots.length;
      while (lo < hi) {
        const index = (lo + hi) >>> 1;
        if (pageSlots[index].wrap.offsetTop <= mid) lo = index + 1;
        else hi = index;
      }
      const found = Math.max(1, lo);
      if (found !== currentPage) {
        currentPage = found;
        updateToolbar();
      }
    }

    function clearHighlights() {
      clearTimeout(highlightTimer);
      activeHighlight = null;
      highlightSlot?.syncLayer?.replaceChildren();
      highlightSlot = null;
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
      let xPt = cssX / scale;
      let yPt = cssY / scale;
      const viewport = pageMeta[pageNum - 1]?.clone({ scale });
      if (viewport && typeof viewport.convertToPdfPoint === "function") {
        const [pdfX, pdfY] = viewport.convertToPdfPoint(cssX, cssY);
        const vb = viewport.viewBox;
        xPt = Math.max(0, pdfX - vb[0]);
        yPt = Math.max(0, vb[3] - pdfY);
      }
      onInverseSync({ path: currentPath, page: pageNum, x: xPt, y: yPt });
    }

    viewportEl.addEventListener("scroll", () => {
      updateCurrentPageFromScroll();
      pruneRasters();
      const slot = pageSlots[currentPage - 1];
      if (slot) renderSlot(slot, loadSerial, layoutSerial);
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
        if (Math.abs(viewportEl.clientWidth - lastLayoutWidth) < 12) return;
        clearTimeout(resizeTimer);
        resizeTimer = setTimeout(() => {
          if (!pdfDoc || !fitMode || viewportEl.clientWidth < 100) return;
          if (Math.abs(viewportEl.clientWidth - lastLayoutWidth) < 12) return;
          if (pageMeta[0] && Math.abs(computeFitScale(pageMeta[0].width) - scale) < 0.01) {
            lastLayoutWidth = viewportEl.clientWidth;
            return;
          }
          buildLayout(scrollAnchor());
        }, 120);
      });
      resizeObserver.observe(viewportEl);
    }

    function resetDoc() {
      disconnectObserver();
      for (const slot of rasters) releaseRaster(slot, true);
      clearHighlights();
      clearTimeout(resizeTimer);
      const loading = pendingLoad;
      pendingLoad = null;
      loading?.controller.abort();
      const task = pdfDoc || loading?.task;
      pdfDoc = null;
      if (task) void task.destroy().catch(() => {});
      pageMeta = [];
      pageSlots = [];
      currentUrl = null;
      currentPath = null;
      currentTitle = "";
      hasSyncTex = false;
      currentPage = 1;
      pagesEl.replaceChildren();
    }

    async function open(url, meta = {}) {
      if (currentUrl === url && pdfDoc) {
        if (meta.hasSyncTex !== undefined) hasSyncTex = Boolean(meta.hasSyncTex);
        notify();
        return;
      }
      if (pendingLoad?.url === url) return;
      const serial = ++loadSerial;
      const anchor = meta.preserveScroll && currentPath === meta.path ? scrollAnchor() : null;
      resetDoc();
      const loading = { url, controller: new AbortController(), task: null, anchor };
      pendingLoad = loading;
      if (statusEl) {
        statusEl.textContent = meta.loadingText || "";
        statusEl.classList.toggle("hidden", !meta.loadingText);
      }
      updateToolbar();
      try {
        const pdfjsLib = await loadPdfJs();
        if (serial !== loadSerial) return;
        const response = await fetch(url, { cache: "no-store", signal: loading.controller.signal });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const buffer = await response.arrayBuffer();
        if (serial !== loadSerial) return;
        loading.task = pdfjsLib.getDocument({
          data: new Uint8Array(buffer),
          isEvalSupported: false,
        });
        const nextDoc = await loading.task.promise;
        if (serial !== loadSerial) return;
        const firstPage = await nextDoc.getPage(1);
        if (serial !== loadSerial) return;
        pageMeta = [firstPage.getViewport({ scale: 1 })];
        pdfDoc = nextDoc;
        pendingLoad = null;
        currentUrl = url || null;
        currentPath = meta.path || null;
        currentTitle = meta.title || "";
        hasSyncTex = Boolean(meta.hasSyncTex);
        if (statusEl) {
          statusEl.textContent = "";
          statusEl.classList.add("hidden");
        }
        buildLayout(loading.anchor);
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
      const next = Math.min(3.0, Math.max(0.4, Math.round(nextScale * 100) / 100));
      if (!fitMode && next === scale) return;
      const anchor = scrollAnchor();
      fitMode = false;
      scale = next;
      buildLayout(anchor);
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
      buildLayout(scrollAnchor());
    }

    function renderHighlightBoxes(pageNum, boxes) {
      if (!pdfDoc || !pageSlots.length) return null;
      const slot = pageSlots[pageNum - 1];
      if (!slot) return null;
      highlightSlot?.syncLayer?.replaceChildren();
      highlightSlot = slot;
      if (!slot.syncLayer) {
        slot.syncLayer = document.createElement("div");
        slot.syncLayer.className = "pdf-synctex-layer";
        slot.wrap.appendChild(slot.syncLayer);
      }

      const list = Array.isArray(boxes) && boxes.length
        ? boxes
        : [{ x: 36, y: 36, width: (pageMeta[pageNum - 1] ?? pageMeta[0]).width - 72, height: 18 }];
      let firstCssTop = 0;
      let firstCssLeft = 0;
      const vp = pageMeta[pageNum - 1]?.clone({ scale });
      const hasVpTransform = vp && typeof vp.convertToViewportRectangle === "function" && Array.isArray(vp.viewBox);

      list.forEach((box, idx) => {
        let left = box.x * scale;
        let top = box.y * scale;
        let width = box.width * scale;
        let height = box.height * scale;
        if (hasVpTransform) {
          try {
            const vb = vp.viewBox;
            const x1 = vb[0] + box.x;
            const y1 = vb[3] - (box.y + box.height);
            const x2 = vb[0] + box.x + box.width;
            const y2 = vb[3] - box.y;
            const rect = vp.convertToViewportRectangle([x1, y1, x2, y2]);
            if (Array.isArray(rect) && rect.length === 4 && rect.every(Number.isFinite)) {
              left = Math.min(rect[0], rect[2]);
              top = Math.min(rect[1], rect[3]);
              width = Math.abs(rect[2] - rect[0]);
              height = Math.abs(rect[3] - rect[1]);
            }
          } catch {}
        }
        if (idx === 0) {
          firstCssTop = top;
          firstCssLeft = left;
        }
        const el = document.createElement("div");
        el.className = "pdf-synctex-box";
        el.style.left = `${Math.max(0, left)}px`;
        el.style.top = `${Math.max(0, top)}px`;
        el.style.width = `${Math.max(18, width)}px`;
        el.style.height = `${Math.max(12, height)}px`;
        slot.syncLayer.appendChild(el);
      });
      return { slot, firstCssTop, firstCssLeft };
    }

    function revealHighlight(rendered, behavior) {
      const { slot, firstCssTop, firstCssLeft } = rendered;
      const targetTop = slot.wrap.offsetTop + firstCssTop - viewportEl.clientHeight * 0.35;
      const targetLeft = slot.wrap.offsetLeft + firstCssLeft;
      const left = targetLeft < viewportEl.scrollLeft || targetLeft > viewportEl.scrollLeft + viewportEl.clientWidth - 36
        ? Math.max(0, targetLeft - 36)
        : viewportEl.scrollLeft;
      viewportEl.scrollTo({ top: Math.max(0, targetTop), left, behavior });
      currentPage = slot.pageNum;
      updateToolbar();
    }

    function highlight(pageNum, boxes) {
      const rendered = renderHighlightBoxes(pageNum, boxes);
      if (!rendered) return false;
      clearTimeout(highlightTimer);
      activeHighlight = { pageNum, boxes, expiresAt: Date.now() + 2600 };
      highlightTimer = setTimeout(clearHighlights, 2600);
      revealHighlight(rendered, "instant");
      renderSlot(rendered.slot, loadSerial, layoutSerial);
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
