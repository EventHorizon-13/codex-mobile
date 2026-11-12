#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{env, error::Error, fs, io::Cursor, path::{Path, PathBuf}};
use base64::Engine;
use serde::{Deserialize, Serialize};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::{Icon, Window, WindowId},
};
use wry::{http::Request, DragDropEvent, Rect, WebContext, WebViewBuilder};

const DEFAULT_URL: &str = "https://codex-go.arcsiny.com/";
const WINDOW_TITLE: &str = "Codex Chat Mobile";
const INITIAL_WIDTH: f64 = 1180.0;
const INITIAL_HEIGHT: f64 = 760.0;
const MIN_WIDTH: f64 = 360.0;
const MIN_HEIGHT: f64 = 480.0;
const ZOOM_MIN: f64 = 0.75;
const ZOOM_MAX: f64 = 2.0;
const ZOOM_STEP: f64 = 0.1;
const MAX_DROP_BYTES: u64 = 24 * 1024 * 1024;

const ZOOM_SHORTCUT_SCRIPT: &str = r#"
(() => {
  let lastWheelAt = 0;
  const send = action => window.ipc?.postMessage(action);
  window.addEventListener('keydown', event => {
    if (!event.ctrlKey || event.altKey || event.metaKey) return;
    const key = event.key;
    let action = '';
    if (key === '+' || key === '=') action = 'zoom-in';
    else if (key === '-' || key === '_') action = 'zoom-out';
    else if (key === '0') action = 'zoom-reset';
    if (!action) return;
    event.preventDefault();
    event.stopPropagation();
    send(action);
  }, true);
  window.addEventListener('wheel', event => {
    if (!event.ctrlKey) return;
    event.preventDefault();
    const now = Date.now();
    if (now - lastWheelAt < 90) return;
    lastWheelAt = now;
    send(event.deltaY < 0 ? 'zoom-in' : 'zoom-out');
  }, { passive: false, capture: true });
  window.addEventListener('contextmenu', event => {
    const row = event.target?.closest?.('.session-item');
    if (!row) return;
    event.preventDefault();
    event.stopPropagation();
    const options = {
      bubbles: true, cancelable: true, clientX: event.clientX, clientY: event.clientY,
      pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0, buttons: 1,
    };
    row.dispatchEvent(new PointerEvent('pointerdown', options));
    setTimeout(() => {
      if (!row.isConnected) return;
      row.dispatchEvent(new PointerEvent('pointerup', { ...options, buttons: 0 }));
    }, 520);
  }, true);
})();

window.__codexDropNotice = message => {
  let node = document.getElementById('__codex_drop_notice');
  if (!node) {
    node = document.createElement('div');
    node.id = '__codex_drop_notice';
    Object.assign(node.style, {
      position: 'fixed', left: '50%', bottom: '92px', transform: 'translateX(-50%)',
      zIndex: '2147483647', padding: '9px 14px', borderRadius: '12px',
      background: 'rgba(30,30,30,.92)', color: '#fff', fontSize: '13px',
      fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
      boxShadow: '0 8px 28px rgba(0,0,0,.28)', pointerEvents: 'none',
      transition: 'opacity .18s ease', maxWidth: '80vw', whiteSpace: 'nowrap',
      overflow: 'hidden', textOverflow: 'ellipsis'
    });
    document.body.appendChild(node);
  }
  node.textContent = message;
  node.style.opacity = '1';
  clearTimeout(window.__codexDropNoticeTimer);
  window.__codexDropNoticeTimer = setTimeout(() => {
    node.style.opacity = '0';
    setTimeout(() => node.remove(), 220);
  }, 2200);
};

window.__codexAttachDroppedFiles = files => {
  const input = document.getElementById('file-input');
  if (!input || !Array.isArray(files) || files.length === 0) {
    window.__codexDropNotice?.('当前页面不能添加附件');
    return false;
  }
  const transfer = new DataTransfer();
  for (const item of files) {
    try {
      const binary = atob(item.data);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
      transfer.items.add(new File([bytes], item.name, { type: item.mimeType || 'application/octet-stream' }));
    } catch (error) {
      window.__codexDropNotice?.(`读取 ${item.name} 失败`);
      return false;
    }
  }
  input.files = transfer.files;
  input.dispatchEvent(new Event('change', { bubbles: true }));
  window.__codexDropNotice?.(`已添加 ${files.length} 个文件`);
  return true;
};
"#;

#[derive(Debug, Clone)]
enum ZoomAction {
    In,
    Out,
    Reset,
}

#[derive(Debug, Clone)]
enum UserEvent {
    Zoom(ZoomAction),
    Files(Vec<DroppedFile>),
    DropError(String),
}

