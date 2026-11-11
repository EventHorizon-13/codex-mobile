#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod gateway;
mod windows;

use base64::Engine;
use gateway::{Command, Event};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{fs, path::PathBuf, sync::{Arc, Mutex}};
use tokio::sync::mpsc;

slint::include_modules!();

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Config {
    server_url: String,
    device_id: String,
    remember: bool,
    theme_mode: i32,
}
impl Default for Config {
    fn default() -> Self {
        Self { server_url: "https://codex-go.arcsiny.com/".into(), device_id: uuid::Uuid::new_v4().simple().to_string(), remember: true, theme_mode: 0 }
    }
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("CodexMobile.Native").join("settings.json")
}
fn load_config() -> Config {
    let path = config_path();
    let config = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice::<Config>(&bytes).ok()).unwrap_or_default();
    if !path.exists() { save_config(&config); }
    config
}
fn save_config(config: &Config) {
    let path = config_path();
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    if let Ok(bytes) = serde_json::to_vec_pretty(config) { let _ = fs::write(path, bytes); }
}
fn origin(url: &str) -> Option<String> { reqwest::Url::parse(url).ok().map(|u| u.origin().ascii_serialization()) }

#[derive(Default)]
struct State {
    config: Config,
    origin: String,
    threads: Vec<ThreadRow>,
    files: Vec<FileRow>,
    needs: Vec<NeedRow>,
    raw_needs: Vec<Value>,
    git: Vec<(String, String)>,
    thread_id: String,
    instance_id: String,
    cwd: String,
    file_path: String,
    action: String,
    attachment: Option<Value>,
    stream_index: Option<usize>,
    busy: bool,
    approved: bool,
}

fn text(value: &Value, key: &str) -> String { value.get(key).and_then(Value::as_str).unwrap_or_default().to_string() }
fn rpc(tx: &mpsc::UnboundedSender<Command>, tag: &str, name: &str, payload: Value) {
    let _ = tx.send(Command::Rpc { tag: tag.into(), name: name.into(), payload });
}
fn messages(ui: &AppWindow) -> ModelRc<MessageRow> { ui.get_messages() }
fn add_message(ui: &AppWindow, state: &mut State, role: &str, body: &str, tone: i32) {
    let model = messages(ui);
    let rows = model.as_any().downcast_ref::<VecModel<MessageRow>>().expect("messages model");
    if rows.row_count() >= 500 { rows.remove(0); state.stream_index = state.stream_index.map(|i| i.saturating_sub(1)); }
    rows.push(MessageRow { role: role.into(), body: body.chars().take(100_000).collect::<String>().into(), tone });
}
fn reset_messages(ui: &AppWindow, state: &mut State) {
    let model = messages(ui);
    model.as_any().downcast_ref::<VecModel<MessageRow>>().expect("messages model").clear();
    state.stream_index = None;
}
fn set_info(ui: &AppWindow, rows: Vec<InfoRow>) { ui.set_info(ModelRc::new(VecModel::from(rows))); }
fn set_status(ui: &AppWindow, value: impl Into<SharedString>) { ui.set_status(value.into()); }
fn request_initial_state(ui: &AppWindow, state: &Arc<Mutex<State>>, tx: &mpsc::UnboundedSender<Command>) {
    let s = state.lock().unwrap();
    rpc(tx, "thread:list", "thread:list", json!({"limit":100}));
    rpc(tx, "models:read", "models:read", json!({"cwd":s.cwd}));
    rpc(tx, "needs-you:snapshot", "needs-you:snapshot", json!({}));
}

