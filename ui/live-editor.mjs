import { Compartment, EditorState, StateField, Transaction } from "@codemirror/state";
import { Decoration, EditorView, GutterMarker, ViewPlugin, WidgetType, drawSelection, keymap, lineNumberWidgetMarker, lineNumbers } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { HighlightStyle, syntaxHighlighting, syntaxTree } from "@codemirror/language";
import { markdown } from "@codemirror/lang-markdown";
import { rust } from "@codemirror/lang-rust";
import { javascript } from "@codemirror/lang-javascript";
import { GFM } from "@lezer/markdown";
import { tags } from "@lezer/highlight";

const highlight = HighlightStyle.define([
  { tag: tags.keyword, class: "hl-kw" },
  { tag: [tags.string, tags.regexp], class: "hl-str" },
  { tag: [tags.number, tags.atom, tags.bool], class: "hl-num" },
  { tag: tags.comment, class: "hl-cmt" },
  { tag: [tags.typeName, tags.namespace, tags.className], class: "hl-ty" },
  { tag: [tags.function(tags.variableName), tags.definition(tags.variableName)], class: "hl-fn" },
  { tag: tags.macroName, class: "hl-mac" },
  { tag: [tags.processingInstruction, tags.url], class: "hl-key" },
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

function selectedLines(state) {
  return state.selection.ranges.map(({ from, to }) => [
    state.doc.lineAt(from).from,
    state.doc.lineAt(to).to,
  ]);
}

function intersects(lines, from, to) {
  return lines.some(([start, end]) => start <= to && end >= from);
}

function sameLines(first, second) {
  return first.length === second.length &&
    first.every(([start, end], i) => start === second[i][0] && end === second[i][1]);
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
    return element;
  }
  get estimatedHeight() { return this.display ? 52 : -1; }
  ignoreEvent() { return false; }
}

class FormulaLineNumber extends GutterMarker {
  constructor(number) {
    super();
    this.number = number;
  }
  elementClass = "cm-formula-line-number";
  eq(other) { return this.number === other.number; }
  toDOM(view) { return view.dom.ownerDocument.createTextNode(String(this.number)); }
}

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
  InlineCode: "code", Link: "link", FencedCode: "fence", Blockquote: "quote",
  TableHeader: "table-header", TableCell: "table-cell",
};
const punctuation = new Set(["HeaderMark", "EmphasisMark", "StrikethroughMark", "CodeMark", "QuoteMark"]);

function visibleMarkdown(view, render, lines) {
  const decorations = [];
  const { state } = view;
  const tree = syntaxTree(state);
  for (const range of view.visibleRanges) {
    tree.iterate({
      from: range.from, to: range.to,
      enter(node) {
        const { name, from, to } = node;
        if (to <= from || to <= range.from || from >= range.to) return;
        const active = intersects(lines, from, to);
        if (name === "DisplayMath") return false;
        let visible = !active;
        if (name === "CodeMark" && node.node.parent?.name === "FencedCode") {
          visible = false;
        }
        if (visible && name === "InlineMath") {
          decorations.push(Decoration.replace({ widget: new Formula(mathText(state.sliceDoc(from, to), false), false, render) }).range(from, to));
          return false;
        }
        if (visible && name === "TaskMarker") {
          decorations.push(Decoration.replace({ widget: new Task(from, to, state.sliceDoc(from, to) !== "[ ]") }).range(from, to));
          return false;
        }
        if (visible && punctuation.has(name)) {
          // Stay on this line, including when the marker is the closing part of a heading.
          const line = state.doc.lineAt(to);
          const trailingSpace = name === "HeaderMark" || name === "QuoteMark" ? /^[ \t]*/.exec(line.text.slice(to - line.from))[0].length : 0;
          decorations.push(hiddenMarker.range(from, to + trailingSpace));
          return false;
        }
        const heading = /^(?:ATX|Setext)Heading([1-6])$/.exec(name);
        const style = heading ? "heading-" + heading[1] : marks[name];
        if (style) decorations.push(mark(style).range(Math.max(from, range.from), Math.min(to, range.to)));
      },
    });
  }
  return Decoration.set(decorations, true);
}

