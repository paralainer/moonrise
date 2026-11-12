using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Shapes;
using Windows.UI;
using Windows.UI.Text;
using Microsoft.UI.Text;

namespace Moonrise.App.Ui;

/// <summary>Small builders for the screens, so they read as layout rather than plumbing.</summary>
public static class F
{
    public static TextBlock Text(string text, double size, FontWeight? weight = null, Brush? fg = null, int maxLines = 0, TextTrimming trim = TextTrimming.None)
    {
        var t = new TextBlock { Text = text, FontSize = size, FontFamily = new FontFamily("Segoe UI Variable Text, Segoe UI"), Foreground = fg ?? Theme.White, TextTrimming = trim };
        if (weight != null) t.FontWeight = weight.Value;
        if (maxLines > 0) { t.MaxLines = maxLines; t.TextWrapping = TextWrapping.WrapWholeWords; t.TextTrimming = TextTrimming.CharacterEllipsis; }
        return t;
    }

    public static FontWeight SemiBold => FontWeights.SemiBold;
    public static FontWeight Bold => FontWeights.Bold;
    public static FontWeight Medium => FontWeights.Medium;

    /// <summary>An icon from Segoe Fluent Icons (Windows 11) / Segoe MDL2 Assets.</summary>
    public static TextBlock Glyph(string code, double size, Brush? fg = null) => new()
    {
        Text = code,
        FontFamily = new FontFamily("Segoe Fluent Icons, Segoe MDL2 Assets"),
        FontSize = size,
        Foreground = fg ?? Theme.White,
        HorizontalAlignment = HorizontalAlignment.Center,
        VerticalAlignment = VerticalAlignment.Center,
        TextAlignment = TextAlignment.Center,
    };

    public static Button Bare(UIElement content, Action onClick, string? tooltip = null)
    {
        var b = new Button { Style = (Style)Application.Current.Resources["BareButton"], Content = content };
        b.Click += (_, _) => onClick();
        if (tooltip != null) ToolTipService.SetToolTip(b, tooltip);
        return b;
    }

    /// <summary>A round glass button (Mac: .glassEffect circle).</summary>
    public static Button GlassCircle(string glyph, double size, double glyphSize, Action onClick, string? tooltip = null)
    {
        var border = new Border
        {
            Width = size, Height = size, CornerRadius = new CornerRadius(size / 2),
            Background = Theme.Glass, BorderBrush = Theme.GlassBorder, BorderThickness = new Thickness(1),
            Child = Glyph(glyph, glyphSize),
        };
        return Bare(border, onClick, tooltip);
    }

    public static Border Capsule(UIElement content, Brush bg, double height, double padH)
    {
        return new Border
        {
            Height = height, CornerRadius = new CornerRadius(height / 2), Background = bg,
            Padding = new Thickness(padH, 0, padH, 0), Child = content,
        };
    }

    /// <summary>A rounded box showing a remote image; empty until it arrives, then fades in (0.2 s).</summary>
    public static Border Image(string? url, double width, double height, double radius, int decodeWidth)
    {
        var host = new Border { Width = width, Height = height, CornerRadius = new CornerRadius(radius), Background = Theme.Surface };
        if (string.IsNullOrEmpty(url)) return host;
        var img = new Image { Stretch = Stretch.UniformToFill, Opacity = 0, OpacityTransition = new ScalarTransition { Duration = TimeSpan.FromMilliseconds(200) } };
        var clip = new Border { CornerRadius = new CornerRadius(radius), Child = img };
        var bmp = new BitmapImage { DecodePixelWidth = decodeWidth, DecodePixelType = DecodePixelType.Logical };
        bmp.ImageOpened += (_, _) => img.Opacity = 1;
        bmp.UriSource = new Uri(url);
        img.Source = bmp;
        host.Child = clip;
        return host;
    }

