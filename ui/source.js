(() => {
  "use strict";

  const keywords = new Set((
    "as async await break case catch class const continue def default defer do else enum except " +
    "export extends final finally fn for from func function go if impl import in interface let " +
    "loop match mod mut namespace new package pass private pub public raise return select static " +
    "struct super switch throw trait try type typeof unsafe use using var where while with yield " +
    "SELECT FROM WHERE INSERT UPDATE DELETE CREATE INTO VALUES JOIN ON AS GROUP ORDER BY HAVING"
  ).split(/\s+/));
  const literals = new Set("false true null nil None False True NULL undefined".split(" "));
  const types = new Set("bool char int float double string str i8 i16 i32 i64 u8 u16 u32 u64 usize void".split(" "));
  const hashLanguages = /^(?:py(?:thon|3)?|sh|bash|zsh|fish|shell|ps1|powershell|toml|ya?ml|rb|ruby|ini)$/;
  const sqlLanguages = /^(?:sql|postgres|mysql|sqlite)$/;
  const prose = /^(\s{0,3}#{1,6})(?=\s|$)|^(\s*(?:[+*-]|\d{1,9}[.)])(?=\s)(?:\s+\[[ xX]\])?)|^(\s*>+)|\x60+[^\x60]+\x60+|(?<!\\)\${1,2}[^$]+\${1,2}|\\\(.+?\\\)|\[[^\]]+\]\([^)]*\)|\*\*[^*]+\*\*|__[^_]+__|\*[^*]+\*|_[^_]+_|~~[^~]+~~|==[^=]+==|~[^~\s]+~|\^[^^\s]+\^/g;
  const escape = (text) => text.replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[ch]);

  function span(kind, text) {
    return '<span class="' + kind + '">' + escape(text) + '</span>';
  }

  function highlightCode(line, language) {
    if (["text", "txt", "plain", "plaintext"].includes(language)) return escape(line);
    const comment = hashLanguages.test(language) ? "#" : sqlLanguages.test(language) ? "--" : "//";
    const tokens = /\/\*[\s\S]*?(?:\*\/|$)|\/\/.*|#.*|--.*|<!--[\s\S]*?(?:-->|$)|"(?:\\.|[^"\\])*"?|'(?:\\.|[^'\\])*'?|\x60(?:\\.|[^\x60\\])*\x60?|\b(?:0[xob][0-9a-f_]+|\d[\d_]*(?:\.\d[\d_]*)?)\b|[A-Za-z_][\w]*!?/gi;
    let html = "";
    let end = 0;
    for (const token of line.matchAll(tokens)) {
      const word = token[0];
      const next = line.slice(token.index + word.length);
      let kind;
      if (word.startsWith(comment) || word.startsWith("/*") || word.startsWith("<!--")) kind = "hl-cmt";
      else if (/^["'\x60]/.test(word)) kind = /^\s*:/.test(next) && /^(jsonc?|ya?ml)$/.test(language) ? "hl-key" : "hl-str";
      else if (/^\d/.test(word) || literals.has(word)) kind = "hl-num";
      else if (keywords.has(word) || (sqlLanguages.test(language) && keywords.has(word.toUpperCase()))) kind = "hl-kw";
      else if (word.endsWith("!") || (word.startsWith("#") && language === "rust")) kind = "hl-mac";
      else if (types.has(word) || /^[A-Z][a-z]/.test(word)) kind = "hl-ty";
      else if (/^\s*\(/.test(next)) kind = "hl-fn";
      else if (/^\s*[:=]/.test(next) && /^(ya?ml|toml|jsonc?)$/.test(language)) kind = "hl-key";
      html += escape(line.slice(end, token.index)) + (kind ? span(kind, word) : escape(word));
      end = token.index + word.length;
    }
    return html + escape(line.slice(end));
  }

  function renderLine(line, previous, first) {
    const trimmed = line.trim();
    let state = previous;
    let html;
    if (state.startsWith("code:")) {
      const [, marker, language] = state.split(":");
      const closed = trimmed.startsWith(marker) && [...trimmed].every((ch) => ch === marker[0]);
      if (closed) state = "";
      html = closed ? span("md-code", line) : highlightCode(line, language);
    } else if (state === "front" || (first && trimmed === "---")) {
      state = state && (trimmed === "---" || trimmed === "...") ? "" : "front";
      html = span("md-fm", line);
    } else if (state.startsWith("math:")) {
      if (trimmed.endsWith(state.slice(5))) state = "";
      html = span("md-math", line);
    } else {
      const fence = /^(?: {0,3})(\x60{3,}|~{3,})\s*([^:\s]*)/.exec(line);
      if (fence) {
        state = "code:" + fence[1] + ":" + fence[2].toLowerCase();
        html = span("md-code", line);
      } else if (trimmed.startsWith("\\begin{") && trimmed.includes("}")) {
        const end = "\\end{" + trimmed.slice(7, trimmed.indexOf("}")) + "}";
        if (!trimmed.includes(end)) state = "math:" + end;
        html = span("md-math", line);
      } else if (trimmed.startsWith("$$") || trimmed.startsWith("\\[")) {
        const close = trimmed[0] === "$" ? "$$" : "\\]";
        if (trimmed.length <= 2 || !trimmed.endsWith(close)) state = "math:" + close;
        html = span("md-math", line);
      } else {
        html = "";
        let end = 0;
        for (const token of line.matchAll(prose)) {
          const word = token[0];
          const kind = token[1] ? "md-h" : token[2] ? "md-list" : token[3] ? "md-quote"
            : word.startsWith("\x60") ? "md-code" : /^[$\\]/.test(word) ? "md-math"
              : word.startsWith("[") ? "md-link" : /^(?:\*\*|__|==)/.test(word) ? "md-bold" : "md-em";
          html += escape(line.slice(end, token.index)) + span(kind, word);
          end = token.index + word.length;
        }
        html += escape(line.slice(end));
        if (tokenHeading(line)) html = '<span class="md-h">' + html + '</span>';
      }
    }
    return { text: line, state, html };
  }

  function tokenHeading(line) {
    return /^ {0,3}#{1,6}(?:\s|$)/.test(line);
  }

  class SourceHighlight {
    static context(line) {
      return renderLine(line, "", line.trim() === "---").state;
    }

    constructor(container) {
      this.container = container;
      this.records = [];
      this.context = "";
      this.front = false;
    }

    update(text, context = "", documentStart = true) {
      const lines = text.split("\n");
      const existing = this.records;
      let start = 0;
      let suffix = 0;
      const overlap = Math.min(existing.length, lines.length);
      const front = documentStart && lines[0].trim() === "---" &&
        lines.slice(1, 83).some((line) => ["---", "..."].includes(line.trim()));
      const reusable = context === this.context && front === this.front;
      if (reusable) {
        while (start < overlap && existing[start].text === lines[start]) start++;
        if (start === existing.length && start === lines.length) return;
        while (suffix < overlap - start &&
               existing[existing.length - suffix - 1].text === lines[lines.length - suffix - 1]) suffix++;
      }
      this.context = context;
      this.front = front;
      const replacement = [];
      let nextState = start ? existing[start - 1].state : context;
      let newEnd = start;
      for (; newEnd < lines.length; newEnd++) {
        const oldIndex = newEnd + existing.length - lines.length;
        const oldState = oldIndex > 0 ? existing[oldIndex - 1]?.state : context;
        if (newEnd >= lines.length - suffix && nextState === oldState) break;
        const data = renderLine(lines[newEnd], nextState, front && newEnd === 0);
        nextState = data.state;
        replacement.push(data);
      }
      const oldEnd = existing.length - (lines.length - newEnd);
      const tail = this.container.children[oldEnd] || null;
      for (let i = start; i < oldEnd; i++) this.container.children[start].remove();
      const fragment = document.createDocumentFragment();
      for (const record of replacement) {
        const element = document.createElement("div");
        element.className = "source-line";
        element.innerHTML = record.html;
        fragment.append(element);
      }
      this.container.insertBefore(fragment, tail);
      this.records = existing.slice(0, start).concat(replacement, existing.slice(oldEnd));
    }
  }

  window.MTSourceHighlight = SourceHighlight;
})();
