//! Full CommonMark + GFM + Typora + LaTeX Markdown block renderer powered by `pulldown-cmark`.
//! Designed to render individual blocks in < 0.2ms for sub-10ms real-time split & live sync.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use pulldown_cmark::{
    BrokenLink, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use regex::Regex;
use unicase::UniCase;

use crate::highlight::{escape_html, escape_html_string, highlight_code};

/// Context for resolving local images and emitting line-anchored interactive elements.
pub struct RenderContext<'a> {
    pub doc_dir: Option<&'a Path>,
    pub block_start_line: usize,
    pub toc_html: Option<&'a str>,
}

#[derive(Default, PartialEq, Eq)]
pub(crate) struct References {
    links: HashMap<UniCase<String>, (String, String)>,
    footnotes: HashSet<String>,
}

fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_OLD_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
}

impl References {
    pub(crate) fn add(&mut self, source: &str) {
        let parser = Parser::new_ext(source, parser_options());
        for (label, def) in parser.reference_definitions().iter() {
            self.links
                .entry(UniCase::new(label.to_owned()))
                .or_insert_with(|| {
                    (
                        def.dest.to_string(),
                        def.title.as_deref().unwrap_or_default().to_owned(),
                    )
                });
        }
        self.footnotes
            .extend(parser.filter_map(|event| match event {
                Event::Start(Tag::FootnoteDefinition(label)) => Some(label.to_string()),
                _ => None,
            }));
    }
}

pub fn slugify(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut prev_dash = false;
    for ch in text.trim().chars() {
        if ch.is_alphanumeric() || ch >= '\u{4e00}' {
            for lc in ch.to_lowercase() {
                slug.push(lc);
            }
            prev_dash = false;
        } else if (ch.is_whitespace() || ch == '-' || ch == '_') && !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "section".to_string()
    } else {
        slug
    }
}

/// Extract plain text from a markdown heading line (stripping leading `#` and inline markup).
pub fn heading_plain_text(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let cleaned = rest
        .trim()
        .trim_end_matches('#')
        .trim()
        .replace(['*', '_', '`', '~'], "");
    Some((hashes as u8, cleaned))
}

#[derive(Clone)]
struct ProtectedSpan {
    placeholder: String,
    html: String,
    text: String,
}

