#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{ResizeDirection, Window, WindowBuilder};
use wry::http::Response;
use wry::{DragDropEvent, NewWindowResponse, WebViewBuilder};

use marktype::assets;
use marktype::ipc::AppState;
use marktype::latex;
use marktype::markdown::is_external_link;
use marktype::synctex::{self, SyncTex};

enum UserEvent {
    IpcMessage(String),
    FileDropped(PathBuf),
    WorkspaceResults(serde_json::Value),
    LatexBuilt(latex::BuildResult),
    LatexProgress(String),
    SyncTexResult(SyncQuery, SyncAnswer),
    #[cfg(target_os = "macos")]
    NativeMenu(String),
}

type SyncTexStamp = Option<(PathBuf, Option<std::time::SystemTime>, u64)>;

enum SyncPosition {
    Source { path: PathBuf, line: u32 },
    Pdf { page: u32, x: f64, y: f64 },
}

struct SyncQuery {
    request: u64,
    revision: u64,
    doc_id: String,
    main: PathBuf,
    pdf: PathBuf,
    position: SyncPosition,
}

enum SyncAnswer {
    Forward(Result<synctex::ForwardSync, String>),
    Inverse(Result<synctex::InverseSync, String>),
}

enum SyncWork {
    Preload { revision: u64, pdf: PathBuf },
    Query(SyncQuery),
}

#[derive(Default)]
struct LatexJob {
    running: bool,
    pdf: Option<PathBuf>,
    sync_sender: Option<mpsc::Sender<SyncWork>>,
    sync_request: u64,
    revision: u64,
}

fn synctex_file_stamp(pdf: &Path) -> SyncTexStamp {
    let path = synctex::synctex_path_for_pdf(pdf)?;
    let meta = std::fs::metadata(&path).ok()?;
    Some((path, meta.modified().ok(), meta.len()))
}

fn synctex_worker(
    requests: mpsc::Receiver<SyncWork>,
    proxy: tao::event_loop::EventLoopProxy<UserEvent>,
) {
    let mut cache = None;
    while let Ok(mut work) = requests.recv() {
        for pending in requests.try_iter() {
            if matches!(work, SyncWork::Preload { .. }) || matches!(pending, SyncWork::Query(_)) {
                work = pending;
            }
        }
        let (revision, pdf) = match &work {
            SyncWork::Preload { revision, pdf } => (*revision, pdf),
            SyncWork::Query(query) => (query.revision, &query.pdf),
        };
        let key = (revision, pdf.clone(), synctex_file_stamp(pdf));
        if matches!(work, SyncWork::Preload { .. })
            && key
                .2
                .as_ref()
                .is_none_or(|(_, _, size)| *size > 4 * 1024 * 1024)
        {
            continue;
        }
        if cache
            .as_ref()
            .is_some_and(|(cached_key, _)| *cached_key != key)
        {
            cache = None;
        }
        let (_, parsed) = cache.get_or_insert_with(|| (key, SyncTex::load_for_pdf(pdf)));
        let SyncWork::Query(query) = work else {
            continue;
        };
        let answer = match &query.position {
            SyncPosition::Source { path, line } => {
                SyncAnswer::Forward(parsed.as_ref().map_err(Clone::clone).and_then(|data| {
                    data.forward(path, *line)
                        .ok_or_else(|| "No SyncTeX location matched this source line.".to_owned())
                }))
            }
            SyncPosition::Pdf { page, x, y } => {
                SyncAnswer::Inverse(parsed.as_ref().map_err(Clone::clone).and_then(|data| {
                    data.inverse(*page, *x, *y)
                        .ok_or_else(|| "No source line matched this PDF position.".to_owned())
                }))
            }
        };
        if proxy
            .send_event(UserEvent::SyncTexResult(query, answer))
            .is_err()
        {
            break;
        }
    }
}

impl LatexJob {
    fn query_sync(&mut self, query: SyncQuery, proxy: &tao::event_loop::EventLoopProxy<UserEvent>) {
        self.sync_request = query.request;
        self.send_sync_work(SyncWork::Query(query), proxy);
    }

    fn preload_sync(&mut self, pdf: PathBuf, proxy: &tao::event_loop::EventLoopProxy<UserEvent>) {
        self.send_sync_work(
            SyncWork::Preload {
                revision: self.revision,
                pdf,
            },
            proxy,
        );
    }