fn handle_event(ui: &AppWindow, state: &Arc<Mutex<State>>, tx: &mpsc::UnboundedSender<Command>, event: Event) {
    match event {
        Event::Status(message) => set_status(ui, message),
        Event::Disconnected(message) => {
            ui.set_connected(false);
            ui.set_busy(false);
            set_status(ui, message);
            let mut s = state.lock().unwrap();
            s.busy = false;
            s.approved = false;
        }
        Event::Connected { origin, issued_secret } => {
            let mut s = state.lock().unwrap();
            s.origin = origin.clone();
            s.approved = false;
            if s.config.remember {
                if let Some(secret) = issued_secret {
                    if let Err(error) = windows::save_secret(&origin, &secret) { set_status(ui, error); }
                }
            }
            ui.set_connected(true);
            set_status(ui, "已连接，正在同步…");
        }
        Event::Reply { tag, value } => handle_reply(ui, state, tx, &tag, &value),
        Event::Agent(value) => handle_agent(ui, state, tx, &value),
    }
}

fn handle_reply(ui: &AppWindow, state: &Arc<Mutex<State>>, tx: &mpsc::UnboundedSender<Command>, tag: &str, value: &Value) {
    if value.get("ok") == Some(&Value::Bool(false)) {
        set_status(ui, text(value, "error"));
        return;
    }
    let mut s = state.lock().unwrap();
    match tag {
        "thread:list" => {
            s.threads = value["threads"].as_array().into_iter().flatten().filter_map(|thread| {
                let id = text(thread, "id");
                if id.is_empty() { return None; }
                let title = text(thread, "title");
                let preview = text(thread, "preview");
                Some(ThreadRow { id: id.into(), title: if title.is_empty() { if preview.is_empty() { "未命名会话".into() } else { preview.into() } } else { title.into() }, cwd: text(thread, "cwd").into(), status: text(thread, "status").into() })
            }).collect();
            ui.set_threads(ModelRc::new(VecModel::from(s.threads.clone())));
            if !s.thread_id.is_empty() {
                ui.set_selected_thread(s.threads.iter().position(|t| t.id.as_str() == s.thread_id).map(|i| i as i32).unwrap_or(-1));
            }
        }
        "session:new" | "thread:select" | "session:fork" => {
            s.thread_id = text(value, "threadId");
            s.instance_id = text(value, "instanceId");
            s.cwd = text(value, "cwd");
            ui.set_cwd(s.cwd.clone().into());
            reset_messages(ui, &mut s);
            if tag == "session:new" { ui.set_title_text("新会话".into()); ui.set_selected_thread(-1); }
            else {
                if tag == "session:fork" { ui.set_title_text("分叉会话".into()); }
                rpc(tx, "thread:history", "thread:history", json!({"threadId":s.thread_id,"cwd":s.cwd,"targetMessages":60}));
            }
            ui.set_panel(0);
            rpc(tx, "models:read", "models:read", json!({"cwd":s.cwd}));
        }
        "thread:history" => {
            reset_messages(ui, &mut s);
            if let Some(items) = value["messages"].as_array() {
                for item in items {
                    let role = text(item, "role");
                    let kind = text(item, "kind");
                    match role.as_str() {
                        "user" => add_message(ui, &mut s, "你", &text(item, "content"), 0),
                        "assistant" => add_message(ui, &mut s, "Codex", &text(item, "content"), 0),
                        _ => match kind.as_str() {
                            "command" => add_message(ui, &mut s, "命令", &text(item, "command"), 1),
                            "reasoning" => add_message(ui, &mut s, "思考", &text(item, "text"), 1),
                            "file_change" => add_message(ui, &mut s, "文件变更", "文件差异可在变更面板查看", 1),
                            "search" => add_message(ui, &mut s, "搜索", &text(item, "query"), 1),
                            "mcp" => add_message(ui, &mut s, "MCP", &format!("{} / {}", text(item, "serverName"), text(item, "toolName")), 1),
                            _ => {}
                        }
                    }
                }
            }
        }
        "user:message" => {
            let thread_id = text(value, "threadId");
            if !thread_id.is_empty() { s.thread_id = thread_id; }
            s.instance_id = text(value, "instanceId");
            s.attachment = None;
            ui.set_attachment_label("".into());
            ui.set_draft("".into());
            ui.set_busy(true);
            s.busy = true;
        }
        "models:read" => {
            let names: Vec<SharedString> = value["models"].as_array().into_iter().flatten().filter(|model| model["hidden"] != true)
                .map(|model| text(model, "model")).filter(|name| !name.is_empty()).map(Into::into).collect();
            ui.set_models(ModelRc::new(VecModel::from(names.clone())));
            if ui.get_model().is_empty() { if let Some(first) = names.first() { ui.set_model(first.clone()); } }
        }
        "needs-you:snapshot" => {
            s.raw_needs = value["needs"].as_array().cloned().unwrap_or_default();
            s.needs = s.raw_needs.iter().map(parse_need).collect();
            ui.set_needs(ModelRc::new(VecModel::from(s.needs.clone())));
        }
        "fs:readDirectory" => {
            s.file_path = text(value, "path");
            ui.set_file_path(s.file_path.clone().into());
            ui.set_preview("".into());
            s.files = value["entries"].as_array().into_iter().flatten().map(|entry| {
                let name = text(entry, "fileName");
                let path = format!("{}/{}", s.file_path.trim_end_matches('/'), name);
                FileRow { name: name.into(), path: path.into(), directory: entry["isDirectory"] == true }
            }).collect();
            ui.set_files(ModelRc::new(VecModel::from(s.files.clone())));
        }
        "fs:readFile" => {
            let encoded = text(value, "dataBase64");
            let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).unwrap_or_default();
            let preview = String::from_utf8_lossy(&bytes[..bytes.len().min(64_000)]);
            ui.set_preview(preview.into_owned().into());
            ui.set_file_path(text(value, "path").into());
        }
        "files:search" => {
            s.files = value["paths"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|path| {
                FileRow { name: path.rsplit('/').next().unwrap_or(path).into(), path: path.into(), directory: false }
            }).collect();
            ui.set_files(ModelRc::new(VecModel::from(s.files.clone())));
        }
        "git:status" => {
            let mut entries = Vec::new();
            for (key, label) in [("staged","已暂存"),("unstaged","未暂存"),("untracked","未跟踪"),("conflicted","冲突")] {
                for entry in value[key].as_array().into_iter().flatten() {
                    let path = text(entry, "path");
                    entries.push((path, if key == "staged" { "staged".to_string() } else { "unstaged".to_string() }, label.to_string()));
                }
            }
            s.git = entries.iter().map(|(path, side, _)| (path.clone(), side.clone())).collect();
            set_info(ui, entries.into_iter().map(|(path, _, label)| InfoRow { title: path.into(), detail: label.into() }).collect());
            ui.set_preview("".into());
        }
        "git:diff" => ui.set_preview(text(value, "patch").into()),
        "account:read" | "health:read" | "mcp:read" | "skills:read" | "devices:list" => {
            let rows = info_rows(tag, value);
            set_info(ui, rows);
        }
        "user:approval" => rpc(tx, "needs-you:snapshot", "needs-you:snapshot", json!({})),
        "thread:archive" | "thread:delete" | "thread:rename" | "thread:compact" | "thread:review" | "thread:rollback" => {
            set_status(ui, "操作已提交");
            rpc(tx, "thread:list", "thread:list", json!({"limit":100}));
            if tag == "thread:delete" { s.thread_id.clear(); reset_messages(ui, &mut s); ui.set_title_text("新会话".into()); }
        }
        _ => {}
    }
}