/// Protect inline & display LaTeX math and Typora extensions (`==mark==`, `~sub~`, `^sup^`)
/// before `pulldown-cmark` parses the block, while strictly respecting code spans and fences.
fn preprocess_block(src: &str) -> (String, Vec<ProtectedSpan>, Vec<(usize, usize)>, String) {
    let mut out = String::with_capacity(src.len() + 64);
    let mut spans = Vec::new();
    let mut extra_lines = Vec::new();
    let mut prefix = String::from("MTMARKTYPE");
    while src.contains(&prefix) {
        prefix.push('X');
    }
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut no_close_bracket = false;
    let mut no_close_paren = false;
    let mut no_close_double_dollar = false;
    let mut unclosed_ticks = std::collections::HashSet::new();

    while i < len {
        // 1. Respect fenced code blocks untouched
        if (i == 0 || bytes[i - 1] == b'\n') && (bytes[i] == b'`' || bytes[i] == b'~') {
            let ch = bytes[i];
            let mut count = 0;
            while i + count < len && bytes[i + count] == ch {
                count += 1;
            }
            if count >= 3 {
                let start = i;
                // advance to end of opening line
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                if i < len {
                    i += 1;
                }
                // find matching closing fence
                while i < len {
                    let line_start = i;
                    let mut c2 = 0;
                    while i + c2 < len && bytes[i + c2] == ch {
                        c2 += 1;
                    }
                    while i < len && bytes[i] != b'\n' {
                        i += 1;
                    }
                    if i < len {
                        i += 1;
                    }
                    if c2 >= count && src[line_start + c2..i].trim().is_empty() {
                        break;
                    }
                }
                out.push_str(&src[start..i]);
                continue;
            }
        }

        // 2. Respect inline code spans (`...`) untouched
        if bytes[i] == b'`' {
            let start = i;
            let mut ticks = 0;
            while i + ticks < len && bytes[i + ticks] == b'`' {
                ticks += 1;
            }
            i += ticks;
            let mut closed = false;
            if !unclosed_ticks.contains(&ticks) {
                while i < len {
                    if bytes[i] == b'`' {
                        let mut t2 = 0;
                        while i + t2 < len && bytes[i + t2] == b'`' {
                            t2 += 1;
                        }
                        i += t2;
                        if t2 == ticks {
                            closed = true;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                if !closed {
                    unclosed_ticks.insert(ticks);
                }
            }
            if !closed {
                i = start + ticks;
            }
            out.push_str(&src[start..i]);
            continue;
        }

        // 3. Escaped dollar \$ -> literal $
        if bytes[i] == b'\\' && i + 1 < len && bytes[i + 1] == b'$' {
            out.push_str("\\$");
            i += 2;
            continue;
        }

        // 4. Display math $$ ... $$
        if !no_close_double_dollar && bytes[i] == b'$' && i + 1 < len && bytes[i + 1] == b'$' {
            let body_start = i + 2;
            if let Some(rel_end) = find_unescaped_double_dollar(&src[body_start..]) {
                let tex = src[body_start..body_start + rel_end].trim();
                let ph = format!("{prefix}{}END", spans.len());
                let html = format!(
                    "<div class=\"mt-math-block\" data-tex=\"{}\"></div>",
                    escape_html_string(tex)
                );
                spans.push(ProtectedSpan {
                    placeholder: ph.clone(),
                    html,
                    text: tex.to_owned(),
                });
                out.push_str(&ph);
                extra_lines.push((
                    out.len(),
                    src[body_start..body_start + rel_end]
                        .bytes()
                        .filter(|&b| b == b'\n')
                        .count(),
                ));
                i = body_start + rel_end + 2;
                continue;
            }
            no_close_double_dollar = true;
        }

        // 5. Display math \[ ... \]
        if !no_close_bracket && bytes[i] == b'\\' && i + 1 < len && bytes[i + 1] == b'[' {
            let body_start = i + 2;
            if let Some(rel_end) = src[body_start..].find("\\]") {
                let tex = src[body_start..body_start + rel_end].trim();
                let ph = format!("{prefix}{}END", spans.len());
                let html = format!(
                    "<div class=\"mt-math-block\" data-tex=\"{}\"></div>",
                    escape_html_string(tex)
                );
                spans.push(ProtectedSpan {
                    placeholder: ph.clone(),
                    html,
                    text: tex.to_owned(),
                });
                out.push_str(&ph);
                extra_lines.push((
                    out.len(),
                    src[body_start..body_start + rel_end]
                        .bytes()
                        .filter(|&b| b == b'\n')
                        .count(),
                ));
                i = body_start + rel_end + 2;
                continue;
            }
            no_close_bracket = true;
        }

        // 6. Inline math \( ... \)
        if !no_close_paren && bytes[i] == b'\\' && i + 1 < len && bytes[i + 1] == b'(' {
            let body_start = i + 2;
            if let Some(rel_end) = src[body_start..].find("\\)") {
                let tex = src[body_start..body_start + rel_end].trim();
                let ph = format!("{prefix}{}END", spans.len());
                let html = format!(
                    "<span class=\"mt-math-inline\" data-tex=\"{}\"></span>",
                    escape_html_string(tex)
                );
                spans.push(ProtectedSpan {
                    placeholder: ph.clone(),
                    html,
                    text: tex.to_owned(),
                });
                out.push_str(&ph);
                extra_lines.push((
                    out.len(),
                    src[body_start..body_start + rel_end]
                        .bytes()
                        .filter(|&b| b == b'\n')
                        .count(),
                ));
                i = body_start + rel_end + 2;
                continue;
            }
            no_close_paren = true;
        }

        // 7. Inline math $ ... $ (single line, non-empty, not currency like "$ 5")
        if bytes[i] == b'$' {
            let body_start = i + 1;
            if body_start < len
                && bytes[body_start] != b' '
                && bytes[body_start] != b'\n'
                && bytes[body_start] != b'$'
                && let Some(rel_end) = find_inline_dollar_end(&src[body_start..])
            {
                let tex = &src[body_start..body_start + rel_end];
                if !tex.is_empty() && !tex.ends_with(' ') {
                    let ph = format!("{prefix}{}END", spans.len());
                    let html = format!(
                        "<span class=\"mt-math-inline\" data-tex=\"{}\"></span>",
                        escape_html_string(tex)
                    );
                    spans.push(ProtectedSpan {
                        placeholder: ph.clone(),
                        html,
                        text: tex.to_owned(),
                    });
                    out.push_str(&ph);
                    i = body_start + rel_end + 1;
                    continue;
                }
            }
        }

        // 8. Typora highlight ==text== (single line)
        if bytes[i] == b'=' && i + 1 < len && bytes[i + 1] == b'=' {
            let body_start = i + 2;
            let line_end = src[body_start..]
                .find('\n')
                .map_or(len, |rel| body_start + rel);
            if let Some(rel_end) = src[body_start..line_end].find("==") {
                let inner = &src[body_start..body_start + rel_end];
                if !inner.is_empty() {
                    let ph = format!("{prefix}{}END", spans.len());
                    let html = format!("<mark>{}</mark>", escape_html_string(inner));
                    spans.push(ProtectedSpan {
                        placeholder: ph.clone(),
                        html,
                        text: inner.to_owned(),
                    });
                    out.push_str(&ph);
                    i = body_start + rel_end + 2;
                    continue;
                }
            }
        }

        // 9. Typora superscript ^text^
        if bytes[i] == b'^' && (i == 0 || bytes[i - 1] != b'[') {
            let body_start = i + 1;
            if let Some(rel_end) = find_delim_no_space(&src[body_start..], b'^') {
                let inner = &src[body_start..body_start + rel_end];
                if !inner.is_empty() {
                    let ph = format!("{prefix}{}END", spans.len());
                    let html = format!("<sup>{}</sup>", escape_html_string(inner));
                    spans.push(ProtectedSpan {
                        placeholder: ph.clone(),
                        html,
                        text: inner.to_owned(),
                    });
                    out.push_str(&ph);
                    i = body_start + rel_end + 1;
                    continue;
                }
            }
        }

        // 10. Typora subscript ~text~ (single tilde, not ~~strikethrough~~)
        if bytes[i] == b'~'
            && (i + 1 >= len || bytes[i + 1] != b'~')
            && (i == 0 || bytes[i - 1] != b'~')
        {
            let body_start = i + 1;
            if let Some(rel_end) = find_delim_no_space(&src[body_start..], b'~') {
                let after = body_start + rel_end + 1;
                if after >= len || bytes[after] != b'~' {
                    let inner = &src[body_start..body_start + rel_end];
                    if !inner.is_empty() {
                        let ph = format!("{prefix}{}END", spans.len());
                        let html = format!("<sub>{}</sub>", escape_html_string(inner));
                        spans.push(ProtectedSpan {
                            placeholder: ph.clone(),
                            html,
                            text: inner.to_owned(),
                        });
                        out.push_str(&ph);
                        i = after;
                        continue;
                    }
                }
            }
        }

        let ch = src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    (out, spans, extra_lines, prefix)
}

fn find_unescaped_double_dollar(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'$' && bytes[i + 1] == b'$' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_inline_dollar_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            return None;
        }
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            // Avoid matching if next char is digit (like "$5 + $10")
            if i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                return None;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_delim_no_space(s: &str, delim: u8) -> Option<usize> {
    let bytes = s.as_bytes();
    for (idx, &b) in bytes.iter().enumerate() {
        if b == b'\n' || b == b' ' || b == b'\t' {
            return None;
        }
        if b == delim {
            return Some(idx);
        }
    }
    None
}

pub fn is_external_link(dest: &str) -> bool {
    let trimmed = dest.trim();
    let lower = trimmed.to_ascii_lowercase();
    !trimmed.chars().any(char::is_control)
        && ["https://", "http://", "mailto:"]
            .iter()
            .any(|scheme| lower.starts_with(scheme))
}

fn safe_link(dest: &str) -> bool {
    let trimmed = dest.trim();
    is_external_link(trimmed)
        || (!trimmed.chars().any(char::is_control)
            && !trimmed.contains([':', '\\'])
            && !trimmed.starts_with("//"))
}

fn render_raw_html(raw: &str, target: &mut String) {
    // Preserve attribute-free formatting tags. All other markup is shown as text.
    static FORMATTING: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)</?(?:abbr|b|br|cite|code|del|details|em|i|ins|kbd|mark|p|pre|s|samp|small|strong|sub|summary|sup|u|var)[\t ]*/?>").unwrap()
    });
    let mut end = 0;
    for tag in FORMATTING.find_iter(raw) {
        escape_html(&raw[end..tag.start()], target);
        target.push_str(tag.as_str());
        end = tag.end();
    }
    escape_html(&raw[end..], target);
}

fn render_text(
    mut text: &str,
    target: &mut String,
    mut plain: Option<&mut String>,
    spans: &[ProtectedSpan],
    prefix: &str,
) {
    while !spans.is_empty() {
        let Some(start) = text.find(prefix) else {
            break;
        };
        let Some(relative_end) = text[start + prefix.len()..].find("END") else {
            break;
        };
        let end = start + prefix.len() + relative_end + 3;
        escape_html(&text[..start], target);
        if let Some(p) = &mut plain {
            p.push_str(&text[..start]);
        }
        let token = &text[start..end];
        let span = token[prefix.len()..token.len() - 3]
            .parse::<usize>()
            .ok()
            .and_then(|index| spans.get(index))
            .filter(|span| span.placeholder == token);
        if let Some(span) = span {
            target.push_str(&span.html);
            if let Some(p) = &mut plain {
                p.push_str(&span.text);
            }
        } else {
            escape_html(token, target);
            if let Some(p) = &mut plain {
                p.push_str(token);
            }
        }
        text = &text[end..];
    }
    escape_html(text, target);
    if let Some(p) = plain {
        p.push_str(text);
    }
}

/// Render a standalone YAML front matter block (`--- ... ---`).
pub fn render_frontmatter(src: &str) -> String {
    let mut rows = String::new();
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed == "---" || trimmed.is_empty() {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            rows.push_str("<div class=\"mt-fm-row\"><span class=\"mt-fm-key\">");
            escape_html(k.trim(), &mut rows);
            rows.push_str("</span><span class=\"mt-fm-val\">");
            escape_html(v.trim(), &mut rows);
            rows.push_str("</span></div>");
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    format!(
        "<details class=\"mt-frontmatter\" open><summary>YAML Front Matter</summary><div class=\"mt-fm-grid\">{rows}</div></details>"
    )
}

/// Render a standalone display math block (`$$...$$`, `\[...\]`, or `\begin{...}...\end{...}`).
pub fn render_math_block(src: &str) -> String {
    let trimmed = src.trim();
    let tex = if trimmed.starts_with("$$") && trimmed.ends_with("$$") && trimmed.len() >= 4 {
        trimmed[2..trimmed.len() - 2].trim()
    } else if let Some(stripped) = trimmed.strip_prefix("$$") {
        stripped.trim()
    } else if trimmed.starts_with("\\[") && trimmed.ends_with("\\]") && trimmed.len() >= 4 {
        trimmed[2..trimmed.len() - 2].trim()
    } else {
        trimmed
    };
    format!(
        "<div class=\"mt-math-block\" data-tex=\"{}\"></div>",
        escape_html_string(tex)
    )
}

/// Render any Markdown block into crisp, Typora-styled HTML.
pub fn render_block(src: &str, ctx: &RenderContext<'_>) -> String {
    let mut references = References::default();
    if src.contains("]:") {
        references.add(src);
    }
    render_block_with_references(src, ctx, &references)
}

pub(crate) fn render_block_with_references(
    src: &str,
    ctx: &RenderContext<'_>,
    references: &References,
) -> String {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // [TOC] special Typora block
    if trimmed.eq_ignore_ascii_case("[toc]") {
        let inner = ctx
            .toc_html
            .unwrap_or("<div class=\"mt-toc-empty\">Table of Contents</div>");
        return format!(
            "<nav class=\"mt-toc\"><div class=\"mt-toc-title\">Table of Contents</div>{inner}</nav>"
        );
    }

    // Standalone display math block
    if (trimmed.starts_with("$$") && trimmed.ends_with("$$") && trimmed.len() >= 4)
        || (trimmed.starts_with("$$") && trimmed.contains('\n'))
        || (trimmed.starts_with("\\[") && trimmed.ends_with("\\]"))
        || (trimmed.starts_with("\\begin{") && trimmed.contains("\\end{"))
    {
        return render_math_block(trimmed);
    }

    // Check for GitHub/Typora Callout blockquote: `> [!NOTE]` etc.
    if let Some(callout_html) = try_render_callout(trimmed, ctx, references) {
        return callout_html;
    }

    let (preprocessed, spans, extra_lines, span_prefix) = preprocess_block(src);
    let parser = Parser::new_with_broken_link_callback(
        &preprocessed,
        parser_options(),
        Some(|link: BrokenLink<'_>| {
            let (dest, title) = references
                .links
                .get(&UniCase::new(link.reference.into_string()))?;
            Some((dest.clone().into(), title.clone().into()))
        }),
    );
    let mut html = String::with_capacity(src.len() * 2 + 64);

    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_buf = String::new();

    let mut in_heading: Option<HeadingLevel> = None;
    let mut heading_inner = String::new();
    let mut heading_plain = String::new();

    let mut in_image = false;
    let mut img_dest = String::new();
    let mut img_title = String::new();
    let mut img_alt = String::new();
    let mut link_tags = Vec::new();
    let newlines: Vec<_> = preprocessed
        .match_indices('\n')
        .map(|(index, _)| index)
        .collect();

    let mut table_alignments: Vec<pulldown_cmark::Alignment> = Vec::new();
    let mut in_table_head = false;
    let mut table_col = 0usize;

    for (event, range) in parser.into_offset_iter() {
        if in_code_block {
            match event {
                Event::Text(t) | Event::Code(t) => code_buf.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    in_code_block = false;
                    let lang_lower = code_lang.trim().to_ascii_lowercase();
                    if matches!(lang_lower.as_str(), "math" | "latex" | "tex") {
                        html.push_str(&format!(
                            "<div class=\"mt-math-block\" data-tex=\"{}\"></div>",
                            escape_html_string(code_buf.trim())
                        ));
                    } else if lang_lower == "mermaid" {
                        html.push_str("<div class=\"mt-diagram\"><pre>");
                        escape_html(&code_buf, &mut html);
                        html.push_str("</pre></div>");
                    } else {
                        let label = if code_lang.trim().is_empty() {
                            "text"
                        } else {
                            code_lang.trim()
                        };
                        let highlighted = highlight_code(&code_buf, &code_lang);
                        html.push_str("<div class=\"mt-code-wrap\"><div class=\"mt-code-head\"><span class=\"mt-code-lang\">");
                        escape_html(label, &mut html);
                        html.push_str("</span><button class=\"mt-code-copy\" type=\"button\" data-copy-code>Copy</button></div><pre class=\"mt-code-pre\"><code>");
                        html.push_str(&highlighted);
                        html.push_str("</code></pre></div>");
                    }
                    code_lang.clear();
                    code_buf.clear();
                }
                _ => {}
            }
            continue;
        }

        if in_image {
            match event {
                Event::Text(t) | Event::Code(t) => img_alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    in_image = false;
                    let resolved = crate::assets::image_url(&img_dest, ctx.doc_dir);
                    let target = if in_heading.is_some() {
                        &mut heading_inner
                    } else {
                        &mut html
                    };
                    if let Some(resolved) = resolved {
                        target.push_str("<img src=\"");
                        escape_html(&resolved, target);
                        target.push_str("\" alt=\"");
                        escape_html(&img_alt, target);
                        if !img_title.is_empty() {
                            target.push_str("\" title=\"");
                            escape_html(&img_title, target);
                        }
                        target.push_str("\" loading=\"lazy\" />");
                    } else {
                        escape_html(&img_alt, target);
                    }
                    img_dest.clear();
                    img_title.clear();
                    img_alt.clear();
                }
                _ => {}
            }
            continue;
        }

        let target = if in_heading.is_some() {
            &mut heading_inner
        } else {
            &mut html
        };

        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => target.push_str("<p>"),
                Tag::Heading { level, .. } => {
                    in_heading = Some(level);
                    heading_inner.clear();
                    heading_plain.clear();
                }
                Tag::BlockQuote(_) => target.push_str("<blockquote>"),
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    code_buf.clear();
                    code_lang = match kind {
                        CodeBlockKind::Fenced(lang) => {
                            lang.split_whitespace().next().unwrap_or("").to_string()
                        }
                        CodeBlockKind::Indented => String::new(),
                    };
                }
                Tag::List(None) => target.push_str("<ul>"),
                Tag::List(Some(1)) => target.push_str("<ol>"),
                Tag::List(Some(start)) => {
                    target.push_str(&format!("<ol start=\"{start}\">"));
                }
                Tag::Item => target.push_str("<li>"),
                Tag::FootnoteDefinition(name) => {
                    target.push_str("<div class=\"mt-footnote\" id=\"fn-");
                    escape_html(&name, target);
                    target.push_str("\"><span class=\"mt-fn-label\">[");
                    escape_html(&name, target);
                    target.push_str("]</span>");
                }
                Tag::Table(aligns) => {
                    table_alignments = aligns;
                    target.push_str("<div class=\"mt-table-wrap\"><table>");
                }
                Tag::TableHead => {
                    in_table_head = true;
                    table_col = 0;
                    target.push_str("<thead><tr>");
                }
                Tag::TableRow => {
                    table_col = 0;
                    target.push_str("<tr>");
                }
                Tag::TableCell => {
                    let align = table_alignments.get(table_col).copied();
                    table_col += 1;
                    let cell_tag = if in_table_head { "th" } else { "td" };
                    match align {
                        Some(pulldown_cmark::Alignment::Center) => {
                            target.push_str(&format!("<{cell_tag} style=\"text-align:center\">"))
                        }
                        Some(pulldown_cmark::Alignment::Right) => {
                            target.push_str(&format!("<{cell_tag} style=\"text-align:right\">"))
                        }
                        Some(pulldown_cmark::Alignment::Left) => {
                            target.push_str(&format!("<{cell_tag} style=\"text-align:left\">"))
                        }
                        _ => target.push_str(&format!("<{cell_tag}>")),
                    }
                }
                Tag::Emphasis => target.push_str("<em>"),
                Tag::Strong => target.push_str("<strong>"),
                Tag::Strikethrough => target.push_str("<del>"),
                Tag::Link {
                    dest_url, title, ..
                } => {
                    let link = safe_link(&dest_url);
                    link_tags.push(link);
                    if link {
                        target.push_str("<a href=\"");
                        escape_html(&dest_url, target);
                        if !title.is_empty() {
                            target.push_str("\" title=\"");
                            escape_html(&title, target);
                        }
                        target.push('"');
                        if is_external_link(&dest_url) {
                            target.push_str(" target=\"_blank\" rel=\"noopener noreferrer\"");
                        }
                        target.push('>');
                    }
                }
                Tag::Image {
                    dest_url, title, ..
                } => {
                    in_image = true;
                    img_dest = dest_url.to_string();
                    img_title = title.to_string();
                    img_alt.clear();
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph => target.push_str("</p>"),
                TagEnd::Heading(_) => {
                    if let Some(lvl) = in_heading.take() {
                        let tag_num = match lvl {
                            HeadingLevel::H1 => 1,
                            HeadingLevel::H2 => 2,
                            HeadingLevel::H3 => 3,
                            HeadingLevel::H4 => 4,
                            HeadingLevel::H5 => 5,
                            HeadingLevel::H6 => 6,
                        };
                        let slug = slugify(&heading_plain);
                        html.push_str(&format!(
                            "<h{tag_num} id=\"{slug}\">{heading_inner}</h{tag_num}>"
                        ));
                    }
                }
                TagEnd::BlockQuote(_) => target.push_str("</blockquote>"),
                TagEnd::List(ordered) => {
                    target.push_str(if ordered { "</ol>" } else { "</ul>" });
                }
                TagEnd::Item => target.push_str("</li>"),
                TagEnd::FootnoteDefinition => target.push_str("</div>"),
                TagEnd::Table => target.push_str("</tbody></table></div>"),
                TagEnd::TableHead => {
                    in_table_head = false;
                    target.push_str("</tr></thead><tbody>");
                }
                TagEnd::TableRow => target.push_str("</tr>"),
                TagEnd::TableCell => {
                    target.push_str(if in_table_head { "</th>" } else { "</td>" });
                }
                TagEnd::Emphasis => target.push_str("</em>"),
                TagEnd::Strong => target.push_str("</strong>"),
                TagEnd::Strikethrough => target.push_str("</del>"),
                TagEnd::Link if link_tags.pop() == Some(true) => target.push_str("</a>"),
                _ => {}
            },
            Event::Text(text) => {
                render_text(
                    &text,
                    target,
                    in_heading.map(|_| &mut heading_plain),
                    &spans,
                    &span_prefix,
                );
            }
            Event::Code(code) => {
                if in_heading.is_some() {
                    heading_plain.push_str(&code);
                }
                target.push_str("<code class=\"mt-inline-code\">");
                escape_html(&code, target);
                target.push_str("</code>");
            }
            Event::Html(raw) | Event::InlineHtml(raw) => {
                render_raw_html(&raw, target);
            }
            Event::FootnoteReference(name) => {
                if references.footnotes.contains(name.as_ref()) {
                    target.push_str("<sup class=\"mt-fn-ref\"><a href=\"#fn-");
                    escape_html(&name, target);
                    target.push_str("\">[");
                    escape_html(&name, target);
                    target.push_str("]</a></sup>");
                } else {
                    target.push_str("[^");
                    escape_html(&name, target);
                    target.push(']');
                }
            }
            Event::SoftBreak => target.push('\n'),
            Event::HardBreak => target.push_str("<br />"),
            Event::Rule => target.push_str("<hr />"),
            Event::TaskListMarker(checked) => {
                let line = ctx.block_start_line
                    + newlines.partition_point(|&index| index < range.start)
                    + extra_lines
                        .iter()
                        .take_while(|(end, _)| *end <= range.start)
                        .map(|(_, lines)| lines)
                        .sum::<usize>();
                let checked_attr = if checked { " checked" } else { "" };
                target.push_str(&format!(
                    "<input type=\"checkbox\" class=\"mt-task-cb\" data-line-offset=\"{line}\"{checked_attr} /> "
                ));
            }
            _ => {}
        }
    }

    html
}

