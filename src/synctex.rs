//! Pure-Rust SyncTeX v1 parser and forward/inverse lookup engine.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use serde::Serialize;

/// Scaled points per PDF Big Point (bp): 65536 * 72.27 / 72.0.
const SP_PER_BP: f64 = 65_781.76;
/// Upper bound on decompressed SyncTeX payload size to guard against gzip bombs.
const MAX_SYNCTEX_BYTES: u64 = 64 * 1024 * 1024;
/// Upper bound on parsed SyncTeX nodes to bound memory usage.
const MAX_SYNCTEX_NODES: usize = 1_500_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NodeKind {
    VBox,
    HBox,
    Leaf,
}

#[derive(Clone, Debug)]
struct Node {
    kind: NodeKind,
    tag: u32,
    line: u32,
    column: Option<u32>,
    page: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Node {
    fn area(&self) -> f64 {
        self.width.max(1.0) * self.height.max(1.0)
    }

    fn contains(&self, px: f64, py: f64, margin: f64) -> bool {
        px >= self.x - margin
            && px <= self.x + self.width + margin
            && py >= self.y - margin
            && py <= self.y + self.height + margin
    }

    fn weighted_distance(&self, px: f64, py: f64) -> f64 {
        let dx = if px < self.x {
            self.x - px
        } else if px > self.x + self.width {
            px - (self.x + self.width)
        } else {
            0.0
        };
        let dy = if py < self.y {
            self.y - py
        } else if py > self.y + self.height {
            py - (self.y + self.height)
        } else {
            0.0
        };
        // Weight vertical distance more strongly since text flows horizontally in lines.
        (dx * dx + (dy * 4.0) * (dy * 4.0)).sqrt()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardSync {
    pub page: u32,
    pub boxes: Vec<SyncRect>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InverseSync {
    pub path: PathBuf,
    pub line: u32,
    pub column: Option<u32>,
}

#[derive(Clone, Debug)]
struct InputFile {
    path: PathBuf,
    normalized: String,
    is_project_source: bool,
    exists: bool,
}

#[derive(Clone, Debug)]
pub struct SyncTex {
    inputs: HashMap<u32, InputFile>,
    nodes: Vec<Node>,
    by_page: HashMap<u32, Range<usize>>,
    by_source: Vec<u32>,
}

impl SyncTex {
    /// Locate and load `<stem>.synctex.gz` or `<stem>.synctex` next to `pdf_path`.
    pub fn load_for_pdf(pdf_path: &Path) -> Result<Self, String> {
        let base_dir = pdf_path
            .parent()
            .ok_or_else(|| "PDF file has no parent directory".to_owned())?
            .to_path_buf();
        let gz = with_synctex_extension(pdf_path, "synctex.gz");
        if gz.is_file() {
            let file =
                File::open(&gz).map_err(|error| format!("Cannot open SyncTeX file: {error}"))?;
            let decoder = GzDecoder::new(file);
            return Self::parse(decoder, &base_dir);
        }
        let plain = with_synctex_extension(pdf_path, "synctex");
        if plain.is_file() {
            let file =
                File::open(&plain).map_err(|error| format!("Cannot open SyncTeX file: {error}"))?;
            return Self::parse(file, &base_dir);
        }
        Err("No SyncTeX file found for this PDF. Compile the LaTeX project first.".into())
    }

    pub fn parse<R: Read>(reader: R, base_dir: &Path) -> Result<Self, String> {
        let base_dir = dunce::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
        let mut inputs = HashMap::new();
        let mut nodes = Vec::new();
        let mut unit: i64 = 1;
        let mut magnification: i64 = 1000;
        let mut x_offset: i64 = 0;
        let mut y_offset: i64 = 0;
        let mut base_scale = 1.0 / SP_PER_BP;
        let mut coord_scale = 1.0 / SP_PER_BP;
        let mut in_content = false;
        let mut current_page: u32 = 0;
        let mut prev_h: i64 = 0;
        let mut prev_v: i64 = 0;
        let mut box_stack: Vec<(f64, f64)> = Vec::new(); // (height_pt, depth_pt)

        let mut buf_reader = BufReader::new(reader.take(MAX_SYNCTEX_BYTES + 1));
        let mut total_read: u64 = 0;
        let mut raw_line = Vec::new();
        loop {
            raw_line.clear();
            let read = buf_reader
                .read_until(b'\n', &mut raw_line)
                .map_err(|error| format!("Cannot read SyncTeX data: {error}"))?;
            if read == 0 {
                break;
            }
            total_read += read as u64;
            if total_read > MAX_SYNCTEX_BYTES {
                return Err("SyncTeX file exceeds the 64 MiB safety limit.".into());
            }
            let line = String::from_utf8_lossy(&raw_line);
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                continue;
            }

            // TeX engines emit `Input:` lazily both in the preamble and inside `Content:`
            // when chapters are `\input` or `\include`d after earlier pages ship out.
            if let Some(rest) = line.strip_prefix("Input:") {
                if let Some((tag_str, path_str)) = rest.split_once(':')
                    && let Ok(tag) = tag_str.parse::<u32>()
                {
                    let resolved = resolve_input_path(&base_dir, path_str);
                    let normalized = normalize_key(&resolved);
                    let exists = resolved.is_file();
                    let is_project_source =
                        resolved.starts_with(&base_dir) && crate::latex::project_source(&resolved);
                    inputs.insert(
                        tag,
                        InputFile {
                            path: resolved,
                            normalized,
                            is_project_source,
                            exists,
                        },
                    );
                }
                continue;
            }

            if !in_content {
                if line == "Content:" {
                    in_content = true;
                    base_scale = (magnification as f64) / 1000.0 / SP_PER_BP;
                    coord_scale = (unit as f64) * base_scale;
                    continue;
                }
                if let Some(val) = line.strip_prefix("Unit:") {
                    if let Ok(parsed) = val.trim().parse::<i64>()
                        && parsed > 0
                    {
                        unit = parsed;
                    }
                } else if let Some(val) = line.strip_prefix("Magnification:") {
                    if let Ok(parsed) = val.trim().parse::<i64>()
                        && parsed > 0
                    {
                        magnification = parsed;
                    }
                } else if let Some(val) = line.strip_prefix("X Offset:") {
                    if let Ok(parsed) = val.trim().parse::<i64>() {
                        x_offset = parsed;
                    }
                } else if let Some(val) = line.strip_prefix("Y Offset:")
                    && let Ok(parsed) = val.trim().parse::<i64>()
                {
                    y_offset = parsed;
                }
                continue;
            }

            if line == "Postamble:" {
                break;
            }

            let mut chars = line.chars();
            let Some(prefix) = chars.next() else {
                continue;
            };
            let rest = chars.as_str();

            match prefix {
                '{' => {
                    if let Ok(page) = rest.trim().parse::<u32>() {
                        current_page = page;
                        box_stack.clear();
                    }
                }
                '}' => {
                    current_page = 0;
                    box_stack.clear();
                }
                ']' | ')' => {
                    box_stack.pop();
                }
                '[' | '(' | 'v' | 'h' if current_page > 0 => {
                    if let Some((tag, src_line, col, h, v, w, height, depth)) =
                        parse_box_record(rest, &mut prev_h, &mut prev_v)
                    {
                        let x_raw = (h as f64) * coord_scale + (x_offset as f64) * base_scale;
                        let h_pt = (height.max(0) as f64) * coord_scale;
                        let d_pt = (depth.max(0) as f64) * coord_scale;
                        let y = (v as f64) * coord_scale + (y_offset as f64) * base_scale - h_pt;
                        let w_pt = (w as f64) * coord_scale;
                        let (x, width) = if w_pt < 0.0 {
                            ((x_raw + w_pt).max(0.0), -w_pt)
                        } else {
                            (x_raw, w_pt)
                        };
                        let total_height = (h_pt + d_pt).max(1.0);
                        if matches!(prefix, '[' | '(') {
                            box_stack.push((h_pt, d_pt));
                        }
                        let kind = match prefix {
                            '[' => NodeKind::VBox,
                            '(' => NodeKind::HBox,
                            _ => NodeKind::Leaf,
                        };
                        if nodes.len() < MAX_SYNCTEX_NODES {
                            nodes.push(Node {
                                kind,
                                tag,
                                line: src_line,
                                column: col,
                                page: current_page,
                                x,
                                y,
                                width,
                                height: total_height,
                            });
                        }
                    } else if matches!(prefix, '[' | '(') {
                        box_stack.push((9.0, 2.5));
                    }
                }
                'x' | 'g' | '$' | 'k' if current_page > 0 => {
                    if let Some((tag, src_line, col, h, v, w_opt)) =
                        parse_point_record(rest, &mut prev_h, &mut prev_v)
                    {
                        let (h_pt, d_pt) = box_stack.last().copied().unwrap_or((9.0, 2.5));
                        let raw_x = (h as f64) * coord_scale + (x_offset as f64) * base_scale;
                        let raw_w = w_opt.map(|w| (w as f64) * coord_scale).unwrap_or(0.0);
                        let (x, width) = if prefix == 'k' {
                            if raw_w >= 0.0 {
                                ((raw_x - raw_w).max(0.0), raw_w.max(12.0))
                            } else {
                                (raw_x.max(0.0), (-raw_w).max(12.0))
                            }
                        } else {
                            (raw_x.max(0.0), raw_w.abs().max(12.0))
                        };
                        let y = (v as f64) * coord_scale + (y_offset as f64) * base_scale - h_pt;
                        let total_height = (h_pt + d_pt).max(10.0);
                        if nodes.len() < MAX_SYNCTEX_NODES {
                            nodes.push(Node {
                                kind: NodeKind::Leaf,
                                tag,
                                line: src_line,
                                column: col,
                                page: current_page,
                                x,
                                y,
                                width,
                                height: total_height,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        if inputs.is_empty() && nodes.is_empty() {
            return Err("SyncTeX file contains no synchronization records.".into());
        }
        if !nodes.is_sorted_by_key(|node| node.page) {
            nodes.sort_by_key(|node| node.page);
        }
        let mut by_page = HashMap::new();
        let mut offset = 0;
        for page in nodes.chunk_by(|left, right| left.page == right.page) {
            by_page.insert(page[0].page, offset..offset + page.len());
            offset += page.len();
        }
        let mut by_source = Vec::with_capacity(nodes.len());
        for (index, node) in (0u32..).zip(&nodes) {
            if node.line > 0 {
                by_source.push(index);
            }
        }
        by_source.sort_unstable_by_key(|&index| {
            let node = &nodes[index as usize];
            (node.tag, node.line, index)
        });
        Ok(Self {
            inputs,
            nodes,
            by_page,
            by_source,
        })
    }

    /// Map a source file path and 1-based line number to PDF page and highlight boxes.
    pub fn forward(&self, source_path: &Path, line: u32) -> Option<ForwardSync> {
        let line = line.max(1);
        let tags = self.matching_tags(source_path);
        if tags.is_empty() {
            return None;
        }

        let key = |index: u32| {
            let node = &self.nodes[index as usize];
            (node.tag, node.line)
        };
        let lower_bound = |tag, line| {
            self.by_source
                .partition_point(|&index| key(index) < (tag, line))
        };
        let best_line = tags
            .iter()
            .flat_map(|&tag| {
                let next = lower_bound(tag, line);
                [next.checked_sub(1), Some(next)]
                    .into_iter()
                    .flatten()
                    .filter_map(move |i| {
                        let (found_tag, found_line) = key(*self.by_source.get(i)?);
                        (found_tag == tag).then_some(found_line)
                    })
            })
            .min_by_key(|&candidate| (candidate.abs_diff(line), candidate < line))?;
        if best_line.abs_diff(line) > 80 {
            return None;
        }

        let mut exact = Vec::new();
        for tag in tags {
            exact.extend(
                self.by_source[lower_bound(tag, best_line)..]
                    .iter()
                    .copied()
                    .take_while(|&index| key(index) == (tag, best_line)),
            );
        }
        exact.sort_unstable();
        let exact: Vec<&Node> = exact
            .into_iter()
            .map(|index| &self.nodes[index as usize])
            .collect();

        // Prefer HBox nodes, then Leaf nodes, and only fall back to VBox.
        let hboxes: Vec<&Node> = exact
            .iter()
            .copied()
            .filter(|node| node.kind == NodeKind::HBox && node.width > 0.5)
            .collect();
        let chosen: Vec<&Node> = if !hboxes.is_empty() {
            hboxes
        } else {
            let leaves: Vec<&Node> = exact
                .iter()
                .copied()
                .filter(|node| node.kind == NodeKind::Leaf)
                .collect();
            if !leaves.is_empty() { leaves } else { exact }
        };

        let page = chosen[0].page;
        let mut boxes: Vec<SyncRect> = chosen
            .into_iter()
            .filter(|node| node.page == page)
            .map(|node| SyncRect {
                x: node.x.max(0.0),
                y: node.y.max(0.0),
                width: node.width.max(18.0),
                height: node.height.max(10.0),
            })
            .collect();

        // Sort top-to-bottom, left-to-right and coalesce near-duplicate boxes.
        boxes.sort_by(|a, b| {
            a.y.partial_cmp(&b.y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        let merged = merge_line_boxes(boxes);
        Some(ForwardSync {
            page,
            boxes: merged,
        })
    }

    /// Map a 1-based PDF page and point `(x, y)` (in PDF Big Points from top-left) to source file & line.
    pub fn inverse(&self, page: u32, x: f64, y: f64) -> Option<InverseSync> {
        let mut best_enclosing: Option<(&Node, &InputFile, bool, u8, f64)> = None;
        let mut best_nearest: Option<(&Node, &InputFile, bool, f64)> = None;

        for node in &self.nodes[self.by_page.get(&page)?.clone()] {
            if node.line == 0 {
                continue;
            }
            let Some(input) = self.inputs.get(&node.tag) else {
                continue;
            };
            let is_local = input.is_project_source && input.exists;
            let kind_rank = match node.kind {
                NodeKind::Leaf => 0u8,
                NodeKind::HBox => 1u8,
                NodeKind::VBox => 2u8,
            };

            if node.contains(x, y, 2.5) {
                let area = node.area();
                let better = match best_enclosing {
                    None => true,
                    Some((_, _, prev_local, prev_rank, prev_area)) => {
                        (is_local, std::cmp::Reverse(kind_rank))
                            > (prev_local, std::cmp::Reverse(prev_rank))
                            || (is_local == prev_local
                                && kind_rank == prev_rank
                                && area < prev_area)
                    }
                };
                if better {
                    best_enclosing = Some((node, input, is_local, kind_rank, area));
                }
            }

            let dist = node.weighted_distance(x, y);
            let better = match best_nearest {
                None => true,
                Some((_, _, prev_local, prev_dist)) => {
                    if is_local != prev_local {
                        is_local && dist <= prev_dist + 48.0
                    } else {
                        dist < prev_dist
                    }
                }
            };
            if better {
                best_nearest = Some((node, input, is_local, dist));
            }
        }

        let (node, input) = best_enclosing
            .map(|(n, i, _, _, _)| (n, i))
            .or_else(|| best_nearest.map(|(n, i, _, _)| (n, i)))?;

        Some(InverseSync {
            path: input.path.clone(),
            line: node.line,
            column: node.column,
        })
    }

    fn matching_tags(&self, source_path: &Path) -> Vec<u32> {
        let canonical =
            dunce::canonicalize(source_path).unwrap_or_else(|_| clean_path(source_path));
        let key = normalize_key(&canonical);
        self.inputs
            .iter()
            .filter(|(_, input)| input.normalized == key)
            .map(|(&tag, _)| tag)
            .collect()
    }
}

pub fn synctex_path_for_pdf(pdf_path: &Path) -> Option<PathBuf> {
    let gz = with_synctex_extension(pdf_path, "synctex.gz");
    if gz.is_file() {
        return Some(gz);
    }
    let plain = with_synctex_extension(pdf_path, "synctex");
    if plain.is_file() {
        return Some(plain);
    }
    None
}

pub(crate) fn with_synctex_extension(path: &Path, extension: &str) -> PathBuf {
    let mut os = path.with_extension("").into_os_string();
    os.push(".");
    os.push(extension);
    PathBuf::from(os)
}

fn parse_link(link: &str) -> Option<(u32, u32, Option<u32>)> {
    let mut parts = link.split(',');
    let tag = parts.next()?.parse::<u32>().ok()?;
    let line = parts.next()?.parse::<u32>().ok()?;
    let col = parts
        .next()
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|&value| value > 0)
        .map(|value| value as u32);
    Some((tag, line, col))
}

fn parse_coord(token: &str, prev: &mut i64) -> Option<i64> {
    if token == "=" {
        return Some(*prev);
    }
    let val = token.parse::<i64>().ok()?;
    *prev = val;
    Some(val)
}

#[allow(clippy::type_complexity)]
fn parse_box_record(
    rest: &str,
    prev_h: &mut i64,
    prev_v: &mut i64,
) -> Option<(u32, u32, Option<u32>, i64, i64, i64, i64, i64)> {
    let mut sections = rest.split(':');
    let link = sections.next()?;
    let pos = sections.next()?;
    let size = sections.next()?;
    let (tag, line, col) = parse_link(link)?;
    let (h_str, v_str) = pos.split_once(',')?;
    let h = parse_coord(h_str, prev_h)?;
    let v = parse_coord(v_str, prev_v)?;
    let mut dims = size.split(',');
    let w = dims.next()?.parse::<i64>().ok()?;
    let height = dims.next()?.parse::<i64>().ok()?;
    let depth = dims.next()?.parse::<i64>().ok()?;
    Some((tag, line, col, h, v, w, height, depth))
}

#[allow(clippy::type_complexity)]
fn parse_point_record(
    rest: &str,
    prev_h: &mut i64,
    prev_v: &mut i64,
) -> Option<(u32, u32, Option<u32>, i64, i64, Option<i64>)> {
    let mut sections = rest.split(':');
    let link = sections.next()?;
    let pos = sections.next()?;
    let width = sections.next().and_then(|value| value.parse::<i64>().ok());
    let (tag, line, col) = parse_link(link)?;
    let (h_str, v_str) = pos.split_once(',')?;
    let h = parse_coord(h_str, prev_h)?;
    let v = parse_coord(v_str, prev_v)?;
    Some((tag, line, col, h, v, width))
}

fn resolve_input_path(base_dir: &Path, raw: &str) -> PathBuf {
    let raw = raw.trim();
    let candidate = Path::new(raw);
    let joined = if candidate.is_absolute() {
        clean_path(candidate)
    } else {
        clean_path(&base_dir.join(candidate))
    };
    dunce::canonicalize(&joined).unwrap_or(joined)
}

fn clean_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(component);
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn normalize_key(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        s.to_ascii_lowercase()
    } else {
        s
    }
}

fn merge_line_boxes(boxes: Vec<SyncRect>) -> Vec<SyncRect> {
    let mut merged: Vec<SyncRect> = Vec::new();
    for rect in boxes {
        if let Some(last) = merged.last_mut()
            && (last.y - rect.y).abs() <= 4.0
            && (last.height - rect.height).abs() <= 6.0
            && rect.x <= last.x + last.width + 12.0
        {
            let right = (last.x + last.width).max(rect.x + rect.width);
            let bottom = (last.y + last.height).max(rect.y + rect.height);
            last.x = last.x.min(rect.x);
            last.y = last.y.min(rect.y);
            last.width = right - last.x;
            last.height = bottom - last.y;
            continue;
        }
        merged.push(rect);
    }
    merged
}
