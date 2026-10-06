using Microsoft.UI.Xaml;

namespace PfwBench;

public sealed partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();
        AppWindow.Resize(new Windows.Graphics.SizeInt32(1280, 800)); // same size in every proof
    }
}
