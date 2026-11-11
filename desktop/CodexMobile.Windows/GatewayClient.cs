using System.Collections.Concurrent;
using System.Net;
using System.Net.Http;
using System.Net.WebSockets;
using System.Text;
using System.Text.Json;

namespace CodexMobile.Windows;

/// <summary>Small Engine.IO 4 / Socket.IO 4 WebSocket client for this gateway's JSON event contract.</summary>
internal sealed class GatewayClient : IAsyncDisposable
{
    private readonly HttpClient http = new() { Timeout = TimeSpan.FromSeconds(20) };
    private readonly SemaphoreSlim sendLock = new(1, 1);
    private readonly ConcurrentDictionary<int, TaskCompletionSource<JsonElement>> pending = new();
    private readonly CancellationTokenSource lifetime = new();
    private readonly TaskCompletionSource<bool> connected = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private ClientWebSocket? socket;
    private int packetId;
    private string? sessionToken;

    public Uri BaseUri { get; }
    public string DeviceToken { get; }
    public event Action<string, JsonElement>? EventReceived;
    public event Action<string>? Disconnected;

    public GatewayClient(Uri baseUri, string deviceToken)
    {
        if (baseUri.Scheme != Uri.UriSchemeHttps &&
            !(baseUri.Scheme == Uri.UriSchemeHttp && baseUri.IsLoopback))
            throw new ArgumentException("远程服务器必须使用 HTTPS。", nameof(baseUri));
        if (!string.IsNullOrEmpty(baseUri.UserInfo) || baseUri.Query.Length > 0 || baseUri.Fragment.Length > 0)
            throw new ArgumentException("服务器地址不能包含账号、查询参数或片段。", nameof(baseUri));
        BaseUri = new Uri(baseUri.GetLeftPart(UriPartial.Authority) + "/");
        DeviceToken = deviceToken;
    }

    public async Task ConnectAsync(string enrollmentToken, CancellationToken cancellationToken = default)
    {
        using var request = new HttpRequestMessage(HttpMethod.Post, new Uri(BaseUri, "auth/session"));
        request.Headers.TryAddWithoutValidation("x-auth-token", enrollmentToken);
        request.Headers.TryAddWithoutValidation("x-device-token", DeviceToken);
        request.Content = new StringContent("{}", Encoding.UTF8, "application/json");
        using var response = await http.SendAsync(request, cancellationToken);
        if (!response.IsSuccessStatusCode)
            throw new InvalidOperationException(response.StatusCode switch
            {
                HttpStatusCode.Unauthorized => "访问口令错误或已失效。",
                (HttpStatusCode)429 => "登录尝试过多，请稍后再试。",
                _ => $"登录失败：HTTP {(int)response.StatusCode}"
            });
        using var body = JsonDocument.Parse(await response.Content.ReadAsStringAsync(cancellationToken));
        sessionToken = body.RootElement.GetProperty("sessionToken").GetString()
            ?? throw new InvalidOperationException("网关未返回会话令牌。");

        var wsUri = new UriBuilder(BaseUri)
        {
            Scheme = BaseUri.Scheme == Uri.UriSchemeHttps ? "wss" : "ws",
            Path = "/socket.io/",
            Query = "EIO=4&transport=websocket"
        }.Uri;
        socket = new ClientWebSocket();
        socket.Options.SetRequestHeader("Origin", BaseUri.GetLeftPart(UriPartial.Authority));
        socket.Options.KeepAliveInterval = TimeSpan.FromSeconds(20);
        await socket.ConnectAsync(wsUri, cancellationToken);
        _ = ReceiveLoopAsync();
        await connected.Task.WaitAsync(TimeSpan.FromSeconds(20), cancellationToken);
    }

