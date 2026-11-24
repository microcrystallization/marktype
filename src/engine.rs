//! Ultra-fast Rope-backed Incremental Markdown Block Engine.
//!
//! Architecture:
//! - Document text lives in `ropey::Rope` (O(log N) edits, low memory on 10MB+ files).
//! - Top-level blocks are partitioned in a single linear scan (~10ms for 10MB).
//! - On every keystroke, `apply_edit` updates the Rope in ~2 µs, re-partitions ONLY the
//!   dirty block neighborhood (stopping as soon as block boundaries re-converge), renders
//!   ONLY the changed block(s) via `pulldown-cmark` (< 0.2ms!), and returns a minimal patch.
//! - Supports viewport-virtualized block & line fetching so 100,000-line files open instantly.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Instant;

use regex::{Captures, Regex, RegexBuilder};
use ropey::Rope;
use serde::Serialize;

use crate::markdown::{self, RenderContext};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    FrontMatter,
    Heading {
        level: u8,
        text: String,
        slug: String,
    },
    CodeFence,
    MathBlock,
    Table,
    Blockquote,
    List,
    Hr,
    Toc,
    Paragraph,
}

#[derive(Debug, Clone)]
pub struct BlockEntry {
    pub id: u64,
    pub start_line: usize,
    pub end_line: usize, // exclusive
    pub kind: BlockKind,
    pub hash: u64,
    pub cached_html: Option<String>,
    has_reference_syntax: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedBlock {
    pub id: u64,
    pub start_line: usize,
    pub end_line: usize,
    pub kind: &'static str,
    pub html: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineItem {
    pub id: u64,
    pub level: u8,
    pub text: String,
    pub slug: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocStats {
    pub lines: usize,
    pub words: usize,
    pub chars: usize,
    pub bytes: usize,
    pub reading_minutes: usize,
    pub total_blocks: usize,
    pub large_mode: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPatch {
    pub version: u64,
    pub total_blocks: usize,
    pub total_lines: usize,
    pub replace_start: usize,
    pub remove_count: usize,
    pub inserted: Vec<RenderedBlock>,
    pub line_delta: isize,
    pub outline_changed: bool,
    pub outline: Option<Vec<OutlineItem>>,
    pub refresh_preview: bool,
    pub stats: DocStats,
    pub rust_us: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub start_char: usize,
    pub end_char: usize,
    pub line: usize,
    pub col: usize,
    pub preview: String,
    pub end_line: usize,
    pub end_col: usize,
}

/// Threshold above which the editor switches to virtualized viewport streaming (~300 KB).
pub const LARGE_FILE_BYTES: usize = 300 * 1024;
const LARGE_FILE_LINES: usize = 6000;

pub struct Document {
    pub id: String,
    pub path: Option<PathBuf>,
    pub title: String,
    pub rope: Rope,
    pub blocks: Vec<BlockEntry>,
    pub dirty: bool,
    pub version: u64,
    next_block_id: u64,
    cached_words: usize,
    cached_chars: usize,
    references: markdown::References,
    line_ending: &'static str,
    has_bom: bool,
}

impl Document {
    pub fn new(id: String, title: String, path: Option<PathBuf>, content: &str) -> Self {
        let has_bom = content.starts_with('\u{feff}');
        let content = content.strip_prefix('\u{feff}').unwrap_or(content);
        let line_ending = if let Some(first) = content.find(['\r', '\n']) {
            if content[first..].starts_with("\r\n") {
                "\r\n"
            } else if content.as_bytes()[first] == b'\r' {
                "\r"
            } else {
                "\n"
            }
        } else {
            "\n"
        };
        let normalized = normalize_newlines(content);
        let content = normalized.as_ref();
        let rope = Rope::from_str(content);
        let (words, chars) = count_words_chars(content);
        let mut doc = Self {
            id,
            path,
            title,
            rope,
            blocks: Vec::new(),
            dirty: false,
            version: 1,
            next_block_id: 1,
            cached_words: words,
            cached_chars: chars,
            references: Default::default(),
            line_ending,
            has_bom,
        };
        doc.rebuild_all_blocks_from_str(content);
        doc
    }

    pub fn is_large(&self) -> bool {
        self.rope.len_bytes() > LARGE_FILE_BYTES || self.rope.len_lines() > LARGE_FILE_LINES
    }

    pub fn stats(&self) -> DocStats {
        let lines = self.rope.len_lines();
        let bytes = self.rope.len_bytes();
        let reading_minutes = (self.cached_words / 220).max(usize::from(self.cached_words > 0));
        DocStats {
            lines,
            words: self.cached_words,
            chars: self.cached_chars,
            bytes,
            reading_minutes,
            total_blocks: self.blocks.len(),
            large_mode: self.is_large(),
        }
    }

    pub fn outline(&self) -> Vec<OutlineItem> {
        self.blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Heading { level, text, slug } => Some(OutlineItem {
                    id: b.id,
                    level: *level,
                    text: text.clone(),
                    slug: slug.clone(),
                    line: b.start_line,
                }),
                _ => None,
            })
            .collect()
    }

    pub fn toc_html(&self) -> String {
        let items = self.outline();
        if items.is_empty() {
            return "<div class=\"mt-toc-empty\">No headings yet</div>".to_string();
        }
        let mut out = String::from("<ul class=\"mt-toc-list\">");
        for item in items {
            let indent = (item.level.saturating_sub(1) as usize) * 14;
            out.push_str(&format!(
                "<li style=\"padding-left:{indent}px\"><a href=\"#{}\" data-jump-line=\"{}\">{}</a></li>",
                item.slug,
                item.line,
                crate::highlight::escape_html_string(&item.text)
            ));
        }
        out.push_str("</ul>");
        out
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_block_id;
        self.next_block_id += 1;
        id
    }

    pub fn line_str(&self, line_idx: usize) -> String {
        if line_idx >= self.rope.len_lines() {
            return String::new();
        }
        let mut s = self.rope.line(line_idx).to_string();
        if s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') {
                s.pop();
            }
        }
        s
    }