    fn send_sync_work(
        &mut self,
        work: SyncWork,
        proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    ) {
        let sender = self.sync_sender.get_or_insert_with(|| {
            let (sender, receiver) = mpsc::channel();
            let proxy = proxy.clone();
            std::thread::spawn(move || synctex_worker(receiver, proxy));
            sender
        });
        let _ = sender.send(work);
    }

    fn active_pdf(state: &AppState) -> Option<(PathBuf, PathBuf)> {
        let main = state.active_latex_main()?;
        let pdf = main.with_extension("pdf");
        pdf.is_file().then_some((main, pdf))
    }

    fn prepare(state: &mut AppState, values: &serde_json::Value) -> Result<latex::Build, String> {
        let main = values["main"]
            .as_str()
            .map(PathBuf::from)
            .ok_or("Choose a LaTeX main file first.")?;
        let engine = serde_json::from_value(values["engine"].clone())
            .map_err(|_| "Choose a valid LaTeX engine.")?;
        let main = latex::main_file(&main)?;
        // Detect the compiler before writing project files. Re-read any engine hint after saving.
        let build = latex::Build::prepare(main.clone(), engine)?;
        state.save_latex_sources(&main)?;
        if engine == latex::Engine::Auto {
            latex::Build::prepare(main, engine)
        } else {
            Ok(build)
        }
    }
}

fn apply_inverse_sync(
    state: &mut AppState,
    webview: &wry::WebView,
    main: PathBuf,
    target: synctex::InverseSync,
) -> Result<synctex::InverseSync, String> {
    let path = dunce::canonicalize(&target.path)
        .ok()
        .filter(|path| path.is_file())
        .ok_or_else(|| "The SyncTeX target file could not be found on disk.".to_owned())?;
    let in_scope = latex::project_source(&path)
        && (main.parent().is_some_and(|dir| path.starts_with(dir))
            || state
                .folder_root
                .as_deref()
                .and_then(|dir| dunce::canonicalize(dir).ok())
                .is_some_and(|dir| path.starts_with(dir)));
    if !in_scope {
        return Err("SyncTeX target is outside the LaTeX project folder.".into());
    }
    state.set_latex_main(&path, main);
    if state.active_doc().and_then(|doc| doc.path.as_deref()) != Some(path.as_path()) {
        state.open_file_path(&path)?;
        let doc = state.active_doc_snapshot(200);
        dispatch_json(
            webview,
            &serde_json::json!({
                "event": "tabSwitched",
                "tabs": state.tab_summaries(),
                "activeTabId": state.active_doc().map(|doc| &doc.id),
                "doc": doc,
                "settings": state.settings,
            }),
        );
    }
    Ok(synctex::InverseSync { path, ..target })
}

#[cfg(target_os = "macos")]
fn install_native_menu(proxy: tao::event_loop::EventLoopProxy<UserEvent>) -> muda::Menu {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

    let item = |id: &str, text: &str, key, shift: bool| {
        let modifiers = if shift {
            Modifiers::SUPER | Modifiers::SHIFT
        } else {
            Modifiers::SUPER
        };
        MenuItem::with_id(id, text, true, Some(Accelerator::new(Some(modifiers), key)))
    };
    let application = Submenu::with_items(
        "MarkType",
        true,
        &[
            &MenuItem::with_id("about", "About MarkType", true, None),
            &item("settings", "Settings…", Code::Comma, false),
            &PredefinedMenuItem::separator(),
            &item("quit", "Quit MarkType", Code::KeyQ, false),
        ],
    )
    .expect("Create application menu");
    let file = Submenu::with_items(
        "File",
        true,
        &[
            &item("btn-new", "New", Code::KeyN, false),
            &item("btn-open", "Open…", Code::KeyO, false),
            &MenuItem::with_id("menu-open-folder", "Open Folder…", true, None),
            &PredefinedMenuItem::separator(),
            &item("btn-save", "Save", Code::KeyS, false),
            &item("btn-save-as", "Save As…", Code::KeyS, true),
            &item("btn-print", "Print…", Code::KeyP, false),
            &MenuItem::with_id("btn-latex", "Compile LaTeX…", true, None),
            &item("menu-close-tab", "Close Tab", Code::KeyW, false),
        ],
    )
    .expect("Create file menu");
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &item("undo", "Undo", Code::KeyZ, false),
            &item("redo", "Redo", Code::KeyZ, true),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
            &item("menu-find", "Find", Code::KeyF, false),
            &item("workspace-search", "Search Workspace", Code::KeyF, true),
        ],
    )
    .expect("Create edit menu");
    let view = Submenu::with_items(
        "View",
        true,
        &[
            &item("font-plus", "Increase Font Size", Code::Equal, false),
            &item("font-minus", "Decrease Font Size", Code::Minus, false),
            &item("font-reset", "Reset Font Size", Code::Digit0, false),
        ],
    )
    .expect("Create view menu");
    let menu =
        Menu::with_items(&[&application, &file, &edit, &view]).expect("Create native menu bar");
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = proxy.send_event(UserEvent::NativeMenu(event.id.0));
    }));
    menu.init_for_nsapp();
    menu
}