fn parse_need(need: &Value) -> NeedRow {
    let payload = &need["payload"];
    let target = &need["target"];
    let kind = text(need, "kind");
    let command = text(payload, "command");
    let title = if kind == "question" { "需要回答".to_string() } else { "需要审批".to_string() };
    let questions = payload["questions"].as_array();
    let question_detail = questions.map(|items| items.iter().map(|q| format!("{}：{}", text(q, "header"), text(q, "question"))).collect::<Vec<_>>().join("；")).unwrap_or_default();
    NeedRow {
        id: text(need, "needId").into(), title: title.into(),
        detail: if kind == "question" { question_detail.into() } else if command.is_empty() { text(payload, "reason").into() } else { command.into() },
        thread_id: text(target, "threadId").into(), instance_id: text(target, "instanceId").into(),
        approval_id: text(target, "requestId").into(), turn_id: text(target, "turnId").into(),
        item_id: text(target, "itemId").into(), decisions: payload["availableDecisions"].to_string().into(), question: kind == "question",
    }
}

fn info_rows(tag: &str, value: &Value) -> Vec<InfoRow> {
    match tag {
        "mcp:read" => value["servers"].as_array().into_iter().flatten().map(|item| InfoRow { title: text(item, "name").into(), detail: item.to_string().into() }).collect(),
        "skills:read" => value["entries"].as_array().into_iter().flatten().flat_map(|entry| entry["skills"].as_array().into_iter().flatten()).map(|item| InfoRow { title: text(item, "name").into(), detail: text(item, "description").into() }).collect(),
        "devices:list" => value["devices"].as_array().into_iter().flatten().map(|item| InfoRow { title: text(item, "deviceRef").into(), detail: format!("{} · {}", text(item, "ip"), if item["current"] == true { "此设备" } else { "其他设备" }).into() }).collect(),
        "health:read" => vec![InfoRow { title: "网关与 Codex".into(), detail: format!("网关已连接 · app-server {} · {} 个实例", if value["appServerRunning"] == true { "运行中" } else { "未运行" }, value["instances"]).into() }],
        "account:read" => vec![InfoRow { title: "账号".into(), detail: value["account"].to_string().into() }, InfoRow { title: "用量".into(), detail: value["usage"].to_string().into() }, InfoRow { title: "限额".into(), detail: value["rateLimits"].to_string().into() }],
        _ => Vec::new(),
    }
}

