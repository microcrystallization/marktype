//! Application state and typed IPC command handlers bridging WebView2 <-> Rust engine.

use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::assets;
use crate::engine::{Document, EditPatch, RenderedBlock, SearchQuery};
use crate::settings::{self, Settings, ThemePreset, ViewMode};
use crate::wallpaper::fx::Effect;
use crate::wallpaper::gallery::{self, BUILTINS};
use crate::wallpaper::store::{self, Prepared};
use crate::wallpaper::theme_tint::{Palette, ResolvedTheme};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabSummary {
    pub id: String,
    pub title: String,
    pub path: Option<String>,
    pub dirty: bool,
    pub large_mode: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Vec<FileNode>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperTile {
    pub kind: &'static str, // "builtin" | "recent"
    pub id: String,
    pub title: String,
    pub path: String,
    pub thumb_url: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperStatePayload {
    pub active_name: String,
    pub active_path: Option<String>,
    pub image_url: Option<String>,
    pub blurred_url: Option<String>,
    pub effect: Effect,
    pub hero_opacity: f32,
    pub fills_window: bool,
    pub theme_colors: bool,
    pub error: Option<String>,
    pub tiles: Vec<WallpaperTile>,
    pub theme: ResolvedTheme,
}

pub struct AppState {
    pub settings: Settings,
    pub docs: Vec<Document>,
    pub active_tab: Option<usize>,
    pub next_doc_num: u64,
    pub prepared_wp: Option<Prepared>,
    pub wp_error: Option<String>,
    pub folder_root: Option<PathBuf>,
    autosaves: HashMap<String, Instant>,
    pending_close: Option<CloseRequest>,
}

enum CloseTarget {
    Tab(String),
    Window,
}

struct CloseRequest {
    target: CloseTarget,
    documents: VecDeque<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

impl AppState {
    pub fn new(initial_file: Option<PathBuf>) -> Self {
        let settings = settings::load();
        let folder_root = settings
            .last_folder
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.is_dir());

        let mut app = Self {
            settings,
            docs: Vec::new(),
            active_tab: None,
            next_doc_num: 1,
            prepared_wp: None,
            wp_error: None,
            folder_root,
            autosaves: HashMap::new(),
            pending_close: None,
        };

        // Prepare initial wallpaper synchronously on fast cache hit, or compute once
        app.sync_wallpaper_now();

        if let Some(path) = initial_file {
            let _ = app.open_file_path(&path);
        }

        app
    }

    fn alloc_doc_id(&mut self) -> String {
        let id = format!("doc-{}", self.next_doc_num);
        self.next_doc_num += 1;
        id
    }

    pub fn active_doc(&self) -> Option<&Document> {
        self.active_tab.and_then(|i| self.docs.get(i))
    }

    pub fn active_doc_mut(&mut self) -> Option<&mut Document> {
        self.active_tab.and_then(|i| self.docs.get_mut(i))
    }

    pub fn tab_summaries(&self) -> Vec<TabSummary> {
        self.docs
            .iter()
            .map(|d| TabSummary {
                id: d.id.clone(),
                title: d.title.clone(),
                path: d.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                dirty: d.dirty,
                large_mode: d.is_large(),
            })
            .collect()
    }

    pub fn open_file_path(&mut self, path: &Path) -> Result<usize, String> {
        let path = dunce::canonicalize(path).map_err(|e| format!("Cannot locate file: {e}"))?;
        // Focus existing tab if already open
        if let Some(idx) = self
            .docs
            .iter()
            .position(|d| d.path.as_deref() == Some(path.as_path()))
        {
            self.active_tab = Some(idx);
            return Ok(idx);
        }
        let t0 = Instant::now();
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("Cannot read UTF-8 document: {e}"))?;
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled.md".to_string());
        let id = self.alloc_doc_id();
        let doc = Document::new(id, title, Some(path.clone()), &text);
        let _open_ms = t0.elapsed().as_millis();
        self.docs.push(doc);
        let idx = self.docs.len() - 1;
        self.active_tab = Some(idx);
        self.settings.remember_recent_file(&path.to_string_lossy());
        let _ = settings::save(&self.settings);
        Ok(idx)
    }

    pub fn new_untitled_doc(&mut self) -> usize {
        let num = self.next_doc_num;
        let id = self.alloc_doc_id();
        let doc = Document::new(id, format!("Untitled-{num}.md"), None, "");
        self.docs.push(doc);
        let idx = self.docs.len() - 1;
        self.active_tab = Some(idx);
        idx
    }

    pub fn sync_wallpaper_now(&mut self) {
        self.wp_error = None;
        let Some(wp_str) = self.settings.wallpaper.as_deref() else {
            self.prepared_wp = None;
            return;
        };
        let path = Path::new(wp_str);
        if !path.is_file() {
            self.prepared_wp = None;
            return;
        }
        let Some(cfg) = settings::config_dir() else {
            return;
        };
        match store::prepare(
            path,
            self.settings.wallpaper_effect,
            self.settings.light,
            &cfg,
        ) {
            Ok(prep) => {
                self.prepared_wp = Some(prep);
            }
            Err(e) => {
                self.wp_error = Some(e);
                self.prepared_wp = None;
            }
        }
    }

    pub fn resolved_theme(&self) -> ResolvedTheme {
        let mut palette = Palette::base(self.settings.theme_preset, self.settings.light);
        if self.settings.wallpaper_theme_colors
            && let Some(accent) = self.prepared_wp.as_ref().and_then(|p| p.accent)
        {
            palette = palette.tinted(accent);
        }
        let backdrop = self.prepared_wp.as_ref().map(|p| p.backdrop.as_slice());
        palette.resolve(
            backdrop,
            self.settings.wallpaper_fills_window && self.prepared_wp.is_some(),
            self.settings.wallpaper_hero_opacity,
        )
    }

    pub fn wallpaper_payload(&self) -> WallpaperStatePayload {
        let cfg = settings::config_dir();
        let current = self.settings.wallpaper.as_deref();
        let mut tiles = Vec::new();

        for b in &BUILTINS {
            let b_path = cfg
                .as_ref()
                .map(|c| gallery::builtin_path(c, b))
                .unwrap_or_else(|| PathBuf::from(format!("builtin:{}", b.id)));
            let selected = current.is_some_and(|c| Path::new(c) == b_path);
            let thumb_url = cfg
                .as_ref()
                .and_then(|c| gallery::builtin_thumbnail(c, b).ok())
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .map(|f| format!("/wallpaper/thumb/{f}"))
                .unwrap_or_else(|| format!("/wallpaper/builtin/{}.jpg", b.id));
            tiles.push(WallpaperTile {
                kind: "builtin",
                id: b.id.to_string(),
                title: b.title.to_string(),
                path: b_path.to_string_lossy().into_owned(),
                thumb_url,
                selected,
            });
        }

        for recent in &self.settings.wallpaper_history {
            let rp = Path::new(recent);
            if !rp.is_file() {
                continue;
            }
            let title = rp
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| recent.clone());
            let thumb_url = cfg
                .as_ref()
                .and_then(|c| gallery::file_thumbnail(c, rp).ok())
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .map(|f| format!("/wallpaper/thumb/{f}"))
                .unwrap_or_default();
            tiles.push(WallpaperTile {
                kind: "recent",
                id: recent.clone(),
                title,
                path: recent.clone(),
                thumb_url,
                selected: current == Some(recent.as_str()),
            });
        }

        let active_name = match current {
            Some(p) if Path::new(p).is_file() => Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_owned()),
            Some(_) => "File not found".to_string(),
            None => "None".to_string(),
        };

        let image_url = self.prepared_wp.as_ref().and_then(|p| {
            p.image
                .file_name()
                .map(|n| format!("/wallpaper/cache/{}", n.to_string_lossy()))
        });
        let blurred_url = self.prepared_wp.as_ref().and_then(|p| {
            p.blurred
                .file_name()
                .map(|n| format!("/wallpaper/cache/{}", n.to_string_lossy()))
        });

        WallpaperStatePayload {
            active_name,
            active_path: current.map(str::to_owned),
            image_url,
            blurred_url,
            effect: self.settings.wallpaper_effect,
            hero_opacity: self.settings.wallpaper_hero_opacity,
            fills_window: self.settings.wallpaper_fills_window,
            theme_colors: self.settings.wallpaper_theme_colors,
            error: self.wp_error.clone(),
            tiles,
            theme: self.resolved_theme(),
        }
    }

    pub fn active_doc_snapshot(&mut self, max_initial_blocks: usize) -> Value {
        let split = self.settings.view_mode == ViewMode::Split;
        let Some(doc) = self.active_doc_mut() else {
            return Value::Null;
        };
        let large = doc.is_large();
        let total_blocks = doc.blocks.len();
        let deferred = large && !split;
        let block_count = if deferred {
            0
        } else if large {
            max_initial_blocks.min(total_blocks)
        } else {
            total_blocks
        };
        let blocks: Vec<RenderedBlock> = doc.render_range(0, block_count, true);
        let outline = doc.outline();
        let stats = doc.stats();
        let source = source_window(doc, 0, if large { 600 } else { doc.rope.len_lines() });
        let text = &source["text"];
        json!({
            "id": doc.id,
            "title": doc.title,
            "path": doc.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "dirty": doc.dirty,
            "version": doc.version,
            "largeMode": large,
            "windowStartLine": 0,
            "windowEndLine": source["endLine"],
            "contextLine": source["contextLine"],
            "text": text,
            "blocks": blocks,
            "previewDeferred": deferred,
            "totalBlocks": total_blocks,
            "outline": outline,
            "stats": stats,
        })
    }

    pub fn scan_folder_tree(&self) -> Option<FileNode> {
        let root = self.folder_root.as_deref()?;
        if !root.is_dir() {
            return None;
        }
        Some(build_file_tree(root, 0))
    }

    pub fn handle_command(&mut self, raw: &str) -> Value {
        let Ok(cmd) = serde_json::from_str::<IpcCommand>(raw) else {
            return json!({ "event": "error", "message": "Invalid IPC payload" });
        };

        let before = self.active_doc().map(|doc| (doc.id.clone(), doc.version));
        let response = self.handle(cmd);
        if let Some((id, version)) = before
            && let Some(doc) = self.active_doc()
            && doc.id == id
            && doc.version != version
            && doc.path.is_some()
            && self.settings.auto_save
        {
            self.autosaves
                .insert(id, Instant::now() + Duration::from_millis(1500));
        }
        response
    }

    pub fn next_autosave(&self) -> Option<Instant> {
        self.autosaves
            .iter()
            .filter(|(id, _)| !self.closing_document(id))
            .map(|(_, at)| *at)
            .min()
    }

    pub fn run_autosaves(&mut self, now: Instant) -> Vec<Value> {
        let due: Vec<_> = self
            .autosaves
            .iter()
            .filter(|(id, at)| **at <= now && !self.closing_document(id))
            .map(|(id, _)| id.clone())
            .collect();
        let mut responses = Vec::new();
        for id in due {
            self.autosaves.remove(&id);
            if let Some(idx) = self
                .docs
                .iter()
                .position(|doc| doc.id == id && doc.dirty && doc.path.is_some())
            {
                match self.save_document(idx, false, false) {
                    Ok(Some(changed_path)) => {
                        responses.push(self.saved_response(idx, changed_path))
                    }
                    Ok(None) => {}
                    Err(e) => responses.push(error_toast(e)),
                }
            }
        }
        responses
    }

    // Keep autosave from writing a document while the user is choosing whether to discard it.
    fn closing_document(&self, id: &str) -> bool {
        match self.pending_close.as_ref().map(|close| &close.target) {
            Some(CloseTarget::Window) => true,
            Some(CloseTarget::Tab(tab)) => tab == id,
            None => false,
        }
    }

    pub fn request_window_close(&mut self) -> Value {
        self.begin_close(CloseTarget::Window)
    }

    fn begin_close(&mut self, target: CloseTarget) -> Value {
        if self.pending_close.is_some() {
            return json!({ "event": "noop" });
        }
        let documents = self
            .docs
            .iter()
            .filter(|doc| {
                doc.dirty
                    && match &target {
                        CloseTarget::Tab(id) => &doc.id == id,
                        CloseTarget::Window => true,
                    }
            })
            .map(|doc| doc.id.clone())
            .collect();
        self.pending_close = Some(CloseRequest { target, documents });
        self.advance_close()
    }

    fn advance_close(&mut self) -> Value {
        let Some(request) = &mut self.pending_close else {
            return json!({ "event": "noop" });
        };
        while let Some(id) = request.documents.front() {
            if let Some(doc) = self.docs.iter().find(|doc| &doc.id == id && doc.dirty) {
                return json!({
                    "event": "confirmClose",
                    "docId": doc.id,
                    "title": doc.title,
                    "tabs": self.tab_summaries(),
                });
            }
            request.documents.pop_front();
        }
        match self.pending_close.take().unwrap().target {
            CloseTarget::Window => json!({ "event": "exitRequested" }),
            CloseTarget::Tab(id) => self.finish_close_tab(&id),
        }
    }

    fn resolve_close(&mut self, id: &str, choice: CloseChoice) -> Value {
        let Some(next) = self
            .pending_close
            .as_ref()
            .and_then(|close| close.documents.front())
        else {
            return json!({ "event": "noop" });
        };
        if next != id {
            return json!({ "event": "noop" });
        }
        match choice {
            CloseChoice::Cancel => return self.cancel_close(None),
            CloseChoice::Save => {
                if let Some(idx) = self.docs.iter().position(|doc| doc.id == id && doc.dirty) {
                    match self.save_document(idx, false, true) {
                        Ok(Some(_)) => {}
                        Ok(None) => return self.cancel_close(None),
                        Err(error) => return self.cancel_close(Some(error)),
                    }
                }
            }
            CloseChoice::Discard => {}
        }
        self.pending_close.as_mut().unwrap().documents.pop_front();
        self.advance_close()
    }

    fn cancel_close(&mut self, message: Option<String>) -> Value {
        self.pending_close = None;
        json!({
            "event": "tabsUpdated",
            "tabs": self.tab_summaries(),
            "toast": message,
        })
    }

    fn finish_close_tab(&mut self, id: &str) -> Value {
        let Some(idx) = self.docs.iter().position(|doc| doc.id == id) else {
            return json!({ "event": "noop" });
        };
        let closing_active = self.active_tab == Some(idx);
        self.autosaves.remove(id);
        self.docs.remove(idx);
        self.active_tab = match self.active_tab {
            _ if self.docs.is_empty() => None,
            Some(active) if active < idx => Some(active),
            Some(active) if active > idx => Some(active - 1),
            _ => Some(idx.min(self.docs.len() - 1)),
        };
        if !closing_active {
            return json!({ "event": "tabsUpdated", "tabs": self.tab_summaries() });
        }
        let doc = self.active_doc_snapshot(200);
        json!({
            "event": "tabSwitched",
            "tabs": self.tab_summaries(),
            "activeTabId": self.active_doc().map(|d| d.id.clone()),
            "doc": doc,
        })
    }

    fn save_document(
        &mut self,
        idx: usize,
        save_as: bool,
        allow_dialog: bool,
    ) -> Result<Option<bool>, String> {
        let doc = &self.docs[idx];
        let target = if save_as || doc.path.is_none() {
            if !allow_dialog {
                return Ok(None);
            }
            let mut dialog = rfd::FileDialog::new()
                .set_file_name(&doc.title)
                .add_filter("Markdown", &["md", "markdown"]);
            if let Some(dir) = doc.path.as_deref().and_then(Path::parent) {
                dialog = dialog.set_directory(dir);
            }
            dialog.save_file()
        } else {
            doc.path.clone()
        };
        let Some(target) = target else {
            return Ok(None);
        };
        let target = dunce::canonicalize(&target).unwrap_or(target);
        if self
            .docs
            .iter()
            .enumerate()
            .any(|(i, doc)| i != idx && doc.path.as_ref() == Some(&target))
        {
            return Err("Save failed: another tab already has this file open".into());
        }
        atomic_write(&target, |file| doc.write_text(file))
            .map_err(|e| format!("Save failed: {e}"))?;
        let path = dunce::canonicalize(&target).unwrap_or(target);
        let doc = &mut self.docs[idx];
        let changed_path = doc.set_path(path.clone());
        if let Some(name) = path.file_name() {
            doc.title = name.to_string_lossy().into_owned();
        }
        doc.dirty = false;
        self.autosaves.remove(&doc.id);
        if changed_path {
            self.settings.remember_recent_file(&path.to_string_lossy());
            let _ = settings::save(&self.settings);
        }
        Ok(Some(changed_path))
    }

    fn saved_response(&self, idx: usize, changed_path: bool) -> Value {
        let doc = &self.docs[idx];
        let mut response = json!({
            "event": "saved",
            "docId": doc.id,
            "tabs": self.tab_summaries(),
            "toast": format!("Saved {}", doc.title),
        });
        if changed_path {
            response["fileTree"] = json!(self.scan_folder_tree());
        }
        response
    }

    fn handle(&mut self, cmd: IpcCommand) -> Value {
        match cmd {
            IpcCommand::Init => {
                let doc = self.active_doc_snapshot(200);
                let tabs = self.tab_summaries();
                let wp = self.wallpaper_payload();
                let tree = self.scan_folder_tree();
                json!({
                    "event": "init",
                    "version": env!("CARGO_PKG_VERSION"),
                    "platform": std::env::consts::OS,
                    "settings": self.settings,
                    "tabs": tabs,
                    "activeTabId": self.active_doc().map(|d| d.id.clone()),
                    "doc": doc,
                    "wallpaper": wp,
                    "fileTree": tree,
                })
            }

            IpcCommand::Edit {
                doc_id,
                base_line,
                start_char,
                end_char,
                text,
                source,
                utf16,
            } => {
                let Some(doc) = self.active_doc_mut().filter(|doc| doc_id == doc.id) else {
                    return json!({ "event": "noop" });
                };
                let Ok(base_offset) = doc.rope.try_line_to_char(base_line) else {
                    return json!({ "event": "noop" });
                };
                if start_char > end_char {
                    return json!({ "event": "noop" });
                }
                let offset = |relative| {
                    if utf16 {
                        let units = doc
                            .rope
                            .char_to_utf16_cu(base_offset)
                            .checked_add(relative)?;
                        let point = doc.rope.try_utf16_cu_to_char(units).ok()?;
                        (doc.rope.char_to_utf16_cu(point) == units).then_some(point)
                    } else {
                        base_offset
                            .checked_add(relative)
                            .filter(|&point| point <= doc.rope.len_chars())
                    }
                };
                let (Some(start), Some(end)) = (offset(start_char), offset(end_char)) else {
                    return json!({ "event": "noop" });
                };
                let patch =
                    doc.apply_edit_with_preview(start, end, &text, (!utf16).then_some(true));
                let mut response = patch_response(doc, patch, source);
                response["userEdit"] = json!(true);
                if utf16 {
                    response["deferredPreview"] = json!(true);
                }
                response
            }

            IpcCommand::ToggleTask {
                doc_id,
                version,
                block_id,
                line_offset,
                source,
            } => {
                let Some(doc) = self
                    .active_doc_mut()
                    .filter(|doc| doc_id == doc.id && version == doc.version)
                else {
                    return resync_response(&doc_id);
                };
                let line = doc.blocks.iter().find(|b| b.id == block_id).and_then(|b| {
                    b.start_line
                        .checked_add(line_offset)
                        .filter(|&line| line < b.end_line)
                });
                match line.and_then(|line| doc.toggle_task_line(line)) {
                    Some(patch) => patch_response(doc, patch, Some(source)),
                    None => resync_response(&doc_id),
                }
            }

            IpcCommand::RenderLiveWidgets {
                doc_id,
                request,
                fragments,
            } => {
                let html = self
                    .active_doc()
                    .filter(|doc| doc.id == doc_id)
                    .map(|doc| {
                        fragments
                            .iter()
                            .take(16)
                            .map(|source| doc.render_live_fragment(source))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                json!({ "event": "liveWidgetsRendered", "request": request, "html": html })
            }

            IpcCommand::FetchBlocks {
                doc_id,
                start,
                count,
                request,
                refresh,
                around_line,
            } => {
                let Some(doc) = self.active_doc_mut().filter(|doc| doc_id == doc.id) else {
                    return json!({ "event": "noop" });
                };
                let start = around_line.map_or(start, |line| {
                    doc.blocks
                        .partition_point(|block| block.end_line <= line)
                        .saturating_sub(35)
                });
                let blocks = doc.render_range(start, count, true);
                json!({
                    "event": "blocksLoaded",
                    "docId": doc.id,
                    "version": doc.version,
                    "request": request,
                    "refresh": refresh,
                    "start": start,
                    "blocks": blocks,
                })
            }

            IpcCommand::FetchLines {
                doc_id,
                start_line,
                count,
                request,
            } => {
                let Some(doc) = self.active_doc().filter(|doc| doc_id == doc.id) else {
                    return json!({ "event": "noop" });
                };
                json!({
                    "event": "linesLoaded",
                    "docId": doc.id,
                    "version": doc.version,
                    "request": request,
                    "source": source_window(doc, start_line, count.min(1000)),
                })
            }

            IpcCommand::NewTab => {
                self.new_untitled_doc();
                let doc = self.active_doc_snapshot(200);
                json!({
                    "event": "tabSwitched",
                    "tabs": self.tab_summaries(),
                    "activeTabId": self.active_doc().map(|d| d.id.clone()),
                    "doc": doc,
                })
            }

            IpcCommand::SwitchTab { id } => {
                if let Some(idx) = self.docs.iter().position(|d| d.id == id) {
                    self.active_tab = Some(idx);
                }
                let doc = self.active_doc_snapshot(200);
                json!({
                    "event": "tabSwitched",
                    "tabs": self.tab_summaries(),
                    "activeTabId": self.active_doc().map(|d| d.id.clone()),
                    "doc": doc,
                })
            }

            IpcCommand::CloseTab { id } => self.begin_close(CloseTarget::Tab(id)),

            IpcCommand::ResolveClose { doc_id, choice } => self.resolve_close(&doc_id, choice),

            IpcCommand::OpenFileDialog => {
                let picked = rfd::FileDialog::new()
                    .add_filter("Markdown", &["md", "markdown", "mdown", "mkd", "txt"])
                    .add_filter("All Files", &["*"])
                    .pick_file();
                if let Some(path) = picked {
                    let t0 = Instant::now();
                    match self.open_file_path(&path) {
                        Ok(_) => {
                            let ms = t0.elapsed().as_millis();
                            let doc = self.active_doc_snapshot(200);
                            json!({
                                "event": "tabSwitched",
                                "tabs": self.tab_summaries(),
                                "activeTabId": self.active_doc().map(|d| d.id.clone()),
                                "doc": doc,
                                "settings": self.settings,
                                "toast": format!("Opened {} in {} ms", path.file_name().unwrap_or_default().to_string_lossy(), ms),
                            })
                        }
                        Err(e) => json!({ "event": "toast", "message": e, "kind": "error" }),
                    }
                } else {
                    json!({ "event": "noop" })
                }
            }

            IpcCommand::OpenPath { path } => {
                let p = PathBuf::from(&path);
                let t0 = Instant::now();
                match self.open_file_path(&p) {
                    Ok(_) => {
                        let ms = t0.elapsed().as_millis();
                        let doc = self.active_doc_snapshot(200);
                        json!({
                            "event": "tabSwitched",
                            "tabs": self.tab_summaries(),
                            "activeTabId": self.active_doc().map(|d| d.id.clone()),
                            "doc": doc,
                            "settings": self.settings,
                            "toast": format!("Opened in {} ms", ms),
                        })
                    }
                    Err(e) => json!({ "event": "toast", "message": e, "kind": "error" }),
                }
            }

            IpcCommand::OpenFolderDialog => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.folder_root = Some(dir.clone());
                    self.settings.last_folder = Some(dir.to_string_lossy().into_owned());
                    let _ = settings::save(&self.settings);
                    let tree = self.scan_folder_tree();
                    json!({
                        "event": "fileTree",
                        "fileTree": tree,
                    })
                } else {
                    json!({ "event": "noop" })
                }
            }

            IpcCommand::CreateFileInFolder { name } => {
                let Some(root) = self.folder_root.clone() else {
                    return json!({ "event": "toast", "message": "Open a folder first", "kind": "error" });
                };
                let clean = name.trim();
                if !matches!(
                    Path::new(clean).components().collect::<Vec<_>>().as_slice(),
                    [Component::Normal(_)]
                ) || clean.contains(['/', '\\', ':'])
                {
                    return json!({ "event": "toast", "message": "Invalid file name", "kind": "error" });
                }
                let final_name = if clean.ends_with(".md") || clean.ends_with(".markdown") {
                    clean.to_string()
                } else {
                    format!("{clean}.md")
                };
                let target = root.join(final_name);
                if !target.exists() {
                    let heading = target
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Untitled".to_string());
                    if let Err(e) = std::fs::write(&target, format!("# {heading}\n\n")) {
                        return json!({ "event": "toast", "message": format!("Create failed: {e}"), "kind": "error" });
                    }
                }
                let _ = self.open_file_path(&target);
                let doc = self.active_doc_snapshot(200);
                let tree = self.scan_folder_tree();
                json!({
                    "event": "tabSwitched",
                    "tabs": self.tab_summaries(),
                    "activeTabId": self.active_doc().map(|d| d.id.clone()),
                    "doc": doc,
                    "fileTree": tree,
                })
            }

            IpcCommand::Save { save_as } => {
                let Some(idx) = self.active_tab else {
                    return json!({ "event": "noop" });
                };
                match self.save_document(idx, save_as, true) {
                    Ok(Some(changed_path)) => self.saved_response(idx, changed_path),
                    Ok(None) => json!({ "event": "noop" }),
                    Err(e) => error_toast(e),
                }
            }

            IpcCommand::Print { doc_id } => {
                let Some(doc) = self.active_doc_mut().filter(|doc| doc_id == doc.id) else {
                    return json!({ "event": "noop" });
                };
                json!({ "event": "printDocument", "docId": doc.id, "html": doc.render_full_html() })
            }

            IpcCommand::ExportHtml => {
                let Some(doc) = self.active_doc_mut() else {
                    return json!({ "event": "noop" });
                };
                let default_name = format!("{}.html", doc.title.trim_end_matches(".md"));
                let Some(dest) = rfd::FileDialog::new()
                    .set_file_name(&default_name)
                    .add_filter("HTML Document", &["html", "htm"])
                    .save_file()
                else {
                    return json!({ "event": "noop" });
                };
                let title = doc.title.clone();
                let body = doc.render_full_html();
                let html = assets::export_html_template(&title, &body, self.settings.light);
                match std::fs::write(&dest, html) {
                    Ok(()) => json!({
                        "event": "toast",
                        "message": format!("Exported HTML to {}", dest.file_name().unwrap_or_default().to_string_lossy()),
                    }),
                    Err(e) => {
                        json!({ "event": "toast", "message": format!("Export failed: {e}"), "kind": "error" })
                    }
                }
            }

            IpcCommand::Search {
                doc_id,
                request,
                query,
                case_sensitive,
                use_regex,
            } => {
                let Some(doc) = self.active_doc().filter(|doc| doc_id == doc.id) else {
                    return json!({ "event": "noop" });
                };
                let matches = doc.search(&query, case_sensitive, use_regex, 500);
                json!({
                    "event": "searchResults",
                    "docId": doc.id,
                    "version": doc.version,
                    "request": request,
                    "matches": matches,
                })
            }

            IpcCommand::ReplaceOne {
                doc_id,
                version,
                query,
                replacement,
                case_sensitive,
                use_regex,
                start_char,
                end_char,
                source,
            } => {
                let Some(doc) = self
                    .active_doc_mut()
                    .filter(|doc| doc_id == doc.id && version == doc.version)
                else {
                    return resync_response(&doc_id);
                };
                let search = SearchQuery {
                    query: &query,
                    case_sensitive,
                    use_regex,
                };
                match doc.replace_match(start_char, end_char, &search, &replacement) {
                    Some(patch) => patch_response(doc, patch, Some(source)),
                    None => resync_response(&doc_id),
                }
            }

            IpcCommand::ReplaceAll {
                doc_id,
                query,
                replacement,
                case_sensitive,
                use_regex,
            } => {
                let Some(doc) = self.active_doc_mut().filter(|doc| doc_id == doc.id) else {
                    return json!({ "event": "noop" });
                };
                let count = doc.replace_all(&query, &replacement, case_sensitive, use_regex);
                let snap = self.active_doc_snapshot(200);
                json!({
                    "event": "tabSwitched",
                    "tabs": self.tab_summaries(),
                    "activeTabId": self.active_doc().map(|d| d.id.clone()),
                    "doc": snap,
                    "toast": format!("Replaced {count} occurrences"),
                })
            }

            IpcCommand::UpdateSettings { patch } => {
                let mut need_wp = false;
                if let Some(v) = patch.language {
                    self.settings.language = v;
                }
                if let Some(v) = patch.light
                    && self.settings.light != v
                {
                    self.settings.light = v;
                    need_wp = true;
                }
                if let Some(v) = patch.theme_preset {
                    self.settings.theme_preset = v;
                }
                if let Some(v) = patch.view_mode {
                    self.settings.view_mode = v;
                }
                if let Some(v) = patch.font_size {
                    self.settings.font_size = v;
                }
                if let Some(v) = patch.preview_max_width {
                    self.settings.preview_max_width = v;
                }
                if let Some(v) = patch.line_height {
                    self.settings.line_height = v;
                }
                if let Some(v) = patch.sync_scroll {
                    self.settings.sync_scroll = v;
                }
                if let Some(v) = patch.focus_mode {
                    self.settings.focus_mode = v;
                }
                if let Some(v) = patch.typewriter_mode {
                    self.settings.typewriter_mode = v;
                }
                if let Some(v) = patch.auto_save {
                    self.settings.auto_save = v;
                    self.autosaves.clear();
                    if v {
                        for doc in &self.docs {
                            if doc.dirty && doc.path.is_some() {
                                self.autosaves.insert(
                                    doc.id.clone(),
                                    Instant::now() + Duration::from_millis(1500),
                                );
                            }
                        }
                    }
                }
                if let Some(v) = patch.show_line_numbers {
                    self.settings.show_line_numbers = v;
                }
                if let Some(v) = patch.reduce_motion {
                    self.settings.reduce_motion = v;
                }
                if let Some(v) = patch.sidebar_open {
                    self.settings.sidebar_open = v;
                }
                if let Some(v) = patch.sidebar_tab {
                    self.settings.sidebar_tab = v;
                }
                if let Some(v) = patch.split_ratio {
                    self.settings.split_ratio = v;
                }
                if let Some(v) = patch.wallpaper_effect
                    && self.settings.wallpaper_effect != v
                {
                    self.settings.wallpaper_effect = v;
                    need_wp = true;
                }
                if let Some(v) = patch.wallpaper_hero_opacity {
                    self.settings.wallpaper_hero_opacity = v;
                }
                if let Some(v) = patch.wallpaper_fills_window {
                    self.settings.wallpaper_fills_window = v;
                }
                if let Some(v) = patch.wallpaper_theme_colors {
                    self.settings.wallpaper_theme_colors = v;
                }

                self.settings.clamp();
                let _ = settings::save(&self.settings);
                if need_wp {
                    self.sync_wallpaper_now();
                }
                let wp = self.wallpaper_payload();
                json!({
                    "event": "settingsUpdated",
                    "settings": self.settings,
                    "wallpaper": wp,
                })
            }

            IpcCommand::SelectBuiltinWallpaper { id } => {
                if let (Some(b), Some(cfg)) = (gallery::find_builtin(&id), settings::config_dir()) {
                    match gallery::install_builtin(&cfg, b) {
                        Ok(path) => {
                            gallery::use_builtin(
                                &mut self.settings,
                                path.to_string_lossy().into_owned(),
                            );
                            let _ = settings::save(&self.settings);
                            self.sync_wallpaper_now();
                        }
                        Err(e) => self.wp_error = Some(e),
                    }
                }
                json!({
                    "event": "wallpaperUpdated",
                    "settings": self.settings,
                    "wallpaper": self.wallpaper_payload(),
                })
            }

            IpcCommand::SelectRecentWallpaper { path } => {
                if Path::new(&path).is_file() {
                    gallery::use_picked(&mut self.settings, path);
                    if let Some(cfg) = settings::config_dir() {
                        store::prune(&cfg, &self.settings.wallpaper_history);
                    }
                    let _ = settings::save(&self.settings);
                    self.sync_wallpaper_now();
                }
                json!({
                    "event": "wallpaperUpdated",
                    "settings": self.settings,
                    "wallpaper": self.wallpaper_payload(),
                })
            }

            IpcCommand::ClearWallpaper => {
                self.settings.wallpaper = None;
                self.prepared_wp = None;
                self.wp_error = None;
                let _ = settings::save(&self.settings);
                json!({
                    "event": "wallpaperUpdated",
                    "settings": self.settings,
                    "wallpaper": self.wallpaper_payload(),
                })
            }

            IpcCommand::PickWallpaperDialog => {
                let picked = rfd::FileDialog::new()
                    .set_title("Choose wallpaper")
                    .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                    .pick_file();
                if let (Some(src), Some(cfg)) = (picked, settings::config_dir()) {
                    match store::import(&src, &cfg) {
                        Ok(imported) => {
                            gallery::use_picked(
                                &mut self.settings,
                                imported.to_string_lossy().into_owned(),
                            );
                            store::prune(&cfg, &self.settings.wallpaper_history);
                            let _ = settings::save(&self.settings);
                            self.sync_wallpaper_now();
                        }
                        Err(e) => self.wp_error = Some(e),
                    }
                    json!({
                        "event": "wallpaperUpdated",
                        "settings": self.settings,
                        "wallpaper": self.wallpaper_payload(),
                    })
                } else {
                    json!({ "event": "noop" })
                }
            }
        }
    }
}