#[derive(Debug, Clone, Serialize)]
struct DroppedFile {
    name: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
    data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct WindowPrefs {
    width: u32,
    height: u32,
    maximized: bool,
    zoom: f64,
}

impl Default for WindowPrefs {
    fn default() -> Self {
        Self {
            width: INITIAL_WIDTH as u32,
            height: INITIAL_HEIGHT as u32,
            maximized: false,
            zoom: 1.0,
        }
    }
}

impl WindowPrefs {
    fn normalize(&mut self) {
        if self.width < MIN_WIDTH as u32 || self.width > 10_000 {
            self.width = INITIAL_WIDTH as u32;
        }
        if self.height < MIN_HEIGHT as u32 || self.height > 10_000 {
            self.height = INITIAL_HEIGHT as u32;
        }
        self.zoom = self.zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    }
}

fn resolve_url(args: impl IntoIterator<Item = String>) -> String {
    args.into_iter()
        .find(|arg| arg.starts_with("https://") || arg.starts_with("http://"))
        .unwrap_or_else(|| DEFAULT_URL.to_string())
}

fn profile_dir() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("CodexMobileWebView2")
        .join("Profile")
}

fn state_path() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("CodexMobileWebView2")
        .join("window.json")
}

fn load_preferences(path: &Path) -> WindowPrefs {
    let mut prefs = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<WindowPrefs>(&bytes).ok())
        .unwrap_or_default();
    prefs.normalize();
    prefs
}

