// LaunchProbe: minimal WinUI 3 launch-path probe (Preview for Windows research).
// Measures harness ShellExecute -> first XAML frame that contains a page-sized bitmap.
// Flags come from the opened file name, for example page.xcr.mica.exit.lprobe:
//   xcr      merge XamlControlsResources (Fluent styles) before the first frame
//   toolbar  add a 6-button CommandBar (needs xcr)
//   mica     set a MicaBackdrop on the window
//   opt      enable WinUI optional perf changes (Windows App SDK 2.3.1+; IDs from WinUI main branch, unverified on 2.5.1)
//   win2d    create the Win2D shared device before the first frame
//   exit     exit after the report (warm runs); without it the process stays resident (hot runs)
// The .lprobe file is: int32 width, int32 height, then BGRA pixels. It stands in for a PDFium-rendered page.
// The probe sends QPC timestamps to the harness window "LaunchProbeHarness" with WM_COPYDATA.
using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Settings;
using Microsoft.Windows.AppLifecycle;
using Windows.ApplicationModel.Activation;

namespace LaunchProbe;

public static partial class Program
{
    internal static long TMain;

    [STAThread]
    static int Main(string[] args)
    {
        TMain = Stopwatch.GetTimestamp();
        WinRT.ComWrappersSupport.InitializeComWrappers();

        AppActivationArguments activation = AppInstance.GetCurrent().GetActivatedEventArgs();
        AppInstance main = AppInstance.FindOrRegisterForKey("main");
        if (!main.IsCurrent)
        {
            Redirect(main, activation);
            return 0;
        }
        main.Activated += (_, a) => App.Instance?.OpenFromRedirect(FilePath(a, null));

        string? file = FilePath(activation, args);
        if (Flags.Parse(file).Opt)
        {
            XamlOptionalChanges.EnableChange(XamlChangeId.DefaultStyleOptimizations);
            XamlOptionalChanges.EnableChange(XamlChangeId.OptimizeApplyStyles);
            XamlOptionalChanges.EnableChange(XamlChangeId.IconNoGridOptimization);
            XamlOptionalChanges.EnableChange(XamlChangeId.DeferContextFlyoutInit);
        }

        Application.Start(_ =>
        {
            SynchronizationContext.SetSynchronizationContext(
                new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
            _ = new App(file);
        });
        return 0;
    }

    // Command line first (cheap), then file or launch activation arguments.
    internal static string? FilePath(AppActivationArguments a, string[]? args)
    {
        if (args is { Length: > 0 } && File.Exists(args[0])) return args[0];
        if (a.Data is IFileActivatedEventArgs f && f.Files.Count > 0) return f.Files[0].Path;
        if (a.Data is ILaunchActivatedEventArgs l)
            foreach (string part in l.Arguments.Split('"'))
                if (part.Trim().EndsWith(".lprobe", StringComparison.OrdinalIgnoreCase)) return part.Trim();
        return null;
    }

    // Learn pattern: redirect on another thread, wait without blocking the STA.
    static void Redirect(AppInstance target, AppActivationArguments a)
    {
        using var done = new ManualResetEvent(false);
        Task.Run(() => { target.RedirectActivationToAsync(a).AsTask().Wait(); done.Set(); });
        nint[] handles = { done.SafeWaitHandle.DangerousGetHandle() };
        CoWaitForMultipleObjects(0, 0xFFFFFFFF, 1, handles, out _);
    }

    internal static void Report(string text)
    {
        nint hwnd = FindWindowW(null, "LaunchProbeHarness");
        if (hwnd == 0) return;
        nint p = Marshal.StringToHGlobalUni(text);
        try
        {
            var cd = new CopyData { DwData = 0x4C50, CbData = (text.Length + 1) * 2, LpData = p };
            SendMessageW(hwnd, 0x004A, 0, ref cd);
        }
        finally { Marshal.FreeHGlobal(p); }
    }

    [StructLayout(LayoutKind.Sequential)]
    struct CopyData { public nint DwData; public int CbData; public nint LpData; }

    [LibraryImport("user32.dll", StringMarshalling = StringMarshalling.Utf16)]
    private static partial nint FindWindowW(string? cls, string title);

    [LibraryImport("user32.dll")]
    private static partial nint SendMessageW(nint hwnd, uint msg, nint wParam, ref CopyData lParam);

    [LibraryImport("ole32.dll")]
    private static partial int CoWaitForMultipleObjects(uint flags, uint timeout, uint count, nint[] handles, out uint index);
}

public readonly record struct Flags(bool Xcr, bool Toolbar, bool Mica, bool Opt, bool Win2D, bool Exit)
{
    public static Flags Parse(string? file)
    {
        string[] t = Path.GetFileName(file ?? "").ToLowerInvariant().Split('.');
        bool Has(string f) => Array.IndexOf(t, f) >= 0;
        return new(Has("xcr"), Has("toolbar"), Has("mica"), Has("opt"), Has("win2d"), Has("exit"));
    }
}

public partial class App : Application
{
    internal static App? Instance;
    readonly string? _file;
    DispatcherQueue? _ui;