#[cfg(target_os = "windows")]
fn configure_windows_frameless_chrome(window: &tao::window::Window, dark: bool) {
    use tao::platform::windows::WindowExtWindows;

    #[repr(C)]
    struct Margins {
        left: i32,
        right: i32,
        top: i32,
        bottom: i32,
    }

    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            dw_attribute: u32,
            pv_attribute: *const i32,
            cb_attribute: u32,
        ) -> i32;
        fn DwmExtendFrameIntoClientArea(hwnd: isize, margins: *const Margins) -> i32;
    }

    let hwnd = window.hwnd();
    let dark_val: i32 = i32::from(dark);
    let round_pref: i32 = 2; // DWMWCP_ROUND (Windows 11 rounded corners)
    let margins = Margins {
        left: 1,
        right: 1,
        top: 1,
        bottom: 1,
    };
    unsafe {
        // 20 = DWMWA_USE_IMMERSIVE_DARK_MODE
        let _ = DwmSetWindowAttribute(hwnd, 20, &dark_val, std::mem::size_of::<i32>() as u32);
        // 33 = DWMWA_WINDOW_CORNER_PREFERENCE
        let _ = DwmSetWindowAttribute(hwnd, 33, &round_pref, std::mem::size_of::<i32>() as u32);
        // Native DWM drop shadow & crisp 1px window border
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}

#[cfg(not(target_os = "windows"))]
fn configure_windows_frameless_chrome(_window: &tao::window::Window, _dark: bool) {}

fn dispatch_json(webview: &wry::WebView, val: &serde_json::Value) {
    if let Ok(mut json) = serde_json::to_string(val) {
        if json.contains('\u{2028}') {
            json = json.replace('\u{2028}', "\\u2028");
        }
        if json.contains('\u{2029}') {
            json = json.replace('\u{2029}', "\\u2029");
        }
        let script = format!("window.__mtReceive && window.__mtReceive({json});");
        let _ = webview.evaluate_script(&script);
    }
}

fn handle_response(
    webview: &wry::WebView,
    control_flow: &mut ControlFlow,
    response: &serde_json::Value,
) {
    match response.get("event").and_then(|event| event.as_str()) {
        Some("exitRequested") => *control_flow = ControlFlow::Exit,
        Some("noop") | None => {}
        _ => dispatch_json(webview, response),
    }
}

fn parse_resize_dir(dir: &str) -> Option<ResizeDirection> {
    Some(match dir {
        "n" => ResizeDirection::North,
        "s" => ResizeDirection::South,
        "e" => ResizeDirection::East,
        "w" => ResizeDirection::West,
        "ne" => ResizeDirection::NorthEast,
        "nw" => ResizeDirection::NorthWest,
        "se" => ResizeDirection::SouthEast,
        "sw" => ResizeDirection::SouthWest,
        _ => return None,
    })
}

fn is_editor_url(url: &str) -> bool {
    let url = url.split('#').next().unwrap_or(url);
    matches!(url.strip_prefix(assets::APP_URL), Some("" | "index.html"))
}

#[cfg(target_os = "windows")]
fn with_app_icon(builder: WindowBuilder) -> WindowBuilder {
    use tao::platform::windows::{IconExtWindows, WindowBuilderExtWindows};
    use tao::window::Icon;

    let icon = Icon::from_resource(1, None).expect("Load the embedded MarkType icon");
    builder
        .with_window_icon(Some(icon.clone()))
        .with_taskbar_icon(Some(icon))
}