fn handle_agent(ui: &AppWindow, state: &Arc<Mutex<State>>, tx: &mpsc::UnboundedSender<Command>, event: &Value) {
    let kind = text(event, "type");
    let payload = &event["payload"];
    if kind == "init" {
        state.lock().unwrap().approved = true;
        ui.set_connected(true);
        set_status(ui, "已连接");
        request_initial_state(ui, state, tx);
        return;
    }
    if kind == "device_status" {
        let status = text(payload, "status");
        let mut s = state.lock().unwrap();
        s.approved = status == "approved";
        ui.set_connected(status != "denied");
        set_status(ui, match status.as_str() { "pending" => "等待服务器批准此设备", "denied" => "设备被拒绝", "approved" => "设备已批准，正在同步…", _ => "设备状态未知" });
        return;
    }
    if kind == "needs_you_changed" { rpc(tx, "needs-you:snapshot", "needs-you:snapshot", json!({})); return; }
    let mut s = state.lock().unwrap();
    let instance = text(event, "instanceId");
    let thread = text(event, "sessionId");
    if !s.instance_id.is_empty() && !instance.is_empty() && s.instance_id != instance { return; }
    if !s.thread_id.is_empty() && !thread.is_empty() && s.thread_id != thread { return; }
    if s.thread_id.is_empty() && !thread.is_empty() { s.thread_id = thread; }
    match kind.as_str() {
        "user_message" => { s.stream_index = None; add_message(ui, &mut s, "你", &text(payload, "text"), 0); }
        "text_delta" => {
            let model = messages(ui);
            let rows = model.as_any().downcast_ref::<VecModel<MessageRow>>().expect("messages model");
            if let Some(i) = s.stream_index {
                if let Some(mut row) = rows.row_data(i) { row.body = format!("{}{}", row.body, text(payload, "text")).into(); rows.set_row_data(i, row); }
            } else {
                add_message(ui, &mut s, "Codex", &text(payload, "text"), 0);
                s.stream_index = Some(rows.row_count().saturating_sub(1));
            }
        }
        "tool_use" => { s.stream_index = None; add_message(ui, &mut s, "工具", &text(payload, "command"), 1); }
        "tool_result" => { s.stream_index = None; }
        "reasoning" => { s.stream_index = None; let summary = text(payload, "text"); if !summary.is_empty() { add_message(ui, &mut s, "思考", &summary, 1); } }
        "approval_request" | "user_input_request" => { rpc(tx, "needs-you:snapshot", "needs-you:snapshot", json!({})); ui.set_panel(3); }
        "result" => { s.stream_index = None; s.busy = false; ui.set_busy(false); rpc(tx, "thread:list", "thread:list", json!({"limit":100})); }
        "error" => { s.stream_index = None; s.busy = false; ui.set_busy(false); add_message(ui, &mut s, "错误", &text(payload, "message"), 2); }
        "status" => { let value = text(payload, "state"); if value == "running" { ui.set_busy(true); } }
        _ => {}
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let config = load_config();
    let ui = AppWindow::new()?;
    let dark = match config.theme_mode { 1 => true, 2 => false, _ => windows::system_dark() };
    ui.set_dark(dark);
    ui.set_theme_mode(config.theme_mode);
    ui.set_server_url(config.server_url.clone().into());
    ui.set_remember_device(config.remember);
    ui.set_messages(ModelRc::new(VecModel::<MessageRow>::default()));
    ui.set_threads(ModelRc::new(VecModel::<ThreadRow>::default()));
    ui.set_files(ModelRc::new(VecModel::<FileRow>::default()));
    ui.set_needs(ModelRc::new(VecModel::<NeedRow>::default()));
    ui.set_info(ModelRc::new(VecModel::<InfoRow>::default()));
    ui.set_models(ModelRc::new(VecModel::<SharedString>::default()));

    let state = Arc::new(Mutex::new(State { config, ..Default::default() }));
    let (tx, rx) = mpsc::unbounded_channel::<Command>();
    let weak = ui.as_weak();
    let event_state = state.clone();
    let event_tx = tx.clone();
    gateway::start(rx, move |event| {
        let state = event_state.clone();
        let tx = event_tx.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| handle_event(&ui, &state, &tx, event));
    });

    let weak = ui.as_weak();
    let theme_state = state.clone();
    windows::watch_theme(move |system_dark| {
        let state = theme_state.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| { if state.lock().unwrap().config.theme_mode == 0 { ui.set_dark(system_dark); } });
    });

    {
        let state = state.clone(); let tx = tx.clone();
        ui.on_connect(move |url, token, remember| {
            let mut s = state.lock().unwrap();
            s.config.server_url = url.to_string(); s.config.remember = remember;
            save_config(&s.config);
            if !remember { if let Some(o) = origin(&url) { windows::forget_secret(&o); } }
            let saved = if token.is_empty() && remember { origin(&url).and_then(|o| windows::load_secret(&o)) } else { None };
            let _ = tx.send(Command::Connect { url: url.to_string(), token: token.to_string(), secret: saved, device: s.config.device_id.clone() });
        });
    }
    {
        let tx = tx.clone(); ui.on_disconnect(move || { let _ = tx.send(Command::Disconnect); });
    }
    {
        let state = state.clone(); let weak = ui.as_weak();
        ui.on_forget_device(move || {
            if let Some(ui) = weak.upgrade() {
                let s = state.lock().unwrap();
                if let Some(o) = origin(&s.config.server_url) { windows::forget_secret(&o); }
                ui.set_status("此设备的本地凭证已清除".into());
            }
        });
    }
    {
        let state = state.clone(); let tx = tx.clone();
        ui.on_refresh(move || { let s = state.lock().unwrap(); rpc(&tx, "thread:list", "thread:list", json!({"cwd":s.cwd,"limit":100})); });
    }
    {
        let state = state.clone(); let tx = tx.clone();
        ui.on_new_thread(move || { let s = state.lock().unwrap(); rpc(&tx, "session:new", "session:new", json!({"cwd":s.cwd})); });
    }
    {
        let state = state.clone(); let tx = tx.clone(); let weak = ui.as_weak();
        ui.on_open_thread(move |index| {
            let Some(ui) = weak.upgrade() else { return; };
            let s = state.lock().unwrap();
            if let Some(thread) = s.threads.get(index as usize) {
                ui.set_selected_thread(index); ui.set_title_text(thread.title.clone());
                rpc(&tx, "thread:select", "thread:select", json!({"threadId":thread.id.as_str(),"cwd":thread.cwd.as_str(),"title":thread.title.as_str()}));
            }
        });
    }
    {
        let state = state.clone(); let tx = tx.clone(); let weak = ui.as_weak();
        ui.on_send_message(move |text| {
            let Some(ui) = weak.upgrade() else { return; };
            let s = state.lock().unwrap();
            if !s.approved { ui.set_status("等待服务器批准此设备".into()); return; }
            if text.trim().is_empty() && s.attachment.is_none() { return; }
            let mut payload = json!({"clientRequestId":uuid::Uuid::new_v4().simple().to_string(),"text":text.trim()});
            if !s.thread_id.is_empty() { payload["threadId"] = json!(s.thread_id); }
            else if !s.instance_id.is_empty() { payload["instanceId"] = json!(s.instance_id); }
            if let Some(attachment) = &s.attachment { payload["attachments"] = json!([attachment]); }
            payload["turn"] = json!({"model":ui.get_model().to_string(),"effort":ui.get_effort().to_string(),"approvalPolicy":ui.get_approval().to_string(),"sandbox":ui.get_sandbox().to_string()});
            rpc(&tx, "user:message", "user:message", payload);
        });
    }
    {
        let state = state.clone(); let tx = tx.clone();
        ui.on_stop_turn(move || { let s = state.lock().unwrap(); rpc(&tx, "user:interrupt", "user:interrupt", json!({"threadId":s.thread_id,"instanceId":s.instance_id})); });
    }
    {
        let state = state.clone(); let weak = ui.as_weak();
        ui.on_attach_image(move || {
            let Some(path) = rfd::FileDialog::new().add_filter("图片", &["png","jpg","jpeg","webp"]).pick_file() else { return; };
            let Some(ui) = weak.upgrade() else { return; };
            match fs::read(&path) {
                Ok(bytes) if bytes.len() <= 12 * 1024 * 1024 => {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                    let mime = match ext.as_str() { "png" => "image/png", "webp" => "image/webp", _ => "image/jpeg" };
                    let name = path.file_name().and_then(|f| f.to_str()).unwrap_or("image").to_string();
                    state.lock().unwrap().attachment = Some(json!({"name":name,"mimeType":mime,"data":base64::engine::general_purpose::STANDARD.encode(bytes)}));
                    ui.set_attachment_label(name.into());
                }
                Ok(_) => ui.set_status("图片超过 12 MiB".into()),
                Err(error) => ui.set_status(format!("读取图片失败：{error}").into()),
            }
        });
    }
    {
        let state = state.clone(); let weak = ui.as_weak();
        ui.on_clear_attachment(move || { state.lock().unwrap().attachment = None; if let Some(ui) = weak.upgrade() { ui.set_attachment_label("".into()); } });
    }
    {
        let state = state.clone(); let weak = ui.as_weak();
        ui.on_set_theme(move |mode| {
            let Some(ui) = weak.upgrade() else { return; };
            let mut s = state.lock().unwrap(); s.config.theme_mode = mode; save_config(&s.config);
            ui.set_theme_mode(mode); ui.set_dark(match mode { 1 => true, 2 => false, _ => windows::system_dark() });
        });
    }
    {
        let weak = ui.as_weak(); ui.on_set_model(move |v| { if let Some(ui) = weak.upgrade() { ui.set_model(v); } });
    }
    {
        let weak = ui.as_weak(); ui.on_set_effort(move |v| { if let Some(ui) = weak.upgrade() { ui.set_effort(v); } });
    }
    {
        let weak = ui.as_weak(); ui.on_set_approval(move |v| { if let Some(ui) = weak.upgrade() { ui.set_approval(v); } });
    }
    {
        let weak = ui.as_weak(); ui.on_set_sandbox(move |v| { if let Some(ui) = weak.upgrade() { ui.set_sandbox(v); } });
    }
    register_workspace_callbacks(&ui, state.clone(), tx.clone());
    register_ops_callbacks(&ui, state.clone(), tx.clone());

    if state.lock().unwrap().config.remember {
        let s = state.lock().unwrap();
        if let Some(o) = origin(&s.config.server_url) {
            if let Some(secret) = windows::load_secret(&o) {
                set_status(&ui, "正在使用保存的设备凭证连接…");
                let _ = tx.send(Command::Connect { url: s.config.server_url.clone(), token: String::new(), secret: Some(secret), device: s.config.device_id.clone() });
            }
        }
    }
    ui.run()
}

