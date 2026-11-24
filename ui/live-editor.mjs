import { Annotation, Compartment, EditorState, RangeSet, StateEffect, StateField } from "@codemirror/state";
import { Decoration, EditorView, GutterMarker, ViewPlugin, WidgetType, drawSelection, gutterLineClass, keymap, lineNumbers } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab, redo, undo } from "@codemirror/commands";
import { HighlightStyle, LanguageDescription, LanguageSupport, StreamLanguage, foldGutter, foldKeymap, syntaxHighlighting, syntaxTree } from "@codemirror/language";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { markdown } from "@codemirror/lang-markdown";
import { GFM } from "@lezer/markdown";
import { tags } from "@lezer/highlight";
import { createWidgetRenderer, enableSourceReveal, RenderedMarkdown } from "./live-widgets.mjs";

const highlight = HighlightStyle.define([
  { tag: tags.heading, class: "hl-md-heading" },
  { tag: [tags.emphasis, tags.strong, tags.strikethrough], class: "hl-md-emphasis" },
  { tag: tags.monospace, class: "hl-md-code" },
  { tag: tags.keyword, class: "hl-kw" },
  { tag: [tags.string, tags.regexp], class: "hl-str" },
  { tag: [tags.number, tags.atom, tags.bool], class: "hl-num" },
  { tag: tags.comment, class: "hl-cmt" },
  { tag: [tags.typeName, tags.namespace, tags.className], class: "hl-ty" },
  { tag: [tags.function(tags.variableName), tags.definition(tags.variableName)], class: "hl-fn" },
  { tag: tags.macroName, class: "hl-mac" },
  { tag: [tags.processingInstruction, tags.propertyName, tags.url], class: "hl-key" },
]);

function displayDelimiter(text) {
  if (text.startsWith("$$")) return { open: "$$", close: "$$" };
  if (text.startsWith("\\[")) return { open: "\\[", close: "\\]" };
  const environment = /^\\begin\{([a-zA-Z*]+)\}/.exec(text);
  return environment && { open: environment[0], close: "\\end{" + environment[1] + "}", unwrap: false };
}

function mathText(text, display) {
  if (display) {
    const source = text.trim();
    const delimiter = displayDelimiter(source);
    if (!delimiter || !source.endsWith(delimiter.close) ||
        source.length < delimiter.open.length + delimiter.close.length) return null;
    return delimiter.unwrap === false ? source : source.slice(delimiter.open.length, -delimiter.close.length).trim();
  }
  return text.startsWith("\\(") ? text.slice(2, -2) : text.slice(1, -1);
}

const math = {
  defineNodes: [{ name: "DisplayMath", block: true }, "InlineMath"],
  parseInline: [{
    name: "InlineMath",
    before: "Escape",
    parse(cx, next, pos) {
      const paren = next === 92 && cx.char(pos + 1) === 40;
      if (!paren && (next !== 36 || [9, 10, 32, 36].includes(cx.char(pos + 1)))) return -1;
      const from = pos + (paren ? 2 : 1);
      for (let end = from; end < cx.end && cx.char(end) !== 10; end++) {
        const ch = cx.char(end);
        if (paren && ch === 92 && cx.char(end + 1) === 41 && end > from) {
          return cx.addElement(cx.elt("InlineMath", pos, end + 2));
        }
        if (!paren && ch === 36 && end > from && ![9, 32].includes(cx.char(end - 1))) {
          return cx.addElement(cx.elt("InlineMath", pos, end + 1));
        }
        if (ch === 92) end++;
      }
      return -1;
    },
  }],
  parseBlock: [{
    name: "DisplayMath",
    before: "FencedCode",
    parse(cx, line) {
      if (cx.depth !== 1) return false;
      const first = line.text.slice(line.pos).trim();
      const delimiter = displayDelimiter(first);
      if (!delimiter) return false;
      const start = cx.lineStart + line.pos;
      if (first.length >= delimiter.open.length + delimiter.close.length && first.endsWith(delimiter.close)) {
        cx.nextLine();
      } else {
        while (cx.nextLine()) {
          if (line.text.trimEnd().endsWith(delimiter.close)) {
            cx.nextLine();
            break;
          }
        }
      }
      cx.addElement(cx.elt("DisplayMath", start, cx.prevLineEnd()));
      return true;
    },
  }],
};

