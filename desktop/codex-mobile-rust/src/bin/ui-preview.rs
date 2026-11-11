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
    ui.set_title_text(if empty { "".into() } else { "Mock thread".into() });
    ui.set_project_name("ccm-e2e-ABIVPS".into());
    ui.set_cwd("/workspace/ccm-e2e-ABIVPS".into());
    ui.set_model("GPT-5.6".into());
    ui.set_threads(ModelRc::new(VecModel::from(vec![
        ThreadRow { id: "mock".into(), title: "Mock thread".into(), cwd: "/workspace/ccm-e2e-ABIVPS".into(), status: "idle".into(), meta: "11月11日 09:11".into() },
    ])));
    ui.set_selected_thread(if empty { -1 } else { 0 });
    ui.set_messages(ModelRc::new(VecModel::from(if empty { Vec::new() } else { vec![
        MessageRow { role: "你".into(), body: "TURN_GROUP_FIXTURE".into(), tone: 0 },
        MessageRow { role: "Codex".into(), body: "Before the tool.".into(), tone: 0 },
        MessageRow { role: "活动".into(), body: "printf grouped".into(), tone: 1 },
        MessageRow { role: "活动".into(), body: "用时 0 秒".into(), tone: 1 },
        MessageRow { role: "Codex".into(), body: "After the tool.".into(), tone: 0 },
    ] })));
    ui.run()
}
