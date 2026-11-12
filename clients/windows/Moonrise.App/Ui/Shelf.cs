using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media.Animation;

namespace Moonrise.App.Ui;

/// <summary>A titled horizontal row (Mac: Shelf): header, cards with 14 gaps, arrows on hover at the ends.</summary>
public static class Shelf
{
    public static FrameworkElement Build(string title, IEnumerable<FrameworkElement> cards, double cardBottomPad = 34, FrameworkElement? accessory = null)
    {
        var header = new Grid { Margin = new Thickness(Theme.Margin, 0, Theme.Margin, Theme.ShelfHeaderGap) };
        header.ColumnDefinitions.Add(F.Star());
        header.ColumnDefinitions.Add(F.Auto);
        header.Children.Add(F.Text(title, Theme.FontTitle3, F.SemiBold).At(0, 0));
        if (accessory != null) header.Children.Add(accessory.At(0, 1));

        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = Theme.CardGap, Padding = new Thickness(Theme.Margin, 0, Theme.Margin, 0) };
        foreach (var c in cards) row.Children.Add(c);
        var scroll = F.Row(row);

        var body = new Grid();
        body.Children.Add(scroll);
        Button Arrow(string glyph, int dir)
        {
            var b = F.GlassCircle(glyph, 40, 14, () => Page(scroll, dir));
            b.HorizontalAlignment = dir < 0 ? HorizontalAlignment.Left : HorizontalAlignment.Right;
            b.VerticalAlignment = VerticalAlignment.Center;
            b.Margin = new Thickness(4, 0, 4, cardBottomPad);
            b.Opacity = 0;
            b.OpacityTransition = new ScalarTransition { Duration = TimeSpan.FromMilliseconds(150) };
            body.Children.Add(b);
            return b;
        }
        var left = Arrow("", -1);
        var right = Arrow("", 1);
        void Update(bool hovering)
        {
            left.Opacity = hovering && scroll.HorizontalOffset > 1 ? 1 : 0;
            left.IsHitTestVisible = left.Opacity > 0;
            right.Opacity = hovering && scroll.HorizontalOffset < scroll.ScrollableWidth - 1 ? 1 : 0;
            right.IsHitTestVisible = right.Opacity > 0;
        }
        var over = false;
        body.PointerEntered += (_, _) => { over = true; Update(true); };
        body.PointerExited += (_, _) => { over = false; Update(false); };
        scroll.ViewChanged += (_, _) => Update(over);
        Update(false);

        var outer = new StackPanel();
        outer.Children.Add(header);
        outer.Children.Add(body);
        return outer;
    }

    private static void Page(ScrollViewer s, int dir)
    {
        var page = s.ActualWidth - 40;
        s.ChangeView(Math.Clamp(s.HorizontalOffset + dir * page, 0, s.ScrollableWidth), null, null);
    }
}