fn register_workspace_callbacks(ui: &AppWindow, state: Arc<Mutex<State>>, tx: mpsc::UnboundedSender<Command>) {
    {
        let state = state.clone(); let tx = tx.clone();
        ui.on_load_files(move || { let s = state.lock().unwrap(); let path = if s.file_path.is_empty() { &s.cwd } else { &s.file_path }; rpc(&tx, "fs:readDirectory", "fs:readDirectory", json!({"cwd":s.cwd,"path":path})); });
    }
    {
        let state = state.clone(); let tx = tx.clone();
        ui.on_file_open(move |index| {
            let s = state.lock().unwrap();
            if let Some(file) = s.files.get(index as usize) {
                let name = if file.directory { "fs:readDirectory" } else { "fs:readFile" };
                rpc(&tx, name, name, json!({"cwd":s.cwd,"path":file.path.as_str()}));
            }
        });
    }
    {
        let state = state.clone(); let tx = tx.clone();
        ui.on_file_up(move || {
            let s = state.lock().unwrap();
            let parent = s.file_path.rsplit_once('/').map(|(p,_)| p).unwrap_or(&s.cwd);
            let path = if parent.starts_with(&s.cwd) { parent } else { &s.cwd };
            rpc(&tx, "fs:readDirectory", "fs:readDirectory", json!({"cwd":s.cwd,"path":path}));
        });
    }
    {
        let state = state.clone(); let tx = tx.clone();
        ui.on_file_search(move |query| { let s = state.lock().unwrap(); rpc(&tx, "files:search", "files:search", json!({"cwd":s.cwd,"query":query.as_str()})); });
    }
    {
        let state = state.clone(); let tx = tx.clone();
        ui.on_load_git(move || { let s = state.lock().unwrap(); rpc(&tx, "git:status", "git:status", json!({"cwd":s.cwd})); });
    }
}

