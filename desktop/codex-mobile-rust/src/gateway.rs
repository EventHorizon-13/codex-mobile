use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

pub enum Command {
    Connect { url: String, token: String, secret: Option<String>, device: String },
    Rpc { tag: String, name: String, payload: Value },
    Disconnect,
}

pub enum Event {
    Connected { origin: String, issued_secret: Option<String> },
    Disconnected(String),
    Reply { tag: String, value: Value },
    Agent(Value),
    Status(String),
}

pub fn start(mut commands: mpsc::UnboundedReceiver<Command>, post: impl Fn(Event) + Send + Sync + 'static) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("network runtime");
        runtime.block_on(async move {
            let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build().expect("HTTP client");
            while let Some(command) = commands.recv().await {
                let Command::Connect { url, token, secret, device } = command else { continue; };
                post(Event::Status("正在验证设备…".into()));
                match run_connection(&http, &mut commands, &post, url, token, secret, device).await {
                    Ok(()) => post(Event::Disconnected("已断开连接".into())),
                    Err(error) => post(Event::Disconnected(error)),
                }
            }
        });
    });
}

async fn run_connection(
    http: &reqwest::Client,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    post: &impl Fn(Event),
    url: String,
    token: String,
    secret: Option<String>,
    device: String,
) -> Result<(), String> {
    let base = reqwest::Url::parse(url.trim()).map_err(|_| "服务器地址无效".to_string())?;
    let local = base.host_str().is_some_and(|host| host == "localhost" || host == "127.0.0.1" || host == "[::1]");
    if base.scheme() != "https" && !(local && base.scheme() == "http") { return Err("远程连接必须使用 HTTPS".into()); }
    if !base.username().is_empty() || base.password().is_some() || base.query().is_some() || base.fragment().is_some() { return Err("服务器地址不能包含凭证或查询参数".into()); }
    let origin = base.origin().ascii_serialization();
    let endpoint = format!("{origin}/auth/session");
    let mut req = http.post(endpoint).header("x-device-token", &device).json(&json!({}));
    if let Some(ref saved) = secret { req = req.header("x-device-secret", saved); }
    else { req = req.header("x-auth-token", token); }
    let response = req.send().await.map_err(|error| format!("登录连接失败：{error}"))?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 => "访问口令或保存的设备凭证已失效，请重新输入访问口令。".into(),
            429 => "登录尝试过多，请稍后再试。".into(),
            code => format!("登录失败：HTTP {code}"),
        });
    }
    let auth: Value = response.json().await.map_err(|_| "网关登录响应无效".to_string())?;
    let session = auth["sessionToken"].as_str().ok_or("网关未返回会话令牌")?.to_string();
    let issued_secret = auth["deviceSecret"].as_str().map(str::to_string);
    let ws_origin = origin.replacen(if base.scheme() == "https" { "https://" } else { "http://" }, if base.scheme() == "https" { "wss://" } else { "ws://" }, 1);
    let ws_url = format!("{ws_origin}/socket.io/?EIO=4&transport=websocket");
    let mut request = ws_url.into_client_request().map_err(|_| "WebSocket 地址无效".to_string())?;
    request.headers_mut().insert("Origin", HeaderValue::from_str(&origin).map_err(|_| "Origin 无效")?);
    post(Event::Status("正在建立实时连接…".into()));
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.map_err(|error| format!("实时连接失败：{error}"))?;
    let mut connected = false;
    let mut next_id = 0u32;
    let mut pending: HashMap<u32, String> = HashMap::new();
    let mut pending_delta: Option<Value> = None;
    let mut flush_at: Option<tokio::time::Instant> = None;

    loop {
        tokio::select! {
            _ = async {
                if let Some(deadline) = flush_at { tokio::time::sleep_until(deadline).await; }
                else { std::future::pending::<()>().await; }
            } => {
                if let Some(delta) = pending_delta.take() { post(Event::Agent(delta)); }
                flush_at = None;
            }
            received = socket.next() => {
                let Some(received) = received else { return Err("网关关闭了实时连接".into()); };
                let message = received.map_err(|error| format!("实时连接中断：{error}"))?;
                let Message::Text(packet) = message else { continue; };
                let packet = packet.as_str();
                if packet.starts_with('0') {
                    socket.send(Message::Text(format!("40{}", json!({"deviceToken":device,"sessionToken":session})).into())).await.map_err(|error| error.to_string())?;
                } else if packet == "2" {
                    socket.send(Message::Text("3".into())).await.map_err(|error| error.to_string())?;
                } else if packet.starts_with("40") {
                    connected = true;
                    post(Event::Connected { origin: origin.clone(), issued_secret: issued_secret.clone() });
                } else if packet.starts_with("44") {
                    return Err(format!("网关拒绝连接：{}", &packet[2..]));
                } else if packet.starts_with("42") {
                    if let Some(start) = packet.find('[') {
                        if let Ok(parts) = serde_json::from_str::<Value>(&packet[start..]) {
                            if parts[0] == "agent:event" {
                                let event = parts[1].clone();
                                if event["type"] == "text_delta" {
                                    if let Some(ref mut merged) = pending_delta {
                                        if merged["sessionId"] == event["sessionId"] && merged["instanceId"] == event["instanceId"] {
                                            let mut text = merged["payload"]["text"].as_str().unwrap_or_default().to_string();
                                            text.push_str(event["payload"]["text"].as_str().unwrap_or_default());
                                            merged["payload"]["text"] = Value::String(text);
                                        } else {
                                            post(Event::Agent(std::mem::replace(merged, event)));
                                        }
                                    } else { pending_delta = Some(event); }
                                    if flush_at.is_none() { flush_at = Some(tokio::time::Instant::now() + std::time::Duration::from_millis(60)); }
                                } else {
                                    if let Some(delta) = pending_delta.take() { post(Event::Agent(delta)); flush_at = None; }
                                    post(Event::Agent(event));
                                }
                            }
                        }
                    }
                } else if packet.starts_with("43") {
                    if let Some(start) = packet.find('[') {
                        if let Ok(id) = packet[2..start].parse::<u32>() {
                            if let Some(tag) = pending.remove(&id) {
                                let value = serde_json::from_str::<Value>(&packet[start..]).ok().and_then(|array| array.get(0).cloned()).unwrap_or(Value::Null);
                                post(Event::Reply { tag, value });
                            }
                        }
                    }
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()); };
                match command {
                    Command::Disconnect => return Ok(()),
                    Command::Connect { .. } => { post(Event::Status("已连接；请先断开当前连接。".into())); }
                    Command::Rpc { tag, name, payload } => {
                        if !connected { post(Event::Status("设备仍在等待网关批准。".into())); continue; }
                        next_id = next_id.wrapping_add(1);
                        pending.insert(next_id, tag);
                        socket.send(Message::Text(format!("42{next_id}{}", json!([name,payload])).into())).await.map_err(|error| format!("发送失败：{error}"))?;
                    }
                }
            }
        }
    }
}