    /// <summary>
    /// A remote picture that fades in when it arrives. <paramref name="opened"/> takes that decision
    /// instead, for a picture that isn't always the one to show (the hero's).
    /// </summary>
    public static Image Art(string url, int decodeWidth, Stretch stretch = Stretch.UniformToFill, Action<Image>? opened = null)
    {
        var img = new Image { Stretch = stretch, Opacity = 0, OpacityTransition = new ScalarTransition { Duration = TimeSpan.FromMilliseconds(300) } };
        var bmp = new BitmapImage { DecodePixelWidth = decodeWidth, DecodePixelType = DecodePixelType.Logical };
        bmp.ImageOpened += (_, _) => { if (opened != null) opened(img); else img.Opacity = 1; };
        bmp.UriSource = new Uri(url);
        img.Source = bmp;
        return img;
    }

    /// <summary>
    /// A row that scrolls sideways. The vertical wheel moves the page around it, not the row, which
    /// a ScrollViewer with only a horizontal axis would otherwise take it for.
    /// </summary>
    public static ScrollViewer Row(FrameworkElement content)
    {
        var scroll = new ScrollViewer
        {
            Content = content, HorizontalScrollMode = ScrollMode.Enabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Hidden,
            VerticalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled, ZoomMode = ZoomMode.Disabled,
            IsHorizontalRailEnabled = true,
        };
        content.PointerWheelChanged += (_, e) =>
        {
            var wheel = e.GetCurrentPoint(content).Properties;
            if (wheel.IsHorizontalMouseWheel) return;
            e.Handled = true;
            for (var o = VisualTreeHelper.GetParent(scroll); o != null; o = VisualTreeHelper.GetParent(o))
            {
                if (o is ScrollViewer page && page.VerticalScrollMode != ScrollMode.Disabled)
                {
                    page.ChangeView(null, Math.Clamp(page.VerticalOffset - wheel.MouseWheelDelta, 0, page.ScrollableHeight), null, true);
                    return;
                }
            }
        };
        return scroll;
    }

    /// <summary>
    /// Makes <paramref name="element"/> the one thing a host shows, for an element that lives longer
    /// than one showing (a page on a tab's stack). The only way such an element goes into a panel:
    /// XAML throws if it's added while still another's child, and that ends in the process dying.
    /// Already showing, it's left alone, so it isn't unloaded and loaded again.
    /// </summary>
    public static void ShowOnly(Panel host, UIElement element)
    {
        if (host.Children.Count == 1 && ReferenceEquals(host.Children[0], element)) return;
        host.Children.Clear();
        host.Children.Add(element);
    }

    public static ColumnDefinition Star(double v = 1) => new() { Width = new GridLength(v, GridUnitType.Star) };
    public static ColumnDefinition Auto => new() { Width = GridLength.Auto };
    public static RowDefinition RowStar => new() { Height = new GridLength(1, GridUnitType.Star) };
    public static RowDefinition RowAuto => new() { Height = GridLength.Auto };

    public static T At<T>(this T e, int row = 0, int col = 0) where T : FrameworkElement
    {
        Grid.SetRow(e, row);
        Grid.SetColumn(e, col);
        return e;
    }

    /// <summary>A hairline, 2 pt inner stroke over a card on hover (Mac: brightness +0.06, 2 pt white 55%).</summary>
    public static Border HoverOverlay(double radius)
    {
        var over = new Border
        {
            CornerRadius = new CornerRadius(radius), BorderBrush = Theme.HoverStroke, BorderThickness = new Thickness(2),
            Background = new SolidColorBrush(Color.FromArgb(15, 255, 255, 255)), Opacity = 0, IsHitTestVisible = false,
            OpacityTransition = new ScalarTransition { Duration = TimeSpan.FromMilliseconds(150) },
        };
        return over;
    }

    public static void HoverReveals(UIElement host, Border overlay)
    {
        host.PointerEntered += (_, _) => overlay.Opacity = 1;
        host.PointerExited += (_, _) => overlay.Opacity = 0;
        host.PointerCanceled += (_, _) => overlay.Opacity = 0;
    }
}