fn register_ops_callbacks(ui: &AppWindow, state: Arc<Mutex<State>>, tx: mpsc::UnboundedSender<Command>) {
    {
        let state = state.clone(); let tx = tx.clone(); let weak = ui.as_weak();
        ui.on_open_info(move |key| {
            let Some(ui) = weak.upgrade() else { return; };
            let s = state.lock().unwrap();
            let key = key.to_string();
            if let Some(path) = key.strip_prefix("git:") {
                if let Some((_, side)) = s.git.iter().find(|(item,_)| item == path) { rpc(&tx, "git:diff", "git:diff", json!({"cwd":s.cwd,"path":path,"side":side})); }
                return;
            }
            let name = match key.as_str() {
                "needs" => "needs-you:snapshot", "account" => "account:read", "health" => "health:read",
                "mcp" => "mcp:read", "skills" => "skills:read", "devices" => "devices:list", _ => "",
            };
            if !name.is_empty() { rpc(&tx, name, name, json!({"cwd":s.cwd})); }
            if key == "settings" { ui.set_info(ModelRc::new(VecModel::from(Vec::<InfoRow>::new()))); }
        });
    }
    {
        let state = state.clone(); let tx = tx.clone(); let weak = ui.as_weak();
        ui.on_need_decision(move |index, decision| {
            let Some(ui) = weak.upgrade() else { return; };
            let s = state.lock().unwrap();
            let Some(need) = s.needs.get(index as usize) else { return; };
            let decision = if decision == "decline" && !need.decisions.contains("decline") { "cancel" } else { decision.as_str() };
            if !need.decisions.contains(decision) { ui.set_status("该请求不支持这项决定".into()); return; }
            rpc(&tx, "user:approval", "user:approval", json!({"needId":need.id.as_str(),"approvalId":need.approval_id.as_str(),"decision":decision,"threadId":need.thread_id.as_str(),"instanceId":need.instance_id.as_str(),"turnId":need.turn_id.as_str(),"itemId":need.item_id.as_str()}));
        });
    }
    {
        let state = state.clone(); let tx = tx.clone(); let weak = ui.as_weak();
        ui.on_need_answer(move |index, answer| {
            let Some(ui) = weak.upgrade() else { return; };
            let s = state.lock().unwrap();
            let Some(need) = s.needs.get(index as usize) else { return; };
            let Some(raw) = s.raw_needs.get(index as usize) else { return; };
            let Some(questions) = raw["payload"]["questions"].as_array() else { return; };
            let answer = answer.to_string();
            let values: Vec<&str> = if answer.is_empty() { Vec::new() } else { answer.split(';').map(str::trim).collect() };
            if !values.is_empty() && values.len() != questions.len() { ui.set_status(format!("此请求有 {} 个问题，请用分号分隔答案", questions.len()).into()); return; }
            let mut answers = serde_json::Map::new();
            for (i, question) in questions.iter().enumerate() {
                let id = text(question, "id");
                if !id.is_empty() { answers.insert(id, json!(values.get(i).filter(|v| !v.is_empty()).map(|v| vec![*v]).unwrap_or_default())); }
            }
            rpc(&tx, "user:approval", "user:approval", json!({"needId":need.id.as_str(),"approvalId":need.approval_id.as_str(),"answers":answers,"threadId":need.thread_id.as_str(),"instanceId":need.instance_id.as_str(),"turnId":need.turn_id.as_str(),"itemId":need.item_id.as_str()}));
        });
    }
    {
        let state = state.clone(); let tx = tx.clone(); let weak = ui.as_weak();
        ui.on_session_action(move |action| {
            let Some(ui) = weak.upgrade() else { return; };
            let mut s = state.lock().unwrap();
            let action = action.to_string();
            if !action.starts_with("submit:") { s.action = action.clone(); ui.set_status(format!("准备{action}。输入参数后点击执行；删除、回退等操作请先确认目标。 ").into()); return; }
            if s.thread_id.is_empty() { ui.set_status("请先打开一个会话".into()); return; }
            let input = action.trim_start_matches("submit:").trim();
            let (name, payload) = match s.action.as_str() {
                "rename" if !input.is_empty() => ("thread:rename", json!({"threadId":s.thread_id,"cwd":s.cwd,"name":input})),
                "fork" => ("session:fork", json!({"threadId":s.thread_id,"instanceId":s.instance_id})),
                "archive" => ("thread:archive", json!({"threadId":s.thread_id,"cwd":s.cwd})),
                "compact" => ("thread:compact", json!({"threadId":s.thread_id,"cwd":s.cwd})),
                "review" => ("thread:review", json!({"threadId":s.thread_id,"cwd":s.cwd,"instructions":input})),
                "rollback" => ("thread:rollback", json!({"threadId":s.thread_id,"cwd":s.cwd,"numTurns":input.parse::<u32>().unwrap_or(1).max(1)})),
                "delete" => ("thread:delete", json!({"threadId":s.thread_id,"cwd":s.cwd})),
                _ => { ui.set_status("请先选操作并填写必要参数".into()); return; }
            };
            rpc(&tx, name, name, payload);
            s.action.clear();
        });
    }
}