    pub fn slice_lines(&self, start_line: usize, end_line: usize) -> String {
        let total = self.rope.len_lines();
        let s = start_line.min(total);
        let e = end_line.min(total);
        if s >= e {
            return String::new();
        }
        let start_char = self.rope.line_to_char(s);
        let end_char = if e >= total {
            self.rope.len_chars()
        } else {
            self.rope.line_to_char(e)
        };
        let mut out = self.rope.slice(start_char..end_char).to_string();
        if out.ends_with('\n') {
            out.pop();
            if out.ends_with('\r') {
                out.pop();
            }
        }
        out
    }

    pub fn full_text(&self) -> String {
        self.rope.to_string()
    }

    pub fn write_text(&self, mut writer: impl std::io::Write) -> std::io::Result<()> {
        if self.has_bom {
            writer.write_all("\u{feff}".as_bytes())?;
        }
        for chunk in self.rope.chunks() {
            if self.line_ending == "\n" {
                writer.write_all(chunk.as_bytes())?;
            } else {
                writer.write_all(chunk.replace('\n', self.line_ending).as_bytes())?;
            }
        }
        Ok(())
    }

    pub fn set_path(&mut self, path: PathBuf) -> bool {
        let changed = self.path.as_ref() != Some(&path);
        if changed {
            for block in &mut self.blocks {
                block.cached_html = None;
            }
            self.path = Some(path);
        }
        changed
    }

    fn refresh_references(&mut self) -> bool {
        let mut references = markdown::References::default();
        for block in &self.blocks {
            if block.has_reference_syntax {
                references.add(&self.slice_lines(block.start_line, block.end_line));
            }
        }
        if self.references == references {
            return false;
        }
        self.references = references;
        for block in &mut self.blocks {
            block.cached_html = None;
        }
        true
    }