function inlinePreview(render) {
  return ViewPlugin.fromClass(class {
    constructor(view) {
      this.lines = selectedLines(view.state);
      this.decorations = visibleMarkdown(view, render, this.lines);
    }
    update(update) {
      const lines = update.selectionSet || update.docChanged ? selectedLines(update.state) : this.lines;
      if (update.docChanged || update.viewportChanged || !sameLines(this.lines, lines) ||
          syntaxTree(update.state) !== syntaxTree(update.startState)) {
        this.decorations = visibleMarkdown(update.view, render, lines);
      }
      this.lines = lines;
    }
  }, { decorations: (plugin) => plugin.decorations });
}

function blockWidgets(render) {
  function mathRanges(state, tree) {
    const ranges = [];
    tree.iterate({
      enter(node) {
        if (node.name === "Document") return;
        if (node.name === "DisplayMath") {
          const tex = mathText(state.sliceDoc(node.from, node.to), true);
          if (tex) ranges.push(Decoration.replace({ block: true, widget: new Formula(tex, true, render) }).range(node.from, node.to));
        }
        return false;
      },
    });
    return Decoration.set(ranges);
  }
  function layout(tree, widgets, lines) {
    return { tree, widgets, lines, ranges: widgets.update({
      filter: (from, to) => !intersects(lines, from, to),
    }) };
  }
  return StateField.define({
    create(state) {
      const tree = syntaxTree(state);
      return layout(tree, mathRanges(state, tree), selectedLines(state));
    },
    update(previous, transaction) {
      const tree = syntaxTree(transaction.state);
      const lines = transaction.docChanged || transaction.selection ? selectedLines(transaction.state) : previous.lines;
      if (transaction.docChanged || tree !== previous.tree) {
        return layout(tree, mathRanges(transaction.state, tree), lines);
      }
      return sameLines(previous.lines, lines) ? previous : layout(tree, previous.widgets, lines);
    },
    provide: (field) => EditorView.decorations.from(field, (value) => value.ranges),
  });
}

const languages = new Map();
function loadCode(info) {
  const name = info.trim().split(/[\s,{]/, 1)[0].toLowerCase();
  const kind = ["rs", "rust"].includes(name) ? "rust"
    : ["js", "javascript", "mjs"].includes(name) ? "js"
      : ["ts", "typescript"].includes(name) ? "ts" : null;
  if (!kind) return null;
  if (!languages.has(kind)) {
    languages.set(kind, (kind === "rust" ? rust() : javascript({ typescript: kind === "ts" })).language);
  }
  return languages.get(kind);
}

// CodeMirror renders only the visible lines. Rust keeps the canonical Rope and on-disk state.
function create(parent, { text, label, renderMath, onEdit, onCursor, showLineNumbers, startLine }) {
  const readOnly = new Compartment();
  const gutter = new Compartment();
  let firstLine = startLine;
  let showNumbers = showLineNumbers;
  const gutterExtension = () => showNumbers ? [
    lineNumbers({ formatNumber: (n) => String(n + firstLine) }),
    lineNumberWidgetMarker.of((view, widget, block) => widget instanceof Formula
      ? new FormulaLineNumber(view.state.doc.lineAt(block.from).number + firstLine) : null),
  ] : [];
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: text,
      extensions: [
        markdown({ extensions: [GFM, math], codeLanguages: loadCode }),
        syntaxHighlighting(highlight),
        history(), keymap.of([...defaultKeymap, ...historyKeymap]), drawSelection(),
        EditorView.lineWrapping,
        EditorView.contentAttributes.of({ "aria-label": label, spellcheck: "false" }),
        readOnly.of(EditorState.readOnly.of(false)), gutter.of(gutterExtension()),
        blockWidgets(renderMath), inlinePreview(renderMath),
        EditorView.updateListener.of((update) => {
          if (update.docChanged && update.transactions.some((tx) => tx.annotation(Transaction.addToHistory) !== false)) {
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
  return {
    get text() { return view.state.doc.toString(); },
    get selection() { return view.state.selection.main; },
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
      const offset = (line, column) => {
        const { from, text } = view.state.doc.line(Math.max(1, Math.min(line + 1, view.state.doc.lines)));
        let units = 0;
        for (const char of text) {
          if (column-- <= 0) break;
          units += char.length;
        }
        return from + units;
      };
      view.dispatch({
        selection: { anchor: offset(relativeLine, col), head: offset(relativeEnd, endCol) },
        scrollIntoView: true,
      });
      if (focus) view.focus();
    },
    focus() { view.focus(); },
    destroy() { view.destroy(); },
  };
}

window.MTLiveEditor = { create };