    public App(string? file)
    {
        Instance = this;
        _file = file;
        InitializeComponent();
        if (Flags.Parse(file).Xcr)
        {
            Resources ??= new ResourceDictionary();
            Resources.MergedDictionaries.Add(new XamlControlsResources());
        }
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        _ui = DispatcherQueue.GetForCurrentThread();
        Open(_file, redirected: false);
    }

    internal void OpenFromRedirect(string? file) => _ui?.TryEnqueue(() => Open(file, redirected: true));

    void Open(string? file, bool redirected)
    {
        Flags f = Flags.Parse(file);
        double win2dMs = -1;
        if (f.Win2D)
        {
            long t = Stopwatch.GetTimestamp();
            CanvasDevice.GetSharedDevice();
            win2dMs = Stopwatch.GetElapsedTime(t).TotalMilliseconds;
        }

        var root = new Grid();
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition());
        if (f.Toolbar && f.Xcr)
        {
            var bar = new CommandBar();
            foreach (Symbol s in new[] { Symbol.Page, Symbol.Zoom, Symbol.Edit, Symbol.Rotate, Symbol.Share, Symbol.Find })
                bar.PrimaryCommands.Add(new AppBarButton { Icon = new SymbolIcon(s), Label = "Tool" });
            root.Children.Add(bar);
        }
        var image = new Image { Stretch = Stretch.None, HorizontalAlignment = HorizontalAlignment.Center };
        Grid.SetRow(image, 1);
        root.Children.Add(image);
        if (file != null) image.Source = LoadPage(file);

        var window = new Window { Title = "LaunchProbe", Content = root };
        if (f.Mica) window.SystemBackdrop = new MicaBackdrop();
        window.Activate();
        long tActivated = Stopwatch.GetTimestamp();

        EventHandler<RenderedEventArgs>? onRendered = null;
        onRendered = (_, _) =>
        {
            CompositionTarget.Rendered -= onRendered;
            long tRendered = Stopwatch.GetTimestamp();
            Program.Report($"pid={Environment.ProcessId};main={Program.TMain};activated={tActivated};rendered={tRendered};win2dMs={win2dMs:F2};redirected={redirected}");
            if (f.Exit) Exit();
            else if (redirected) window.Close();
        };
        CompositionTarget.Rendered += onRendered;
    }

    static WriteableBitmap LoadPage(string path)
    {
        byte[] data = File.ReadAllBytes(path);
        int w = BitConverter.ToInt32(data, 0), h = BitConverter.ToInt32(data, 4);
        var bmp = new WriteableBitmap(w, h);
        using (Stream s = bmp.PixelBuffer.AsStream()) s.Write(data, 8, w * h * 4);
        bmp.Invalidate();
        return bmp;
    }
}