    fn hash_range(&self, start_line: usize, end_line: usize) -> u64 {
        let mut hasher = DefaultHasher::new();
        let total = self.rope.len_lines();
        let s = start_line.min(total);
        let e = end_line.min(total);
        for line_idx in s..e {
            let line = self.line_str(line_idx);
            line.hash(&mut hasher);
            if line_idx + 1 < e || e < total {
                "\n".hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn rebuild_all_blocks_from_str(&mut self, content: &str) {
        self.blocks.clear();
        let lines: Vec<&str> = content
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        let total = lines.len();
        let mut line = 0;
        while line < total {
            if let Some((start, end, kind)) = scan_block(line, total, |index| lines[index].into()) {
                let mut hasher = DefaultHasher::new();
                for (i, l) in lines[start..end].iter().enumerate() {
                    l.hash(&mut hasher);
                    if i + 1 < end - start || end < total {
                        "\n".hash(&mut hasher);
                    }
                }
                let hash = hasher.finish();
                let id = self.alloc_id();
                let has_reference_syntax = has_reference_syntax(&kind, &lines[start..end]);
                self.blocks.push(BlockEntry {
                    id,
                    start_line: start,
                    end_line: end,
                    kind,
                    hash,
                    cached_html: None,
                    has_reference_syntax,
                });
                line = end;
            } else {
                break;
            }
        }
        if content.contains("]:") {
            self.refresh_references();
        } else {
            self.references = Default::default();
        }
    }

    fn rebuild_all_blocks(&mut self) {
        let text = self.rope.to_string();
        self.rebuild_all_blocks_from_str(&text);
    }

    fn kind_label(kind: &BlockKind) -> &'static str {
        match kind {
            BlockKind::FrontMatter => "frontmatter",
            BlockKind::Heading { .. } => "heading",
            BlockKind::CodeFence => "code",
            BlockKind::MathBlock => "math",
            BlockKind::Table => "table",
            BlockKind::Blockquote => "quote",
            BlockKind::List => "list",
            BlockKind::Hr => "hr",
            BlockKind::Toc => "toc",
            BlockKind::Paragraph => "paragraph",
        }
    }

    pub fn render_block_index(
        &mut self,
        idx: usize,
        include_source: bool,
    ) -> Option<RenderedBlock> {
        if idx >= self.blocks.len() {
            return None;
        }
        let toc_str = if matches!(self.blocks[idx].kind, BlockKind::Toc) {
            Some(self.toc_html())
        } else {
            None
        };
        let start_line = self.blocks[idx].start_line;
        let end_line = self.blocks[idx].end_line;
        let id = self.blocks[idx].id;
        let kind_str = Self::kind_label(&self.blocks[idx].kind);

        let src = if self.blocks[idx].cached_html.is_none() || include_source || toc_str.is_some() {
            Some(self.slice_lines(start_line, end_line))
        } else {
            None
        };

        let html = if toc_str.is_none() {
            if let Some(cached) = &self.blocks[idx].cached_html {
                cached.clone()
            } else {
                let text = src.as_deref().unwrap_or("");
                let doc_dir = self.path.as_deref().and_then(Path::parent);
                let rendered = match self.blocks[idx].kind {
                    BlockKind::FrontMatter => markdown::render_frontmatter(text),
                    BlockKind::MathBlock => markdown::render_math_block(text),
                    _ => markdown::render_block_with_references(
                        text,
                        &RenderContext {
                            doc_dir,
                            block_start_line: 0,
                            toc_html: None,
                        },
                        &self.references,
                    ),
                };
                self.blocks[idx].cached_html = Some(rendered.clone());
                rendered
            }
        } else {
            let text = src.as_deref().unwrap_or("[TOC]");
            let doc_dir = self.path.as_deref().and_then(Path::parent);
            markdown::render_block(
                text,
                &RenderContext {
                    doc_dir,
                    block_start_line: start_line,
                    toc_html: toc_str.as_deref(),
                },
            )
        };

        Some(RenderedBlock {
            id,
            start_line,
            end_line,
            kind: kind_str,
            html,
            source: if include_source { src } else { None },
        })
    }

    pub fn render_range(
        &mut self,
        start_block: usize,
        count: usize,
        include_source: bool,
    ) -> Vec<RenderedBlock> {
        let end = start_block.saturating_add(count).min(self.blocks.len());
        let mut out = Vec::with_capacity(end.saturating_sub(start_block));
        for idx in start_block..end {
            if let Some(rb) = self.render_block_index(idx, include_source) {
                out.push(rb);
            }
        }
        out
    }

    /// **Core <10ms Incremental Edit Path**
    /// Applies a character-offset edit `[start_char, end_char)` -> `insert_text` on the Rope,
    /// incrementally updates ONLY the affected block neighborhood, renders ONLY changed blocks,
    /// and returns a compact DOM patch!
    pub fn apply_edit(
        &mut self,
        start_char: usize,
        end_char: usize,
        insert_text: &str,
        include_source: bool,
    ) -> EditPatch {
        self.apply_edit_with_preview(start_char, end_char, insert_text, Some(include_source))
    }

    pub(crate) fn apply_edit_with_preview(
        &mut self,
        start_char: usize,
        end_char: usize,
        insert_text: &str,
        include_source: Option<bool>,
    ) -> EditPatch {
        let t0 = Instant::now();
        let insert_text = normalize_newlines(insert_text);
        let insert_text = insert_text.as_ref();
        let len_chars = self.rope.len_chars();
        let s = start_char.min(len_chars);
        let e = end_char.min(len_chars).max(s);

        let old_lines = self.rope.len_lines();
        let edit_start_line = self.rope.char_to_line(s);
        let old_edit_end_line = self.rope.char_to_line(e);

        // A list can continue across a blank line. Front matter may gain a closing delimiter.
        let lookback_line = if edit_start_line <= 81 && self.line_str(0).trim() == "---" {
            0
        } else {
            edit_start_line.saturating_sub(2)
        };
        let b_start = self.blocks.partition_point(|b| b.end_line <= lookback_line);
        let old_scan_line = self
            .blocks
            .get(b_start)
            .map_or(lookback_line, |b| b.start_line.min(lookback_line));

        // Include adjacent characters so a word split or join contributes the correct delta.
        let context_start = s.saturating_sub(1);
        let context_end = e.saturating_add(1).min(len_chars);
        let (old_words, old_chars) =
            count_words_chars_iter(self.rope.slice(context_start..context_end).chars());
        if e > s {
            self.rope.remove(s..e);
        }
        if !insert_text.is_empty() {
            self.rope.insert(s, insert_text);
        }
        let new_context_end = context_end - (e - s) + insert_text.chars().count();
        let (new_words, new_chars) =
            count_words_chars_iter(self.rope.slice(context_start..new_context_end).chars());
        self.cached_words = self.cached_words - old_words + new_words;
        self.cached_chars = self.cached_chars - old_chars + new_chars;
        self.dirty = true;
        self.version += 1;

        let new_lines = self.rope.len_lines();
        let line_delta = new_lines as isize - old_lines as isize;
        let new_edit_end_line = ((old_edit_end_line as isize) + line_delta).max(0) as usize;

        // 2. Scan forward from `scan_from` until past `new_edit_end_line + 1` AND re-converged
        //    with an existing trailing block's shifted `start_line`.
        let mut new_blocks: Vec<BlockEntry> = Vec::new();
        let mut cursor_line = old_scan_line;
        let mut converge_old_idx = self.blocks.len();

        while cursor_line < new_lines {
            // Skip blank lines before checking convergence with trailing blocks!
            while cursor_line < new_lines && self.line_str(cursor_line).trim().is_empty() {
                cursor_line += 1;
            }
            if cursor_line >= new_lines {
                break;
            }
            // Check convergence once we're safely past the edited region
            if cursor_line > new_edit_end_line + 1 {
                // Binary search for an old block whose shifted start_line >= cursor_line
                let mut idx = b_start;
                if idx < self.blocks.len() {
                    let tail = &self.blocks[b_start..];
                    let pos = tail.partition_point(|b| {
                        (b.start_line as isize + line_delta) < cursor_line as isize
                    });
                    idx = b_start + pos;
                }
                if idx < self.blocks.len() {
                    let shifted_start =
                        (self.blocks[idx].start_line as isize + line_delta) as usize;
                    if shifted_start == cursor_line {
                        let shifted_end =
                            (self.blocks[idx].end_line as isize + line_delta) as usize;
                        if shifted_end <= new_lines
                            && self.hash_range(shifted_start, shifted_end) == self.blocks[idx].hash
                        {
                            converge_old_idx = idx;
                            break;
                        }
                    }
                }
            }

            if let Some((bs, be, kind)) =
                scan_block(cursor_line, new_lines, |index| self.line_str(index).into())
            {
                let hash = self.hash_range(bs, be);
                let id = self.alloc_id();
                let has_reference_syntax = can_define_references(&kind)
                    && (bs..be).any(|line| self.line_str(line).contains("]:"));
                new_blocks.push(BlockEntry {
                    id,
                    start_line: bs,
                    end_line: be,
                    kind,
                    hash,
                    cached_html: None,
                    has_reference_syntax,
                });
                cursor_line = be;
            } else {
                break;
            }
        }

        // Keep identities when rescanning unchanged neighbors or editing a block in place.
        let old_slice_len = converge_old_idx.saturating_sub(b_start);
        let old_blocks = &self.blocks[b_start..converge_old_idx];
        let mut next_old = 0;
        for new in &mut new_blocks {
            let old_line = if new.start_line > new_edit_end_line {
                new.start_line.saturating_add_signed(-line_delta)
            } else {
                new.start_line
            };
            next_old += old_blocks[next_old..].partition_point(|block| block.start_line < old_line);
            if let Some(old) = old_blocks
                .get(next_old)
                .filter(|block| block.start_line == old_line)
            {
                new.id = old.id;
                if old.hash == new.hash {
                    new.cached_html.clone_from(&old.cached_html);
                }
                next_old += 1;
            }
        }

        // Detect if outline (any Heading) changed
        let old_has_heading = self.blocks[b_start..converge_old_idx]
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Heading { .. }));
        let new_has_heading = new_blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Heading { .. }));
        let outline_changed = old_has_heading
            || new_has_heading
            || (line_delta != 0
                && self.blocks[converge_old_idx..]
                    .iter()
                    .any(|b| matches!(b.kind, BlockKind::Heading { .. })));

        let check_references = old_blocks.iter().any(|b| b.has_reference_syntax)
            || new_blocks.iter().any(|b| b.has_reference_syntax);

        // Shift all trailing blocks by line_delta
        if line_delta != 0 {
            for b in &mut self.blocks[converge_old_idx..] {
                b.start_line = (b.start_line as isize + line_delta) as usize;
                b.end_line = (b.end_line as isize + line_delta) as usize;
            }
        }

        let new_count = new_blocks.len();
        self.blocks.splice(b_start..converge_old_idx, new_blocks);
        let refresh_preview = check_references && self.refresh_references();

        // Render ONLY the newly inserted/updated blocks!
        let inserted = include_source.map_or_else(Vec::new, |include_source| {
            (b_start..b_start + new_count)
                .filter_map(|i| self.render_block_index(i, include_source))
                .collect()
        });

        let outline = if outline_changed {
            Some(self.outline())
        } else {
            None
        };
        let stats = self.stats();
        let rust_us = t0.elapsed().as_micros() as u64;

        EditPatch {
            version: self.version,
            total_blocks: self.blocks.len(),
            total_lines: new_lines,
            replace_start: b_start,
            remove_count: old_slice_len,
            inserted,
            line_delta,
            outline_changed,
            outline,
            refresh_preview,
            stats,
            rust_us,
        }
    }

    /// Replace an entire block's markdown source (used in Typora Live Mode!).
    pub fn replace_block_source(&mut self, block_id: u64, new_source: &str) -> Option<EditPatch> {
        let idx = self.blocks.iter().position(|b| b.id == block_id)?;
        let start_line = self.blocks[idx].start_line;
        let end_line = self.blocks[idx].end_line;
        let total = self.rope.len_lines();
        let start_char = self.rope.line_to_char(start_line);
        let mut end_char = if end_line >= total {
            self.rope.len_chars()
        } else {
            self.rope.line_to_char(end_line)
        };
        // Keep trailing newline if block wasn't the last line
        let mut replacement = new_source.to_string();
        if end_line < total && end_char > start_char && self.rope.char(end_char - 1) == '\n' {
            if !replacement.ends_with('\n') {
                replacement.push('\n');
            }
        } else if end_char > start_char && self.rope.char(end_char - 1) == '\n' {
            end_char -= 1;
        }
        Some(self.apply_edit(start_char, end_char, &replacement, true))
    }

    /// Toggle a Markdown task checkbox (`- [ ]` <-> `- [x]`) on the given 0-based line.
    pub fn toggle_task_line(&mut self, line_idx: usize) -> Option<EditPatch> {
        static MARKER: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
            Regex::new(r"^[\t ]*(?:>[\t ]*)*(?:[-+*]|[0-9]{1,9}[.)])[\t ]+\[([ xX])\]").unwrap()
        });
        if line_idx >= self.rope.len_lines() {
            return None;
        }
        let line = self.line_str(line_idx);
        let marker = MARKER.captures(&line)?.get(1)?;
        let replacement = if marker.as_str() == " " { "x" } else { " " };
        let char_offset_in_line = line[..marker.start()].chars().count();
        let line_start_char = self.rope.line_to_char(line_idx);
        let s = line_start_char + char_offset_in_line;
        Some(self.apply_edit(s, s + 1, replacement, true))
    }

    /// Fast full-document search directly over the Rope (supports regex & case-sensitivity).
    pub fn search(
        &self,
        query: &str,
        case_sensitive: bool,
        use_regex: bool,
        limit: usize,
    ) -> Vec<SearchMatch> {
        if query.is_empty() {
            return Vec::new();
        }
        let Some(re) = search_regex(query, case_sensitive, use_regex) else {
            return Vec::new();
        };

        let full = self.full_text();
        let mut matches = Vec::new();
        for m in re.find_iter(&full).take(limit) {
            let start_char = self.rope.byte_to_char(m.start());
            let end_char = self.rope.byte_to_char(m.end());
            let line = self.rope.char_to_line(start_char);
            let end_line = self.rope.char_to_line(end_char);
            matches.push(SearchMatch {
                start_char,
                end_char,
                line,
                col: start_char - self.rope.line_to_char(line),
                end_line,
                end_col: end_char - self.rope.line_to_char(end_line),
                preview: self.line_str(line).trim().chars().take(80).collect(),
            });
        }
        matches
    }

    pub fn replace_match(
        &mut self,
        start: usize,
        end: usize,
        search: &SearchQuery<'_>,
        replacement: &str,
    ) -> Option<EditPatch> {
        let re = search_regex(search.query, search.case_sensitive, search.use_regex)?;
        let start_byte = self.rope.try_char_to_byte(start).ok()?;
        let end_byte = self.rope.try_char_to_byte(end).ok()?;
        let full = self.full_text();
        let captures = re.captures_at(&full, start_byte)?;
        if captures.get(0)?.range() != (start_byte..end_byte) {
            return None;
        }
        let replacement = replacement_text(&captures, replacement, search.use_regex);
        Some(self.apply_edit(start, end, &replacement, true))
    }

    /// Replace all occurrences in the Rope and return the number of replacements.
    pub fn replace_all(
        &mut self,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
        use_regex: bool,
    ) -> usize {
        if query.is_empty() {
            return 0;
        }
        let Some(re) = search_regex(query, case_sensitive, use_regex) else {
            return 0;
        };
        let full = self.rope.to_string();
        let mut count = 0;
        let replaced = re.replace_all(&full, |captures: &Captures<'_>| {
            count += 1;
            replacement_text(captures, replacement, use_regex)
        });
        if count == 0 {
            return 0;
        }
        let replaced = normalize_newlines(&replaced);
        let (w, c) = count_words_chars(&replaced);
        self.rope = Rope::from_str(&replaced);
        self.cached_words = w;
        self.cached_chars = c;
        self.dirty = true;
        self.version += 1;
        self.rebuild_all_blocks();
        count
    }

    /// Render the entire document into standalone HTML for Export to HTML.
    pub fn render_full_html(&mut self) -> String {
        let mut body = String::with_capacity(self.rope.len_bytes() * 2);
        for idx in 0..self.blocks.len() {
            if let Some(rb) = self.render_block_index(idx, false) {
                body.push_str(&rb.html);
                body.push('\n');
            }
        }
        body
    }
}

