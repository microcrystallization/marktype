#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Instant;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{ResizeDirection, WindowBuilder};
use wry::http::Response;
use wry::{DragDropEvent, NewWindowResponse, WebViewBuilder};

use marktype::assets;
use marktype::ipc::AppState;
use marktype::markdown::is_external_link;

enum UserEvent {
    IpcMessage(String),
    FileDropped(PathBuf),
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
    url == assets::APP_URL || url == "http://marktype.localhost/index.html"
}

fn main() {
    let initial_file = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.is_file());
    let mut state = AppState::new(initial_file);

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy_ipc = event_loop.create_proxy();
    let proxy_drop = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("MarkType")
        .with_decorations(false)
        .with_inner_size(LogicalSize::new(1420.0, 900.0))
        .with_min_inner_size(LogicalSize::new(820.0, 560.0))
        .build(&event_loop)
        .expect("Failed to create MarkType window");

    configure_windows_frameless_chrome(&window, !state.settings.light);

    let webview = WebViewBuilder::new()
        .with_custom_protocol("marktype".into(), |_webview_id, request| {
            let uri = request.uri().to_string();
            let (status, mime, body) = assets::serve_request(&uri);
            let csp = if mime.starts_with("text/html") {
                assets::APP_CSP
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
            false
        })
        .with_devtools(cfg!(debug_assertions))
        .build(&window)
        .expect("Failed to initialize WebView2");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = state
            .next_autosave()
            .map_or(ControlFlow::Wait, ControlFlow::WaitUntil);

        match event {
            Event::UserEvent(UserEvent::IpcMessage(body)) => {
                // Fast-path native frameless window chrome commands
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body)
                    && let Some(cmd) = v.get("cmd").and_then(|c| c.as_str())
                {
                    match cmd {
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
                            if state.confirm_window_close() {
                                *control_flow = ControlFlow::Exit;
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
                if response.get("event").and_then(|v| v.as_str()) != Some("noop") {
                    dispatch_json(&webview, &response);
                }
            }
            Event::UserEvent(UserEvent::FileDropped(path)) => {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase);
                if matches!(
                    ext.as_deref(),
                    Some("md" | "markdown" | "mdown" | "mkd" | "txt" | "tex")
                ) {
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
                event: WindowEvent::Resized(_),
                ..
            } => {
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
                if state.confirm_window_close() {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::MainEventsCleared => {
                for response in state.run_autosaves(Instant::now()) {
                    dispatch_json(&webview, &response);
                }
                *control_flow = state
                    .next_autosave()
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