fn try_render_callout(
    trimmed: &str,
    ctx: &RenderContext<'_>,
    references: &References,
) -> Option<String> {
    let first_line = trimmed.lines().next()?;
    let after_gt = first_line.trim_start().strip_prefix('>')?.trim();
    let (kind, default_title, icon) = if let Some(rest) = after_gt.strip_prefix("[!NOTE]") {
        (
            "note",
            if rest.trim().is_empty() {
                "Note"
            } else {
                rest.trim()
            },
            "ℹ",
        )
    } else if let Some(rest) = after_gt.strip_prefix("[!TIP]") {
        (
            "tip",
            if rest.trim().is_empty() {
                "Tip"
            } else {
                rest.trim()
            },
            "✦",
        )
    } else if let Some(rest) = after_gt.strip_prefix("[!IMPORTANT]") {
        (
            "important",
            if rest.trim().is_empty() {
                "Important"
            } else {
                rest.trim()
            },
            "◆",
        )
    } else if let Some(rest) = after_gt.strip_prefix("[!WARNING]") {
        (
            "warning",
            if rest.trim().is_empty() {
                "Warning"
            } else {
                rest.trim()
            },
            "▲",
        )
    } else {
        let rest = after_gt.strip_prefix("[!CAUTION]")?;
        (
            "caution",
            if rest.trim().is_empty() {
                "Caution"
            } else {
                rest.trim()
            },
            "✕",
        )
    };

    let body: String = trimmed
        .lines()
        .skip(1)
        .map(|l| l.trim_start().strip_prefix('>').unwrap_or(l).trim_start())
        .collect::<Vec<_>>()
        .join("\n");

    let inner_ctx = RenderContext {
        doc_dir: ctx.doc_dir,
        block_start_line: ctx.block_start_line + 1,
        toc_html: ctx.toc_html,
    };
    let body_html = if body.trim().is_empty() {
        String::new()
    } else {
        render_block_with_references(&body, &inner_ctx, references)
    };

    let title_html = render_inline_mini(default_title);
    Some(format!(
        "<div class=\"mt-callout mt-callout-{kind}\"><div class=\"mt-callout-title\"><span class=\"mt-callout-icon\">{icon}</span>{title_html}</div><div class=\"mt-callout-body\">{body_html}</div></div>"
    ))
}

fn render_inline_mini(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' {
            let mut code = String::new();
            let mut closed = false;
            for ch in chars.by_ref() {
                if ch == '`' {
                    closed = true;
                    break;
                }
                code.push(ch);
            }
            if closed {
                out.push_str("<code class=\"mt-inline-code\">");
                escape_html(&code, &mut out);
                out.push_str("</code>");
            } else {
                out.push('`');
                escape_html(&code, &mut out);
            }
        } else {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                _ => out.push(c),
            }
        }
    }
    out
}
