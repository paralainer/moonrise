using Microsoft.UI;
using Microsoft.UI.Xaml.Media;
using Windows.UI;

namespace Moonrise.App.Ui;

/// <summary>The Mac app's palette and metrics (clients/apple/Moonrise/Support/Theme.swift).</summary>
public static class Theme
{
    public static readonly Color BackgroundColor = Color.FromArgb(255, 0x0D, 0x0F, 0x14);
    public static readonly SolidColorBrush Background = new(BackgroundColor);
    public static readonly SolidColorBrush Surface = new(Color.FromArgb(18, 255, 255, 255));
    public static readonly SolidColorBrush SurfaceStrong = new(Color.FromArgb(31, 255, 255, 255));
    public static readonly SolidColorBrush White = new(Colors.White);
    public static readonly SolidColorBrush Black = new(Colors.Black);
    public static readonly SolidColorBrush Secondary = new(Color.FromArgb(158, 255, 255, 255));
    public static readonly SolidColorBrush Glass = new(Color.FromArgb(120, 34, 34, 42));
    public static readonly SolidColorBrush GlassBorder = new(Color.FromArgb(40, 255, 255, 255));
    public static readonly SolidColorBrush HoverStroke = new(Color.FromArgb(140, 255, 255, 255));
    public static readonly SolidColorBrush Transparent = new(Colors.Transparent);
    public static readonly SolidColorBrush ImdbYellow = new(Color.FromArgb(255, 245, 196, 23));

    /// <summary>The page margin. The Mac's Metrics.pad value: WindowMetrics applies it once the window is measured (Theme.hPadding, 28, is only the value before that).</summary>
    public const double Margin = Moonrise.Core.ClientRules.PageMargin;
    public const double CardGap = 14;
    public const double ShelfHeaderGap = 10;
    public const double SectionGap = 28;

    public const double FontLargeTitle = 26;
    public const double FontTitle3 = 16;
    public const double FontHeadline = 14;
    public const double FontBody = 14;
    public const double FontSubheadline = 13;
    public const double FontCaption = 12;
    public const double FontCaption2 = 11;

    public static LinearGradientBrush Gradient(double x0, double y0, double x1, double y1, params (double Offset, Color Color)[] stops)
    {
        var b = new LinearGradientBrush { StartPoint = new(x0, y0), EndPoint = new(x1, y1), MappingMode = BrushMappingMode.RelativeToBoundingBox };
        foreach (var (o, c) in stops) b.GradientStops.Add(new GradientStop { Offset = o, Color = c });
        return b;
    }

    public static Color WithAlpha(Color c, double a) => Color.FromArgb((byte)(a * 255), c.R, c.G, c.B);
}
