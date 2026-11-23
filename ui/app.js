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
      el.title = t(el.dataset.i18nTitle);
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
      updateCursorStatus();
      updateLoadMoreVisibility();
      updateLargeBar();
    } else {
      $("st-cursor").textContent = "—";
      $("st-words").textContent = t("stWords", { n: 0 });
      $("st-chars").textContent = t("stChars", { n: 0 });
      $("st-read").textContent = t("stRead", { n: 0 });
    }
    if (state.settings) {
      renderHeroRecent(state.settings.recentFiles || []);
      const modeKey =
        state.settings.viewMode === "split"
          ? "modeSplitFull"
          : state.settings.viewMode === "live"
            ? "modeLiveFull"
            : "modeSourceFull";
      $("st-mode").textContent = t(modeKey);
    }
  }

  // ===== Application State =====
  const state = {
    settings: null,
    tabs: [],
    activeTabId: null,
    doc: null, // active doc metadata
    prevText: "",
    windowStartLine: 0,
    windowEndLine: 0,
    sourceContext: "",
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
    live: null,
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
  const elLineGutter = $("line-gutter");
  const elSourceHighlight = $("source-highlight");
  const sourceHighlight = new window.MTSourceHighlight(elSourceHighlight);
  const elSourceEditor = $("source-editor");
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

  function computeMinimalDiff(oldStr, newStr) {
    if (oldStr === newStr) return null;
    const oldLen = oldStr.length;
    const newLen = newStr.length;
    let prefix = 0;
    const minLen = Math.min(oldLen, newLen);
    while (prefix < minLen && oldStr.charCodeAt(prefix) === newStr.charCodeAt(prefix)) {
      prefix++;
    }
    // Never split a UTF-16 surrogate pair
    if (prefix > 0 && prefix < oldLen) {
      const c = oldStr.charCodeAt(prefix - 1);
      if (c >= 0xd800 && c <= 0xdbff) prefix--;
    }
    let oldEnd = oldLen;
    let newEnd = newLen;
    while (
      oldEnd > prefix &&
      newEnd > prefix &&
      oldStr.charCodeAt(oldEnd - 1) === newStr.charCodeAt(newEnd - 1)
    ) {
      oldEnd--;
      newEnd--;
    }
    if (oldEnd < oldLen) {
      const c = oldStr.charCodeAt(oldEnd);
      if (c >= 0xdc00 && c <= 0xdfff) {
        oldEnd++;
        newEnd++;
      }
    }
    return {
      startChar: utf16ToScalarCount(oldStr, prefix),
      endChar: utf16ToScalarCount(oldStr, oldEnd),
      text: newStr.slice(prefix, newEnd),
    };
  }

  let toastTimer = null;
  function showToast(msg) {
    if (!msg) return;
    elToast.textContent = msg;
    elToast.classList.remove("hidden");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => elToast.classList.add("hidden"), 2600);
  }

  // Reuse line elements so typing updates the changed tokens rather than the whole overlay.
  let gutterStart = -1;
  let gutterLength = 0;
  function updateSourceOverlayAndGutter() {
    sourceHighlight.update(elSourceEditor.value, state.sourceContext, state.windowStartLine === 0);
    const length = sourceHighlight.records.length;
    if (gutterStart !== state.windowStartLine || gutterLength !== length) {
      gutterStart = state.windowStartLine;
      gutterLength = length;
      elLineGutter.textContent = Array.from({ length }, (_, i) => gutterStart + i + 1).join("\n");
    }
    mirrorSourceScroll();
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
    placeholder.className = "mt-live-empty";
    const label = document.createElement("p");
    label.className = "mt-live-empty-label";
    label.textContent = t("liveEmptyPlaceholder");
    placeholder.append(label);
    elPreviewDoc.append(placeholder);
  }

  function replacePreviewBlocks(blocks, start = 0) {
    const focused = detachLiveAnchor();
    const fragment = document.createDocumentFragment();
    for (const block of blocks) fragment.append(createBlockElement(block));
    elPreviewDoc.replaceChildren(fragment);
    state.loadedBlocks = blocks.length;
    state.previewStart = start;
    ensureEmptyPreview();
    mountLiveAnchor(focused);
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
    const focused = state.live && document.activeElement === state.live.editor;
    if (elPreviewDoc.firstElementChild?.classList.contains("mt-live-empty")) {
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
    mountLiveAnchor(focused);
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

  function applyIncrementalPatch(patch) {
    state.totalBlocks = patch.totalBlocks;
    state.totalLines = patch.totalLines;
    state.doc.version = patch.version;
    state.doc.stats = patch.stats;
    patchPreviewBlocks(patch);

    if (patch.outlineChanged && patch.outline) {
      state.outline = patch.outline;
      renderOutline();
      updatePreviewToc();
    }
    if (patch.refreshPreview) {
      state.previewNeedsRefresh = true;
      requestPreviewBlocks();
    }

    updateStatsUI(patch.stats);
    updateActiveBlockHighlight();
    updateLoadMoreVisibility();
    updateLargeBar();

  }

  function requestPreviewBlocks(count = 150) {
    if (!state.doc || state.blockRequest) return;
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

  // Live editing uses text offsets. A block split or merge can move the same editor.
  function detachLiveAnchor() {
    const live = state.live;
    if (!live) return false;
    const focused = document.activeElement === live.editor;
    live.anchor?.classList.remove("editing-live");
    live.anchor?.firstElementChild?.classList.remove("hidden");
    live.wrap.remove();
    live.anchor = null;
    return focused;
  }

  function mountLiveAnchor(restoreFocus = false) {
    const live = state.live;
    if (!live) return;
    const anchor = findBlockForLine(live.baseLine) || elPreviewDoc.firstElementChild;
    if (!anchor) return;
    if (anchor !== live.anchor || !anchor.contains(live.wrap)) {
      restoreFocus = detachLiveAnchor() || restoreFocus;
      live.anchor = anchor;
      anchor.classList.add("editing-live");
      anchor.append(live.wrap);
    }
    const content = anchor.firstElementChild;
    const containsLine = !anchor.dataset.blockId ||
      (Number(anchor.dataset.startLine) <= live.baseLine && live.baseLine < Number(anchor.dataset.endLine));
    content.classList.toggle("hidden", containsLine);
    const math = containsLine && content.querySelector(".mt-math-block, .mt-math-inline");
    const mathHtml = math ? content.innerHTML : "";
    if (mathHtml !== live.mathHtml) {
      live.mathHtml = mathHtml;
      live.math.innerHTML = mathHtml;
      live.math.classList.toggle("hidden", !mathHtml);
    }
    if (restoreFocus && document.activeElement !== live.editor) live.editor.focus({ preventScroll: true });
  }

  function enterLiveBlockEdit(block, focusAtEnd = true) {
    if (state.settings?.viewMode !== "live" || !state.doc || state.pendingSourceEdit) return;
    if (state.live?.anchor === block) return;
    exitLiveBlockEdit();
    const baseLine = Number(block.dataset.startLine || 0);
    const wrap = document.createElement("div");
    wrap.className = "mt-live-editor-wrap";
    const input = document.createElement("div");
    input.className = "mt-live-input";
    const overlay = document.createElement("div");
    overlay.className = "mt-live-source";
    overlay.setAttribute("aria-hidden", "true");
    const highlight = new window.MTSourceHighlight(overlay);
    const editor = document.createElement("textarea");
    editor.className = "mt-live-textarea";
    editor.value = block.dataset.source ?? "";
    editor.spellcheck = false;
    editor.setAttribute("aria-label", t("liveEmptyPlaceholder"));
    const math = document.createElement("div");
    math.className = "mt-live-math-preview hidden";
    input.append(overlay, editor);
    wrap.append(input, math);

    const live = { baseLine, wrap, editor, math, mathHtml: "", text: editor.value, anchor: null };
    state.live = live;
    mountLiveAnchor(true);
    const resize = () => {
      highlight.update(editor.value, "", baseLine === 0);
      editor.style.height = "auto";
      editor.style.height = Math.max(28, editor.scrollHeight) + "px";
    };
    resize();
    const position = focusAtEnd ? editor.value.length : 0;
    editor.setSelectionRange(position, position);

    editor.addEventListener("input", () => {
      if (state.live !== live) return;
      const diff = computeMinimalDiff(live.text, editor.value);
      live.text = editor.value;
      resize();
      if (diff) sendEdit(baseLine, diff, sourceRequestOptions());
    });
    editor.addEventListener("keydown", (event) => {
      if (event.isComposing) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        exitLiveBlockEdit();
        return;
      }
      const direction = event.key === "ArrowUp" && editor.selectionEnd === 0 ? -1 :
        event.key === "ArrowDown" && editor.selectionStart === editor.value.length ? 1 : 0;
      const next = direction < 0 ? live.anchor.previousElementSibling :
        direction > 0 ? live.anchor.nextElementSibling : null;
      if (next?.classList.contains("mt-block")) {
        event.preventDefault();
        enterLiveBlockEdit(next, direction < 0);
      }
    });
  }

  function exitLiveBlockEdit() {
    detachLiveAnchor();
    state.live = null;
  }

  function sourceRequestOptions() {
    return { startLine: state.windowStartLine, count: state.largeMode ? 600 : null };
  }

  function sendEdit(baseLine, diff, source = null) {
    if (!state.doc) return;
    state.pendingEdits++;
    invalidateSearch();
    send({ cmd: "edit", docId: state.activeTabId, baseLine, ...diff, source });
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
    if (currEl.src === url && currEl.classList.contains("visible")) {
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
    if (s.viewMode !== "live") exitLiveBlockEdit();
    state.settings = s;
    const root = document.documentElement.style;

    root.setProperty("--editor-font-size", s.fontSize + "px");
    root.setProperty("--preview-max-width", s.previewMaxWidth + "px");
    root.setProperty("--line-height", String(s.lineHeight));

    document.body.classList.toggle("focus-mode", !!s.focusMode);
    document.body.classList.toggle("reduce-motion", !!s.reduceMotion);

    elLineGutter.classList.toggle("hidden-gutter", !s.showLineNumbers);
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
    }

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
      state.previewNeedsRefresh = true;
      requestPreviewBlocks();
    }
    renderTabs();
  }

  function renderTabs() {
    for (const id of ["btn-save", "btn-save-as", "btn-export", "btn-print", "menu-close-tab", "menu-find", "menu-replace"]) {
      $(id).disabled = !state.activeTabId;
    }
    elTabBar.innerHTML = "";
    for (const tab of state.tabs) {
      const el = document.createElement("div");
      el.className = "doc-tab" + (tab.id === state.activeTabId ? " active" : "");
      el.title = tab.path || tab.title;
      el.onclick = () => {
        if (tab.id !== state.activeTabId) {
          exitLiveBlockEdit();
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
    add.title = t("newTabTitle");
    add.onclick = () => {
      closeSettings();
      send({ cmd: "newTab" });
    };
    elTabBar.appendChild(add);
  }

  function renderOutline() {
    elOutlineList.innerHTML = "";
    if (!state.outline || state.outline.length === 0) {
      elOutlineList.innerHTML = `<div class="sidebar-empty">${t("outlineEmpty")}</div>`;
      return;
    }
    for (const item of state.outline) {
      const btn = document.createElement("button");
      btn.className = `outline-item lvl-${item.level}`;
      btn.textContent = item.text || "(Untitled)";
      btn.dataset.line = String(item.line);
      btn.onclick = () => jumpToLine(item.line);
      elOutlineList.appendChild(btn);
    }
  }

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
        row.style.paddingLeft = `${8 + depth * 12}px`;
        row.textContent = `▾ ${node.name}`;
        elFileTree.appendChild(row);
        for (const child of children) {
          renderNode(child, depth + 1);
        }
      } else {
        if (query && !node.name.toLowerCase().includes(query)) return;
        const row = document.createElement("button");
        row.className = "file-row" + (activePath === node.path ? " active" : "");
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
    $("st-read").textContent = t("stRead", { n: stats.readingMinutes });
    $("st-large-badge").classList.toggle("hidden", !stats.largeMode);
  }

  function loadDocumentSnapshot(doc) {
    exitLiveBlockEdit();
    state.doc = doc;
    state.sourceRequest = null;
    state.pendingSourceEdit = false;
    state.pendingEdits = 0;
    state.blockRequest = null;
    state.previewNeedsRefresh = false;
    state.previewStart = 0;
    state.previewTarget = null;
    elSourceEditor.readOnly = false;
    invalidateSearch();
    if (!doc) {
      elEmptyView.classList.remove("hidden");
      elEditorView.classList.add("hidden");
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
    elSourceEditor.setSelectionRange(0, 0);
    setSyncedScroll(elSourceEditor, 0);
    setSyncedScroll(elPreviewPane, 0);
    replacePreviewBlocks(doc.blocks || []);
    renderOutline();
    updateStatsUI(doc.stats);
    updateLargeBar();
    renderFileTree();
    updateCursorStatus();
  }

  function applySourceWindow(source) {
    state.windowStartLine = source.startLine;
    state.windowEndLine = source.endLine;
    state.totalLines = source.totalLines;
    state.sourceContext = source.contextLine ? window.MTSourceHighlight.context(source.contextLine) : "";
    state.prevText = source.text;
    if (elSourceEditor.value !== source.text) {
      const position = Math.min(elSourceEditor.selectionStart, source.text.length);
      elSourceEditor.value = source.text;
      elSourceEditor.setSelectionRange(position, position);
    }
    updateSourceOverlayAndGutter();
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
  function updateCursorStatus(allowTypewriter = true) {
    const pos = elSourceEditor.selectionStart || 0;
    const before = elSourceEditor.value.slice(0, pos);
    const lines = before.split("\n");
    const relLine = lines.length - 1;
    const col = utf16ToScalarCount(lines.at(-1), lines.at(-1).length) + 1;
    state.activeLine = state.windowStartLine + relLine;
    $("st-cursor").textContent = t("stCursor", { ln: state.activeLine + 1, col });
    updateActiveBlockHighlight();
    if (allowTypewriter && state.settings?.typewriterMode && state.settings.viewMode !== "live") {
      elSourceEditor.scrollTop = Math.max(0, sourceLineTop(relLine) - elSourceEditor.clientHeight * 0.42);
    }
  }

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
    const index = indexAtOrBefore(state.outline.length, (i) => state.outline[i].line <= line);
    const item = elOutlineList.children[index];
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

  function sourceLineTop(index) {
    const lines = elSourceHighlight.children;
    const first = lines[0]?.offsetTop || 0;
    if (index < lines.length) return (lines[index]?.offsetTop || first) - first;
    const last = lines[lines.length - 1];
    return last ? last.offsetTop + last.offsetHeight - first : 0;
  }

  function sourceLineAtPixel(pixel) {
    const length = elSourceHighlight.children.length;
    const origin = elSourceHighlight.children[0]?.offsetTop || 0;
    const index = Math.max(0, indexAtOrBefore(length, (i) => elSourceHighlight.children[i].offsetTop - origin <= pixel));
    const top = sourceLineTop(index);
    const height = Math.max(1, sourceLineTop(index + 1) - top);
    return state.windowStartLine + index + Math.min(1, Math.max(0, (pixel - top) / height));
  }

  function sourcePixelAtLine(line) {
    const index = Math.max(0, line - state.windowStartLine);
    const floor = Math.floor(index);
    const top = sourceLineTop(floor);
    return top + (sourceLineTop(floor + 1) - top) * (index - floor);
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

  function mirrorSourceScroll() {
    elSourceHighlight.scrollTop = elSourceEditor.scrollTop;
    elSourceHighlight.scrollLeft = elSourceEditor.scrollLeft;
    elLineGutter.scrollTop = elSourceEditor.scrollTop;
  }

  function syncScrollFrom(element) {
    const expected = expectedScroll.get(element);
    expectedScroll.delete(element);
    if (expected !== undefined && Math.abs(element.scrollTop - expected) < 1) return;
    if (!state.doc || !state.settings?.syncScroll || state.settings.viewMode !== "split") return;
    scrollSource = element;
    if (scrollFrame) return;
    scrollFrame = requestAnimationFrame(() => {
      scrollFrame = 0;
      if (!state.doc || !state.settings?.syncScroll || state.settings.viewMode !== "split") return;
      if (scrollSource === elSourceEditor) {
        const line = sourceLineAtPixel(elSourceEditor.scrollTop);
        showPreviewLine(line);
      } else {
        const line = translatePreview(elPreviewPane.scrollTop, "y", "line");
        if (line >= state.windowStartLine && line < state.windowEndLine) {
          setSyncedScroll(elSourceEditor, sourcePixelAtLine(line));
        }
      }
    });
  }

  elSourceEditor.addEventListener("scroll", () => {
    mirrorSourceScroll();
    syncScrollFrom(elSourceEditor);
  });
  elPreviewPane.addEventListener("scroll", () => {
    if (elPreviewPane.scrollTop + elPreviewPane.clientHeight >= elPreviewPane.scrollHeight - 400) {
      requestPreviewBlocks();
    }
    syncScrollFrom(elPreviewPane);
  });

  function scalarToUtf16(text, count) {
    let units = 0;
    for (const char of text) {
      if (count-- <= 0) break;
      units += char.length;
    }
    return units;
  }

  function sourceOffset(line, col) {
    const text = elSourceEditor.value;
    let start = 0;
    for (let i = state.windowStartLine; i < line; i++) {
      const newline = text.indexOf("\n", start);
      if (newline < 0) return text.length;
      start = newline + 1;
    }
    const end = text.indexOf("\n", start);
    return start + scalarToUtf16(text.slice(start, end < 0 ? text.length : end), col);
  }

  function applySourceSelection(target) {
    const start = sourceOffset(target.line, target.col || 0);
    const end = sourceOffset(target.endLine ?? target.line, target.endCol ?? target.col ?? 0);
    if (target.focus && state.settings?.viewMode !== "live") elSourceEditor.focus({ preventScroll: true });
    elSourceEditor.setSelectionRange(start, end);
    setSyncedScroll(elSourceEditor, Math.max(0, sourcePixelAtLine(target.line) - 36));
    state.activeLine = target.line;
    updateCursorStatus(false);
  }

  function requestSourceWindow(startLine, target = null) {
    if (!state.doc) return;
    const request = ++requestSequence;
    state.sourceRequest = { request, target };
    elSourceEditor.readOnly = true;
    send({ cmd: "fetchLines", docId: state.activeTabId, startLine, count: 600, request });
  }

  function jumpToLine(line, match = null, focus = true) {
    if (!state.doc) return;
    exitLiveBlockEdit();
    line = Math.min(state.totalLines - 1, Math.max(0, Math.trunc(line)));
    const target = { ...match, line, focus };
    if (state.largeMode && (line < state.windowStartLine || line >= state.windowEndLine)) {
      requestSourceWindow(Math.max(0, line - 40), target);
    } else {
      applySourceSelection(target);
    }
    showPreviewLine(line);
    updateActiveBlockHighlight();
  }

  elSourceEditor.addEventListener("input", () => {
    const next = elSourceEditor.value;
    const diff = computeMinimalDiff(state.prevText, next);
    state.prevText = next;
    state.windowEndLine = state.windowStartLine + next.split("\n").length;
    if (diff) sendEdit(state.windowStartLine, diff);
    updateSourceOverlayAndGutter();
    updateCursorStatus();
  });

  elSourceEditor.addEventListener("keyup", () => updateCursorStatus());
  elSourceEditor.addEventListener("click", () => updateCursorStatus());

  // ===== Smart Typora Editing Helpers (Auto-Pair, Smart Lists, Tab, URL Paste) =====
  const PAIRS = { "(": ")", "[": "]", "{": "}", '"': '"', "`": "`", "$": "$" };

  function replaceEditorSelection(newText, selStartOffset, selEndOffset) {
    const s = elSourceEditor.selectionStart;
    const e = elSourceEditor.selectionEnd;
    elSourceEditor.setRangeText(newText, s, e, "end");
    if (selStartOffset !== undefined) {
      elSourceEditor.selectionStart = s + selStartOffset;
      elSourceEditor.selectionEnd = s + (selEndOffset ?? selStartOffset);
    }
    elSourceEditor.dispatchEvent(new Event("input"));
  }

  elSourceEditor.addEventListener("keydown", (e) => {
    if (e.isComposing || elSourceEditor.readOnly) return;
    const s = elSourceEditor.selectionStart;
    const end = elSourceEditor.selectionEnd;
    const val = elSourceEditor.value;

    // Tab / Shift+Tab indentation
    if (e.key === "Tab") {
      e.preventDefault();
      if (s === end && !e.shiftKey) {
        replaceEditorSelection("  ", 2, 2);
      } else {
        const lineStart = val.lastIndexOf("\n", s - 1) + 1;
        const selected = val.slice(lineStart, end);
        const updated = e.shiftKey
          ? selected.replace(/^  /gm, "")
          : selected.replace(/^/gm, "  ");
        elSourceEditor.setRangeText(updated, lineStart, end, "select");
        elSourceEditor.dispatchEvent(new Event("input"));
      }
      return;
    }

    // Auto-pair brackets / quotes / $
    if (!e.ctrlKey && !e.altKey && !e.metaKey && PAIRS[e.key]) {
      const close = PAIRS[e.key];
      if (s !== end) {
        e.preventDefault();
        const sel = val.slice(s, end);
        replaceEditorSelection(e.key + sel + close, 1, 1 + sel.length);
        return;
      }
      // Skip over existing closing char
      if (e.key === close && val[s] === close) {
        e.preventDefault();
        elSourceEditor.selectionStart = elSourceEditor.selectionEnd = s + 1;
        return;
      }
    }

    // Smart Enter for lists & blockquotes
    if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.altKey) {
      const lineStart = val.lastIndexOf("\n", s - 1) + 1;
      const curLine = val.slice(lineStart, s);
      const taskM = curLine.match(/^(\s*[-*+]\s+)\[[ xX]\]\s+(.*)$/);
      const ulM = curLine.match(/^(\s*[-*+]\s+)(.*)$/);
      const olM = curLine.match(/^(\s*)(\d+)(\.\s+)(.*)$/);
      const qtM = curLine.match(/^(\s*>\s+)(.*)$/);

      if (taskM) {
        e.preventDefault();
        if (!taskM[2].trim()) {
          elSourceEditor.setRangeText("\n", lineStart, s, "end");
        } else {
          replaceEditorSelection("\n" + taskM[1] + "[ ] ");
        }
        elSourceEditor.dispatchEvent(new Event("input"));
      } else if (ulM) {
        e.preventDefault();
        if (!ulM[2].trim()) {
          elSourceEditor.setRangeText("\n", lineStart, s, "end");
        } else {
          replaceEditorSelection("\n" + ulM[1]);
        }
        elSourceEditor.dispatchEvent(new Event("input"));
      } else if (olM) {
        e.preventDefault();
        if (!olM[4].trim()) {
          elSourceEditor.setRangeText("\n", lineStart, s, "end");
        } else {
          replaceEditorSelection(`\n${olM[1]}${Number(olM[2]) + 1}${olM[3]}`);
        }
        elSourceEditor.dispatchEvent(new Event("input"));
      } else if (qtM && qtM[2].trim()) {
        e.preventDefault();
        replaceEditorSelection("\n" + qtM[1]);
      }
    }
  });

  // Paste URL over selected text -> [text](url)
  elSourceEditor.addEventListener("paste", (e) => {
    const s = elSourceEditor.selectionStart;
    const end = elSourceEditor.selectionEnd;
    if (s === end) return;
    const clip = (e.clipboardData || window.clipboardData).getData("text");
    if (/^https?:\/\/\S+$/.test(clip.trim())) {
      e.preventDefault();
      const sel = elSourceEditor.value.slice(s, end);
      replaceEditorSelection(`[${sel}](${clip.trim()})`);
    }
  });

  // ===== Formatting Commands =====
  function applyFormat(fmt) {
    if (state.settings && state.settings.viewMode === "live") {
      // Focus active block or switch to split if none
      const activeTa = elPreviewDoc.querySelector(".mt-live-textarea");
      if (activeTa) {
        wrapTextareaFormat(activeTa, fmt);
        return;
      }
    }
    elSourceEditor.focus();
    wrapTextareaFormat(elSourceEditor, fmt);
  }

  function wrapTextareaFormat(ta, fmt) {
    const s = ta.selectionStart;
    const e = ta.selectionEnd;
    const sel = ta.value.slice(s, e);
    const wrap = (l, r, fallback) => {
      const inner = sel || fallback;
      ta.setRangeText(l + inner + r, s, e, "end");
      if (!sel) {
        ta.selectionStart = s + l.length;
        ta.selectionEnd = s + l.length + inner.length;
      }
      ta.dispatchEvent(new Event("input"));
    };
    const prefixLine = (prefix) => {
      const lineStart = ta.value.lastIndexOf("\n", s - 1) + 1;
      ta.setRangeText(prefix, lineStart, lineStart, "end");
      ta.dispatchEvent(new Event("input"));
    };

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

    // 4. Block click -> Live Mode in-place edit OR Split Mode cursor sync
    const blockEl = e.target.closest(".mt-block, .mt-live-empty");
    if (!blockEl) return;
    if (state.settings && state.settings.viewMode === "live") {
      if (!e.target.closest(".mt-live-editor-wrap") && !e.target.closest("a")) {
        enterLiveBlockEdit(blockEl, true);
      }
    } else if (state.settings?.viewMode === "split" && blockEl.dataset.startLine !== undefined) {
      const startLine = Number(blockEl.dataset.startLine);
      state.activeLine = startLine;
      updateActiveBlockHighlight();
    }
  });

  document.addEventListener("mousedown", (e) => {
    if (state.live && !state.live.wrap.contains(e.target) && !e.target.closest(".format-bar")) {
      exitLiveBlockEdit();
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
    exitLiveBlockEdit();
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
    clearTimeout(state.searchTimer);
    elFindBar.classList.add("hidden");
  }

  function invalidateSearch() {
    state.searchVersion = null;
    state.findMatches = [];
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
    elSourceEditor.readOnly = true;
    exitLiveBlockEdit();
    return true;
  }

  function endSourceAction() {
    state.pendingSourceEdit = false;
    elSourceEditor.readOnly = Boolean(state.sourceRequest);
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
    if (state.doc) send({ cmd: "print", docId: state.activeTabId });
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
    const images = Array.from(paper.querySelectorAll("img"));
    for (const image of images) image.loading = "eager";
    let timeout;
    await Promise.race([
      Promise.allSettled([document.fonts.ready, ...images.map((image) => image.decode())]),
      new Promise((resolve) => { timeout = setTimeout(resolve, 5000); }),
    ]);
    clearTimeout(timeout);
    window.print();
  }

  // Menu and settings actions.
  $("btn-sidebar").onclick = () => send({ cmd: "updateSettings", patch: { sidebarOpen: !state.settings?.sidebarOpen } });
  $("btn-new").onclick = () => { closeSettings(); send({ cmd: "newTab" }); };
  $("btn-open").onclick = () => { closeSettings(); send({ cmd: "openFileDialog" }); };
  $("menu-open-folder").onclick = () => { closeSettings(); send({ cmd: "openFolderDialog" }); };
  $("btn-save").onclick = () => send({ cmd: "save", saveAs: false });
  $("btn-save-as").onclick = () => send({ cmd: "save", saveAs: true });
  $("btn-export").onclick = () => send({ cmd: "exportHtml" });
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
      send({ cmd: "createFileInFolder", name });
      $("new-file-input").value = "";
      $("new-file-inline").classList.add("hidden");
    }
  };
  $("new-file-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") $("new-file-confirm").click();
    if (e.key === "Escape") $("new-file-inline").classList.add("hidden");
  });
  elFileFilter.addEventListener("input", renderFileTree);

  // ===== Frameless Window Chrome Controls, Drag & Edge Resize =====
  const elTopbar = $("topbar");
  elTopbar.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    if (e.target.closest("button, input, .doc-tab, .mode-switch")) return;
    e.preventDefault();
    send({ cmd: "windowDrag" });
  });
  elTopbar.addEventListener("dblclick", (e) => {
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
    const mod = e.ctrlKey || e.metaKey;

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
    if (k === ",") { e.preventDefault(); state.settingsOpen ? closeSettings() : openSettings(); }
    else if (k === "s") { e.preventDefault(); send({ cmd: "save", saveAs: e.shiftKey }); }
    else if (k === "o") { e.preventDefault(); send({ cmd: "openFileDialog" }); }
    else if (k === "n") { e.preventDefault(); closeSettings(); send({ cmd: "newTab" }); }
    else if (k === "f") { e.preventDefault(); openFind(false); }
    else if (k === "h") { e.preventDefault(); openFind(true); }
    else if (k === "p") { e.preventDefault(); requestPrint(); }
    else if (k === "\\") { e.preventDefault(); send({ cmd: "updateSettings", patch: { viewMode: "split" } }); }
    else if (e.code === "Slash" || k === "/") {
      e.preventDefault();
      const target = e.shiftKey ? "source" : state.settings?.viewMode === "live" ? "split" : "live";
      send({ cmd: "updateSettings", patch: { viewMode: target } });
    }
    else if (k === "l" && e.shiftKey) { e.preventDefault(); $("btn-sidebar").click(); }
    else if (k === "b") { e.preventDefault(); applyFormat("bold"); }
    else if (k === "i") { e.preventDefault(); applyFormat("italic"); }
    else if (k === "k" && e.shiftKey) { e.preventDefault(); applyFormat("code-block"); }
    else if (k === "k") { e.preventDefault(); applyFormat("link"); }
    else if (k === "`") { e.preventDefault(); applyFormat("code"); }
    else if (k === "m" && e.shiftKey) { e.preventDefault(); applyFormat("math-block"); }
    else if (k === "m") { e.preventDefault(); applyFormat("math-inline"); }
    else if (k === "t") { e.preventDefault(); applyFormat("table"); }
    else if (k >= "1" && k <= "6") { e.preventDefault(); applyFormat("h" + k); }
    else if (k === "=" || k === "+") { e.preventDefault(); $("font-plus").click(); }
    else if (k === "-") { e.preventDefault(); $("font-minus").click(); }
    else if (k === "0") { e.preventDefault(); $("font-reset").click(); }
  });

  // ===== Rust -> WebView2 Event Receiver =====
  window.__mtReceive = function (msg) {
    if (!msg || !msg.event) return;
    switch (msg.event) {
      case "init":
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
          applySourceWindow(msg.source);
        }
        if (!msg.userEdit) {
          endSourceAction();
          invalidateSearch();
        }
        applyIncrementalPatch(msg.patch);
        const tab = state.tabs.find((item) => item.id === msg.docId);
        if (tab && !tab.dirty && msg.dirty) {
          tab.dirty = true;
          renderTabs();
        }
        break;
      }

      case "resyncSource":
        if (msg.docId === state.activeTabId) {
          endSourceAction();
          invalidateSearch();
        }
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
        elSourceEditor.readOnly = state.pendingSourceEdit;
        applySourceWindow(msg.source);
        if (target) applySourceSelection(target);
        else {
          elSourceEditor.setSelectionRange(0, 0);
          setSyncedScroll(elSourceEditor, 0);
          updateCursorStatus(false);
        }
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
        if (msg.toast) showToast(msg.toast);
        break;

      case "tabsUpdated":
        applyTabSummaries(msg.tabs);
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

      case "searchResults":
        if (state.findOpen && msg.docId === state.activeTabId && msg.request === state.searchRequest &&
            msg.version === state.doc?.version && !state.pendingEdits) {
          state.searchVersion = msg.version;
          state.findMatches = msg.matches;
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
          const liveEditor = state.live?.editor;
          if (liveEditor) {
            liveEditor.setRangeText(image, liveEditor.selectionStart, liveEditor.selectionEnd, "end");
            liveEditor.dispatchEvent(new Event("input"));
          } else {
            replaceEditorSelection(image);
          }
        }
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
