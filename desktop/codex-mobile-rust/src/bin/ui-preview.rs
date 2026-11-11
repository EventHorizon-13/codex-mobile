use slint::{ComponentHandle, ModelRc, VecModel};

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let preview_arg = std::env::args().nth(1).unwrap_or_default();
    let empty = preview_arg.starts_with("--empty");
    let ui = AppWindow::new()?;
    ui.window().set_size(slint::PhysicalSize::new(1180, 760));
    ui.set_dark(!preview_arg.ends_with("light"));
    ui.set_connected(true);
    ui.set_status("已连接 · 预览数据".into());
    ui.set_title_text("Windows 原生客户端".into());
    ui.set_cwd("/volume/codex/workspace".into());
    ui.set_model("gpt-6.1-sol".into());
    ui.set_threads(ModelRc::new(VecModel::from(vec![
        ThreadRow { id: "one".into(), title: "Windows 原生客户端".into(), cwd: "/volume/codex/workspace".into(), status: "needs-you".into() },
        ThreadRow { id: "two".into(), title: "Codex Mobile 的侧边栏布局".into(), cwd: "/volume/codex/workspace".into(), status: "idle".into() },
        ThreadRow { id: "three".into(), title: "优化运行时资源消耗".into(), cwd: "/volume/codex/workspace".into(), status: "running".into() },
    ])));
    ui.set_selected_thread(0);
    ui.set_messages(ModelRc::new(VecModel::from(if empty { Vec::new() } else { vec![
        MessageRow { role: "你".into(), body: "本机的 Codex Mobile 做个 Windows 原生前端软件。界面要简洁、美观，并且能跟随系统切换深色和浅色。".into(), tone: 0 },
        MessageRow { role: "Codex".into(), body: "我会沿用现有网页版的视觉语言：专注阅读的对话区、清晰的会话入口，以及始终容易找到的输入框。Windows 版使用 Rust 和 Slint，不嵌入网页。".into(), tone: 0 },
        MessageRow { role: "你".into(), body: "注意内存和 CPU 占用，做成小而美的软件。".into(), tone: 0 },
        MessageRow { role: "活动".into(), body: "命令 · cargo test --release · 完成".into(), tone: 1 },
        MessageRow { role: "Codex".into(), body: "会话列表只创建可见行；流式文本按批次更新；系统主题变化由 Windows 通知触发。空闲时无需持续轮询。".into(), tone: 0 },
    ] })));
    ui.run()
}