function sameNumbers(first, second) {
  return first.length === second.length &&
    first.every((number, index) => number === second[index]);
}

class Formula extends WidgetType {
  constructor(tex, display, render) {
    super();
    this.tex = tex;
    this.display = display;
    this.render = render;
  }
  eq(other) { return this.tex === other.tex && this.display === other.display; }
  toDOM(view) {
    const element = view.dom.ownerDocument.createElement(this.display ? "div" : "span");
    element.className = this.display ? "cm-live-math-block" : "cm-live-math-inline";
    element.innerHTML = this.render(this.tex, this.display);
    enableSourceReveal(view, element);
    return element;
  }
  get estimatedHeight() { return this.display ? 52 : -1; }
}

const headingLineNumber = new class extends GutterMarker {
  elementClass = "cm-heading-line-number";
}();

class Task extends WidgetType {
  constructor(from, to, checked) {
    super();
    Object.assign(this, { from, to, checked });
  }
  eq(other) { return this.from === other.from && this.to === other.to && this.checked === other.checked; }
  toDOM(view) {
    const box = view.dom.ownerDocument.createElement("input");
    box.type = "checkbox";
    box.checked = this.checked;
    box.className = "mt-task-cb";
    box.addEventListener("change", () => {
      if (view.state.readOnly) {
        box.checked = this.checked;
        return;
      }
      view.dispatch({ changes: { from: this.from, to: this.to, insert: box.checked ? "[x]" : "[ ]" },
        userEvent: "input" });
    });
    return box;
  }
}

const hiddenMarker = Decoration.replace({});
const styles = new Map();
function mark(name) {
  if (!styles.has(name)) styles.set(name, Decoration.mark({ class: "cm-live-" + name }));
  return styles.get(name);
}

