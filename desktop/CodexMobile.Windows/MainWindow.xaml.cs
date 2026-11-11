using System.Collections.ObjectModel;
using System.IO;
using System.Security.Cryptography;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace CodexMobile.Windows;

public partial class MainWindow : Window
{
    private readonly ObservableCollection<ThreadItem> threads = [];
    private GatewayClient? gateway;
    private string? currentThreadId;
    private string? currentInstanceId;
    private string? currentCwd;
    private TextBlock? streamingText;
    private bool choosingThread;

    public MainWindow()
    {
        InitializeComponent();
        ThreadList.ItemsSource = threads;
        Closed += async (_, _) => { if (gateway != null) await gateway.DisposeAsync(); };
    }

    private static string DeviceToken()
    {
        var directory = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "CodexMobile.Windows");
        Directory.CreateDirectory(directory);
        var path = Path.Combine(directory, "device-id.txt");
        if (File.Exists(path))
        {
            var saved = File.ReadAllText(path).Trim();
            if (saved.Length is >= 16 and <= 128 && saved.All(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '_' or ':' or '-')) return saved;
        }
        var token = Convert.ToHexString(RandomNumberGenerator.GetBytes(24)).ToLowerInvariant();
        File.WriteAllText(path, token);
        return token;
    }

    private async void Connect_Click(object sender, RoutedEventArgs e)
    {
        ConnectButton.IsEnabled = false;
        try
        {
            if (gateway != null) await gateway.DisposeAsync();
            gateway = new GatewayClient(new Uri(ServerBox.Text.Trim()), DeviceToken());
            gateway.EventReceived += (name, payload) => Dispatcher.InvokeAsync(() => HandleEvent(name, payload));
            gateway.Disconnected += reason => Dispatcher.InvokeAsync(() => ConnectionText.Text = $"已断开：{reason}。重新输入口令连接。 ");
            ConnectionText.Text = "正在登录…";
            await gateway.ConnectAsync(TokenBox.Password.Trim());
            TokenBox.Clear();
            ConnectionText.Text = "已连接";
            await RefreshThreadsAsync();
        }
        catch (Exception error) { ConnectionText.Text = error.Message; }
        finally { ConnectButton.IsEnabled = true; }
    }

    private async Task RefreshThreadsAsync()
    {
        if (gateway == null) return;
        try
        {
            var response = await gateway.EmitAsync("thread:list", new { limit = 100 });
            if (!response.TryGetProperty("threads", out var items) || items.ValueKind != JsonValueKind.Array) return;
            choosingThread = true;
            threads.Clear();
            foreach (var item in items.EnumerateArray())
            {
                var id = String(item, "id");
                if (id.Length == 0) continue;
                var title = String(item, "title");
                var preview = String(item, "preview");
                threads.Add(new ThreadItem(id, title.Length > 0 ? title : (preview.Length > 0 ? preview : id), String(item, "cwd")));
            }
            ThreadList.SelectedItem = threads.FirstOrDefault(t => t.Id == currentThreadId);
        }
        catch (Exception error) { ConnectionText.Text = $"会话加载失败：{error.Message}"; }
        finally { choosingThread = false; }
    }

    private async void Refresh_Click(object sender, RoutedEventArgs e) => await RefreshThreadsAsync();

    private async void New_Click(object sender, RoutedEventArgs e)
    {
        if (gateway == null) { ConnectionText.Text = "请先连接。"; return; }
        try
        {
            var response = await gateway.EmitAsync("session:new", new { cwd = currentCwd });
            currentThreadId = null;
            currentInstanceId = String(response, "instanceId");
            currentCwd = String(response, "cwd");
            choosingThread = true;
            ThreadList.SelectedItem = null;
            choosingThread = false;
            TranscriptPanel.Children.Clear();
            streamingText = null;
            TitleText.Text = "新会话";
            ComposerBox.Focus();
        }
        catch (Exception error) { ConnectionText.Text = error.Message; }
    }

    private async void ThreadList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (choosingThread || gateway == null || ThreadList.SelectedItem is not ThreadItem thread) return;
        try
        {
            var response = await gateway.EmitAsync("thread:select", new { threadId = thread.Id, cwd = thread.Cwd, title = thread.Title });
            currentThreadId = thread.Id;
            currentInstanceId = String(response, "instanceId");
            currentCwd = thread.Cwd;
            TitleText.Text = thread.Title;
            TranscriptPanel.Children.Clear();
            streamingText = null;
            var history = await gateway.EmitAsync("thread:history", new { threadId = thread.Id, cwd = thread.Cwd, targetMessages = 60 });
            if (history.TryGetProperty("messages", out var messages) && messages.ValueKind == JsonValueKind.Array)
            {
                foreach (var message in messages.EnumerateArray()) AddHistoryMessage(message);
            }
            ScrollToEnd();
        }
        catch (Exception error) { ConnectionText.Text = $"打开会话失败：{error.Message}"; }
    }

    private void AddHistoryMessage(JsonElement item)
    {
        var role = String(item, "role");
        if (role is "user" or "assistant") { AddMessage(role == "user" ? "你" : "Codex", String(item, "content")); return; }
        var kind = String(item, "kind");
        if (kind == "command") AddMessage("命令", String(item, "command"), true);
        else if (kind == "file_change") AddMessage("文件变更", "查看原会话获取完整差异", true);
        else if (kind == "reasoning") AddMessage("思考摘要", String(item, "text"), true);
        else if (kind == "search") AddMessage("搜索", String(item, "query"), true);
    }

    private async void Send_Click(object sender, RoutedEventArgs e) => await SendAsync();

    private async void Composer_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter && (Keyboard.Modifiers & ModifierKeys.Shift) == 0)
        {
            e.Handled = true;
            await SendAsync();
        }
    }

    private async Task SendAsync()
    {
        var text = ComposerBox.Text.Trim();
        if (gateway == null || text.Length == 0) return;
        SendButton.IsEnabled = false;
        try
        {
            var request = new Dictionary<string, object?> { ["clientRequestId"] = Guid.NewGuid().ToString("N"), ["text"] = text };
            if (!string.IsNullOrEmpty(currentThreadId)) request["threadId"] = currentThreadId;
            else if (!string.IsNullOrEmpty(currentInstanceId)) request["instanceId"] = currentInstanceId;
            var response = await gateway.EmitAsync("user:message", request);
            ComposerBox.Clear();
            var threadId = String(response, "threadId");
            if (threadId.Length > 0) currentThreadId = threadId;
            StopButton.Visibility = Visibility.Visible;
        }
        catch (Exception error) { ConnectionText.Text = $"发送失败：{error.Message}"; }
        finally { SendButton.IsEnabled = true; }
    }

    private async void Stop_Click(object sender, RoutedEventArgs e)
    {
        if (gateway == null) return;
        try
        {
            await gateway.EmitAsync("user:interrupt", new { threadId = currentThreadId, instanceId = currentInstanceId });
        }
        catch (Exception error) { ConnectionText.Text = error.Message; }
    }

    private void HandleEvent(string name, JsonElement envelope)
    {
        if (name != "agent:event") return;
        var type = String(envelope, "type");
        var sessionId = String(envelope, "sessionId");
        var instanceId = String(envelope, "instanceId");
        if (type == "device_status")
        {
            var status = envelope.TryGetProperty("payload", out var device) ? String(device, "status") : "";
            if (status == "pending") ConnectionText.Text = "等待服务器批准此设备…";
            else if (status == "denied") ConnectionText.Text = "此设备已被拒绝。";
            else if (status == "approved") ConnectionText.Text = "已连接";
            return;
        }
        if (type == "init" && currentInstanceId == null && instanceId.Length > 0) currentInstanceId = instanceId;
        if (type == "init") return;
        if (currentInstanceId != null && instanceId.Length > 0 && instanceId != currentInstanceId) return;
        if (currentThreadId != null && sessionId.Length > 0 && sessionId != currentThreadId) return;
        if (currentThreadId == null && sessionId.Length > 0) currentThreadId = sessionId;
        if (!envelope.TryGetProperty("payload", out var payload) || payload.ValueKind != JsonValueKind.Object) return;
        switch (type)
        {
            case "user_message":
                streamingText = null;
                AddMessage("你", String(payload, "text"));
                break;
            case "text_delta":
                var delta = String(payload, "text");
                if (streamingText == null) streamingText = AddMessage("Codex", "");
                streamingText.Text += delta;
                ScrollToEnd();
                break;
            case "tool_use": AddMessage("工具", String(payload, "command"), true); break;
            case "tool_result": streamingText = null; break;
            case "approval_request": AddApproval(payload, sessionId, instanceId); break;
            case "result":
                streamingText = null;
                StopButton.Visibility = Visibility.Collapsed;
                _ = RefreshThreadsAsync();
                break;
            case "error":
                streamingText = null;
                StopButton.Visibility = Visibility.Collapsed;
                AddMessage("错误", String(payload, "message"), true);
                break;
        }
    }

    private void AddApproval(JsonElement payload, string threadId, string instanceId)
    {
        var box = new StackPanel { Margin = new Thickness(0, 10, 0, 12) };
        box.Children.Add(new TextBlock { Text = "需要审批", FontWeight = FontWeights.SemiBold, Foreground = Brushes.Orange });
        box.Children.Add(new TextBlock
        {
            Text = String(payload, "command") is { Length: > 0 } command ? command : String(payload, "reason"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 6, 0, 8),
            FontFamily = new FontFamily("Consolas")
        });
        var row = new StackPanel { Orientation = Orientation.Horizontal };
        var decisions = payload.TryGetProperty("availableDecisions", out var available) && available.ValueKind == JsonValueKind.Array
            ? available.EnumerateArray().Select(v => v.GetString()).ToHashSet()
            : [];
        foreach (var (label, decision) in new[] { ("批准", "accept"), ("本会话批准", "acceptForSession"), ("拒绝", decisions.Contains("decline") ? "decline" : "cancel") })
        {
            if (!decisions.Contains(decision)) continue;
            var button = new Button { Content = label, Margin = new Thickness(0, 0, 8, 0) };
            button.Click += async (_, _) =>
            {
                foreach (Button child in row.Children) child.IsEnabled = false;
                try
                {
                    if (gateway == null) return;
                    var request = new
                    {
                        needId = String(payload, "needId"), approvalId = String(payload, "approvalId"), decision,
                        turnId = String(payload, "turnId"), itemId = String(payload, "itemId"),
                        threadId, instanceId
                    };
                    await gateway.EmitAsync("user:approval", request);
                    box.Children.Remove(row);
                    box.Children.Add(new TextBlock { Text = $"已{label}", Foreground = (Brush)FindResource("MutedBrush") });
                }
                catch (Exception error)
                {
                    ConnectionText.Text = $"审批结果未知，请刷新会话确认：{error.Message}";
                }
            };
            row.Children.Add(button);
        }
        box.Children.Add(row);
        TranscriptPanel.Children.Add(box);
        ScrollToEnd();
    }

    private TextBlock AddMessage(string speaker, string content, bool muted = false)
    {
        var box = new StackPanel { Margin = new Thickness(0, 8, 0, 14), MaxWidth = 760, HorizontalAlignment = HorizontalAlignment.Left };
        box.Children.Add(new TextBlock
        {
            Text = speaker, FontWeight = FontWeights.SemiBold,
            Foreground = muted ? (Brush)FindResource("MutedBrush") : (Brush)FindResource("AccentBrush"),
            Margin = new Thickness(0, 0, 0, 5)
        });
        var body = new TextBlock
        {
            Text = content, TextWrapping = TextWrapping.Wrap,
            Foreground = muted ? (Brush)FindResource("MutedBrush") : (Brush)FindResource("TextBrush"),
            FontSize = 14, LineHeight = 22
        };
        box.Children.Add(body);
        TranscriptPanel.Children.Add(box);
        ScrollToEnd();
        return body;
    }

    private void ScrollToEnd() => Dispatcher.InvokeAsync(TranscriptScroll.ScrollToEnd);

    private static string String(JsonElement value, string key) =>
        value.ValueKind == JsonValueKind.Object && value.TryGetProperty(key, out var field) && field.ValueKind == JsonValueKind.String
            ? field.GetString() ?? "" : "";

    private sealed record ThreadItem(string Id, string Title, string Cwd);
}
