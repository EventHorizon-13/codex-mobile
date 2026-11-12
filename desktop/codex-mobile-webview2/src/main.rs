#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{env, error::Error, fs, io::Cursor, path::PathBuf};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Icon, Window, WindowId},
};
use wry::{Rect, WebContext, WebViewBuilder};

const DEFAULT_URL: &str = "https://codex-go.arcsiny.com/";
const WINDOW_TITLE: &str = "Codex Chat Mobile";
const INITIAL_WIDTH: f64 = 1180.0;
const INITIAL_HEIGHT: f64 = 760.0;
const MIN_WIDTH: f64 = 360.0;
const MIN_HEIGHT: f64 = 480.0;

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

fn load_window_icon() -> Option<Icon> {
    let decoder = png::Decoder::new(Cursor::new(include_bytes!("../assets/codex-192.png")));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
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

#[derive(Default)]
struct App {
    url: String,
    profile_dir: PathBuf,
    window: Option<Window>,
    webview: Option<wry::WebView>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title(WINDOW_TITLE)
            .with_inner_size(LogicalSize::new(INITIAL_WIDTH, INITIAL_HEIGHT))
            .with_min_inner_size(LogicalSize::new(MIN_WIDTH, MIN_HEIGHT))
            .with_window_icon(load_window_icon());
        let window = event_loop
            .create_window(attributes)
            .expect("create the Windows application window");

        let mut web_context = WebContext::new(Some(self.profile_dir.clone()));
        let webview = WebViewBuilder::new_with_web_context(&mut web_context)
            .with_url(&self.url)
            .with_bounds(Rect {
                position: LogicalPosition::new(0.0, 0.0).into(),
                size: LogicalSize::new(INITIAL_WIDTH, INITIAL_HEIGHT).into(),
            })
            .build_as_child(&window)
            .expect("create the WebView2 controller; install the Edge WebView2 Runtime if missing");

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
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let (Some(window), Some(webview)) = (&self.window, &self.webview) else {
                    return;
                };
                let logical = size.to_logical::<u32>(window.scale_factor());
                let _ = webview.set_bounds(Rect {
                    position: LogicalPosition::new(0, 0).into(),
                    size: LogicalSize::new(logical.width, logical.height).into(),
                });
            }
            _ => {}
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let profile_dir = profile_dir();
    fs::create_dir_all(&profile_dir)?;

    let event_loop = EventLoop::new()?;
    let mut app = App {
        url: resolve_url(env::args().skip(1)),
        profile_dir,
        ..Default::default()
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
    use super::{resolve_url, DEFAULT_URL};

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
}