const marks = {
  Emphasis: "em", StrongEmphasis: "strong", Strikethrough: "strike",
  InlineCode: "code", FencedCode: "fence", Blockquote: "quote",
};
const punctuation = new Set(["HeaderMark", "EmphasisMark", "StrikethroughMark", "CodeMark", "QuoteMark"]);
const headingNode = /^(?:ATX|Setext)Heading([1-6])$/;
const alertMarker = /^>[ \t]*\[!(?:NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]/;
const refreshWidgets = StateEffect.define();

function blockKind(state, node) {
  if (node.name === "Table") return "table";
  if (node.name === "FencedCode") {
    const info = node.getChild("CodeInfo");
    if (info && state.sliceDoc(info.from, info.to).trim().split(/\s/, 1)[0].toLowerCase() === "mermaid") return "diagram";
  }
  if (node.name === "Blockquote" && alertMarker.test(state.sliceDoc(node.from, state.doc.lineAt(node.from).to))) {
    return "callout";
  }
  if (node.name === "Paragraph") {
    const child = node.firstChild;
    if (child?.name === "Image" && child.from === node.from && child.to === node.to) return "image";
  }
  return null;
}

function linkLabel(state, node) {
  const markers = node.getChildren("LinkMark");
  return markers.length < 2 ? "" : state.sliceDoc(markers[0].to, markers[1].from);
}

function touches(selection, from, to) {
  return selection.ranges.some((range) => range.from <= to && range.to >= from);
}

// The syntax tree is visited only when text, parser output, or the visible ranges change.
function collectInline(view, render, renderer, blocks) {
  const fixed = [];
  const marksToHide = [];
  const seen = new Set();
  const { state } = view;
  const tree = syntaxTree(state);

  function hide(owner, from, to, decoration) {
    const key = from + ":" + to;
    if (seen.has(key)) return;
    seen.add(key);
    marksToHide.push({ from: owner.from, to: owner.to, range: decoration.range(from, to) });
  }

  for (const range of view.visibleRanges) {
    tree.iterate({
      from: range.from, to: range.to,
      enter(node) {
        const { name, from, to } = node;
        if (to <= from || to <= range.from || from >= range.to) return;
        const syntax = node.node;
        if (name === "DisplayMath" || (syntax.parent?.name === "Document" &&
          blocks.starts.has(from) && !blocks.open.has(from))) return false;
        if (name === "InlineMath") {
          hide(syntax, from, to, Decoration.replace({
            widget: new Formula(mathText(state.sliceDoc(from, to), false), false, render),
          }));
          return false;
        }
        if (name === "TaskMarker") {
          hide(syntax, from, to, Decoration.replace({
            widget: new Task(from, to, state.sliceDoc(from, to) !== "[ ]"),
          }));
          return false;
        }
        const explicitLink = name === "Link" && (syntax.getChild("URL") || syntax.getChild("LinkLabel"));
        if ((name === "Image" || explicitLink) && to <= state.doc.lineAt(from).to) {
          const kind = name === "Image" ? "image" : "link";
          hide(syntax, from, to, Decoration.replace({
            widget: new RenderedMarkdown(state.sliceDoc(from, to), kind, false, renderer, linkLabel(state, syntax)),
          }));
          return false;
        }
        if (punctuation.has(name) && !(name === "CodeMark" && syntax.parent?.name === "FencedCode")) {
          const line = state.doc.lineAt(to);
          const trailing = name === "HeaderMark" || name === "QuoteMark"
            ? /^[ \t]*/.exec(line.text.slice(to - line.from))[0].length : 0;
          const owner = name === "QuoteMark" ? syntax : syntax.parent ?? syntax;
          hide(owner, from, to + trailing, hiddenMarker);
          return false;
        }
        const heading = headingNode.exec(name);
        const style = heading ? "heading-" + heading[1] : marks[name];
        if (style) fixed.push(mark(style).range(Math.max(from, range.from), Math.min(to, range.to)));
      },
    });
  }
  return { fixed, marksToHide };
}

function layoutInline({ fixed, marksToHide }, selection, previous) {
  const open = [];
  for (let i = 0; i < marksToHide.length; i++) {
    const { from, to } = marksToHide[i];
    if (touches(selection, from, to)) open.push(i);
  }
  if (previous && sameNumbers(open, previous.open)) return previous;
  const opened = new Set(open);
  const decorations = fixed.concat(marksToHide.filter((_, i) => !opened.has(i)).map(({ range }) => range));
  return { open, decorations: Decoration.set(decorations, true) };
}

function inlinePreview(render, renderer, blockField) {
  return ViewPlugin.fromClass(class {
    constructor(view) {
      this.rebuild(view);
    }
    rebuild(view) {
      this.candidates = collectInline(view, render, renderer, view.state.field(blockField));
      this.layout = layoutInline(this.candidates, view.state.selection);
    }
    update(update) {
      if (update.docChanged || update.viewportChanged ||
          syntaxTree(update.state) !== syntaxTree(update.startState) ||
          update.state.field(blockField).ranges !== update.startState.field(blockField).ranges ||
          update.transactions.some((tx) => tx.effects.some((effect) => effect.is(refreshWidgets)))) {
        this.rebuild(update.view);
      } else if (update.selectionSet) {
        this.layout = layoutInline(this.candidates, update.state.selection, this.layout);
      }
    }
  }, { decorations: (plugin) => plugin.layout.decorations });
}

function blockWidgets(render, renderer) {
  function collectBlocks(state, tree) {
    const ranges = [];
    const headings = [];
    const starts = new Set();
    tree.iterate({
      enter(ref) {
        if (ref.name === "Document") return;
        const node = ref.node;
        let widget;
        if (node.name === "DisplayMath") {
          const tex = mathText(state.sliceDoc(node.from, node.to), true);
          if (tex) widget = new Formula(tex, true, render);
        } else if (headingNode.test(node.name)) {
          headings.push(headingLineNumber.range(state.doc.lineAt(node.from).from));
        } else {
          const kind = blockKind(state, node);
          if (kind) {
            const source = state.sliceDoc(node.from, node.to);
            const label = kind === "image" ? linkLabel(state, node.firstChild) : "";
            const tableRows = kind === "table" ? node.getChildren("TableRow").length + 1 : 0;
            widget = new RenderedMarkdown(source, kind, true, renderer, label, tableRows);
          }
        }
        if (widget) {
          starts.add(node.from);
          ranges.push(Decoration.replace({ block: true, widget }).range(node.from, node.to));
        }
        return false;
      },
    });
    return { tree, widgets: Decoration.set(ranges), headings: RangeSet.of(headings), starts };
  }
  function layout(blocks, selection, previous) {
    const open = new Set();
    for (const range of selection.ranges) {
      blocks.widgets.between(range.from, range.to, (from) => { open.add(from); });
    }
    const openStarts = [...open].sort((a, b) => a - b);
    if (previous && sameNumbers(openStarts, previous.openStarts)) return previous;
    const ranges = open.size ? blocks.widgets.update({
      filterFrom: openStarts[0], filterTo: openStarts.at(-1),
      filter: (from) => !open.has(from),
    }) : blocks.widgets;
    return { ...blocks, openStarts, open, ranges };
  }
  return StateField.define({
    create(state) {
      const tree = syntaxTree(state);
      return layout(collectBlocks(state, tree), state.selection);
    },
    update(previous, transaction) {
      const { state } = transaction;
      const tree = syntaxTree(state);
      if (transaction.docChanged || tree !== previous.tree ||
          transaction.effects.some((effect) => effect.is(refreshWidgets))) {
        return layout(collectBlocks(state, tree), state.selection);
      }
      return transaction.selection ? layout(previous, state.selection, previous) : previous;
    },
    provide: (field) => [
      EditorView.decorations.from(field, (value) => value.ranges),
      gutterLineClass.from(field, (value) => value.headings),
    ],
  });
}

const codeLanguages = [
  { name: "Rust", alias: ["rs"], load: () => import("@codemirror/lang-rust").then(({ rust }) => rust()) },
  { name: "JavaScript", alias: ["js", "mjs", "cjs"], load: () => import("@codemirror/lang-javascript").then(({ javascript }) => javascript()) },
  { name: "TypeScript", alias: ["ts", "mts", "cts"], load: () => import("@codemirror/lang-javascript").then(({ javascript }) => javascript({ typescript: true })) },
  { name: "Python", alias: ["py", "python3"], load: () => import("@codemirror/lang-python").then(({ python }) => python()) },
  { name: "C++", alias: ["cpp", "cxx", "cc", "c", "h", "hpp"], load: () => import("@codemirror/lang-cpp").then(({ cpp }) => cpp()) },
  { name: "Go", alias: ["golang"], load: () => import("@codemirror/lang-go").then(({ go }) => go()) },
  { name: "Bash", alias: ["sh", "shell", "zsh"], load: () => import("@codemirror/legacy-modes/mode/shell").then(({ shell }) =>
    new LanguageSupport(StreamLanguage.define(shell))) },
  { name: "JSON", load: () => import("@codemirror/lang-json").then(({ json }) => json()) },
  { name: "YAML", alias: ["yml"], load: () => import("@codemirror/lang-yaml").then(({ yaml }) => yaml()) },
].map((description) => LanguageDescription.of(description));

function loadCode(info) {
  return LanguageDescription.matchLanguageName(codeLanguages, info.trim().split(/[\s,{]/, 1)[0], false) ?? null;
}

// CodeMirror renders only the visible lines. Rust keeps the canonical Rope and on-disk state.
const nativeChange = Annotation.define();
const showMatches = StateEffect.define();
const matchesField = StateField.define({
  create: () => Decoration.none,
  update(matches, transaction) {
    if (transaction.docChanged) matches = Decoration.none;
    for (const effect of transaction.effects) if (effect.is(showMatches)) matches = effect.value;
    return matches;
  },
  provide: (field) => EditorView.decorations.from(field),
});
const matchMark = Decoration.mark({ class: "cm-marktype-match" });

function create(parent, { text, mode, label, renderMath, renderWidgets, hydrateWidgets, onEdit, onCursor, onScroll, showLineNumbers, startLine }) {
  const readOnly = new Compartment();
  const gutter = new Compartment();
  const visual = new Compartment();
  const renderer = createWidgetRenderer(renderWidgets, hydrateWidgets);
  const blockField = blockWidgets(renderMath, renderer);
  const visualExtensions = [blockField, inlinePreview(renderMath, renderer, blockField)];
  let firstLine = startLine;
  let showNumbers = showLineNumbers;
  const gutterExtension = () => showNumbers ? lineNumbers({ formatNumber: (n) => String(n + firstLine) }) : [];
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: text,
      extensions: [
        markdown({ extensions: [GFM, math], codeLanguages: loadCode }),
        syntaxHighlighting(highlight),
        history(), drawSelection(), closeBrackets(), foldGutter(), matchesField,
        EditorState.allowMultipleSelections.of(true),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap.filter((binding) => binding.key !== "Mod-/"),
          ...historyKeymap, ...foldKeymap, indentWithTab]),
        EditorView.lineWrapping,
        EditorView.contentAttributes.of({ "aria-label": label, spellcheck: "false" }),
        readOnly.of(EditorState.readOnly.of(false)), gutter.of(gutterExtension()),
        visual.of(mode === "live" ? visualExtensions : []),
        EditorView.updateListener.of((update) => {
          if (update.docChanged && update.transactions.some((tx) => !tx.annotation(nativeChange))) {
            // Combine multi-cursor transactions into one UTF-16 range on the native Rope.
            let start = -1, end, insertedStart, insertedEnd;
            update.changes.iterChanges((fromA, toA, fromB, toB) => {
              if (start < 0) {
                start = fromA;
                insertedStart = fromB;
              }
              end = toA;
              insertedEnd = toB;
            });
            if (start >= 0) onEdit(start, end, update.state.sliceDoc(insertedStart, insertedEnd), update.state.doc.lines);
          }
          if (update.selectionSet || update.docChanged) {
            const head = update.state.selection.main.head;
            const line = update.state.doc.lineAt(head);
            onCursor(line.number, update.state.sliceDoc(line.from, head));
          }
        }),
      ],
    }),
  });
  view.scrollDOM.addEventListener("scroll", () => onScroll(view.scrollDOM));
  function offset(line, column) {
    const { from, text } = view.state.doc.line(Math.max(1, Math.min(line + 1, view.state.doc.lines)));
    let units = 0;
    for (const char of text) {
      if (column-- <= 0) break;
      units += char.length;
    }
    return from + units;
  }
  return {
    get text() { return view.state.doc.toString(); },
    get selection() { return view.state.selection.main; },
    get scrollElement() { return view.scrollDOM; },
    get readOnly() { return view.state.readOnly; },
    undo() { undo(view); },
    redo() { redo(view); },
    setMode(next) {
      if (mode === next) return;
      mode = next;
      view.dispatch({ effects: visual.reconfigure(mode === "live" ? visualExtensions : []), scrollIntoView: true });
    },
    applyText(text) {
      if (view.state.doc.toString() === text) return;
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, annotations: nativeChange.of(true) });
    },
    setMatches(matches, firstLine) {
      const ranges = [];
      for (const match of matches) {
        const line = match.line - firstLine;
        const end = (match.endLine ?? match.line) - firstLine;
        if (line < 0 || end >= view.state.doc.lines) continue;
        const from = offset(line, match.col);
        const to = offset(end, match.endCol);
        if (to > from) ranges.push(matchMark.range(from, to));
      }
      const decorations = Decoration.set(ranges, true);
      if (decorations.size || view.state.field(matchesField).size) view.dispatch({ effects: showMatches.of(decorations) });
    },
    lineAtScroll(pixel) {
      const block = view.lineBlockAtHeight(pixel);
      return view.state.doc.lineAt(block.from).number - 1 +
        Math.min(1, Math.max(0, (pixel - block.top) / Math.max(1, block.height)));
    },
    scrollAtLine(line) {
      const whole = Math.floor(line);
      const from = offset(whole, 0);
      const block = view.lineBlockAt(from);
      return block.top + (line - whole) * block.height;
    },
    slice(from, to) { return view.state.sliceDoc(from, to); },
    lineStart(position) { return view.state.doc.lineAt(position).from; },
    replace(from, to, insert, anchor = from + insert.length, head = anchor) {
      if (view.state.readOnly) return;
      view.dispatch({ changes: { from, to, insert }, selection: { anchor, head }, userEvent: "input" });
    },
    setReadOnly(value) {
      if (view.state.readOnly !== value) view.dispatch({ effects: readOnly.reconfigure(EditorState.readOnly.of(value)) });
    },
    configureGutter(value, start) {
      if (showNumbers === value && firstLine === start) return;
      showNumbers = value;
      firstLine = start;
      view.dispatch({ effects: gutter.reconfigure(gutterExtension()) });
    },
    select(relativeLine, col = 0, relativeEnd = relativeLine, endCol = col, focus = false) {
      view.dispatch({
        selection: { anchor: offset(relativeLine, col), head: offset(relativeEnd, endCol) },
        scrollIntoView: true,
      });
      if (focus) view.focus();
    },
    focus() { view.focus(); },
    invalidateWidgets() {
      renderer.invalidate();
      view.dispatch({ effects: refreshWidgets.of(null) });
    },
    destroy() {
      renderer.destroy();
      view.destroy();
    },
  };
}

window.MTEditor = { create };