fn atomic_write(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    write(temp.as_file_mut())?;
    #[cfg(unix)]
    if let Ok(metadata) = std::fs::metadata(path) {
        temp.as_file().set_permissions(metadata.permissions())?;
    }
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

fn error_toast(message: String) -> Value {
    json!({ "event": "toast", "message": message, "kind": "error" })
}

fn resync_response(doc_id: &str) -> Value {
    json!({ "event": "resyncSource", "docId": doc_id })
}

fn patch_response(doc: &Document, patch: EditPatch, source: Option<SourceRequest>) -> Value {
    let source = source.map(|req| {
        source_window(
            doc,
            req.start_line,
            req.count.unwrap_or(doc.rope.len_lines()),
        )
    });
    json!({ "event": "patch", "docId": doc.id, "dirty": doc.dirty, "patch": patch, "source": source })
}

fn source_window(doc: &Document, start: usize, count: usize) -> Value {
    let total = doc.rope.len_lines();
    let start = start.min(total - 1);
    let end = start.saturating_add(count).min(total);
    let text = if end == total {
        doc.rope.slice(doc.rope.line_to_char(start)..).to_string()
    } else {
        doc.slice_lines(start, end)
    };
    let context_line = doc
        .blocks
        .get(doc.blocks.partition_point(|b| b.end_line <= start))
        .filter(|b| {
            b.start_line < start
                && matches!(
                    b.kind,
                    crate::engine::BlockKind::CodeFence
                        | crate::engine::BlockKind::MathBlock
                        | crate::engine::BlockKind::FrontMatter
                )
        })
        .map(|b| doc.line_str(b.start_line));
    json!({ "startLine": start, "endLine": end, "totalLines": total, "text": text, "contextLine": context_line })
}

fn build_file_tree(dir: &Path, depth: usize) -> FileNode {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.to_string_lossy().into_owned());
    let mut children = Vec::new();
    if depth < 4
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let p = entry.path();
            let fname = entry.file_name().to_string_lossy().into_owned();
            if fname.starts_with('.') || fname == "target" || fname == "node_modules" {
                continue;
            }
            if p.is_dir() {
                dirs.push(build_file_tree(&p, depth + 1));
            } else if p.is_file() {
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase);
                if matches!(
                    ext.as_deref(),
                    Some("md" | "markdown" | "mdown" | "mkd" | "txt" | "tex")
                ) {
                    files.push(FileNode {
                        name: fname,
                        path: p.to_string_lossy().into_owned(),
                        is_dir: false,
                        children: Vec::new(),
                    });
                }
            }
        }
        dirs.sort_by_key(|a| a.name.to_lowercase());
        files.sort_by_key(|a| a.name.to_lowercase());
        children.extend(dirs);
        children.extend(files);
    }
    FileNode {
        name,
        path: dir.to_string_lossy().into_owned(),
        is_dir: true,
        children,
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum IpcCommand {
    Init,
    Edit {
        doc_id: String,
        #[serde(default)]
        base_line: usize,
        start_char: usize,
        end_char: usize,
        text: String,
        source: Option<SourceRequest>,
        #[serde(default)]
        utf16: bool,
    },
    ToggleTask {
        doc_id: String,
        version: u64,
        block_id: u64,
        line_offset: usize,
        source: SourceRequest,
    },
    RenderLiveWidgets {
        doc_id: String,
        request: u64,
        fragments: Vec<String>,
    },
    FetchBlocks {
        doc_id: String,
        start: usize,
        count: usize,
        request: u64,
        refresh: bool,
        around_line: Option<usize>,
    },
    FetchLines {
        doc_id: String,
        start_line: usize,
        count: usize,
        request: u64,
    },
    NewTab,
    SwitchTab {
        id: String,
    },
    CloseTab {
        id: String,
    },
    ResolveClose {
        doc_id: String,
        choice: CloseChoice,
    },
    OpenFileDialog,
    OpenPath {
        path: String,
    },
    OpenFolderDialog,
    CreateFileInFolder {
        name: String,
    },
    Save {
        #[serde(rename = "saveAs")]
        save_as: bool,
    },
    ExportHtml,
    Print {
        doc_id: String,
    },
    Search {
        doc_id: String,
        request: u64,
        query: String,
        #[serde(rename = "caseSensitive")]
        case_sensitive: bool,
        #[serde(rename = "useRegex")]
        use_regex: bool,
    },
    ReplaceOne {
        doc_id: String,
        version: u64,
        query: String,
        replacement: String,
        case_sensitive: bool,
        use_regex: bool,
        start_char: usize,
        end_char: usize,
        source: SourceRequest,
    },
    ReplaceAll {
        doc_id: String,
        query: String,
        replacement: String,
        #[serde(rename = "caseSensitive")]
        case_sensitive: bool,
        #[serde(rename = "useRegex")]
        use_regex: bool,
    },
    UpdateSettings {
        patch: SettingsPatch,
    },
    SelectBuiltinWallpaper {
        id: String,
    },
    SelectRecentWallpaper {
        path: String,
    },
    ClearWallpaper,
    PickWallpaperDialog,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceRequest {
    start_line: usize,
    count: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPatch {
    language: Option<String>,
    light: Option<bool>,
    theme_preset: Option<ThemePreset>,
    view_mode: Option<ViewMode>,
    font_size: Option<f32>,
    preview_max_width: Option<u32>,
    line_height: Option<f32>,
    sync_scroll: Option<bool>,
    focus_mode: Option<bool>,
    typewriter_mode: Option<bool>,
    auto_save: Option<bool>,
    show_line_numbers: Option<bool>,
    reduce_motion: Option<bool>,
    sidebar_open: Option<bool>,
    sidebar_tab: Option<String>,
    split_ratio: Option<f32>,
    wallpaper_effect: Option<Effect>,
    wallpaper_hero_opacity: Option<f32>,
    wallpaper_fills_window: Option<bool>,
    wallpaper_theme_colors: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(docs: Vec<Document>) -> AppState {
        AppState {
            settings: Settings {
                auto_save: true,
                ..Settings::default()
            },
            docs,
            active_tab: Some(0),
            next_doc_num: 3,
            prepared_wp: None,
            wp_error: None,
            folder_root: None,
            autosaves: HashMap::new(),
            pending_close: None,
        }
    }

    fn saved_document(path: &Path) -> Document {
        std::fs::write(path, "original").unwrap();
        Document::new(
            "first".into(),
            "first.md".into(),
            Some(dunce::canonicalize(path).unwrap()),
            "original",
        )
    }

    #[test]
    fn live_widgets_resolve_document_relative_images_and_references() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("first.md");
        std::fs::write(directory.path().join("chart.svg"), "<svg/>").unwrap();
        let doc = Document::new(
            "first".into(),
            "first.md".into(),
            Some(path),
            "[chart]: chart.svg\n[example]: https://example.com/\n",
        );
        let mut state = test_state(vec![doc]);
        let response = state.handle_command(
            r#"{"cmd":"renderLiveWidgets","docId":"first","request":10,"fragments":["![A chart][chart]","[Click here][example]"]}"#,
        );
        assert_eq!(response["event"], "liveWidgetsRendered");
        assert_eq!(response["request"], 10);
        assert!(
            response["html"][0]
                .as_str()
                .unwrap()
                .contains("/local-image/")
        );
        assert!(
            response["html"][1]
                .as_str()
                .unwrap()
                .contains("https://example.com/")
        );
    }

    #[test]
    fn large_live_snapshot_defers_split_blocks_until_requested() {
        let doc = Document::new(
            "first".into(),
            "first.md".into(),
            None,
            &"Paragraph\n\n".repeat(4000),
        );
        let mut state = test_state(vec![doc]);
        state.settings.view_mode = ViewMode::Live;
        let live = state.active_doc_snapshot(200);
        assert_eq!(live["previewDeferred"], true);
        assert_eq!(live["blocks"].as_array().unwrap().len(), 0);
        assert_eq!(live["windowEndLine"], 600);
        let split = state.handle_command(r#"{"cmd":"fetchBlocks","docId":"first","start":0,"count":200,"request":10,"refresh":true,"aroundLine":0}"#);
        assert_eq!(split["blocks"].as_array().unwrap().len(), 200);
    }

    #[test]
    fn autosave_writes_the_edited_tab_after_switching_to_another() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("first.md");
        let mut state = test_state(vec![saved_document(&path)]);
        let edit = json!({
            "cmd": "edit", "docId": "first", "baseLine": 0,
            "startChar": 0, "endChar": 8, "text": "saved",
            "source": {"startLine": 0, "count": null},
        });
        assert_eq!(state.handle_command(&edit.to_string())["event"], "patch");
        state.new_untitled_doc();
        let events = state.run_autosaves(Instant::now() + Duration::from_secs(2));
        assert_eq!(events[0]["docId"], "first");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved");
        assert!(!state.docs[0].dirty);
        assert!(state.active_doc().unwrap().path.is_none());
    }

    #[test]
    fn closing_a_dirty_tab_requires_a_decision_for_that_document() {
        let mut doc = Document::new("first".into(), "first.md".into(), None, "original");
        doc.apply_edit(0, 8, "edited", false);
        let mut state = test_state(vec![doc]);
        let close = r#"{"cmd":"closeTab","id":"first"}"#;
        assert_eq!(state.handle_command(close)["event"], "confirmClose");
        assert_eq!(
            state.resolve_close("other", CloseChoice::Discard)["event"],
            "noop"
        );
        assert_eq!(state.active_doc().unwrap().full_text(), "edited");
        assert_eq!(
            state.resolve_close("first", CloseChoice::Cancel)["event"],
            "tabsUpdated"
        );
        assert_eq!(state.docs.len(), 1);
        assert_eq!(state.handle_command(close)["docId"], "first");
        assert_eq!(
            state.resolve_close("first", CloseChoice::Discard)["event"],
            "tabSwitched"
        );
        assert!(state.docs.is_empty());
    }

    #[test]
    fn closing_the_window_pauses_autosave_and_cancelling_keeps_all_tabs() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("first.md");
        let mut first = saved_document(&path);
        first.apply_edit(0, 8, "saved changes", false);
        let mut second = Document::new("second".into(), "second.md".into(), None, "");
        second.apply_edit(0, 0, "another change", false);
        let mut state = test_state(vec![first, second]);
        let now = Instant::now();
        state.autosaves.insert("first".into(), now);

        assert_eq!(state.request_window_close()["docId"], "first");
        assert_eq!(
            state.resolve_close("first", CloseChoice::Discard)["docId"],
            "second"
        );
        assert!(state.next_autosave().is_none());
        assert!(state.run_autosaves(now).is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(
            state.resolve_close("second", CloseChoice::Cancel)["event"],
            "tabsUpdated"
        );
        assert_eq!(state.docs.len(), 2);
        assert!(state.docs.iter().all(|doc| doc.dirty));

        assert_eq!(state.request_window_close()["docId"], "first");
        assert_eq!(
            state.resolve_close("first", CloseChoice::Save)["docId"],
            "second"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved changes");
        assert_eq!(
            state.resolve_close("second", CloseChoice::Discard)["event"],
            "exitRequested"
        );
    }
}