    public async Task<JsonElement> EmitAsync(string name, object payload, CancellationToken cancellationToken = default)
    {
        if (!connected.Task.IsCompletedSuccessfully || socket?.State != WebSocketState.Open)
            throw new InvalidOperationException("尚未连接网关。请重新连接。");
        var id = Interlocked.Increment(ref packetId);
        var completion = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        pending[id] = completion;
        try
        {
            await SendAsync($"42{id}{JsonSerializer.Serialize(new object[] { name, payload })}", cancellationToken);
            var result = await completion.Task.WaitAsync(TimeSpan.FromSeconds(25), cancellationToken);
            if (result.ValueKind == JsonValueKind.Object && result.TryGetProperty("ok", out var ok) && ok.ValueKind == JsonValueKind.False)
            {
                var error = result.TryGetProperty("error", out var detail) ? detail.GetString() : "请求失败";
                throw new InvalidOperationException(error);
            }
            return result;
        }
        finally { pending.TryRemove(id, out _); }
    }

    private async Task SendAsync(string packet, CancellationToken cancellationToken)
    {
        await sendLock.WaitAsync(cancellationToken);
        try
        {
            if (socket?.State != WebSocketState.Open) throw new WebSocketException("连接已断开。");
            var data = Encoding.UTF8.GetBytes(packet);
            await socket.SendAsync(new ArraySegment<byte>(data), WebSocketMessageType.Text, true, cancellationToken);
        }
        finally { sendLock.Release(); }
    }

    private async Task ReceiveLoopAsync()
    {
        var buffer = new byte[8192];
        try
        {
            while (socket?.State == WebSocketState.Open && !lifetime.IsCancellationRequested)
            {
                using var stream = new MemoryStream();
                WebSocketReceiveResult part;
                do
                {
                    part = await socket.ReceiveAsync(new ArraySegment<byte>(buffer), lifetime.Token);
                    if (part.MessageType == WebSocketMessageType.Close) throw new WebSocketException("服务器关闭了连接。");
                    stream.Write(buffer, 0, part.Count);
                    if (stream.Length > 8 * 1024 * 1024) throw new InvalidOperationException("网关消息超过 8 MiB。 ");
                } while (!part.EndOfMessage);
                await HandlePacketAsync(Encoding.UTF8.GetString(stream.ToArray()));
            }
        }
        catch (OperationCanceledException) when (lifetime.IsCancellationRequested) { }
        catch (Exception error)
        {
            connected.TrySetException(error);
            Disconnected?.Invoke(error.Message);
        }
        finally
        {
            foreach (var item in pending.Values) item.TrySetException(new IOException("网关连接已断开。"));
        }
    }

    private async Task HandlePacketAsync(string packet)
    {
        if (packet.StartsWith('0'))
        {
            await SendAsync("40" + JsonSerializer.Serialize(new { deviceToken = DeviceToken, sessionToken }), lifetime.Token);
            return;
        }
        if (packet == "2") { await SendAsync("3", lifetime.Token); return; }
        if (packet.StartsWith("40")) { connected.TrySetResult(true); return; }
        if (packet.StartsWith("44"))
        {
            var detail = packet.Length > 2 ? packet[2..] : "Socket.IO 登录失败";
            throw new InvalidOperationException(detail);
        }
        if (packet.Length < 3 || packet[0] != '4') return;
        var kind = packet[1];
        var arrayStart = packet.IndexOf('[', 2);
        if (arrayStart < 0) return;
        var number = packet[2..arrayStart];
        using var document = JsonDocument.Parse(packet[arrayStart..]);
        var items = document.RootElement;
        if (kind == '3' && int.TryParse(number, out var id) && pending.TryGetValue(id, out var completion))
        {
            completion.TrySetResult(items.GetArrayLength() > 0 ? items[0].Clone() : default);
        }
        else if (kind == '2' && items.GetArrayLength() > 1)
        {
            var name = items[0].GetString();
            if (name != null) EventReceived?.Invoke(name, items[1].Clone());
        }
    }

    public async ValueTask DisposeAsync()
    {
        lifetime.Cancel();
        if (socket is { State: WebSocketState.Open })
        {
            try { await socket.CloseAsync(WebSocketCloseStatus.NormalClosure, "Closing", CancellationToken.None); }
            catch (WebSocketException) { }
        }
        socket?.Dispose();
        http.Dispose();
        sendLock.Dispose();
        lifetime.Dispose();
    }
}