#[cfg(target_os = "linux")]
fn with_app_icon(builder: WindowBuilder) -> WindowBuilder {
    let icon = image::load_from_memory(include_bytes!("../packaging/windows/icon-256.png"))
        .expect("Decode the bundled MarkType icon")
        .into_rgba8();
    let (width, height) = icon.dimensions();
    builder.with_window_icon(Some(
        tao::window::Icon::from_rgba(icon.into_raw(), width, height)
            .expect("Load the bundled MarkType icon"),
    ))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn with_app_icon(builder: WindowBuilder) -> WindowBuilder {
    builder
}

#[cfg(target_os = "linux")]
fn build_webview(builder: WebViewBuilder<'_>, window: &Window) -> wry::Result<wry::WebView> {
    use tao::platform::unix::WindowExtUnix;
    use wry::WebViewBuilderExtUnix;

    builder.build_gtk(window.default_vbox().expect("Get the GTK window content"))
}

#[cfg(not(target_os = "linux"))]
fn build_webview(builder: WebViewBuilder<'_>, window: &Window) -> wry::Result<wry::WebView> {
    builder.build(window)
}

fn remember_window(window: &Window, state: &mut AppState) {
    if window.is_minimized() {
        return;
    }
    let previous = state.settings.window_geometry;
    let mut geometry = previous;
    geometry.maximized = window.is_maximized();
    if !geometry.maximized {
        let size = window.inner_size().to_logical(window.scale_factor());
        geometry.size = [size.width, size.height];
        geometry.position = window
            .outer_position()
            .ok()
            .map(|position| [position.x, position.y]);
    }
    if geometry != previous {
        state.settings.window_geometry = geometry;
        let _ = marktype::settings::save(&state.settings);
    }
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    let smoke = first.as_deref().is_some_and(|arg| arg == "--smoke-test");
    let initial_file = if smoke { args.next() } else { first }
        .map(PathBuf::from)
        .filter(|path| path.is_file());
    let smoke_deadline = smoke.then(|| Instant::now() + Duration::from_secs(40));
    let mut state = AppState::new(initial_file);

    let mut event_loop_builder = EventLoopBuilder::<UserEvent>::with_user_event();
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::EventLoopBuilderExtUnix;
        event_loop_builder.with_app_id("com.marktype.MarkType");
    }
    let event_loop = event_loop_builder.build();
    let proxy_ipc = event_loop.create_proxy();
    let proxy_drop = event_loop.create_proxy();
    let proxy_search = event_loop.create_proxy();
    let proxy_latex = event_loop.create_proxy();
    let workspace_query = Arc::new(AtomicU64::new(0));

    let geometry = state.settings.window_geometry;
    let mut window_builder = with_app_icon(WindowBuilder::new())
        .with_title("MarkType")
        .with_decorations(!cfg!(target_os = "windows"))
        .with_inner_size(LogicalSize::new(geometry.size[0], geometry.size[1]))
        .with_min_inner_size(LogicalSize::new(820.0, 560.0))
        .with_maximized(geometry.maximized);
    if let Some([x, y]) = geometry.position
        && event_loop.available_monitors().any(|monitor| {
            let origin = monitor.position();
            let size = monitor.size();
            let dx = i64::from(x) - i64::from(origin.x);
            let dy = i64::from(y) - i64::from(origin.y);
            dx >= -32
                && dx < i64::from(size.width) - 32
                && dy >= -32
                && dy < i64::from(size.height) - 32
        })
    {
        window_builder = window_builder.with_position(PhysicalPosition::new(x, y));
    }
    let window = window_builder
        .build(&event_loop)
        .expect("Failed to create MarkType window");

    #[cfg(target_os = "macos")]
    let native_menu = install_native_menu(event_loop.create_proxy());
    configure_windows_frameless_chrome(&window, !state.settings.light);

    let builder = WebViewBuilder::new()
        .with_custom_protocol("marktype".into(), |_webview_id, request| {
            let uri = request.uri().to_string();
            let (status, mime, body) = assets::serve_request(&uri);
            let csp = if mime.starts_with("text/html") {
                assets::APP_CSP
            } else if mime.starts_with("application/javascript") {
                "default-src 'none'; script-src 'self'"
            } else {
                "default-src 'none'; sandbox"
            };
            Response::builder()
                .status(status)
                .header("Content-Type", mime)
                .header("Cache-Control", "no-cache")
                .header("Content-Security-Policy", csp)
                .header("X-Content-Type-Options", "nosniff")
                .body(body.into())
                .unwrap()
        })
        .with_url(assets::APP_URL)
        .with_navigation_handler(|url| is_editor_url(&url))
        .with_new_window_req_handler(|url, _| {
            let is_app_resource = url.to_ascii_lowercase().starts_with(assets::APP_URL);
            if is_external_link(&url) && !is_app_resource {
                let _ = open::that_detached(url);
            }
            NewWindowResponse::Deny
        })
        .with_ipc_handler(move |req| {
            if is_editor_url(&req.uri().to_string()) {
                let _ = proxy_ipc.send_event(UserEvent::IpcMessage(req.into_body()));
            }
        })
        .with_drag_drop_handler(move |ev| {
            if let DragDropEvent::Drop { paths, .. } = ev {
                for p in paths {
                    if p.is_file() {
                        let _ = proxy_drop.send_event(UserEvent::FileDropped(p));
                    }
                }
            }
            true
        })
        .with_clipboard(true)
        .with_devtools(cfg!(debug_assertions));
    let builder = if smoke {
        builder.with_initialization_script(include_str!("../tests/fixtures/desktop-smoke.js"))
    } else {
        builder
    };
    let webview = build_webview(builder, &window).expect("Initialize the platform WebView");

    let mut geometry_at = None;
    let mut latex_job = LatexJob::default();
    event_loop.run(move |event, _, control_flow| {
        #[cfg(target_os = "macos")]
        let _keep_menu_alive = &native_menu;
        if smoke_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            eprintln!("WebView smoke test timed out");
            *control_flow = ControlFlow::ExitWithCode(1);
            return;
        }
        *control_flow = state
            .next_maintenance()
            .into_iter().chain(smoke_deadline).chain(geometry_at).min()
            .map_or(ControlFlow::Wait, ControlFlow::WaitUntil);

        match event {
            #[cfg(target_os = "macos")]
            Event::UserEvent(UserEvent::NativeMenu(id)) => {
                dispatch_json(&webview, &serde_json::json!({ "event": "nativeMenu", "id": id }));
            }
            Event::UserEvent(UserEvent::IpcMessage(body)) => {
                // Fast-path native frameless window chrome commands
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body)
                    && let Some(cmd) = v.get("cmd").and_then(|c| c.as_str())
                {
                    match cmd {
                        "latexProject" => {
                            let main = state.active_latex_main();
                            dispatch_json(&webview, &serde_json::json!({"event":"latexProject","main":main,"request":v["request"]}));
                            return;
                        }
                        "chooseLatexMain" => {
                            let mut dialog = rfd::FileDialog::new().add_filter("LaTeX main file", &["tex", "ltx"]);
                            if let Some(folder) = state.active_doc().and_then(|doc| doc.path.as_deref())
                                .and_then(std::path::Path::parent).or(state.folder_root.as_deref()) {
                                dialog = dialog.set_directory(folder);
                            }
                            if let Some(path) = dialog.pick_file() {
                                let response = match latex::main_file(&path) {
                                    Ok(main) => {
                                        if let Some(path) = state.active_doc().and_then(|doc| doc.path.clone()) {
                                            state.set_latex_main(&path, main.clone());
                                        }
                                        serde_json::json!({"event":"latexProject","main":main,"request":v["request"]})
                                    }
                                    Err(error) => serde_json::json!({"event":"latexProject","error":error,"request":v["request"]}),
                                };
                                dispatch_json(&webview, &response);
                            }
                            return;
                        }
                        "compileLatex" => {
                            if latex_job.running { return; }
                            match LatexJob::prepare(&mut state, &v) {
                                Ok(build) => {
                                    latex_job.running = true;
                                    if let Some(path) = state.active_doc().and_then(|doc| doc.path.clone()) {
                                        state.set_latex_main(&path, build.main.clone());
                                    }
                                    dispatch_json(&webview, &serde_json::json!({"event":"latexStarted","main":build.main,
                                        "docId":state.active_doc().map(|doc| &doc.id),"compiler":build.compiler(),"tabs":state.tab_summaries()}));
                                    let proxy = proxy_latex.clone();
                                    let main_for_panic = build.main.clone();
                                    let compiler_for_panic = build.compiler();
                                    std::thread::spawn(move || {
                                        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                            build.run(|log| {
                                                let _ = proxy.send_event(UserEvent::LatexProgress(log));
                                            })
                                        }));
                                        let result = outcome.unwrap_or_else(|_| latex::BuildResult {
                                            main: main_for_panic,
                                            compiler: compiler_for_panic,
                                            pdf: None,
                                            synctex: false,
                                            log: String::new(),
                                            error: Some("LaTeX compiler worker terminated unexpectedly.".into()),
                                        });
                                        let _ = proxy.send_event(UserEvent::LatexBuilt(result));
                                    });
                                }
                                Err(error) => dispatch_json(&webview, &serde_json::json!({"event":"latexFinished","error":error,
                                    "log":"","tabs":state.tab_summaries()})),
                            }
                            return;
                        }
                        "openLatexPdf" => {
                            let target = v["path"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .map(PathBuf::from)
                                .filter(|path| assets::is_registered_pdf(path))
                                .or_else(|| {
                                    latex_job
                                        .pdf
                                        .clone()
                                        .filter(|path| assets::is_registered_pdf(path) || path.is_file())
                                });
                            if let Some(path) = target && let Err(error) = open::that_detached(&path) {
                                dispatch_json(&webview, &serde_json::json!({"event":"toast","message":format!("Cannot open PDF: {error}")}));
                            }
                            return;
                        }
                        "openPdfPreview" => {
                            let Some((main, pdf)) = LatexJob::active_pdf(&state) else {
                                dispatch_json(&webview, &serde_json::json!({
                                    "event": "pdfError",
                                    "request": v["request"],
                                    "message": "No compiled PDF found for this LaTeX project yet. Compile first."
                                }));
                                return;
                            };
                            let Some(url) = assets::register_pdf(&pdf) else {
                                dispatch_json(&webview, &serde_json::json!({
                                    "event": "pdfError",
                                    "request": v["request"],
                                    "message": "Cannot read the compiled PDF file."
                                }));
                                return;
                            };
                            latex_job.pdf = Some(pdf.clone());
                            let has_synctex = synctex::synctex_path_for_pdf(&pdf).is_some();
                            if has_synctex {
                                latex_job.preload_sync(pdf.clone(), &proxy_latex);
                            }
                            let title = pdf
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "PDF".to_owned());
                            dispatch_json(&webview, &serde_json::json!({
                                "event": "pdfLoaded",
                                "request": v["request"],
                                "main": main,
                                "url": url,
                                "path": pdf,
                                "title": title,
                                "hasSyncTex": has_synctex,
                                "preserveScroll": v["preserveScroll"].as_bool().unwrap_or(false),
                            }));
                            return;
                        }
                        "synctexCancel" => {
                            if v["request"].as_u64() == Some(latex_job.sync_request) {
                                latex_job.sync_request = 0;
                            }
                            return;
                        }
                        "synctexForward" | "synctexInverse" => {
                            let inverse = cmd == "synctexInverse";
                            let event = if inverse { "synctexInverseResult" } else { "synctexForwardResult" };
                            let Some(request) = v["request"].as_u64().filter(|&value| value != 0) else { return; };
                            let active = LatexJob::active_pdf(&state).filter(|(_, pdf)| {
                                !inverse || v["pdf"].as_str().is_some_and(|path| pdf == Path::new(path))
                            });
                            let Some(((main, pdf), doc)) = active.zip(state.active_doc()) else {
                                dispatch_json(&webview, &serde_json::json!({
                                    "event": event, "request": request,
                                    "error": "No compiled PDF is open for SyncTeX."
                                }));
                                return;
                            };
                            let number = |field: &str| v[field].as_u64().unwrap_or(1).clamp(1, u64::from(u32::MAX)) as u32;
                            let position = if inverse {
                                SyncPosition::Pdf {
                                    page: number("page"),
                                    x: v["x"].as_f64().unwrap_or(0.0),
                                    y: v["y"].as_f64().unwrap_or(0.0),
                                }
                            } else {
                                SyncPosition::Source {
                                    path: doc.path.clone().expect("active LaTeX source has a path"),
                                    line: number("line"),
                                }
                            };
                            latex_job.query_sync(SyncQuery {
                                request, revision: latex_job.revision, doc_id: doc.id.clone(),
                                main, pdf, position,
                            }, &proxy_latex);
                            return;
                        }
                        "workspaceSearch" => {
                            if let Ok(request) = serde_json::from_value::<marktype::workspace::SearchRequest>(v) {
                                workspace_query.store(request.request, Ordering::Relaxed);
                                if request.query.is_empty() { return; }
                                if let Some(root) = state.folder_root.clone() {
                                    let latest = Arc::clone(&workspace_query);
                                    let proxy = proxy_search.clone();
                                    std::thread::spawn(move || {
                                        let id = request.request;
                                        let response = marktype::workspace::search(&root, request, &latest);
                                        if latest.load(Ordering::Relaxed) == id { let _ = proxy.send_event(UserEvent::WorkspaceResults(response)); }
                                    });
                                } else {
                                    dispatch_json(&webview, &serde_json::json!({"event":"workspaceSearchResults","request":request.request,
                                        "error":"Open a workspace folder first"}));
                                }
                            }
                            return;
                        }
                        "smokeResult" if smoke => {
                            if let Some(error) = v.get("error").and_then(|value| value.as_str()) {
                                eprintln!("WebView smoke test failed: {error}");
                                *control_flow = ControlFlow::ExitWithCode(1);
                            } else {
                                println!("WebView loaded the document, local assets and KaTeX, then rendered an edit.");
                                *control_flow = ControlFlow::Exit;
                            }
                            return;
                        }
                        "windowDrag" => {
                            let _ = window.drag_window();
                            return;
                        }
                        "windowDragResize" => {
                            if let Some(dir) = v
                                .get("dir")
                                .and_then(|d| d.as_str())
                                .and_then(parse_resize_dir)
                            {
                                let _ = window.drag_resize_window(dir);
                            }
                            return;
                        }
                        "windowMinimize" => {
                            window.set_minimized(true);
                            return;
                        }
                        "windowToggleMaximize" => {
                            let next = !window.is_maximized();
                            window.set_maximized(next);
                            dispatch_json(
                                &webview,
                                &serde_json::json!({ "event": "windowState", "maximized": next }),
                            );
                            return;
                        }
                        "windowClose" => {
                            if latex_job.running {
                                dispatch_json(&webview, &serde_json::json!({"event":"latexCloseWhileBuilding"}));
                                return;
                            }
                            remember_window(&window, &mut state);
                            handle_response(&webview, control_flow, &state.request_window_close());
                            return;
                        }
                        "windowPrint" => {
                            if let Err(error) = webview.print() {
                                dispatch_json(&webview, &serde_json::json!({
                                    "event": "toast", "message": format!("Print failed: {error}"),
                                }));
                            }
                            return;
                        }
                        _ => {}
                    }
                }

                let prev_light = state.settings.light;
                let response = state.handle_command(&body);
                if state.settings.light != prev_light {
                    configure_windows_frameless_chrome(&window, !state.settings.light);
                }
                handle_response(&webview, control_flow, &response);
            }
            #[cfg(target_os = "macos")]
            Event::Opened { urls } => {
                for path in urls.iter().filter_map(|url| url.to_file_path().ok()) {
                    if state.open_file_path(&path).is_ok() {
                        let doc = state.active_doc_snapshot(200);
                        let payload = serde_json::json!({
                            "event": "tabSwitched",
                            "tabs": state.tab_summaries(),
                            "activeTabId": state.active_doc().map(|d| d.id.clone()),
                            "doc": doc,
                            "settings": state.settings,
                        });
                        dispatch_json(&webview, &payload);
                    }
                }
            }
            Event::UserEvent(UserEvent::WorkspaceResults(response)) => {
                if response["request"].as_u64() == Some(workspace_query.load(Ordering::Relaxed)) {
                    dispatch_json(&webview, &response);
                }
            }
            Event::UserEvent(UserEvent::LatexBuilt(result)) => {
                latex_job.running = false;
                let (pdf_url, has_synctex) = if let Some(ref pdf) = result.pdf {
                    latex_job.pdf = Some(pdf.clone());
                    let url = assets::register_pdf(pdf);
                    latex_job.revision += 1;
                    let st = result.synctex;
                    if st && url.is_some() && state.active_latex_main().as_ref() == Some(&result.main) {
                        latex_job.preload_sync(pdf.clone(), &proxy_latex);
                    }
                    (url, st)
                } else {
                    (None, false)
                };
                dispatch_json(
                    &webview,
                    &serde_json::json!({
                        "event": "latexFinished",
                        "result": result,
                        "pdfUrl": pdf_url,
                        "hasSyncTex": has_synctex,
                    }),
                );
            }
            Event::UserEvent(UserEvent::LatexProgress(log)) => {
                dispatch_json(&webview, &serde_json::json!({"event":"latexProgress","log":log}));
            }
            Event::UserEvent(UserEvent::SyncTexResult(query, answer)) => {
                if query.request != latex_job.sync_request || query.revision != latex_job.revision
                    || state.active_doc().is_none_or(|doc| doc.id != query.doc_id)
                    || state.active_latex_main().as_deref() != Some(query.main.as_path())
                    || matches!(&query.position, SyncPosition::Source { path, .. }
                        if state.active_doc().and_then(|doc| doc.path.as_ref()) != Some(path))
                {
                    return;
                }
                let request = query.request;
                let response = match answer {
                    SyncAnswer::Forward(result) => match result {
                        Ok(hit) => serde_json::json!({
                            "event": "synctexForwardResult", "request": request,
                            "main": query.main, "path": query.pdf, "page": hit.page, "boxes": hit.boxes,
                        }),
                        Err(error) => serde_json::json!({"event":"synctexForwardResult", "request": request, "error": error}),
                    },
                    SyncAnswer::Inverse(result) => match result.and_then(|target| apply_inverse_sync(&mut state, &webview, query.main, target)) {
                        Ok(hit) => serde_json::json!({
                            "event": "synctexInverseResult", "request": request,
                            "path": hit.path, "line": hit.line, "column": hit.column,
                        }),
                        Err(error) => serde_json::json!({"event":"synctexInverseResult", "request": request, "error": error}),
                    },
                };
                dispatch_json(&webview, &response);
            }
            Event::UserEvent(UserEvent::FileDropped(path)) => {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase);
                if marktype::workspace::document_file(&path) {
                    if state.open_file_path(&path).is_ok() {
                        let doc = state.active_doc_snapshot(200);
                        let payload = serde_json::json!({
                            "event": "tabSwitched",
                            "tabs": state.tab_summaries(),
                            "activeTabId": state.active_doc().map(|d| d.id.clone()),
                            "doc": doc,
                            "settings": state.settings,
                        });
                        dispatch_json(&webview, &payload);
                    }
                } else if matches!(
                    ext.as_deref(),
                    Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "svg")
                ) {
                    assets::register_picked_image(&path);
                    let payload = serde_json::json!({
                        "event": "insertImagePath",
                        "path": path.to_string_lossy(),
                    });
                    dispatch_json(&webview, &payload);
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_) | WindowEvent::Moved(_),
                ..
            } => {
                geometry_at = Some(Instant::now() + Duration::from_millis(650));
                dispatch_json(
                    &webview,
                    &serde_json::json!({
                        "event": "windowState",
                        "maximized": window.is_maximized(),
                    }),
                );
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if latex_job.running {
                    dispatch_json(&webview, &serde_json::json!({"event":"latexCloseWhileBuilding"}));
                    return;
                }
                remember_window(&window, &mut state);
                handle_response(&webview, control_flow, &state.request_window_close());
            }
            Event::MainEventsCleared => {
                if geometry_at.is_some_and(|at| at <= Instant::now()) {
                    geometry_at = None;
                    remember_window(&window, &mut state);
                }
                for response in state.run_maintenance(Instant::now()) {
                    dispatch_json(&webview, &response);
                }
                *control_flow = state
                    .next_maintenance()
                    .into_iter().chain(smoke_deadline).chain(geometry_at).min()
                    .map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
                let title = state.active_doc().map_or_else(
                    || "MarkType".to_owned(),
                    |doc| {
                        let star = if doc.dirty { "• " } else { "" };
                        format!("{star}{} — MarkType", doc.title)
                    },
                );
                if title != window.title() {
                    window.set_title(&title);
                }
            }
            _ => {}
        }
    });
}