pub struct SearchQuery<'a> {
    pub query: &'a str,
    pub case_sensitive: bool,
    pub use_regex: bool,
}

fn search_regex(query: &str, case_sensitive: bool, use_regex: bool) -> Option<Regex> {
    if query.is_empty() {
        return None;
    }
    let pattern = if use_regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .multi_line(true)
        .build()
        .ok()
}

fn replacement_text(captures: &Captures<'_>, replacement: &str, use_regex: bool) -> String {
    if !use_regex {
        return replacement.to_owned();
    }
    let mut text = String::new();
    captures.expand(replacement, &mut text);
    text
}

fn normalize_newlines(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n").into()
    } else {
        text.into()
    }
}

fn can_define_references(kind: &BlockKind) -> bool {
    !matches!(
        kind,
        BlockKind::FrontMatter | BlockKind::CodeFence | BlockKind::MathBlock | BlockKind::Toc
    )
}

fn has_reference_syntax(kind: &BlockKind, lines: &[&str]) -> bool {
    can_define_references(kind) && lines.iter().any(|line| line.contains("]:"))
}

/// Scans one top-level Markdown block starting at or after `from_line`.
fn scan_block<'a>(
    from_line: usize,
    total_lines: usize,
    mut line_at: impl FnMut(usize) -> std::borrow::Cow<'a, str>,
) -> Option<(usize, usize, BlockKind)> {
    let mut line = from_line;
    while line < total_lines {
        let s = line_at(line);
        if !s.trim().is_empty() {
            break;
        }
        line += 1;
    }
    if line >= total_lines {
        return None;
    }

    let first = line_at(line);
    let trimmed = first.trim();

    // 1. YAML Front Matter at line 0
    if line == 0 && trimmed == "---" {
        let mut end = line + 1;
        while end < total_lines {
            let l = line_at(end);
            if l.trim() == "---" || l.trim() == "..." {
                return Some((0, end + 1, BlockKind::FrontMatter));
            }
            if end > 80 {
                break;
            }
            end += 1;
        }
    }

    // 2. Fenced Code Block (``` or ~~~)
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        let fence_char = trimmed.as_bytes()[0];
        let fence_len = trimmed.bytes().take_while(|&b| b == fence_char).count();
        let mut end = line + 1;
        while end < total_lines {
            let l = line_at(end);
            let lt = l.trim();
            let c2 = lt.bytes().take_while(|&b| b == fence_char).count();
            end += 1;
            if c2 >= fence_len && lt[c2..].trim().is_empty() {
                break;
            }
        }
        return Some((line, end, BlockKind::CodeFence));
    }

    // 3. Display Math Block ($$ ... $$, \[ ... \], \begin{...}...\end{...})
    if trimmed.starts_with("$$") {
        if trimmed.len() >= 4 && trimmed.ends_with("$$") {
            return Some((line, line + 1, BlockKind::MathBlock));
        }
        let mut end = line + 1;
        while end < total_lines {
            let l = line_at(end);
            end += 1;
            if l.trim().ends_with("$$") {
                break;
            }
        }
        return Some((line, end, BlockKind::MathBlock));
    }
    if trimmed.starts_with("\\[") {
        if trimmed.ends_with("\\]") && trimmed.len() >= 4 {
            return Some((line, line + 1, BlockKind::MathBlock));
        }
        let mut end = line + 1;
        while end < total_lines {
            let l = line_at(end);
            end += 1;
            if l.trim().ends_with("\\]") {
                break;
            }
        }
        return Some((line, end, BlockKind::MathBlock));
    }
    if trimmed.starts_with("\\begin{")
        && let Some(close) = trimmed.find('}')
    {
        let env_name = &trimmed[7..close];
        let end_marker = format!("\\end{{{env_name}}}");
        if trimmed.contains(&end_marker) {
            return Some((line, line + 1, BlockKind::MathBlock));
        }
        let mut end = line + 1;
        while end < total_lines {
            let l = line_at(end);
            end += 1;
            if l.contains(&end_marker) {
                break;
            }
        }
        return Some((line, end, BlockKind::MathBlock));
    }

    // 4. ATX Heading (# .. ######)
    if let Some((level, text)) = markdown::heading_plain_text(&first) {
        let slug = markdown::slugify(&text);
        return Some((line, line + 1, BlockKind::Heading { level, text, slug }));
    }

    // 5. [TOC]
    if trimmed.eq_ignore_ascii_case("[toc]") {
        return Some((line, line + 1, BlockKind::Toc));
    }

    // 6. Horizontal rule (---, ***, ___)
    if is_hr_line(trimmed) {
        return Some((line, line + 1, BlockKind::Hr));
    }

    // 7. Blockquote (>)
    if trimmed.starts_with('>') {
        let mut end = line + 1;
        while end < total_lines {
            let l = line_at(end);
            let lt = l.trim();
            if lt.is_empty() || (!lt.starts_with('>') && is_block_starter(lt)) {
                break;
            }
            end += 1;
        }
        return Some((line, end, BlockKind::Blockquote));
    }

    // 8. GFM Table (header line with `|` followed by delimiter `|---|---|`)
    if first.contains('|') && line + 1 < total_lines {
        let next = line_at(line + 1);
        if is_table_delimiter(next.trim()) {
            let mut end = line + 2;
            while end < total_lines {
                let l = line_at(end);
                let lt = l.trim();
                if lt.is_empty() || !lt.contains('|') {
                    break;
                }
                end += 1;
            }
            return Some((line, end, BlockKind::Table));
        }
    }

    // 9. List (unordered or ordered)
    if is_list_line(&first) {
        let mut end = line + 1;
        while end < total_lines {
            let l = line_at(end);
            let lt = l.trim();
            if lt.is_empty() {
                // Check if loose list continues on the very next non-empty line
                if end + 1 < total_lines {
                    let peek = line_at(end + 1);
                    if is_list_line(&peek) || peek.starts_with("  ") || peek.starts_with('\t') {
                        end += 1;
                        continue;
                    }
                }
                break;
            }
            if !is_list_line(&l)
                && !l.starts_with("  ")
                && !l.starts_with('\t')
                && is_block_starter(lt)
            {
                break;
            }
            end += 1;
        }
        return Some((line, end, BlockKind::List));
    }

    // 10. Setext heading (paragraph line followed by === or ---)
    if line + 1 < total_lines {
        let next = line_at(line + 1);
        let nt = next.trim();
        if !nt.is_empty() && nt.bytes().all(|b| b == b'=') && nt.len() >= 2 {
            let text = trimmed.replace(['*', '_', '`'], "");
            let slug = markdown::slugify(&text);
            return Some((
                line,
                line + 2,
                BlockKind::Heading {
                    level: 1,
                    text,
                    slug,
                },
            ));
        }
        if !nt.is_empty() && nt.bytes().all(|b| b == b'-') && nt.len() >= 2 {
            let text = trimmed.replace(['*', '_', '`'], "");
            let slug = markdown::slugify(&text);
            return Some((
                line,
                line + 2,
                BlockKind::Heading {
                    level: 2,
                    text,
                    slug,
                },
            ));
        }
    }

    // 11. Paragraph (default)
    let mut end = line + 1;
    while end < total_lines {
        let l = line_at(end);
        let lt = l.trim();
        if lt.is_empty() || is_block_starter(lt) || is_list_line(&l) {
            break;
        }
        end += 1;
    }
    Some((line, end, BlockKind::Paragraph))
}

