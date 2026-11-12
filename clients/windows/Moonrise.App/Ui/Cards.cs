using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.UI;
using Moonrise.Core;

namespace Moonrise.App.Ui;

/// <summary>Poster and landscape cards, with the Mac app's sizes, radii, progress bars and hover.</summary>
public static class Cards
{
    /// <summary>The watched tick or unwatched count, and the progress bar, drawn afresh in their own layer.</summary>
    internal static void Marks(Grid layer, Item item, double barInset, double width)
    {
        layer.Children.Clear();
        Corners(layer, item);
        Progress(layer, item.Progress, barInset, width);
    }

    private static void Progress(Grid art, double? progress, double inset, double width)
    {
        if (progress is not { } p) return;
        var track = new Grid { Height = 4, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(inset, 0, inset, inset) };
        track.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = new SolidColorBrush(Color.FromArgb(128, 0, 0, 0)) });
        track.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = Theme.White, HorizontalAlignment = HorizontalAlignment.Left, Width = Math.Max(4, (width - 2 * inset) * p) });
        art.Children.Add(track);
    }

    private static void Corners(Grid art, Item item)
    {
        if (item.User?.Played == true)
        {
            art.Children.Add(new Border
            {
                Child = F.Glyph("", 18), HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(0, 5, 5, 0),
            });
        }
        else if ((item.User?.UnplayedItemCount ?? 0) > 0)
        {
            art.Children.Add(new Border
            {
                Background = Theme.White, CornerRadius = new CornerRadius(9), Height = 18, MinWidth = 18, Padding = new Thickness(6, 0, 6, 0),
                HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(0, 5, 5, 0),
                Child = new TextBlock { Text = item.User!.UnplayedItemCount!.Value.ToString(), FontSize = 11, FontWeight = Microsoft.UI.Text.FontWeights.Bold, Foreground = Theme.Black, VerticalAlignment = VerticalAlignment.Center },
            });
        }
    }

    public static FrameworkElement Poster(MoonriseApi api, Item item, double width, Action open) => new PosterCard(api, item, width, open).Button;

    public static FrameworkElement Landscape(MoonriseApi api, Item item, double width, Action activate)
    {
        var height = Math.Round(width * 9 / 16);
        var url = item.CardImage != null ? api.ImageUrl(item.CardImage, 640) : null;
        var art = new Grid { Width = width, Height = height };
        art.Children.Add(F.Image(url, width, height, 16, (int)Math.Min(640, width * 2)));
        var scrim = new Border
        {
            CornerRadius = new CornerRadius(16), IsHitTestVisible = false,
            Background = Theme.Gradient(0.5, 0.5, 0.5, 1, (0, Color.FromArgb(0, 0, 0, 0)), (1, Color.FromArgb(153, 0, 0, 0))),
        };
        art.Children.Add(scrim);
        var hover = F.HoverOverlay(16);
        art.Children.Add(hover);
        Corners(art, item);
        Progress(art, item.Progress, 8, width);

        var stack = new StackPanel { Width = width, Spacing = 8 };
        stack.Children.Add(art);
        var text = new StackPanel { Spacing = 2 };
        text.Children.Add(F.Text(item.CardTitle, Theme.FontSubheadline, F.Medium, trim: TextTrimming.CharacterEllipsis));
        text.Children.Add(F.Text(item.CardSubtitle, Theme.FontCaption, fg: Theme.Secondary, trim: TextTrimming.CharacterEllipsis));
        stack.Children.Add(text);
        var button = F.Bare(stack, activate);
        button.HorizontalContentAlignment = HorizontalAlignment.Left;
        F.HoverReveals(button, hover);
        return button;
    }

    internal static UIElement Missing() => new TextBlock
    {
        Text = "", FontFamily = new FontFamily("Segoe Fluent Icons, Segoe MDL2 Assets"), FontSize = 40,
        Foreground = Theme.Secondary, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
    };
}

/// <summary>
/// A poster with its title. A new width or new watch state changes this card in place: the same
/// elements, in the same parents, so a resize never moves an element between parents.
/// </summary>
public sealed class PosterCard
{
    private readonly Grid _art;
    private readonly Border _image;
    private readonly Grid _marks = new() { IsHitTestVisible = false };
    private readonly StackPanel _stack;
    private double _width;

    public Button Button { get; }
    public Item Item { get; private set; }

    public PosterCard(MoonriseApi api, Item item, double width, Action open)
    {
        Item = item;
        _width = width;
        var height = Math.Round(width * 1.5);
        var url = item.PosterImage != null ? api.ImageUrl(item.PosterImage, 400) : null;
        _art = new Grid { Width = width, Height = height };
        _image = F.Image(url, width, height, 12, (int)Math.Min(400, width * 2));
        _art.Children.Add(_image);
        var hover = F.HoverOverlay(12);
        _art.Children.Add(hover);
        if (url == null) _art.Children.Add(Cards.Missing());
        _art.Children.Add(_marks);
        Cards.Marks(_marks, item, 6, width);

        _stack = new StackPanel { Width = width, Spacing = 6 };
        _stack.Children.Add(_art);
        var text = new StackPanel { Spacing = 1 };
        text.Children.Add(F.Text(item.CardTitle, Theme.FontCaption, F.Medium, trim: TextTrimming.CharacterEllipsis));
        text.Children.Add(F.Text(item.CardSubtitle, Theme.FontCaption2, fg: Theme.Secondary, trim: TextTrimming.CharacterEllipsis));
        _stack.Children.Add(text);
        Button = F.Bare(_stack, open);
        Button.HorizontalContentAlignment = HorizontalAlignment.Left;
        F.HoverReveals(Button, hover);
    }

    public void Resize(double width)
    {
        if (width == _width) return;
        _width = width;
        var height = Math.Round(width * 1.5);
        _art.Width = width;
        _art.Height = height;
        _image.Width = width;
        _image.Height = height;
        _stack.Width = width;
        Cards.Marks(_marks, Item, 6, width);
    }

    /// <summary>The same title with newer watch state (the badge, the tick, the progress bar).</summary>
    public void Update(Item item)
    {
        Item = item;
        Cards.Marks(_marks, item, 6, _width);
    }
}
