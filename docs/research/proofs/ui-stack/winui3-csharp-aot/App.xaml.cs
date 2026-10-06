using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;

namespace PfwBench;

public partial class App : Application
{
    [LibraryImport("dwmapi.dll")]
    private static partial int DwmFlush();

    public App() => InitializeComponent();

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        new MainWindow().Activate();

        // Benchmark marker (see ../measure.py): after the first XAML frame,
        // wait for DWM to present it, then write the QPC value.
        var path = Environment.GetEnvironmentVariable("PFW_BENCH_OUT");
        if (path is null) return;
        EventHandler<RenderedEventArgs>? onRendered = null;
        onRendered = (_, _) =>
        {
            CompositionTarget.Rendered -= onRendered;
            DwmFlush();
            File.WriteAllText(path, Stopwatch.GetTimestamp().ToString()); // Stopwatch uses QPC
        };
        CompositionTarget.Rendered += onRendered;
    }
}