fn is_hr_line(trimmed: &str) -> bool {
    if trimmed.len() < 3 {
        return false;
    }
    let first = trimmed.as_bytes()[0];
    if !matches!(first, b'-' | b'*' | b'_') {
        return false;
    }
    let mut count = 0;
    for b in trimmed.bytes() {
        if b == first {
            count += 1;
        } else if b != b' ' && b != b'\t' {
            return false;
        }
    }
    count >= 3
}

fn is_list_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
        || trimmed == "-"
        || trimmed == "*"
    {
        return true;
    }
    let digits = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 && digits <= 9 {
        let rest = &trimmed[digits..];
        if rest.starts_with(". ") || rest.starts_with(") ") {
            return true;
        }
    }
    false
}

fn is_table_delimiter(trimmed: &str) -> bool {
    if !trimmed.contains('-') || !trimmed.contains('|') {
        return false;
    }
    trimmed
        .bytes()
        .all(|b| matches!(b, b'|' | b'-' | b':' | b' ' | b'\t'))
}

fn is_block_starter(trimmed: &str) -> bool {
    trimmed.starts_with('#')
        || trimmed.starts_with("```")
        || trimmed.starts_with("~~~")
        || trimmed.starts_with("$$")
        || trimmed.starts_with("\\[")
        || trimmed.starts_with("\\begin{")
        || trimmed.starts_with('>')
        || is_hr_line(trimmed)
        || trimmed.eq_ignore_ascii_case("[toc]")
}

/// Counts English words + CJK characters accurately (just like Typora!).
fn count_words_chars(s: &str) -> (usize, usize) {
    count_words_chars_iter(s.chars())
}

fn count_words_chars_iter(chars_iter: impl Iterator<Item = char>) -> (usize, usize) {
    let mut words = 0usize;
    let mut chars = 0usize;
    let mut in_word = false;
    for ch in chars_iter {
        if !ch.is_whitespace() {
            chars += 1;
        }
        if is_cjk(ch) {
            words += 1;
            in_word = false;
        } else if ch.is_alphanumeric() {
            if !in_word {
                words += 1;
                in_word = true;
            }
        } else {
            in_word = false;
        }
    }
    (words, chars)
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x30FF | 0xAC00..=0xD7AF
    )
}