fn mime_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or_default().to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn read_dropped_files(paths: Vec<PathBuf>) -> Result<Vec<DroppedFile>, String> {
    let files: Vec<PathBuf> = paths.into_iter().filter(|path| path.is_file()).collect();
    if files.is_empty() {
        return Err("只支持拖入文件，不支持文件夹".to_string());
    }

    let mut total = 0_u64;
    let mut result = Vec::with_capacity(files.len());
    for path in files {
        let size = fs::metadata(&path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?.len();
        if size > MAX_DROP_BYTES || total.saturating_add(size) > MAX_DROP_BYTES {
            return Err("单次拖入文件总大小不能超过 24MB".to_string());
        }
        total += size;
        let bytes = fs::read(&path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
        result.push(DroppedFile {
            name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            mime_type: mime_type_for_path(&path).to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }
    Ok(result)
}

fn adjust_zoom(current: f64, action: ZoomAction) -> f64 {
    let next = match action {
        ZoomAction::In => current + ZOOM_STEP,
        ZoomAction::Out => current - ZOOM_STEP,
        ZoomAction::Reset => 1.0,
    };
    (next * 10.0).round().clamp(ZOOM_MIN * 10.0, ZOOM_MAX * 10.0) / 10.0
}

fn window_title(zoom: f64) -> String {
    if (zoom - 1.0).abs() < 0.001 {
        WINDOW_TITLE.to_string()
    } else {
        format!("{WINDOW_TITLE} · {}%", (zoom * 100.0).round() as i32)
    }
}

fn load_window_icon() -> Option<Icon> {
    let decoder = png::Decoder::new(Cursor::new(include_bytes!("../assets/codex-192.png")));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for pixel in pixels.chunks_exact(3) {
                rgba.extend_from_slice(pixel);
                rgba.push(255);
            }
            rgba
        }
        _ => return None,
    };
    Icon::from_rgba(rgba, info.width, info.height).ok()
}

struct App {
    url: String,
    profile_dir: PathBuf,
    state_path: PathBuf,
    prefs: WindowPrefs,
    normal_size: LogicalSize<u32>,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Window>,
    webview: Option<wry::WebView>,
}

impl App {
    fn save_preferences(&self) {
        if let Some(parent) = self.state_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.prefs) {
            let _ = fs::write(&self.state_path, bytes);
        }
    }

    fn capture_window_size(&mut self) {
        let Some(window) = &self.window else { return; };
        self.prefs.maximized = window.is_maximized();
        if !self.prefs.maximized {
            self.normal_size = window.inner_size().to_logical::<u32>(window.scale_factor());
        }
        self.prefs.width = self.normal_size.width.max(MIN_WIDTH as u32);
        self.prefs.height = self.normal_size.height.max(MIN_HEIGHT as u32);
    }

    fn apply_zoom(&mut self, action: ZoomAction) {
        self.prefs.zoom = adjust_zoom(self.prefs.zoom, action);
        if let Some(webview) = &self.webview {
            let _ = webview.zoom(self.prefs.zoom);
        }
        if let Some(window) = &self.window {
            window.set_title(&window_title(self.prefs.zoom));
        }
        self.save_preferences();
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let initial_size = LogicalSize::new(self.prefs.width as f64, self.prefs.height as f64);
        let attributes = Window::default_attributes()
            .with_title(window_title(self.prefs.zoom))
            .with_inner_size(initial_size)
            .with_min_inner_size(LogicalSize::new(MIN_WIDTH, MIN_HEIGHT))
            .with_window_icon(load_window_icon());
        let window = event_loop
            .create_window(attributes)
            .expect("create the Windows application window");

        let mut web_context = WebContext::new(Some(self.profile_dir.clone()));
        let proxy = self.proxy.clone();
        let ipc_handler = move |request: Request<String>| {
            let action = match request.body().as_str() {
                "zoom-in" => ZoomAction::In,
                "zoom-out" => ZoomAction::Out,
                "zoom-reset" => ZoomAction::Reset,
                _ => return,
            };
            let _ = proxy.send_event(UserEvent::Zoom(action));
        };
        let drop_proxy = self.proxy.clone();
        let drag_drop_handler = move |event: DragDropEvent| {
            if let DragDropEvent::Drop { paths, .. } = event {
                let proxy = drop_proxy.clone();
                std::thread::spawn(move || {
                    let event = match read_dropped_files(paths) {
                        Ok(files) => UserEvent::Files(files),
                        Err(message) => UserEvent::DropError(message),
                    };
                    let _ = proxy.send_event(event);
                });
            }
            true
        };
        let webview = WebViewBuilder::new_with_web_context(&mut web_context)
            .with_url(&self.url)
            .with_bounds(Rect {
                position: LogicalPosition::new(0.0, 0.0).into(),
                size: initial_size.into(),
            })
            .with_initialization_script(ZOOM_SHORTCUT_SCRIPT)
            .with_ipc_handler(ipc_handler)
            .with_drag_drop_handler(drag_drop_handler)
            .build_as_child(&window)
            .expect("create the WebView2 controller; install the Edge WebView2 Runtime if missing");

        let _ = webview.zoom(self.prefs.zoom);
        if self.prefs.maximized {
            window.set_maximized(true);
        }

        self.window = Some(window);
        self.webview = Some(webview);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.capture_window_size();
                self.save_preferences();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                let (Some(window), Some(webview)) = (&self.window, &self.webview) else {
                    return;
                };
                let logical = size.to_logical::<u32>(window.scale_factor());
                let maximized = window.is_maximized();
                let _ = webview.set_bounds(Rect {
                    position: LogicalPosition::new(0, 0).into(),
                    size: LogicalSize::new(logical.width, logical.height).into(),
                });
                if !maximized {
                    self.normal_size = logical;
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Zoom(action) => self.apply_zoom(action),
            UserEvent::Files(files) => {
                if let Some(webview) = &self.webview {
                    match serde_json::to_string(&files) {
                        Ok(payload) => {
                            let script = format!("window.__codexAttachDroppedFiles?.({payload});");
                            let _ = webview.evaluate_script(&script);
                        }
                        Err(_) => {}
                    }
                }
            }
            UserEvent::DropError(message) => {
                if let Some(webview) = &self.webview {
                    let script = format!("window.__codexDropNotice?.({});", serde_json::to_string(&message).unwrap_or_else(|_| "\"文件读取失败\"".to_string()));
                    let _ = webview.evaluate_script(&script);
                }
            }
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let profile_dir = profile_dir();
    fs::create_dir_all(&profile_dir)?;

    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let state_path = state_path();
    let prefs = load_preferences(&state_path);
    let normal_size = LogicalSize::new(prefs.width, prefs.height);
    let mut app = App {
        url: resolve_url(env::args().skip(1)),
        profile_dir,
        state_path,
        prefs,
        normal_size,
        proxy,
        window: None,
        webview: None,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        #[cfg(debug_assertions)]
        eprintln!("Codex WebView2 failed: {error}");
        let _ = error;
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        adjust_zoom, mime_type_for_path, resolve_url, window_title, ZoomAction, DEFAULT_URL,
        ZOOM_MAX, ZOOM_MIN,
    };
    use std::path::Path;

    #[test]
    fn uses_the_hosted_web_ui_by_default() {
        assert_eq!(resolve_url(Vec::<String>::new()), DEFAULT_URL);
        assert_eq!(resolve_url(["--unknown".to_string()]), DEFAULT_URL);
    }

    #[test]
    fn accepts_an_http_url_override() {
        assert_eq!(
            resolve_url(["http://127.0.0.1:3232/".to_string()]),
            "http://127.0.0.1:3232/"
        );
    }

    #[test]
    fn zoom_steps_are_clamped_and_resettable() {
        assert_eq!(adjust_zoom(1.0, ZoomAction::In), 1.1);
        assert_eq!(adjust_zoom(1.0, ZoomAction::Out), 0.9);
        assert_eq!(adjust_zoom(ZOOM_MIN, ZoomAction::Out), ZOOM_MIN);
        assert_eq!(adjust_zoom(ZOOM_MAX, ZoomAction::In), ZOOM_MAX);
        assert_eq!(adjust_zoom(1.7, ZoomAction::Reset), 1.0);
    }

    #[test]
    fn non_default_zoom_is_visible_in_the_title() {
        assert_eq!(window_title(1.0), "Codex Chat Mobile");
        assert_eq!(window_title(1.2), "Codex Chat Mobile · 120%");
    }

    #[test]
    fn dropped_images_keep_image_mime_types() {
        assert_eq!(mime_type_for_path(Path::new("photo.PNG")), "image/png");
        assert_eq!(mime_type_for_path(Path::new("scan.jpeg")), "image/jpeg");
        assert_eq!(mime_type_for_path(Path::new("notes.md")), "text/markdown");
        assert_eq!(mime_type_for_path(Path::new("binary.bin")), "application/octet-stream");
    }
}

