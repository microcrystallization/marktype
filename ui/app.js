(() => {
  "use strict";

  // ===== IPC Bridge =====
  function send(payload) {
    if (window.ipc && typeof window.ipc.postMessage === "function") {
      window.ipc.postMessage(JSON.stringify(payload));
    }
  }

  // ===== Multi-Language i18n Engine =====
  const I18N = window.MT_I18N || { languages: [{ code: "en", label: "English" }], dict: { en: {} } };
  function currentLang() {
    const code = (state.settings && state.settings.language) || "zh-CN";
    return I18N.dict[code] ? code : "en";
  }

  function platformShortcut(text) {
    return state.platform === "macos" ? text.replace(/\bCtrl\s*\+\s*/g, "⌘") : text;
  }

  function t(key, vars) {
    const lang = currentLang();
    let str = (I18N.dict[lang] && I18N.dict[lang][key]) || (I18N.dict.en && I18N.dict.en[key]) || key;
    if (vars) {
      for (const [k, v] of Object.entries(vars)) {
        str = str.replaceAll(`{${k}}`, String(v));
      }
    }
    return str;
  }

  function applyI18n() {
    const code = currentLang();
    document.documentElement.lang = code;
    document.querySelectorAll("[data-i18n]").forEach((el) => {
      el.textContent = t(el.dataset.i18n);
    });
    document.querySelectorAll("[data-i18n-title]").forEach((el) => {
      el.title = platformShortcut(t(el.dataset.i18nTitle));
    });
    document.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
      el.placeholder = t(el.dataset.i18nPlaceholder);
    });
    $("language-select").value = code;
    elSettingsView.setAttribute("aria-label", t("settingsBack"));
    if (state.wallpaper) renderWallpaperSettingsUI(state.wallpaper);
    renderOutline();
    renderFileTree();
    renderTabs();
    if (state.doc) {
      updateStatsUI(state.doc.stats);
      updateLoadMoreVisibility();
      updateLargeBar();
    } else {
      $("st-cursor").textContent = "—";
      updateStatsUI({ words: 0, chars: 0, largeMode: false });
    }
    if (state.settings) {
      renderHeroRecent(state.settings.recentFiles || []);
      const modeKey =
        state.settings.viewMode === "split"
          ? "modeSplitFull"
          : state.settings.viewMode === "live"
            ? "modeLiveFull"
            : "modeSourceFull";
      $("st-mode").textContent = t(state.doc?.texSource ? "latexSourceMode" : modeKey);
    }
  }

  // ===== Application State =====
  const state = {
    settings: null,
    platform: "windows",
    tabs: [],
    activeTabId: null,
    doc: null, // active doc metadata
    prevText: "",
    windowStartLine: 0,
    windowEndLine: 0,
    totalLines: 1,
    totalBlocks: 0,
    loadedBlocks: 0,
    previewStart: 0,
    previewTarget: null,
    largeMode: false,
    outline: [],
    fileTree: null,
    wallpaper: null,
    settingsOpen: false,
    findOpen: false,
    findCase: false,
    findRegex: false,
    findMatches: [],
    findIndex: -1,
    editor: null,
    editorReadOnly: false,
    editorTarget: null,
    sourceRequest: null,
    pendingSourceEdit: false,
    pendingEdits: 0,
    blockRequest: null,
    previewNeedsRefresh: false,
    searchVersion: null,
    searchRequest: 0,
    searchTimer: null,
    closingDocId: null,
    prevBlurUrl: "",
    prevHeroUrl: "",
    activeLine: 0,
  };
  let requestSequence = 0;
  const liveWidgetRequests = new Map();

  function renderLiveWidgets(docId, fragments) {
    const request = ++requestSequence;
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        liveWidgetRequests.delete(request);
        resolve([]);
      }, 5000);
      liveWidgetRequests.set(request, (html) => {
        clearTimeout(timer);
        liveWidgetRequests.delete(request);
        resolve(html);
      });
      send({ cmd: "renderLiveWidgets", docId, request, fragments });
    });
  }

  // ===== LRU KaTeX Cache (0.005ms cache hit!) =====
  const KATEX_MAX = 4000;
  const katexCache = new Map();

  function renderKatexCached(tex, displayMode) {
    const key = (displayMode ? "D:" : "I:") + tex;
    const hit = katexCache.get(key);
    if (hit !== undefined) {
      return hit;
    }
    let rendered;
    try {
      if (window.katex) {
        rendered = window.katex.renderToString(tex, {
          displayMode,
          throwOnError: false,
          strict: false,
          trust: false,
        });
      } else {
        rendered = `<span class="mt-math-err">${escapeHtml(tex)}</span>`;
      }
    } catch (e) {
      rendered = `<span class="mt-math-err">${escapeHtml(tex)}</span>`;
    }
    if (katexCache.size >= KATEX_MAX) {
      const firstKey = katexCache.keys().next().value;
      katexCache.delete(firstKey);
    }
    katexCache.set(key, rendered);
    return rendered;
  }

  function hydrateMath(container) {
    const blocks = container.querySelectorAll(".mt-math-block");
    for (let i = 0; i < blocks.length; i++) {
      const el = blocks[i];
      const tex = el.dataset.tex || "";
      el.innerHTML = renderKatexCached(tex, true);
    }
    const inlines = container.querySelectorAll(".mt-math-inline");
    for (let i = 0; i < inlines.length; i++) {
      const el = inlines[i];
      const tex = el.dataset.tex || "";
      el.innerHTML = renderKatexCached(tex, false);
    }
  }

  // ===== DOM Elements =====
  const $ = (id) => document.getElementById(id);
  const elTabBar = $("tab-bar");
  const elSidebar = $("sidebar");
  const elOutlineList = $("outline-list");
  const elFileTree = $("file-tree");
  const elFileFilter = $("file-filter");
  const elEmptyView = $("empty-view");
  const elEditorView = $("editor-view");
  const elSettingsView = $("settings-view");
  const elPanesWrap = $("panes-wrap");
  const elSourcePane = $("source-pane");
  const elEditorHost = $("editor-host");
  const elSplitDivider = $("split-divider");
  const elPreviewPane = $("preview-pane");
  const elPreviewDoc = $("preview-doc");
  const elLoadMoreWrap = $("load-more-wrap");
  const elLargeBar = $("large-bar");
  const elLargeInfo = $("large-info");
  const elFindBar = $("find-bar");
  const elFindInput = $("find-input");
  const elReplaceInput = $("replace-input");
  const elFindCount = $("find-count");
  const elToast = $("toast");
  const elCloseDialog = $("close-dialog");
  const externalDialog = $("external-change-dialog");
  externalDialog.addEventListener("cancel", (event) => event.preventDefault());
  for (const [id, reload] of [["external-keep", false], ["external-reload", true]]) {
    $(id).onclick = () => {
      send({ cmd: "resolveExternalChange", docId: externalDialog.dataset.docId, reload });
      externalDialog.close();
    };
  }

  const elWpBlurPrev = $("wp-blur-prev");
  const elWpBlurCurr = $("wp-blur-curr");
  const elHeroPrev = $("hero-prev");
  const elHeroCurr = $("hero-curr");

  // ===== Utilities =====
  function escapeHtml(str) {
    return str
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // Convert JS UTF-16 index to Rust Unicode scalar (`char`) index cleanly (handles emojis!)
  function utf16ToScalarCount(str, endUtf16) {
    let count = 0;
    const len = Math.min(str.length, endUtf16);
    for (let i = 0; i < len; i++) {
      const code = str.charCodeAt(i);
      if (code >= 0xd800 && code <= 0xdbff && i + 1 < len) {
        const next = str.charCodeAt(i + 1);
        if (next >= 0xdc00 && next <= 0xdfff) {
          i++;
        }
      }
      count++;
    }
    return count;
  }

  let toastTimer = null;
  function showToast(msg) {
    if (!msg) return;
    elToast.textContent = msg;
    elToast.classList.remove("hidden");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => elToast.classList.add("hidden"), 2600);
  }

  // Preview blocks retain their DOM identity as long as their backend ID survives.
  function updateBlockElement(node, block) {
    node.dataset.blockId = String(block.id);
    node.dataset.startLine = String(block.startLine);
    node.dataset.endLine = String(block.endLine);
    node.dataset.kind = block.kind;
    node.dataset.source = block.source ?? "";
    const content = node.firstElementChild;
    if (node.renderedHtml !== block.html) {
      node.renderedHtml = block.html;
      content.innerHTML = block.html;
      hydrateMath(content);
      window.MTDiagrams.observe(content);
    }
  }

  function createBlockElement(block) {
    const node = document.createElement("div");
    node.className = "mt-block";
    const content = document.createElement("div");
    content.className = "mt-block-content";
    node.append(content);
    updateBlockElement(node, block);
    return node;
  }

  function ensureEmptyPreview() {
    if (elPreviewDoc.children.length) return;
    const placeholder = document.createElement("div");
    placeholder.className = "mt-preview-empty";
    const label = document.createElement("p");
    label.className = "mt-preview-empty-label";
    label.textContent = t("liveEmptyPlaceholder");
    placeholder.append(label);
    elPreviewDoc.append(placeholder);
  }

  function replacePreviewBlocks(blocks, start = 0) {
    const fragment = document.createDocumentFragment();
    for (const block of blocks) fragment.append(createBlockElement(block));
    elPreviewDoc.replaceChildren(fragment);
    state.loadedBlocks = blocks.length;
    state.previewStart = start;
    ensureEmptyPreview();
    updateLoadMoreVisibility();
  }

  function patchPreviewBlocks({ replaceStart, removeCount, inserted, lineDelta }) {
    const oldStart = state.previewStart;
    const oldEnd = replaceStart + removeCount;
    if (replaceStart < oldStart && oldEnd <= oldStart) {
      state.previewStart += inserted.length - removeCount;
      shiftPreviewLines(0, lineDelta);
      return;
    }
    if (replaceStart < oldStart) {
      removeCount = oldEnd - oldStart;
      state.previewStart = replaceStart;
      replaceStart = 0;
    } else {
      replaceStart -= oldStart;
    }
    if (replaceStart > state.loadedBlocks) return;
    if (elPreviewDoc.firstElementChild?.classList.contains("mt-preview-empty")) {
      elPreviewDoc.firstElementChild.remove();
    }
    const children = elPreviewDoc.children;
    const end = Math.min(state.loadedBlocks, replaceStart + removeCount);
    const old = new Map();
    for (let i = replaceStart; i < end; i++) {
      const node = children[i];
      old.set(Number(node.dataset.blockId), node);
    }
    let next = children[end] || null;
    for (let i = inserted.length - 1; i >= 0; i--) {
      const block = inserted[i];
      let node = old.get(block.id);
      if (node) {
        old.delete(block.id);
        updateBlockElement(node, block);
      } else {
        node = createBlockElement(block);
      }
      if (node.parentNode !== elPreviewDoc || node.nextSibling !== next) {
        elPreviewDoc.insertBefore(node, next);
      }
      next = node;
    }
    for (const node of old.values()) node.remove();

    shiftPreviewLines(replaceStart + inserted.length, lineDelta);
    state.loadedBlocks = children.length;
    ensureEmptyPreview();
  }

  function shiftPreviewLines(start, delta) {
    if (delta === 0) return;
    const children = elPreviewDoc.children;
    for (let i = start; i < children.length; i++) {
      const node = children[i];
      node.dataset.startLine = String(Number(node.dataset.startLine) + delta);
      node.dataset.endLine = String(Number(node.dataset.endLine) + delta);
    }
  }

  function updatePreviewToc() {
    for (const toc of elPreviewDoc.querySelectorAll(".mt-toc")) {
      const title = toc.firstElementChild;
      if (!state.outline.length) {
        const empty = document.createElement("div");
        empty.className = "mt-toc-empty";
        empty.textContent = t("tabOutline");
        toc.replaceChildren(title, empty);
        continue;
      }
      const list = document.createElement("ul");
      list.className = "mt-toc-list";
      for (const heading of state.outline) {
        const item = document.createElement("li");
        item.style.paddingLeft = ((heading.level - 1) * 14) + "px";
        const link = document.createElement("a");
        link.setAttribute("href", "#" + heading.slug);
        link.dataset.jumpLine = String(heading.line);
        link.textContent = heading.text;
        item.append(link);
        list.append(item);
      }
      toc.replaceChildren(title, list);
    }
  }

  function applyIncrementalPatch(patch, deferredPreview) {
    state.totalBlocks = patch.totalBlocks;
    state.totalLines = patch.totalLines;
    state.doc.version = patch.version;
    state.doc.stats = patch.stats;
    if (deferredPreview) state.previewNeedsRefresh = true;
    if (!state.previewNeedsRefresh) patchPreviewBlocks(patch);

    if (patch.outlineChanged && patch.outline) {
      state.outline = patch.outline;
      renderOutline();
    }
    if (patch.refreshPreview) {
      state.previewNeedsRefresh = true;
      state.editor?.invalidateWidgets();
    }
    if (state.previewNeedsRefresh) requestPreviewBlocks();
    else if (patch.outlineChanged) updatePreviewToc();

    updateStatsUI(patch.stats);
    updateActiveBlockHighlight();
    updateLoadMoreVisibility();
    updateLargeBar();

  }

  function requestPreviewBlocks(count = 150) {
    if (!state.doc || state.doc.texSource || state.blockRequest || state.settings?.viewMode !== "split") return;
    const aroundLine = state.previewTarget;
    const refresh = state.previewNeedsRefresh || aroundLine !== null;
    const end = state.previewStart + state.loadedBlocks;
    if (!refresh && end >= state.totalBlocks) return;
    const request = ++requestSequence;
    state.blockRequest = request;
    send({
      cmd: "fetchBlocks",
      docId: state.activeTabId,
      start: refresh ? state.previewStart : end,
      count: aroundLine !== null ? 200 : refresh ? Math.max(state.loadedBlocks, 200) : count,
      request,
      refresh,
      aroundLine,
    });
  }

  function updateLoadMoreVisibility() {
    const end = state.previewStart + state.loadedBlocks;
    const more = end < state.totalBlocks;
    elLoadMoreWrap.classList.toggle("hidden", !more);
    if (more) {
      $("btn-load-more").textContent = t("loadMore", {
        loaded: end,
        total: state.totalBlocks,
      });
    }
  }

  let editorBundle;
  function loadEditorBundle() {
    if (!editorBundle) editorBundle = new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = "/vendor/codemirror.js";
      script.onload = resolve;
      script.onerror = () => {
        script.remove();
        editorBundle = null;
        reject(new Error("The editor could not load"));
      };
      document.head.append(script);
    });
    return editorBundle;
  }

  function mountEditor(focus = false) {
    if (!state.doc || !state.settings) return;
    if (!window.MTEditor) {
      const doc = state.doc;
      void loadEditorBundle().then(() => {
        if (state.doc === doc) mountEditor(focus);
      }).catch((error) => showToast(error.message));
      return;
    }
    if (!state.editor) {
      const docId = state.activeTabId;
      state.editor = window.MTEditor.create(elEditorHost, {
        text: state.prevText,
        mode: state.settings.viewMode,
        texSource: Boolean(state.doc.texSource),
        label: t("editorPlaceholder"),
        renderMath: renderKatexCached,
        renderWidgets: (fragments) => renderLiveWidgets(docId, fragments),
        hydrateWidgets(element) {
          hydrateMath(element);
          return window.MTDiagrams.render(element);
        },
        showLineNumbers: state.settings.showLineNumbers,
        startLine: state.windowStartLine,
        onScroll: syncScrollFrom,
        onEdit(startChar, endChar, text, lines) {
          state.windowEndLine = state.windowStartLine + lines;
          const deferPreview = state.doc.texSource || state.settings.viewMode !== "split";
          if (deferPreview) state.previewNeedsRefresh = true;
          sendEdit(state.windowStartLine, { startChar, endChar, text }, deferPreview);
        },
        onCursor(line, prefix) {
          state.activeLine = state.windowStartLine + line - 1;
          const col = utf16ToScalarCount(prefix, prefix.length) + 1;
          $("st-cursor").textContent = t("stCursor", { ln: state.activeLine + 1, col });
          updateActiveBlockHighlight();
          if (state.settings.typewriterMode && state.editor) {
            const editor = state.editor;
            requestAnimationFrame(() => {
              if (state.editor !== editor) return;
              const scroll = editor.scrollElement;
              setSyncedScroll(scroll, Math.max(0, editor.scrollAtLine(line - 1) - scroll.clientHeight * 0.42));
            });
          }
        },
      });
    }
    state.editor.setMode(state.settings.viewMode);
    state.editor.configureGutter(state.settings.showLineNumbers, state.windowStartLine);
    state.editor.setReadOnly(state.editorReadOnly);
    if (state.editorTarget) applySourceSelection(state.editorTarget);
    if (focus) state.editor.focus();
  }

  function unmountEditor() {
    state.editor?.destroy();
    state.editor = null;
  }

  function setSourceReadOnly(value) {
    state.editorReadOnly = value;
    state.editor?.setReadOnly(value);
  }

  function sourceRequestOptions() {
    return { startLine: state.windowStartLine, count: state.largeMode ? 600 : null };
  }

  function sendEdit(baseLine, diff, deferPreview) {
    if (!state.doc) return;
    state.pendingEdits++;
    invalidateSearch();
    send({ cmd: "edit", docId: state.activeTabId, baseLine, ...diff, utf16: true, deferPreview });
  }

  // ===== Theme & Kite Wallpaper Application =====
  function crossfadeImage(prevEl, currEl, url, opacity = 1) {
    if (!url) {
      prevEl.classList.remove("visible");
      currEl.classList.remove("visible");
      prevEl.removeAttribute("src");
      currEl.removeAttribute("src");
      return;
    }
    if (currEl.getAttribute("src") === url && currEl.classList.contains("visible")) {
      currEl.style.opacity = String(opacity);
      return;
    }
    if (currEl.src && currEl.classList.contains("visible")) {
      prevEl.src = currEl.src;
      prevEl.style.opacity = String(opacity);
      prevEl.classList.add("visible");
    }
    currEl.classList.remove("visible");
    currEl.style.opacity = "0";
    const img = new Image();
    img.onload = () => {
      currEl.src = url;
      requestAnimationFrame(() => {
        currEl.style.opacity = String(opacity);
        currEl.classList.add("visible");
      });
    };
    img.src = url;
  }

  function applyWallpaperAndTheme(wp) {
    if (!wp) return;
    state.wallpaper = wp;
    const t = wp.theme;
    const root = document.documentElement.style;

    document.body.classList.toggle("light", !!t.light);
    document.body.classList.toggle("dark", !t.light);

    root.setProperty("--shell", t.shell);
    root.setProperty("--shell-panel", t.shellPanel);
    root.setProperty("--editor-bg", t.editorBg);
    root.setProperty("--editor-panel", t.editorPanel);
    root.setProperty("--preview-bg", t.previewBg);
    root.setProperty("--preview-panel", t.previewPanel);
    root.setProperty("--popup", t.popup);
    root.setProperty("--text", t.text);
    root.setProperty("--muted", t.muted);
    root.setProperty("--faint", t.faint);
    root.setProperty("--accent", t.accent);
    root.setProperty("--accent-soft", t.accentSoft);
    root.setProperty("--on-accent", t.onAccent);
    root.setProperty("--code-bg", t.codeBg);
    root.setProperty("--border", t.border);
    root.setProperty("--hairline", t.hairline);
    root.setProperty("--row-hover", t.rowHover);
    root.setProperty("--row-active", t.rowActive);

    // Full-window frosted backdrop (when `Wallpaper fills the window` is ON)
    if (wp.fillsWindow && wp.blurredUrl) {
      crossfadeImage(elWpBlurPrev, elWpBlurCurr, wp.blurredUrl, 1);
    } else {
      crossfadeImage(elWpBlurPrev, elWpBlurCurr, "", 0);
    }

    // Sharp Empty-View Hero wallpaper
    if (wp.imageUrl) {
      crossfadeImage(elHeroPrev, elHeroCurr, wp.imageUrl, wp.heroOpacity);
    } else {
      crossfadeImage(elHeroPrev, elHeroCurr, "", 0);
    }

    renderWallpaperSettingsUI(wp);
  }

  function renderWallpaperSettingsUI(wp) {
    const statusEl = $("wp-status");
    statusEl.hidden = !wp.error;
    statusEl.textContent = wp.error || "";

    const gallery = $("wp-gallery");
    gallery.innerHTML = "";

    // 1. "None" tile
    const noneTile = document.createElement("button");
    noneTile.className = "wp-tile" + (!wp.activePath ? " selected" : "");
    noneTile.title = t("wpNoWallpaper");
    noneTile.textContent = t("wpNone");
    noneTile.onclick = () => send({ cmd: "clearWallpaper" });
    gallery.appendChild(noneTile);

    // 2. Built-in 7 Kite artworks + Recent custom wallpapers
    for (const tile of wp.tiles) {
      const btn = document.createElement("button");
      btn.className = "wp-tile" + (tile.selected ? " selected" : "");
      btn.setAttribute("aria-label", tile.title);
      btn.setAttribute("aria-pressed", String(tile.selected));
      const img = document.createElement("img");
      img.src = tile.thumbUrl;
      img.alt = "";
      btn.appendChild(img);
      btn.onclick = () => {
        if (tile.kind === "builtin") {
          send({ cmd: "selectBuiltinWallpaper", id: tile.id });
        } else {
          send({ cmd: "selectRecentWallpaper", path: tile.path });
        }
      };
      gallery.appendChild(btn);
    }

    // 3. "+" Add custom image tile
    const addTile = document.createElement("button");
    addTile.className = "wp-tile wp-tile-add";
    addTile.title = t("wpChooseImage");
    addTile.textContent = "+";
    addTile.onclick = () => send({ cmd: "pickWallpaperDialog" });
    gallery.appendChild(addTile);

    // Effect pills
    document.querySelectorAll("#wp-effect-group .pill-opt").forEach((btn) => {
      btn.classList.toggle("selected", btn.dataset.effect === wp.effect);
    });

    $("wp-vis-val").textContent = `${Math.round(wp.heroOpacity * 100)}%`;
    $("toggle-wp-fills").setAttribute("aria-checked", String(!!wp.fillsWindow));
    $("toggle-wp-colors").setAttribute("aria-checked", String(!!wp.themeColors));
  }

  function applySettingsUI(s) {
    if (!s) return;
    const modeChanged = state.settings?.viewMode !== s.viewMode;
    const editorFocused = elEditorHost.contains(document.activeElement);
    state.settings = s;
    const root = document.documentElement.style;

    root.setProperty("--editor-font-size", s.fontSize + "px");
    root.setProperty("--preview-max-width", s.previewMaxWidth + "px");
    root.setProperty("--line-height", String(s.lineHeight));

    document.body.classList.toggle("focus-mode", !!s.focusMode);
    document.body.classList.toggle("reduce-motion", !!s.reduceMotion);

    elSidebar.classList.toggle("collapsed", !s.sidebarOpen);
    $("btn-sidebar").classList.toggle("active", !!s.sidebarOpen);

    // View mode
    elPanesWrap.classList.remove("mode-split", "mode-live", "mode-source");
    elPanesWrap.classList.add("mode-" + s.viewMode);
    document.querySelectorAll(".mode-btn").forEach((b) => {
      b.classList.toggle("active", b.dataset.mode === s.viewMode);
    });
    const modeKey =
      s.viewMode === "split" ? "modeSplitFull" : s.viewMode === "live" ? "modeLiveFull" : "modeSourceFull";
    $("st-mode").textContent = t(modeKey);

    // Split ratio
    if (s.viewMode === "split") {
      elSourcePane.style.flex = `0 0 ${(s.splitRatio * 100).toFixed(1)}%`;
      if (modeChanged && state.previewNeedsRefresh) {
        state.previewTarget = state.activeLine;
        requestPreviewBlocks();
      }
    }
    mountEditor(modeChanged && editorFocused);

    // Toolbar toggles
    $("btn-focus").classList.toggle("active", !!s.focusMode);
    $("btn-typewriter").classList.toggle("active", !!s.typewriterMode);
    $("btn-sync").classList.toggle("active", !!s.syncScroll);

    // Settings controls sync
    document.querySelectorAll("[data-light]").forEach((b) => {
      b.classList.toggle("selected", b.dataset.light === String(!!s.light));
    });
    document.querySelectorAll("[data-preset]").forEach((b) => {
      b.classList.toggle("selected", b.dataset.preset === s.themePreset);
    });
    $("font-val").textContent = `${Math.round(s.fontSize)} pt`;
    $("width-val").textContent = `${s.previewMaxWidth} px`;
    $("toggle-line-nums").setAttribute("aria-checked", String(!!s.showLineNumbers));
    $("toggle-sync-scroll").setAttribute("aria-checked", String(!!s.syncScroll));
    $("toggle-autosave").setAttribute("aria-checked", String(!!s.autoSave));
    $("toggle-reduce-motion").setAttribute("aria-checked", String(!!s.reduceMotion));

    // Sidebar tab
    switchSidebarTab(s.sidebarTab || "outline", false);

    // Recent files in Hero view
    renderHeroRecent(s.recentFiles || []);
    applyI18n();
  }

  function renderHeroRecent(recent) {
    const box = $("hero-recent");
    if (!recent.length) {
      box.classList.add("hidden");
      return;
    }
    box.classList.remove("hidden");
    box.innerHTML = "";
    for (const p of recent.slice(0, 5)) {
      const name = p.split(/[\\/]/).pop() || p;
      const btn = document.createElement("button");
      btn.className = "hero-action";
      btn.title = p;
      btn.innerHTML = `<span class="hero-key">${escapeHtml(t("heroRecentKey"))}</span><span>${escapeHtml(name)}</span>`;
      btn.onclick = () => send({ cmd: "openPath", path: p });
      box.appendChild(btn);
    }
  }

  // ===== Tabs, Outline, File Tree, Stats =====
  function applyTabSummaries(tabs) {
    state.tabs = tabs || state.tabs;
    const active = state.tabs.find((tab) => tab.id === state.activeTabId);
    if (state.doc && active && state.doc.path !== active.path) {
      state.doc.path = active.path;
      state.doc.texSource = active.texSource;
      state.editor?.setSourceType(active.texSource);
      applyTexMode();
      state.previewNeedsRefresh = true;
      state.editor?.invalidateWidgets();
      requestPreviewBlocks();
    }
    renderTabs();
  }

  function applyTexMode() {
    const tex = Boolean(state.doc?.texSource);
    elPanesWrap.classList.toggle("tex-document", tex);
    $("tex-tools").classList.toggle("hidden", !tex);
    $("format-bar").classList.toggle("hidden", tex);
    document.querySelectorAll(".mode-btn").forEach((button) => { button.disabled = tex; });
    $("btn-print").disabled = !state.doc || tex;
    if (tex) $("st-mode").textContent = t("latexSourceMode");
    syncPdfPaneVisibility();
  }

  function renderTabs() {
    for (const id of ["btn-export-docx", "btn-export-tex"]) {
      $(id).disabled = !state.activeTabId || !state.pandocAvailable;
      $(id).title = state.pandocAvailable ? "" : t("exportNeedsPandoc");
    }
    for (const id of ["btn-save", "btn-save-as", "btn-export", "btn-print", "menu-close-tab", "menu-find", "menu-replace"]) {
      $(id).disabled = !state.activeTabId;
    }
    if (state.doc?.texSource) $("btn-print").disabled = true;
    elTabBar.innerHTML = "";
    for (const tab of state.tabs) {
      const el = document.createElement("div");
      el.className = "doc-tab" + (tab.id === state.activeTabId ? " active" : "");
      el.title = tab.path || tab.title;
      el.onclick = () => {
        if (tab.id !== state.activeTabId) {
          send({ cmd: "switchTab", id: tab.id });
        }
        closeSettings();
      };
      if (tab.dirty) {
        const dot = document.createElement("span");
        dot.className = "doc-tab-dirty";
        el.appendChild(dot);
      }
      const title = document.createElement("span");
      title.className = "doc-tab-title";
      title.textContent = tab.title;
      el.appendChild(title);

      const close = document.createElement("button");
      close.className = "doc-tab-close";
      close.textContent = "×";
      close.title = t("closeTabTitle");
      close.onclick = (e) => {
        e.stopPropagation();
        send({ cmd: "closeTab", id: tab.id });
      };
      el.appendChild(close);
      elTabBar.appendChild(el);
    }

    const add = document.createElement("button");
    add.className = "tab-add-btn";
    add.textContent = "+";
    add.title = platformShortcut(t("newTabTitle"));
    add.onclick = () => {
      closeSettings();
      send({ cmd: "newTab" });
    };
    elTabBar.appendChild(add);
  }

  const OUTLINE_ROW_HEIGHT = 28;
  let renderedOutline;
  let outlineRows = elOutlineList;
  let outlineFirst = -1;
  let outlineLast = -1;
  let outlineFrame = 0;

  function renderOutline() {
    const items = state.outline;
    const virtual = items.length > 300;
    const scroller = elOutlineList;
    if (renderedOutline !== items) {
      const scrollTop = scroller.scrollTop;
      renderedOutline = items;
      outlineFirst = outlineLast = -1;
      scroller.classList.toggle("virtual-outline", virtual);
      scroller.replaceChildren();
      outlineRows = scroller;
      if (virtual) {
        const track = document.createElement("div");
        track.className = "outline-track";
        track.style.height = `${items.length * OUTLINE_ROW_HEIGHT}px`;
        outlineRows = document.createElement("div");
        outlineRows.className = "outline-window";
        track.append(outlineRows);
        scroller.append(track);
      }
      scroller.scrollTop = scrollTop;
    }
    if (!items.length) {
      scroller.innerHTML = `<div class="sidebar-empty">${t("outlineEmpty")}</div>`;
      return;
    }
    const first = virtual ? Math.max(0, Math.floor(scroller.scrollTop / OUTLINE_ROW_HEIGHT) - 8) : 0;
    const last = virtual ? Math.min(items.length, first + Math.ceil(scroller.clientHeight / OUTLINE_ROW_HEIGHT) + 17) : items.length;
    if (first === outlineFirst && last === outlineLast) return;
    outlineFirst = first;
    outlineLast = last;
    if (virtual) outlineRows.style.top = `${first * OUTLINE_ROW_HEIGHT}px`;
    const fragment = document.createDocumentFragment();
    for (let index = first; index < last; index++) {
      const item = items[index];
      const btn = document.createElement("button");
      btn.className = `outline-item lvl-${item.level}`;
      btn.textContent = item.text || "(Untitled)";
      btn.dataset.index = String(index);
      btn.onclick = () => jumpToLine(item.line);
      fragment.appendChild(btn);
    }
    outlineRows.replaceChildren(fragment);
    updateOutlineHighlight();
  }

  function scheduleOutlineWindow() {
    if (state.outline.length <= 300 || outlineFrame) return;
    outlineFrame = requestAnimationFrame(() => {
      outlineFrame = 0;
      renderOutline();
    });
  }

  elOutlineList.style.setProperty("--outline-row-height", `${OUTLINE_ROW_HEIGHT}px`);
  elOutlineList.addEventListener("scroll", scheduleOutlineWindow);
  new ResizeObserver(scheduleOutlineWindow).observe(elOutlineList);
  elOutlineList.addEventListener("keydown", (event) => {
    const index = Number(event.target.dataset.index);
    if (!Number.isInteger(index)) return;
    const next = { ArrowUp: index - 1, ArrowDown: index + 1, Home: 0, End: state.outline.length - 1 }[event.key];
    if (next === undefined || next < 0 || next >= state.outline.length) return;
    event.preventDefault();
    if (next < outlineFirst || next >= outlineLast) {
      elOutlineList.scrollTop = next * OUTLINE_ROW_HEIGHT;
      renderOutline();
    }
    outlineRows.children[next - outlineFirst]?.focus();
  });

  const collapsedFolders = new Set();
  function renderFileTree() {
    elFileTree.innerHTML = "";
    if (!state.fileTree) {
      elFileTree.innerHTML = `<div class="sidebar-empty">${escapeHtml(t("noFolder"))}<br/><button class="mini-btn accent" style="margin-top:10px" id="tree-open-btn">${escapeHtml(t("openFolderBtn"))}</button></div>`;
      const btn = $("tree-open-btn");
      if (btn) btn.onclick = () => send({ cmd: "openFolderDialog" });
      return;
    }
    const query = (elFileFilter.value || "").trim().toLowerCase();
    const activePath = state.doc ? state.doc.path : null;

    function renderNode(node, depth) {
      if (node.isDir) {
        const children = node.children || [];
        const hasMatch =
          !query ||
          node.name.toLowerCase().includes(query) ||
          children.some((c) => matchesFilter(c, query));
        if (!hasMatch) return;
        const row = document.createElement("button");
        row.className = "file-row dir";
        row.dataset.workspacePath = node.path;
        row.dataset.workspaceName = node.name;
        row.style.paddingLeft = `${8 + depth * 12}px`;
        const expanded = Boolean(query) || !collapsedFolders.has(node.path);
        row.textContent = `${expanded ? "▾" : "▸"} ${node.name}`;
        row.setAttribute("aria-expanded", String(expanded));
        row.onclick = () => {
          if (expanded) collapsedFolders.add(node.path);
          else collapsedFolders.delete(node.path);
          renderFileTree();
        };
        elFileTree.appendChild(row);
        for (const child of expanded ? children : []) {
          renderNode(child, depth + 1);
        }
      } else {
        if (query && !node.name.toLowerCase().includes(query)) return;
        const row = document.createElement("button");
        row.className = "file-row" + (activePath === node.path ? " active" : "");
        row.dataset.workspacePath = node.path;
        row.dataset.workspaceName = node.name;
        row.style.paddingLeft = `${12 + depth * 12}px`;
        row.textContent = `📄 ${node.name}`;
        row.title = node.path;
        row.onclick = () => {
          closeSettings();
          send({ cmd: "openPath", path: node.path });
        };
        elFileTree.appendChild(row);
      }
    }

    function matchesFilter(node, q) {
      if (node.name.toLowerCase().includes(q)) return true;
      return (node.children || []).some((c) => matchesFilter(c, q));
    }

    renderNode(state.fileTree, 0);
  }

  function updateStatsUI(stats) {
    if (!stats) return;
    $("st-words").textContent = t("stWords", { n: stats.words.toLocaleString() });
    $("st-chars").textContent = t("stChars", { n: stats.chars.toLocaleString() });
    $("st-large-badge").classList.toggle("hidden", !stats.largeMode);
  }

  function loadDocumentSnapshot(doc) {
    unmountEditor();
    state.editorTarget = null;
    state.doc = doc;
    applyTexMode();
    state.sourceRequest = null;
    state.pendingSourceEdit = false;
    state.pendingEdits = 0;
    state.blockRequest = null;
    state.previewNeedsRefresh = Boolean(doc?.previewDeferred);
    state.previewStart = 0;
    state.previewTarget = null;
    elOutlineList.scrollTop = 0;
    setSourceReadOnly(false);
    invalidateSearch();
    if (!doc) {
      elEmptyView.classList.remove("hidden");
      elEditorView.classList.add("hidden");
      $("st-cursor").textContent = "—";
      updateStatsUI({ words: 0, chars: 0, largeMode: false });
      state.outline = [];
      state.totalBlocks = 0;
      state.loadedBlocks = 0;
      renderOutline();
      return;
    }
    elEmptyView.classList.add("hidden");
    elEditorView.classList.remove("hidden");

    state.largeMode = !!doc.largeMode;
    state.totalBlocks = doc.totalBlocks;
    state.outline = doc.outline || [];
    applySourceWindow({
      startLine: doc.windowStartLine || 0,
      endLine: doc.windowEndLine || doc.stats.lines,
      totalLines: doc.stats.lines,
      text: doc.text || "",
      contextLine: doc.contextLine,
    });
    setSyncedScroll(elPreviewPane, 0);
    replacePreviewBlocks(doc.blocks || []);
    renderOutline();
    updateStatsUI(doc.stats);
    updateLargeBar();
    renderFileTree();
    updateActiveBlockHighlight();
  }

  function applySourceWindow(source, preserveHistory = false) {
    const reuse = preserveHistory && state.editor && state.windowStartLine === source.startLine;
    const editorFocused = elEditorHost.contains(document.activeElement);
    if (!reuse) unmountEditor();
    state.windowStartLine = source.startLine;
    state.windowEndLine = source.endLine;
    state.totalLines = source.totalLines;
    state.prevText = source.text;
    if (reuse) {
      state.editor.applyText(source.text);
    } else {
      state.editorTarget = { line: source.startLine, focus: editorFocused };
    }
    mountEditor();
    state.editor?.setMatches(state.findMatches, state.windowStartLine);
    updateLargeBar();
  }

  function updateLargeBar() {
    if (state.largeMode) {
      elLargeBar.classList.remove("hidden");
      elLargeInfo.textContent = `Large Rope Mode · Showing lines ${state.windowStartLine + 1}–${state.windowEndLine} of ${state.totalLines.toLocaleString()}`;
    } else {
      elLargeBar.classList.add("hidden");
    }
  }

  // ===== Cursor, Active Block, Jump & Bidirectional Sync Scroll =====
  let activePreviewBlock;
  let activeOutlineItem;
  function updateActiveBlockHighlight() {
    const line = state.activeLine;
    const candidate = findBlockForLine(line);
    const block = candidate && line >= Number(candidate.dataset.startLine) && line < Number(candidate.dataset.endLine) ? candidate : null;
    if (block !== activePreviewBlock) {
      activePreviewBlock?.classList.remove("active-block");
      block?.classList.add("active-block");
      activePreviewBlock = block;
    }
    updateOutlineHighlight();
  }

  function updateOutlineHighlight() {
    const index = indexAtOrBefore(state.outline.length, (i) => state.outline[i].line <= state.activeLine);
    const item = outlineRows.children[index - outlineFirst];
    if (item !== activeOutlineItem) {
      activeOutlineItem?.classList.remove("active");
      item?.classList.add("active");
      activeOutlineItem = item;
    }
  }

  function indexAtOrBefore(length, condition) {
    let start = 0;
    let end = length;
    while (start < end) {
      const middle = (start + end) >>> 1;
      if (condition(middle)) start = middle + 1;
      else end = middle;
    }
    return start - 1;
  }

  function findBlockForLine(line) {
    const blocks = elPreviewDoc.children;
    const index = indexAtOrBefore(state.loadedBlocks, (i) => Number(blocks[i].dataset.startLine) <= line);
    const block = blocks[index];
    if (block && line < Number(block.dataset.endLine)) return block;
    return blocks[index + 1] || block || null;
  }

  function previewAnchor(index) {
    const block = elPreviewDoc.children[index];
    const origin = elPreviewDoc.children[0]?.offsetTop || 0;
    if (block && index < state.loadedBlocks) {
      return { line: Number(block.dataset.startLine), y: block.offsetTop - origin };
    }
    const last = elPreviewDoc.children[state.loadedBlocks - 1];
    return last ? { line: Number(last.dataset.endLine), y: last.offsetTop + last.offsetHeight - origin } : { line: 0, y: 0 };
  }

  function translatePreview(position, from, to) {
    if (!state.loadedBlocks) return position;
    const index = Math.max(0, indexAtOrBefore(state.loadedBlocks, (i) => previewAnchor(i)[from] <= position));
    const before = previewAnchor(index);
    const after = previewAnchor(index + 1);
    const span = Math.max(1, after[from] - before[from]);
    const fraction = Math.min(1, Math.max(0, (position - before[from]) / span));
    return before[to] + (after[to] - before[to]) * fraction;
  }

  function showPreviewLine(line) {
    const end = state.previewStart + state.loadedBlocks;
    const firstLine = previewAnchor(0).line;
    const lastLine = previewAnchor(state.loadedBlocks).line;
    if ((state.previewStart > 0 && line < firstLine) || (end < state.totalBlocks && line >= lastLine)) {
      state.previewTarget = Math.floor(line);
      requestPreviewBlocks();
      return;
    }
    state.previewTarget = null;
    setSyncedScroll(elPreviewPane, translatePreview(line, "line", "y"));
  }

  const expectedScroll = new WeakMap();
  let scrollFrame = 0;
  let scrollSource = null;
  function setSyncedScroll(element, top) {
    const old = element.scrollTop;
    element.scrollTop = top;
    if (Math.abs(element.scrollTop - old) > 0.5) expectedScroll.set(element, element.scrollTop);
  }

  function syncScrollFrom(element) {
    const expected = expectedScroll.get(element);
    expectedScroll.delete(element);
    if (expected !== undefined && Math.abs(element.scrollTop - expected) < 1) return;
    if (!state.editor || !state.settings?.syncScroll || state.settings.viewMode !== "split") return;
    scrollSource = element;
    if (scrollFrame) return;
    scrollFrame = requestAnimationFrame(() => {
      scrollFrame = 0;
      const editor = state.editor;
      if (!editor || !state.settings?.syncScroll || state.settings.viewMode !== "split") return;
      const scroll = editor.scrollElement;
      if (scrollSource === scroll) {
        showPreviewLine(state.windowStartLine + editor.lineAtScroll(scroll.scrollTop));
      } else if (scrollSource === elPreviewPane) {
        const line = translatePreview(elPreviewPane.scrollTop, "y", "line");
        if (line >= state.windowStartLine && line < state.windowEndLine) {
          setSyncedScroll(scroll, editor.scrollAtLine(line - state.windowStartLine));
        }
      }
    });
  }

  elPreviewPane.addEventListener("scroll", () => {
    if (elPreviewPane.scrollTop + elPreviewPane.clientHeight >= elPreviewPane.scrollHeight - 400) {
      requestPreviewBlocks();
    }
    syncScrollFrom(elPreviewPane);
  });

  function applySourceSelection(target) {
    if (!state.editor) {
      state.editorTarget = target;
      return;
    }
    state.editorTarget = null;
    state.editor.select(target.line - state.windowStartLine, target.col,
      (target.endLine ?? target.line) - state.windowStartLine, target.endCol, target.focus);
  }

  function requestSourceWindow(startLine, target = null) {
    if (!state.doc) return;
    const request = ++requestSequence;
    state.sourceRequest = { request, target };
    setSourceReadOnly(true);
    send({ cmd: "fetchLines", docId: state.activeTabId, startLine, count: 600, request });
  }

  function jumpToLine(line, match = null, focus = true) {
    if (!state.doc) return;
    line = Math.min(state.totalLines - 1, Math.max(0, Math.trunc(line)));
    const target = { ...match, line, focus };
    if (state.largeMode && (line < state.windowStartLine || line >= state.windowEndLine)) {
      requestSourceWindow(Math.max(0, line - 40), target);
    } else {
      applySourceSelection(target);
    }
    if (state.settings?.viewMode === "split") showPreviewLine(line);
    updateActiveBlockHighlight();
  }

  let imagePaste;
  document.addEventListener("paste", (event) => {
    if (!event.target.closest("#editor-host .cm-content") || !state.editor) return;
    const image = Array.from(event.clipboardData?.files ?? []).find((file) => file.type.startsWith("image/")) ??
      Array.from(event.clipboardData?.items ?? []).find((item) => item.kind === "file" && item.type.startsWith("image/"))?.getAsFile();
    if (!image) return;
    event.preventDefault();
    event.stopPropagation();
    if (!state.doc || state.pendingSourceEdit || state.sourceRequest || state.pendingEdits) return;
    if (image.size > 32 * 1024 * 1024) {
      showToast("Clipboard image exceeds 32 MiB");
      return;
    }
    const { from, to } = state.editor.selection;
    const paste = { doc: state.doc, id: state.activeTabId, request: ++requestSequence, from, to, timer: null };
    imagePaste = paste;
    state.pendingSourceEdit = true;
    setSourceReadOnly(true);
    const failPaste = (message) => {
      if (imagePaste !== paste) return;
      clearTimeout(paste.timer);
      imagePaste = null;
      if (state.doc === paste.doc) endSourceAction();
      if (message) showToast(message);
    };
    paste.timer = setTimeout(() => failPaste("Timed out saving clipboard image"), 15000);
    const reader = new FileReader();
    reader.onerror = () => failPaste("Cannot read clipboard image");
    reader.onabort = () => failPaste();
    reader.onload = () => {
      if (imagePaste !== paste || state.doc !== paste.doc) return;
      const data = String(reader.result);
      send({ cmd: "saveClipboardImage", docId: paste.id, request: paste.request, encoded: data.slice(data.indexOf(",") + 1) });
    };
    reader.readAsDataURL(image);
  }, true);

  function applyFormat(fmt) {
    if (state.doc?.texSource) return;
    const editor = state.editor;
    if (!editor || editor.readOnly) return;
    const { from, to } = editor.selection;
    editor.focus();
    formatSelection(fmt, from, to, editor.slice(from, to), editor.lineStart(from), editor.replace);
  }

  function formatSelection(fmt, s, e, sel, lineStart, replace) {
    const wrap = (l, r, fallback) => {
      const inner = sel || fallback;
      const value = l + inner + r;
      if (sel) replace(s, e, value);
      else replace(s, e, value, s + l.length, s + l.length + inner.length);
    };
    const prefixLine = (prefix) => replace(lineStart, lineStart, prefix);

    switch (fmt) {
      case "bold": wrap("**", "**", "bold text"); break;
      case "italic": wrap("*", "*", "italic text"); break;
      case "strike": wrap("~~", "~~", "strikethrough"); break;
      case "highlight": wrap("==", "==", "highlighted"); break;
      case "code": wrap("`", "`", "code"); break;
      case "math-inline": wrap("$", "$", "E = mc^2"); break;
      case "math-block": wrap("\n$$\n", "\n$$\n", "\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2}"); break;
      case "code-block": wrap("\n```rust\n", "\n```\n", "fn main() {}"); break;
      case "link": wrap("[", "](https://)", sel || "link text"); break;
      case "h1": prefixLine("# "); break;
      case "h2": prefixLine("## "); break;
      case "h3": prefixLine("### "); break;
      case "h4": prefixLine("#### "); break;
      case "h5": prefixLine("##### "); break;
      case "h6": prefixLine("###### "); break;
      case "quote": prefixLine("> "); break;
      case "callout": wrap("\n> [!NOTE] Note\n> ", "\n", "Key takeaway..."); break;
      case "ul": prefixLine("- "); break;
      case "ol": prefixLine("1. "); break;
      case "task": prefixLine("- [ ] "); break;
      case "hr": wrap("\n---\n", "", ""); break;
      case "table":
        wrap(
          "\n| Column 1 | Column 2 | Column 3 |\n| :--- | :---: | ---: |\n| Cell 1 | Cell 2 | Cell 3 |\n",
          "",
          ""
        );
        break;
    }
  }

  // ===== Preview Interactions (Task Checkboxes, Code Copy, Live Mode Click, TOC Jump) =====
  elPreviewDoc.addEventListener("click", (e) => {
    // 1. Interactive Task Checkbox
    const cb = e.target.closest(".mt-task-cb");
    if (cb) {
      e.preventDefault();
      e.stopPropagation();
      const block = cb.closest(".mt-block");
      if (!block || !beginSourceAction()) return;
      send({
        cmd: "toggleTask",
        docId: state.activeTabId,
        version: state.doc.version,
        blockId: Number(block.dataset.blockId),
        lineOffset: Number(cb.dataset.lineOffset),
        source: sourceRequestOptions(),
      });
      return;
    }

    // 2. Copy Code Button
    const copyBtn = e.target.closest("[data-copy-code]");
    if (copyBtn) {
      e.stopPropagation();
      const codeEl = copyBtn.closest(".mt-code-wrap")?.querySelector("code");
      if (codeEl && navigator.clipboard) {
        navigator.clipboard.writeText(codeEl.textContent || "");
        copyBtn.textContent = "Copied!";
        setTimeout(() => (copyBtn.textContent = "Copy"), 1400);
      }
      return;
    }

    // 3. TOC internal link jump
    const tocLink = e.target.closest("[data-jump-line]");
    if (tocLink) {
      e.preventDefault();
      e.stopPropagation();
      jumpToLine(Number(tocLink.dataset.jumpLine));
      return;
    }

    const blockEl = e.target.closest(".mt-block");
    if (state.settings?.viewMode === "split" && blockEl) {
      const startLine = Number(blockEl.dataset.startLine);
      state.activeLine = startLine;
      updateActiveBlockHighlight();
    }
  });

  // ===== Draggable Split Divider =====
  let draggingSplit = false;
  elSplitDivider.addEventListener("mousedown", (e) => {
    e.preventDefault();
    draggingSplit = true;
    elSplitDivider.classList.add("dragging");
  });
  window.addEventListener("mousemove", (e) => {
    if (!draggingSplit) return;
    const rect = elPanesWrap.getBoundingClientRect();
    const ratio = Math.min(0.8, Math.max(0.2, (e.clientX - rect.left) / rect.width));
    elSourcePane.style.flex = `0 0 ${(ratio * 100).toFixed(1)}%`;
    if (state.settings) state.settings.splitRatio = ratio;
  });
  window.addEventListener("mouseup", () => {
    if (!draggingSplit) return;
    draggingSplit = false;
    elSplitDivider.classList.remove("dragging");
    if (state.settings) {
      send({ cmd: "updateSettings", patch: { splitRatio: state.settings.splitRatio } });
    }
  });

  // ===== Settings Overlay & Sidebar Tabs =====
  function openSettings() {
    state.settingsOpen = true;
    elSettingsView.classList.remove("hidden");
    $("workspace").inert = true;
    elSidebar.inert = true;
    $("btn-settings").classList.add("active");
    $("btn-settings").setAttribute("aria-expanded", "true");
    $("settings-back").focus();
  }
  function closeSettings(restoreFocus = false) {
    state.settingsOpen = false;
    elSettingsView.classList.add("hidden");
    $("workspace").inert = false;
    elSidebar.inert = false;
    $("btn-settings").classList.remove("active");
    $("btn-settings").setAttribute("aria-expanded", "false");
    if (restoreFocus) $("btn-settings").focus();
  }

  function switchSidebarTab(tab, persist = true) {
    document.querySelectorAll(".sidebar-tab").forEach((b) => {
      b.classList.toggle("active", b.dataset.tab === tab);
    });
    $("sidebar-outline").classList.toggle("hidden", tab !== "outline");
    $("sidebar-files").classList.toggle("hidden", tab !== "files");
    $("files-actions").classList.toggle("hidden", tab !== "files");
    if (persist) {
      send({ cmd: "updateSettings", patch: { sidebarTab: tab } });
    }
  }

  // Find positions stay tied to the document version that produced the results.
  function openFind(focusReplace = false) {
    if (!state.doc) return;
    closeSettings();
    state.findOpen = true;
    elFindBar.classList.remove("hidden");
    const input = focusReplace ? elReplaceInput : elFindInput;
    input.focus();
    input.select();
    triggerSearch();
  }

  function closeFind() {
    state.findOpen = false;
    state.findMatches = [];
    state.editor?.setMatches([], state.windowStartLine);
    clearTimeout(state.searchTimer);
    elFindBar.classList.add("hidden");
  }

  function invalidateSearch() {
    state.searchVersion = null;
    state.findMatches = [];
    const editor = state.editor;
    queueMicrotask(() => {
      if (editor && editor === state.editor && !state.findMatches.length) editor.setMatches([], state.windowStartLine);
    });
    state.findIndex = -1;
    state.searchRequest = ++requestSequence;
    elFindCount.textContent = "0 / 0";
    clearTimeout(state.searchTimer);
    if (state.findOpen && state.doc && elFindInput.value) {
      state.searchTimer = setTimeout(triggerSearch, 90);
    }
  }

  function findOptions() {
    return { docId: state.activeTabId, query: elFindInput.value, caseSensitive: state.findCase, useRegex: state.findRegex };
  }

  function triggerSearch() {
    clearTimeout(state.searchTimer);
    const options = findOptions();
    if (!options.query || !state.doc) {
      invalidateSearch();
      return;
    }
    state.searchVersion = null;
    state.searchRequest = ++requestSequence;
    send({ cmd: "search", ...options, request: state.searchRequest });
  }

  function navigateMatch(delta, focus = false) {
    if (!state.findMatches.length || state.searchVersion !== state.doc?.version) return;
    state.findIndex = (state.findIndex + delta + state.findMatches.length) % state.findMatches.length;
    elFindCount.textContent = (state.findIndex + 1) + " / " + state.findMatches.length;
    const match = state.findMatches[state.findIndex];
    jumpToLine(match.line, match, focus);
  }

  function beginSourceAction() {
    if (!state.doc || state.pendingSourceEdit || state.pendingEdits || state.sourceRequest) return false;
    state.pendingSourceEdit = true;
    setSourceReadOnly(true);
    return true;
  }

  function endSourceAction() {
    state.pendingSourceEdit = false;
    setSourceReadOnly(Boolean(state.sourceRequest));
  }

  // Text menus stay under the title bar and close on activation, Escape or an outside click.
  const menus = Array.from(document.querySelectorAll("#app-menu details"));
  function closeMenus(except = null) {
    for (const menu of menus) {
      if (menu !== except) menu.open = false;
    }
  }
  for (const menu of menus) {
    menu.addEventListener("toggle", () => { if (menu.open) closeMenus(menu); });
    menu.addEventListener("click", (event) => {
      if (event.target.closest("button")) {
        menu.open = false;
        if (menu.contains(document.activeElement)) menu.querySelector("summary").focus();
      }
    });
  }
  document.addEventListener("pointerdown", (event) => {
    if (!event.target.closest("#app-menu details")) closeMenus();
  });

  function requestPrint() {
    if (state.doc && !state.doc.texSource) send({ cmd: "print", docId: state.activeTabId });
  }

  function resolveClose(choice) {
    const docId = state.closingDocId;
    if (!docId) return;
    state.closingDocId = null;
    elCloseDialog.close();
    send({ cmd: "resolveClose", docId, choice });
  }

  elCloseDialog.querySelectorAll("[data-close-choice]").forEach((button) => {
    button.onclick = () => resolveClose(button.dataset.closeChoice);
  });
  elCloseDialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    resolveClose("cancel");
  });

  async function printDocument(html) {
    const paper = $("print-document");
    paper.innerHTML = html;
    hydrateMath(paper);
    await window.MTDiagrams.render(paper);
    const images = Array.from(paper.querySelectorAll("img"));
    for (const image of images) image.loading = "eager";
    let timeout;
    await Promise.race([
      Promise.allSettled([document.fonts.ready, ...images.map((image) => image.decode())]),
      new Promise((resolve) => { timeout = setTimeout(resolve, 5000); }),
    ]);
    clearTimeout(timeout);
    if (state.platform === "windows") window.print();
    else send({ cmd: "windowPrint" });
  }

  // Menu and settings actions.
  $("btn-sidebar").onclick = () => send({ cmd: "updateSettings", patch: { sidebarOpen: !state.settings?.sidebarOpen } });
  $("btn-new").onclick = () => { closeSettings(); send({ cmd: "newTab" }); };
  $("btn-open").onclick = () => { closeSettings(); send({ cmd: "openFileDialog" }); };
  $("menu-open-folder").onclick = () => { closeSettings(); send({ cmd: "openFolderDialog" }); };
  $("btn-save").onclick = () => send({ cmd: "save", saveAs: false });
  $("btn-save-as").onclick = () => send({ cmd: "save", saveAs: true });
  $("btn-export").onclick = () => $("html-export-dialog").showModal();
  $("html-export-cancel").onclick = () => $("html-export-dialog").close();
  $("html-export-form").addEventListener("submit", () => send({ cmd: "exportHtml", inlineImages: $("html-inline-images").checked }));
  $("btn-export-docx").onclick = () => send({ cmd: "exportPandoc", format: "docx" });
  $("btn-export-tex").onclick = () => send({ cmd: "exportPandoc", format: "tex" });
  const latexDialog = $("latex-dialog");
  const latexMain = $("latex-main");
  const latexLog = $("latex-log");
  const latexStatus = $("latex-status");
  const latexRoots = new Map();
  let latexContext, latexRequest, latexBuilding = false, latexOutput, latexLastMain, latexPdfUrl = null, latexHasSyncTex = false;

  const elPdfPane = $("pdf-pane");
  let pdfPaneOpen = false;
  let pdfState = { loaded: false, path: null, hasSyncTex: false };
  let pendingForwardSync = null;

  function normDir(filePath) {
    if (!filePath) return "";
    const norm = filePath.replaceAll("\\", "/");
    const idx = norm.lastIndexOf("/");
    const dir = idx >= 0 ? norm.slice(0, idx + 1) : "";
    return state.platform === "windows" ? dir.toLowerCase() : dir;
  }

  function activeDocMatchesPdf(pdfPath) {
    if (!pdfPath || !state.doc?.path) return false;
    const docNorm = state.platform === "windows"
      ? state.doc.path.replaceAll("\\", "/").toLowerCase()
      : state.doc.path.replaceAll("\\", "/");
    const dir = normDir(pdfPath);
    return Boolean(dir) && docNorm.startsWith(dir);
  }

  function syncPdfPaneVisibility() {
    if (!state.doc) {
      if (pdfPaneOpen || pdfState.loaded) closeBuiltInPdf();
      return;
    }
    if (state.doc.texSource && pdfState.path && !activeDocMatchesPdf(pdfState.path)) {
      closeBuiltInPdf();
      return;
    }
    const show = Boolean(state.doc?.texSource) && pdfPaneOpen;
    elPanesWrap.classList.toggle("tex-pdf-open", show);
    elPdfPane.classList.toggle("hidden", !show);
    $("btn-tex-pdf").classList.toggle("active", show);
    const matchingOutput = activeDocMatchesPdf(latexOutput) && latexHasSyncTex;
    const canSync = Boolean(state.doc?.texSource) && (pdfState.hasSyncTex || matchingOutput);
    $("btn-tex-sync").disabled = !canSync;
    $("pdf-sync-forward").disabled = !pdfState.hasSyncTex;
  }

  const pdfViewer = window.MTPdfViewer.create({
    viewportEl: $("pdf-viewport"),
    pagesEl: $("pdf-pages"),
    pageInputEl: $("pdf-page-input"),
    pageTotalEl: $("pdf-page-total"),
    zoomLabelEl: $("pdf-zoom-label"),
    statusEl: $("pdf-status"),
    onInverseSync(info) {
      send({ cmd: "synctexInverse", pdf: info.path, page: info.page, x: info.x, y: info.y });
    },
    onStateChange(next) {
      pdfState = next;
      syncPdfPaneVisibility();
      if (next.loaded && pendingForwardSync) {
        const target = pendingForwardSync;
        pendingForwardSync = null;
        pdfViewer.highlight(target.page, target.boxes);
      }
    },
  });

  function openBuiltInPdf(url, meta) {
    pdfPaneOpen = true;
    syncPdfPaneVisibility();
    pdfViewer.open(url, { ...meta, loadingText: t("pdfLoading") });
  }

  function closeBuiltInPdf() {
    pendingForwardSync = null;
    pdfPaneOpen = false;
    pdfViewer.close();
    elPanesWrap.classList.remove("tex-pdf-open");
    elPdfPane.classList.add("hidden");
    $("btn-tex-pdf").classList.remove("active");
    $("pdf-sync-forward").disabled = true;
    const matchingOutput = Boolean(state.doc?.texSource) && activeDocMatchesPdf(latexOutput) && latexHasSyncTex;
    $("btn-tex-sync").disabled = !matchingOutput;
  }

  function triggerForwardSync() {
    if (!state.doc?.texSource) return;
    const activePdf = activeDocMatchesPdf(pdfState.path)
      ? pdfState.path
      : activeDocMatchesPdf(latexOutput)
        ? latexOutput
        : undefined;
    const knownMain = latexRoots.get(state.doc.path || "") || undefined;
    send({
      cmd: "synctexForward",
      pdf: activePdf,
      main: knownMain,
      file: state.doc.path || undefined,
      line: (state.activeLine || 0) + 1,
    });
  }

  function refreshLatexControls() {
    $("latex-build").disabled = latexBuilding || !latexMain.value;
    $("latex-choose").disabled = latexBuilding;
    $("latex-engine").disabled = latexBuilding;
    $("latex-copy").disabled = !latexLog.textContent;
    const canOpen = !latexBuilding && Boolean(latexOutput) && latexLastMain === latexMain.value;
    $("latex-open").disabled = !canOpen;
    $("latex-open-external").disabled = !canOpen;
    latexDialog.setAttribute("aria-busy", String(latexBuilding));
  }
  function setLatexLog(log) {
    const follow = latexLog.scrollHeight - latexLog.scrollTop - latexLog.clientHeight < 32;
    latexLog.textContent = log || "";
    latexLog.classList.toggle("hidden", !log);
    if (follow) latexLog.scrollTop = latexLog.scrollHeight;
  }
  function openLatexDialog() {
    if (latexDialog.open) return;
    if (!latexBuilding) {
      const active = state.tabs.find((tab) => tab.id === state.activeTabId);
      const context = active?.path || "";
      if (latexContext !== context || !latexMain.value) {
        latexContext = context;
        latexMain.value = latexRoots.get(context) || "";
        latexStatus.textContent = t("latexSelect");
        setLatexLog("");
        if (!latexMain.value) send({ cmd: "latexProject", request: latexRequest = ++requestSequence });
      }
    }
    refreshLatexControls();
    latexDialog.showModal();
  }
  $("btn-latex").onclick = openLatexDialog;
  $("btn-tex-tools").onclick = openLatexDialog;
  $("btn-tex-pdf").onclick = () => {
    if (pdfPaneOpen) {
      closeBuiltInPdf();
      return;
    }
    const knownMain = latexRoots.get(state.doc?.path || "") || undefined;
    const activePdf = activeDocMatchesPdf(latexOutput) ? latexOutput : undefined;
    send({ cmd: "openPdfPreview", main: knownMain, pdf: activePdf });
  };
  $("btn-tex-sync").onclick = triggerForwardSync;
  $("pdf-prev").onclick = () => pdfViewer.prevPage();
  $("pdf-next").onclick = () => pdfViewer.nextPage();
  $("pdf-page-input").addEventListener("change", () => pdfViewer.goToPage($("pdf-page-input").value));
  $("pdf-page-input").addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      pdfViewer.goToPage($("pdf-page-input").value);
    }
  });
  $("pdf-zoom-in").onclick = () => pdfViewer.zoomIn();
  $("pdf-zoom-out").onclick = () => pdfViewer.zoomOut();
  $("pdf-fit").onclick = () => pdfViewer.fitWidth();
  $("pdf-sync-forward").onclick = triggerForwardSync;
  $("pdf-open-external").onclick = () => send({ cmd: "openLatexPdf", path: pdfState.path || latexOutput || undefined });
  $("pdf-close").onclick = closeBuiltInPdf;
  $("latex-close").onclick = () => latexDialog.close();
  $("latex-choose").onclick = () => send({ cmd: "chooseLatexMain", request: latexRequest = ++requestSequence });
  $("latex-open").onclick = () => {
    latexDialog.close();
    if (latexPdfUrl && latexOutput) {
      openBuiltInPdf(latexPdfUrl, {
        path: latexOutput,
        hasSyncTex: latexHasSyncTex,
        preserveScroll: true,
      });
    } else {
      send({ cmd: "openPdfPreview", pdf: latexOutput || undefined, main: latexMain.value || undefined });
    }
  };
  $("latex-open-external").onclick = () => send({ cmd: "openLatexPdf", path: latexOutput || undefined });
  $("latex-copy").onclick = async () => {
    try {
      await navigator.clipboard.writeText(latexLog.textContent);
      latexStatus.textContent = t("latexCopied");
    } catch {
      const range = document.createRange();
      range.selectNodeContents(latexLog);
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      latexLog.focus();
    }
  };
  $("latex-build").onclick = () => {
    if (state.pendingEdits || state.pendingSourceEdit || state.sourceRequest) {
      latexStatus.textContent = t("latexSavePending");
      return;
    }
    latexBuilding = true;
    latexOutput = null;
    const compiler = $("latex-engine");
    latexStatus.textContent = t("latexRunning", { compiler: compiler.selectedOptions[0].textContent });
    setLatexLog("");
    refreshLatexControls();
    send({ cmd: "compileLatex", main: latexMain.value, engine: compiler.value });
  };
  $("btn-print").onclick = requestPrint;
  $("menu-close-tab").onclick = () => {
    if (state.activeTabId) send({ cmd: "closeTab", id: state.activeTabId });
  };
  $("menu-exit").onclick = () => send({ cmd: "windowClose" });
  $("menu-find").onclick = () => openFind();
  $("menu-replace").onclick = () => openFind(true);
  $("btn-settings").onclick = () => state.settingsOpen ? closeSettings(true) : openSettings();
  $("settings-back").onclick = () => closeSettings(true);
  const languageSelect = $("language-select");
  for (const language of I18N.languages) {
    languageSelect.add(new Option(language.label, language.code));
  }
  languageSelect.onchange = () => send({ cmd: "updateSettings", patch: { language: languageSelect.value } });
  window.addEventListener("afterprint", () => $("print-document").replaceChildren());

  document.querySelectorAll(".mode-btn").forEach((btn) => {
    btn.onclick = () => send({ cmd: "updateSettings", patch: { viewMode: btn.dataset.mode } });
  });

  document.querySelectorAll(".sidebar-tab").forEach((btn) => {
    btn.onclick = () => switchSidebarTab(btn.dataset.tab, true);
  });

  $("btn-open-folder").onclick = () => send({ cmd: "openFolderDialog" });
  $("btn-new-file").onclick = () => {
    $("new-file-inline").classList.toggle("hidden");
    $("new-file-input").focus();
  };
  $("new-file-confirm").onclick = () => {
    const name = $("new-file-input").value.trim();
    if (name) {
    send({ cmd: "workspaceAction", action: "newDocument", name });
      $("new-file-input").value = "";
      $("new-file-inline").classList.add("hidden");
    }
  };
  $("new-file-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") $("new-file-confirm").click();
    if (e.key === "Escape") $("new-file-inline").classList.add("hidden");
  });
  elFileFilter.addEventListener("input", renderFileTree);

  const workspaceMenu = $("workspace-menu");
  const workspaceItemDialog = $("workspace-item-dialog");
  const workspaceName = $("workspace-item-name");
  let workspaceItem;
  elFileTree.addEventListener("contextmenu", (event) => {
    const row = event.target.closest("[data-workspace-path]");
    if (!state.fileTree) return;
    event.preventDefault();
    workspaceItem = row ? { path: row.dataset.workspacePath, name: row.dataset.workspaceName } : state.fileTree;
    for (const action of ["rename", "trash"]) {
      workspaceMenu.querySelector(`[data-workspace-action="${action}"]`).disabled = !row;
    }
    workspaceMenu.classList.remove("hidden");
    workspaceMenu.style.left = `${Math.min(event.clientX, innerWidth - workspaceMenu.offsetWidth - 8)}px`;
    workspaceMenu.style.top = `${Math.min(event.clientY, innerHeight - workspaceMenu.offsetHeight - 8)}px`;
    workspaceMenu.querySelector("button").focus();
  });
  document.addEventListener("pointerdown", (event) => {
    if (!workspaceMenu.contains(event.target)) workspaceMenu.classList.add("hidden");
  });
  workspaceMenu.addEventListener("keydown", (event) => {
    if (event.key === "Escape") workspaceMenu.classList.add("hidden");
  });
  workspaceMenu.addEventListener("click", (event) => {
    const button = event.target.closest("[data-workspace-action]");
    if (!button || !workspaceItem) return;
    workspaceMenu.classList.add("hidden");
    const action = button.dataset.workspaceAction;
    if (action === "reveal") {
      send({ cmd: "workspaceAction", action, path: workspaceItem.path });
      return;
    }
    workspaceItem.action = action;
    $("workspace-item-title").textContent = button.textContent;
    $("workspace-item-confirm").textContent = button.textContent;
    $("workspace-item-description").textContent = action === "trash" ? workspaceItem.name : "";
    workspaceName.classList.toggle("hidden", action === "trash");
    workspaceName.required = action !== "trash";
    workspaceName.value = action === "rename" ? workspaceItem.name : "";
    workspaceItemDialog.showModal();
    if (workspaceName.required) { workspaceName.focus(); workspaceName.select(); }
  });
  $("workspace-item-cancel").onclick = () => workspaceItemDialog.close();
  $("workspace-item-form").addEventListener("submit", () => {
    if (workspaceItem) send({ cmd: "workspaceAction", action: workspaceItem.action,
      path: workspaceItem.path, name: workspaceName.value });
  });

  const workspaceSearch = $("workspace-search-dialog");
  const workspaceQuery = $("workspace-query");
  const workspaceResults = $("workspace-search-results");
  let workspaceSearchTimer, workspaceRequest, workspaceHit;
  function updateWorkspaceSearch() {
    clearTimeout(workspaceSearchTimer);
    const query = workspaceQuery.value;
    workspaceRequest = ++requestSequence;
    workspaceResults.replaceChildren();
    $("workspace-search-status").textContent = query ? t("workspaceSearching") : "";
    const submit = () => send({ cmd: "workspaceSearch", request: workspaceRequest, query,
      caseSensitive: $("workspace-case").checked, useRegex: $("workspace-regex").checked });
    if (query) workspaceSearchTimer = setTimeout(submit, 130);
    else submit();
  }
  function openWorkspaceSearch() {
    if (!workspaceSearch.open) workspaceSearch.showModal();
    workspaceQuery.focus();
    workspaceQuery.select();
    updateWorkspaceSearch();
  }
  workspaceQuery.addEventListener("input", updateWorkspaceSearch);
  $("workspace-case").addEventListener("change", updateWorkspaceSearch);
  $("workspace-regex").addEventListener("change", updateWorkspaceSearch);
  $("workspace-search-close").onclick = () => workspaceSearch.close();
  workspaceSearch.addEventListener("close", () => {
    clearTimeout(workspaceSearchTimer);
    workspaceRequest = ++requestSequence;
    send({ cmd: "workspaceSearch", request: workspaceRequest, query: "", caseSensitive: false, useRegex: false });
  });

  // ===== Frameless Window Chrome Controls, Drag & Edge Resize =====
  const elTopbar = $("topbar");
  elTopbar.addEventListener("mousedown", (e) => {
    if (state.platform !== "windows" || e.button !== 0) return;
    if (e.target.closest("button, input, .doc-tab, .mode-switch")) return;
    e.preventDefault();
    send({ cmd: "windowDrag" });
  });
  elTopbar.addEventListener("dblclick", (e) => {
    if (state.platform !== "windows") return;
    if (e.target.closest("button, input, .doc-tab, .mode-switch")) return;
    send({ cmd: "windowToggleMaximize" });
  });
  document.querySelectorAll(".resize-handle").forEach((h) => {
    h.addEventListener("mousedown", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      send({ cmd: "windowDragResize", dir: h.dataset.resize });
    });
  });
  $("win-min").onclick = () => send({ cmd: "windowMinimize" });
  $("win-max").onclick = () => send({ cmd: "windowToggleMaximize" });
  $("win-close").onclick = () => send({ cmd: "windowClose" });

  // Hero buttons
  $("hero-new").onclick = () => send({ cmd: "newTab" });
  $("hero-open").onclick = () => send({ cmd: "openFileDialog" });
  $("hero-folder").onclick = () => send({ cmd: "openFolderDialog" });
  $("hero-settings").onclick = openSettings;

  // Format bar buttons
  document.querySelectorAll("[data-fmt]").forEach((btn) => {
    btn.onclick = () => applyFormat(btn.dataset.fmt);
  });
  $("btn-find").onclick = () => openFind(false);
  $("btn-focus").onclick = () => {
    send({ cmd: "updateSettings", patch: { focusMode: !(state.settings && state.settings.focusMode) } });
  };
  $("btn-typewriter").onclick = () => {
    send({
      cmd: "updateSettings",
      patch: { typewriterMode: !(state.settings && state.settings.typewriterMode) },
    });
  };
  $("btn-sync").onclick = () => {
    send({ cmd: "updateSettings", patch: { syncScroll: !(state.settings && state.settings.syncScroll) } });
  };

  // Large file window navigation
  $("large-prev").onclick = () => {
    const nextStart = Math.max(0, state.windowStartLine - 500);
    requestSourceWindow(nextStart);
  };
  $("large-next").onclick = () => {
    const nextStart = Math.min(Math.max(0, state.totalLines - 100), state.windowStartLine + 500);
    requestSourceWindow(nextStart);
  };
  $("large-goto-btn").onclick = () => {
    const line = Math.max(1, Number($("large-goto-input").value) || 1) - 1;
    jumpToLine(line);
  };
  $("btn-load-more").onclick = () => {
    requestPreviewBlocks(200);
  };

  // Find bar events
  elFindInput.addEventListener("input", triggerSearch);
  elFindInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") navigateMatch(e.shiftKey ? -1 : 1);
    if (e.key === "Escape") closeFind();
  });
  $("find-case").onclick = () => {
    state.findCase = !state.findCase;
    $("find-case").classList.toggle("active", state.findCase);
    triggerSearch();
  };
  $("find-regex").onclick = () => {
    state.findRegex = !state.findRegex;
    $("find-regex").classList.toggle("active", state.findRegex);
    triggerSearch();
  };
  $("find-prev").onclick = () => navigateMatch(-1);
  $("find-next").onclick = () => navigateMatch(1);
  $("find-close").onclick = closeFind;
  $("btn-replace-one").onclick = () => {
    const result = state.findMatches[state.findIndex];
    if (!result || state.searchVersion !== state.doc?.version || !beginSourceAction()) return;
    send({
      cmd: "replaceOne",
      ...findOptions(),
      version: state.searchVersion,
      startChar: result.startChar,
      endChar: result.endChar,
      replacement: elReplaceInput.value,
      source: sourceRequestOptions(),
    });
  };
  $("btn-replace-all").onclick = () => {
    const options = findOptions();
    if (!options.query || !beginSourceAction()) return;
    send({ cmd: "replaceAll", ...options, replacement: elReplaceInput.value });
  };

  // Settings navigation & controls
  document.querySelectorAll(".settings-nav-item").forEach((btn) => {
    btn.onclick = () => {
      document.querySelectorAll(".settings-nav-item").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      const sec = btn.dataset.section;
      document.querySelectorAll(".settings-section").forEach((s) => s.classList.add("hidden"));
      $("sec-" + sec).classList.remove("hidden");
    };
  });

  document.querySelectorAll("[data-light]").forEach((btn) => {
    btn.onclick = () => send({ cmd: "updateSettings", patch: { light: btn.dataset.light === "true" } });
  });
  document.querySelectorAll("[data-preset]").forEach((btn) => {
    btn.onclick = () => send({ cmd: "updateSettings", patch: { themePreset: btn.dataset.preset } });
  });
  document.querySelectorAll("#wp-effect-group .pill-opt").forEach((btn) => {
    btn.onclick = () => send({ cmd: "updateSettings", patch: { wallpaperEffect: btn.dataset.effect } });
  });
  $("wp-vis-minus").onclick = () => {
    const cur = state.settings ? state.settings.wallpaperHeroOpacity : 1;
    send({ cmd: "updateSettings", patch: { wallpaperHeroOpacity: Math.max(0.1, cur - 0.1) } });
  };
  $("wp-vis-plus").onclick = () => {
    const cur = state.settings ? state.settings.wallpaperHeroOpacity : 1;
    send({ cmd: "updateSettings", patch: { wallpaperHeroOpacity: Math.min(1.0, cur + 0.1) } });
  };
  $("toggle-wp-fills").onclick = () => {
    send({
      cmd: "updateSettings",
      patch: { wallpaperFillsWindow: !(state.settings && state.settings.wallpaperFillsWindow) },
    });
  };
  $("toggle-wp-colors").onclick = () => {
    send({
      cmd: "updateSettings",
      patch: { wallpaperThemeColors: !(state.settings && state.settings.wallpaperThemeColors) },
    });
  };
  $("toggle-reduce-motion").onclick = () => {
    send({
      cmd: "updateSettings",
      patch: { reduceMotion: !(state.settings && state.settings.reduceMotion) },
    });
  };
  $("font-minus").onclick = () => {
    send({ cmd: "updateSettings", patch: { fontSize: (state.settings?.fontSize || 14) - 1 } });
  };
  $("font-plus").onclick = () => {
    send({ cmd: "updateSettings", patch: { fontSize: (state.settings?.fontSize || 14) + 1 } });
  };
  let zoomTimer;
  let zoomPending;
  function zoomEditor(size) {
    zoomPending = Math.max(10, Math.min(28, Math.round(size)));
    clearTimeout(zoomTimer);
    zoomTimer = setTimeout(() => {
      send({ cmd: "updateSettings", patch: { fontSize: zoomPending } });
      zoomPending = undefined;
    }, 75);
  }
  elPanesWrap.addEventListener("wheel", (event) => {
    if (!(event.ctrlKey || event.metaKey) || !event.deltaY) return;
    event.preventDefault();
    zoomEditor((zoomPending ?? state.settings?.fontSize ?? 14) - Math.sign(event.deltaY));
  }, { passive: false });
  let gestureBase = 14;
  elPanesWrap.addEventListener("gesturestart", (event) => {
    event.preventDefault();
    gestureBase = state.settings?.fontSize ?? 14;
  });
  elPanesWrap.addEventListener("gesturechange", (event) => {
    event.preventDefault();
    if (Number.isFinite(event.scale)) zoomEditor(gestureBase * event.scale);
  });

  $("font-reset").onclick = () => {
    send({ cmd: "updateSettings", patch: { fontSize: 14 } });
  };
  $("width-minus").onclick = () => {
    send({ cmd: "updateSettings", patch: { previewMaxWidth: (state.settings?.previewMaxWidth || 860) - 60 } });
  };
  $("width-plus").onclick = () => {
    send({ cmd: "updateSettings", patch: { previewMaxWidth: (state.settings?.previewMaxWidth || 860) + 60 } });
  };
  $("toggle-line-nums").onclick = () => {
    send({
      cmd: "updateSettings",
      patch: { showLineNumbers: !(state.settings && state.settings.showLineNumbers) },
    });
  };
  $("toggle-sync-scroll").onclick = () => {
    send({
      cmd: "updateSettings",
      patch: { syncScroll: !(state.settings && state.settings.syncScroll) },
    });
  };
  $("toggle-autosave").onclick = () => {
    send({
      cmd: "updateSettings",
      patch: { autoSave: !(state.settings && state.settings.autoSave) },
    });
  };

  // ===== Global Keyboard Shortcuts =====
  window.addEventListener("keydown", (e) => {
    if (e.isComposing || e.defaultPrevented || elCloseDialog.open) return;
    const openDialog = document.querySelector("dialog[open]");
    const mod = e.ctrlKey || e.metaKey;
    if (openDialog) {
      if (openDialog === latexDialog && mod && e.key.toLowerCase() === "enter") {
        e.preventDefault();
        $("latex-build").click();
      }
      return;
    }

    if (e.key === "Escape") {
      const menu = menus.find((item) => item.open);
      if (menu) {
        closeMenus();
        menu.querySelector("summary").focus();
      }
      else if (state.settingsOpen) closeSettings(true);
      else if (state.findOpen) closeFind();
      return;
    }
    if (e.key === "F8") {
      e.preventDefault();
      $("btn-focus").click();
      return;
    }
    if (e.key === "F9") {
      e.preventDefault();
      $("btn-typewriter").click();
      return;
    }
    if (!mod) return;

    const k = e.key.toLowerCase();
    if (k === "enter") {
      e.preventDefault();
      if (latexDialog.open) $("latex-build").click();
      else openLatexDialog();
    }
    else if (e.altKey && k === "j" && state.doc?.texSource) {
      e.preventDefault();
      triggerForwardSync();
    }
    else if (state.platform === "macos" && k === "q") { e.preventDefault(); send({ cmd: "windowClose" }); }
    else if (state.platform === "macos" && k === "w") { e.preventDefault(); $("menu-close-tab").click(); }
    else if (k === ",") { e.preventDefault(); state.settingsOpen ? closeSettings() : openSettings(); }
    else if (k === "s") { e.preventDefault(); send({ cmd: "save", saveAs: e.shiftKey }); }
    else if (k === "o") { e.preventDefault(); send({ cmd: "openFileDialog" }); }
    else if (k === "n") { e.preventDefault(); closeSettings(); send({ cmd: "newTab" }); }
    else if (k === "f") { e.preventDefault(); if (e.shiftKey) openWorkspaceSearch(); else openFind(false); }
    else if (k === "h") { e.preventDefault(); openFind(true); }
    else if (k === "p") { e.preventDefault(); requestPrint(); }
    else if (k === "\\") { e.preventDefault(); send({ cmd: "updateSettings", patch: { viewMode: "split" } }); }
    else if (e.code === "Slash" || k === "/") {
      e.preventDefault();
      const target = e.shiftKey ? "source" : state.settings?.viewMode === "live" ? "split" : "live";
      send({ cmd: "updateSettings", patch: { viewMode: target } });
    }
    else if (k === "l" && e.shiftKey) { e.preventDefault(); $("btn-sidebar").click(); }
    else if (k === "=" || k === "+") { e.preventDefault(); $("font-plus").click(); }
    else if (k === "-") { e.preventDefault(); $("font-minus").click(); }
    else if (k === "0") { e.preventDefault(); $("font-reset").click(); }
    else if (elEditorHost.contains(document.activeElement)) {
      if (k === "b") { e.preventDefault(); applyFormat("bold"); }
      else if (k === "i") { e.preventDefault(); applyFormat("italic"); }
      else if (k === "k" && e.shiftKey) { e.preventDefault(); applyFormat("code-block"); }
      else if (k === "k") { e.preventDefault(); applyFormat("link"); }
      else if (k === "`") { e.preventDefault(); applyFormat("code"); }
      else if (k === "m" && e.shiftKey) { e.preventDefault(); applyFormat("math-block"); }
      else if (k === "m") { e.preventDefault(); applyFormat("math-inline"); }
      else if (k === "t") { e.preventDefault(); applyFormat("table"); }
      else if (k >= "1" && k <= "6") { e.preventDefault(); applyFormat("h" + k); }
    }
  });

  // ===== Rust -> WebView Event Receiver =====
  window.__mtReceive = function (msg) {
    if (!msg || !msg.event) return;
    switch (msg.event) {
      case "init":
        state.pandocAvailable = Boolean(msg.pandocAvailable);
        state.platform = msg.platform || "windows";
        document.body.classList.toggle("native-chrome", state.platform !== "windows");
        if (state.platform === "macos") {
          document.querySelectorAll(".app-menu kbd, .shortcut-grid kbd").forEach((el) => {
            el.textContent = platformShortcut(el.textContent);
          });
          document.querySelectorAll("[title]:not([data-i18n-title])").forEach((el) => {
            el.title = platformShortcut(el.title);
          });
        }
        if (msg.version) {
          const version = $("about-version");
          version.textContent = "v" + msg.version;
          version.hidden = false;
        }
        state.tabs = msg.tabs || [];
        state.activeTabId = msg.activeTabId || null;
        state.fileTree = msg.fileTree || null;
        applySettingsUI(msg.settings);
        applyWallpaperAndTheme(msg.wallpaper);
        renderTabs();
        loadDocumentSnapshot(msg.doc);
        renderFileTree();
        break;

      case "patch": {
        if (msg.docId !== state.activeTabId || !state.doc || msg.patch.version <= state.doc.version) break;
        if (msg.userEdit) state.pendingEdits = Math.max(0, state.pendingEdits - 1);
        if (msg.source && (!msg.userEdit || !state.pendingEdits) && !state.sourceRequest) {
          applySourceWindow(msg.source, true);
        }
        if (!msg.userEdit) {
          endSourceAction();
          invalidateSearch();
        }
        applyIncrementalPatch(msg.patch, msg.deferredPreview);
        const tab = state.tabs.find((item) => item.id === msg.docId);
        if (tab && !tab.dirty && msg.dirty) {
          tab.dirty = true;
          renderTabs();
        }
        break;
      }

      case "resyncSource":
        if (msg.docId === state.activeTabId) {
          const hadPending = state.pendingEdits > 0;
          state.pendingEdits = 0;
          endSourceAction();
          invalidateSearch();
          if (hadPending) requestSourceWindow(state.windowStartLine);
        }
        break;

      case "liveWidgetsRendered":
        liveWidgetRequests.get(msg.request)?.(msg.html);
        break;

      case "externalChange":
        if (msg.docId !== state.activeTabId) break;
        externalDialog.dataset.docId = msg.docId;
        $("external-change-name").textContent = msg.title;
        $("external-change-description").textContent = t(msg.missing ? "externalMissingDescription" : "externalChangeDescription");
        $("external-reload").disabled = msg.missing;
        if (!externalDialog.open) externalDialog.showModal();
        break;

      case "blocksLoaded": {
        if (msg.docId !== state.activeTabId || msg.request !== state.blockRequest) break;
        state.blockRequest = null;
        if (msg.version !== state.doc?.version) {
          if (state.previewNeedsRefresh || state.previewTarget !== null) requestPreviewBlocks();
          break;
        }
        if (msg.refresh) {
          state.previewNeedsRefresh = false;
          const target = state.previewTarget ?? translatePreview(elPreviewPane.scrollTop, "y", "line");
          replacePreviewBlocks(msg.blocks, msg.start);
          updatePreviewToc();
          updateActiveBlockHighlight();
          showPreviewLine(target);
        } else if (msg.start === state.previewStart + state.loadedBlocks) {
          const fragment = document.createDocumentFragment();
          for (const block of msg.blocks) fragment.append(createBlockElement(block));
          elPreviewDoc.append(fragment);
          state.loadedBlocks += msg.blocks.length;
          updateActiveBlockHighlight();
          updateLoadMoreVisibility();
        }
        if (state.previewNeedsRefresh || state.previewTarget !== null) requestPreviewBlocks();
        break;
      }

      case "linesLoaded": {
        if (msg.docId !== state.activeTabId || msg.request !== state.sourceRequest?.request) break;
        const target = state.sourceRequest.target;
        state.sourceRequest = null;
        setSourceReadOnly(state.pendingSourceEdit);
        applySourceWindow(msg.source);
        if (target) applySourceSelection(target);
        if (state.settings?.viewMode === "split" && state.settings.syncScroll) {
          showPreviewLine(target?.line ?? state.windowStartLine);
        }
        break;
      }

      case "tabSwitched":
        state.tabs = msg.tabs || [];
        state.activeTabId = msg.activeTabId || null;
        if (msg.settings) applySettingsUI(msg.settings);
        if (msg.fileTree !== undefined) state.fileTree = msg.fileTree;
        renderTabs();
        loadDocumentSnapshot(msg.doc);
        if (workspaceHit) {
          const hit = workspaceHit;
          workspaceHit = null;
          if (msg.doc?.path === hit.path) jumpToLine(hit.line, hit);
        }
        if (msg.toast) showToast(msg.toast);
        break;

      case "tabsUpdated":
        applyTabSummaries(msg.tabs);
        if (msg.fileTree !== undefined) { state.fileTree = msg.fileTree; renderFileTree(); }
        if (msg.toast) showToast(msg.toast);
        break;

      case "confirmClose":
        applyTabSummaries(msg.tabs);
        state.closingDocId = msg.docId;
        $("close-document-name").textContent = msg.title;
        if (!elCloseDialog.open) elCloseDialog.showModal();
        break;

      case "saved":
        applyTabSummaries(msg.tabs);
        if (msg.fileTree !== undefined) {
          state.fileTree = msg.fileTree;
          renderFileTree();
        }
        if (msg.toast) showToast(msg.toast);
        break;

      case "fileTree":
        state.fileTree = msg.fileTree;
        switchSidebarTab("files", true);
        if (state.settings && !state.settings.sidebarOpen) {
          send({ cmd: "updateSettings", patch: { sidebarOpen: true } });
        }
        renderFileTree();
        break;

      case "settingsUpdated":
      case "wallpaperUpdated":
        if (msg.settings) applySettingsUI(msg.settings);
        if (msg.wallpaper) applyWallpaperAndTheme(msg.wallpaper);
        break;

      case "workspaceSearchResults": {
        if (msg.request !== workspaceRequest || !workspaceSearch.open) break;
        const hits = msg.matches || [];
        $("workspace-search-status").textContent = msg.error ||
          t(msg.truncated ? "workspaceMoreMatches" : "workspaceMatches", { n: hits.length });
        const fragment = document.createDocumentFragment();
        for (const hit of hits) {
          const button = document.createElement("button");
          button.className = "workspace-hit";
          const location = document.createElement("strong");
          location.textContent = `${hit.relative}:${hit.line + 1}`;
          const excerpt = document.createElement("span");
          excerpt.textContent = hit.excerpt;
          button.append(location, excerpt);
          button.onclick = () => {
            workspaceHit = hit;
            workspaceSearch.close();
            send({ cmd: "openPath", path: hit.path });
          };
          fragment.appendChild(button);
        }
        workspaceResults.replaceChildren(fragment);
        break;
      }

      case "searchResults":
        if (state.findOpen && msg.docId === state.activeTabId && msg.request === state.searchRequest &&
            msg.version === state.doc?.version && !state.pendingEdits) {
          state.searchVersion = msg.version;
          state.findMatches = msg.matches;
          state.editor?.setMatches(msg.matches, state.windowStartLine);
          state.findIndex = state.findMatches.length ? 0 : -1;
          elFindCount.textContent = "0 / 0";
          if (state.findMatches.length) navigateMatch(0);
        }
        break;

      case "printDocument":
        if (msg.docId === state.activeTabId) void printDocument(msg.html);
        break;

      case "insertImagePath":
        if (msg.path && state.doc && !elCloseDialog.open) {
          const path = msg.path.replaceAll("\\", "/").split("/").map(encodeURIComponent).join("/");
          const name = (msg.path.split(/[\\/]/).pop() || "image").replace(/[[\]\\]/g, "\\$&");
          const image = "\n![" + name + "](<" + path + ">)\n";
          if (state.editor) {
            const { from, to } = state.editor.selection;
            state.editor.replace(from, to, image);
          }
        }
        break;

      case "clipboardImageSaved": {
        const paste = imagePaste;
        if (!paste || paste.request !== msg.request || paste.id !== msg.docId) break;
        clearTimeout(paste.timer);
        imagePaste = null;
        applyTabSummaries(msg.tabs);
        if (state.doc !== paste.doc) break;
        endSourceAction();
        if (msg.error) showToast(msg.error);
        if (!msg.path) break;
        const encodedPath = msg.path.replaceAll("\\", "/").split("/").map(encodeURIComponent).join("/");
        const markdown = `![](<${encodedPath}>)`;
        if (state.editor) {
          const maxLen = state.editor.text.length;
          const from = Math.max(0, Math.min(paste.from, maxLen));
          const to = Math.max(from, Math.min(paste.to, maxLen));
          state.editor.replace(from, to, markdown);
        }
        break;
      }

      case "latexProject":
        if (msg.request !== latexRequest) break;
        if (msg.main) {
          latexMain.value = msg.main;
          latexRoots.set(latexContext, msg.main);
          latexStatus.textContent = "";
          setLatexLog("");
        }
        if (msg.error) latexStatus.textContent = msg.error;
        refreshLatexControls();
        break;

      case "latexStarted":
        if (msg.tabs) applyTabSummaries(msg.tabs);
        latexMain.value = msg.main;
        latexStatus.textContent = t("latexRunning", { compiler: msg.compiler });
        break;

      case "latexProgress":
        setLatexLog(msg.log);
        refreshLatexControls();
        break;

      case "latexFinished":
        if (msg.tabs) applyTabSummaries(msg.tabs);
        latexBuilding = false;
        latexLastMain = msg.result?.main;
        if (msg.result?.pdf) {
          latexOutput = msg.result.pdf;
          latexPdfUrl = msg.pdfUrl || null;
          latexHasSyncTex = Boolean(msg.hasSyncTex);
        }
        latexStatus.textContent = msg.error || msg.result?.error || t("latexReady", { path: latexOutput });
        setLatexLog(msg.result?.log || msg.log);
        refreshLatexControls();
        if (msg.result?.pdf && latexPdfUrl && state.doc?.texSource && activeDocMatchesPdf(latexOutput)) {
          openBuiltInPdf(latexPdfUrl, {
            path: latexOutput,
            hasSyncTex: latexHasSyncTex,
            preserveScroll: true,
          });
        } else {
          syncPdfPaneVisibility();
        }
        break;

      case "pdfLoaded":
        latexOutput = msg.path;
        latexPdfUrl = msg.url;
        latexHasSyncTex = Boolean(msg.hasSyncTex);
        openBuiltInPdf(msg.url, {
          path: msg.path,
          title: msg.title,
          hasSyncTex: latexHasSyncTex,
          preserveScroll: Boolean(msg.preserveScroll),
        });
        break;

      case "pdfError":
        pendingForwardSync = null;
        showToast(msg.message || "Cannot open PDF");
        break;

      case "synctexForwardResult":
        if (msg.error) {
          showToast(msg.error);
        } else if (!pdfPaneOpen || !pdfState.loaded) {
          pendingForwardSync = { page: msg.page, boxes: msg.boxes };
          if (latexPdfUrl && latexOutput) {
            openBuiltInPdf(latexPdfUrl, {
              path: latexOutput,
              hasSyncTex: true,
              preserveScroll: true,
            });
          } else {
            send({ cmd: "openPdfPreview", pdf: latexOutput || undefined, main: latexMain.value || undefined, preserveScroll: true });
          }
        } else {
          pdfViewer.highlight(msg.page, msg.boxes);
        }
        break;

      case "synctexInverseResult":
        if (msg.error) {
          showToast(msg.error);
        } else if (msg.line) {
          jumpToLine(Math.max(0, Number(msg.line) - 1));
        }
        break;

      case "latexCloseWhileBuilding":
        openLatexDialog();
        showToast(t("latexBusy"));
        break;

      case "nativeMenu":
        if (msg.id === "quit") send({ cmd: "windowClose" });
        else if (msg.id === "settings") openSettings();
        else if (msg.id === "about") {
          openSettings();
          document.querySelector('[data-section="about"]')?.click();
        } else if (msg.id === "undo") state.editor?.undo();
        else if (msg.id === "redo") state.editor?.redo();
        else if (msg.id === "workspace-search") openWorkspaceSearch();
        else if (["btn-new", "btn-open", "menu-open-folder", "btn-save", "btn-save-as", "btn-print", "btn-latex",
          "menu-close-tab", "menu-find", "font-plus", "font-minus", "font-reset"].includes(msg.id)) $(msg.id).click();
        break;

      case "windowState": {
        const maximized = !!msg.maximized;
        document.body.classList.toggle("maximized", maximized);
        const icon = $("win-max-icon");
        if (icon) {
          icon.innerHTML = maximized
            ? '<path d="M3.5 4.2V2.4h6.1v6.1H7.8" fill="none" stroke="currentColor" stroke-width="1.2"/><rect x="2.2" y="4.2" width="5.8" height="5.8" rx="1" fill="none" stroke="currentColor" stroke-width="1.2"/>'
            : '<rect x="2.2" y="2.2" width="7.6" height="7.6" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.2"/>';
        }
        $("win-max").title = maximized ? t("winRestore") : t("winMax");
        break;
      }

      case "toast":
        showToast(msg.message);
        break;
    }
  };

  // Bootstrap!
  send({ cmd: "init" });
})();
