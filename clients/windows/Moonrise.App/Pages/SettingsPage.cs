using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Moonrise.App.Ui;
using Moonrise.Core;
using Windows.UI;

namespace Moonrise.App.Pages;

/// <summary>
/// Settings kept on this device, as the Mac app's Settings (UI/SettingsView.swift): the quality
/// titles start at, what to do with intros and credits, and the audio and subtitles a title starts
/// with and how subtitles look. Picking a quality or tracks in the player changes them too.
/// </summary>
public sealed class SettingsPage : Grid, IRefreshable
{
    private readonly List<Action> _syncs = [];
    private bool _syncing;

    public SettingsPage()
    {
        Background = Theme.Background;
        var s = Prefs.Current;
        var column = new StackPanel { Spacing = 6, MaxWidth = 640, Margin = new Thickness(Theme.Margin, Shell.TopBarHeight + 16, Theme.Margin, 40) };
        column.Children.Add(F.Text("Settings", Theme.FontLargeTitle, F.Bold));

        Section(column, "Playback", "Titles start at this quality; you can pick another in the player.",
            Picker("Streaming Quality", QualityOption.All.Select(q => q.Label).ToList(),
                () => QualityOption.Find(s.StreamingQuality).Label, v => s.StreamingQuality = v));

        var skip = Enum.GetValues<SegmentKind>().Select(kind => Picker(kind.Title(), Enum.GetValues<SkipMode>().Select(m => m.Title()).ToList(),
            () => s.SkipMode(kind).Title(), v => s.SkipModes[kind] = Enum.GetValues<SkipMode>().First(m => m.Title() == v))).ToList();
        skip.Add(Picker("Auto-Skip Countdown", PlayerSettings.AutoSkipDelays.Select(DelayLabel).ToList(),
            () => DelayLabel(PlayerSettings.AutoSkipDelays.Contains(s.AutoSkipDelay) ? s.AutoSkipDelay : 3),
            v => s.AutoSkipDelay = PlayerSettings.AutoSkipDelays.First(d => DelayLabel(d) == v)));
        Section(column, "Skip Intros & Credits",
            "Markers come from your server: the files' chapter names, the end credits it finds in them, and the community databases your server looks up. This device asks no one else.",
            [.. skip]);

        Section(column, "Audio", "Titles play in this language when they have it. Picking a language or subtitles in the player sets these.",
            LanguagePicker("Preferred Language", original: true, () => s.PreferredAudioLanguage, v => s.PreferredAudioLanguage = v));

        Section(column, "Subtitles", "How subtitles look on this device.",
            LanguagePicker("Preferred Language", original: false, () => s.PreferredSubtitleLanguage, v => s.PreferredSubtitleLanguage = v),
            Toggle("Turn On Automatically", () => s.AutoSelectSubtitles, v => s.AutoSelectSubtitles = v),
            Picker("Size", Enum.GetValues<SubtitleSize>().Select(z => z.Title()).ToList(),
                () => s.SubtitleSize.Title(), v => s.SubtitleSize = Enum.GetValues<SubtitleSize>().First(z => z.Title() == v)),
            Toggle("Background Box", () => s.SubtitleBackground, v => s.SubtitleBackground = v));

        var scroll = new ScrollViewer { HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = column };
        Children.Add(scroll);
    }

    private static string DelayLabel(double d) => d <= 0 ? "None" : $"{d:0} s";

    /// <summary>Back on this page: the player may have changed a preference meanwhile.</summary>
    public void Refresh()
    {
        _syncing = true;
        try { foreach (var sync in _syncs) sync(); }
        finally { _syncing = false; }
    }

    private void Changed(Action apply)
    {
        if (_syncing) return;
        apply();
        Prefs.Save();
    }

    /// <summary>A grouped section (Mac: Form .grouped): its header, the rows in a rounded box, the footer.</summary>
    private static void Section(StackPanel column, string header, string footer, params FrameworkElement[] rows)
    {
        var head = F.Text(header, Theme.FontSubheadline, F.SemiBold);
        head.Margin = new Thickness(12, 22, 0, 6);
        column.Children.Add(head);
        var box = new StackPanel { Background = Theme.Surface, CornerRadius = new CornerRadius(10) };
        for (var i = 0; i < rows.Length; i++)
        {
            if (i > 0) box.Children.Add(new Border { Height = 1, Margin = new Thickness(14, 0, 0, 0), Background = new SolidColorBrush(Color.FromArgb(20, 255, 255, 255)) });
            box.Children.Add(rows[i]);
        }
        column.Children.Add(box);
        var foot = F.Text(footer, Theme.FontCaption, fg: Theme.Secondary);
        foot.TextWrapping = TextWrapping.WrapWholeWords;
        foot.Margin = new Thickness(12, 6, 12, 0);
        column.Children.Add(foot);
    }

    private static Grid Row(string label, FrameworkElement control)
    {
        var row = new Grid { MinHeight = 44, Padding = new Thickness(14, 4, 10, 4) };
        row.ColumnDefinitions.Add(F.Star());
        row.ColumnDefinitions.Add(F.Auto);
        var text = F.Text(label, Theme.FontBody);
        text.VerticalAlignment = VerticalAlignment.Center;
        row.Children.Add(text.At(0, 0));
        control.VerticalAlignment = VerticalAlignment.Center;
        row.Children.Add(control.At(0, 1));
        return row;
    }

    private Grid Picker(string label, IReadOnlyList<string> choices, Func<string> get, Action<string> set)
    {
        var box = new ComboBox { MinWidth = 200, ItemsSource = choices.ToList() };
        void Sync() => box.SelectedIndex = Math.Max(0, choices.ToList().IndexOf(get()));
        Sync();
        _syncs.Add(Sync);
        box.SelectionChanged += (_, _) => { if (box.SelectedItem is string v) Changed(() => set(v)); };
        AutomationName(box, label);
        return Row(label, box);
    }

    private Grid LanguagePicker(string label, bool original, Func<string> get, Action<string> set)
    {
        // "Original" (each title's own default) for audio; the common languages, plus the current one.
        List<(string Code, string Name)> Choices() => [.. original ? [("", "Original")] : Array.Empty<(string, string)>(), .. PlayerSettings.LanguageChoices(get())];
        var choices = Choices();
        return Picker(label, choices.Select(c => c.Name).ToList(), () => choices.FirstOrDefault(c => c.Code == get()).Name ?? choices[0].Name,
            v => set(choices.First(c => c.Name == v).Code));
    }

    private Grid Toggle(string label, Func<bool> get, Action<bool> set)
    {
        // The template keeps room for a label after the switch (empty here): taken back, so the switch ends where the pickers do.
        var toggle = new ToggleSwitch { OnContent = "", OffContent = "", MinWidth = 0, IsOn = get(), Margin = new Thickness(0, 0, -12, 0) };
        _syncs.Add(() => toggle.IsOn = get());
        toggle.Toggled += (_, _) => Changed(() => set(toggle.IsOn));
        AutomationName(toggle, label);
        return Row(label, toggle);
    }

    private static void AutomationName(DependencyObject o, string name) => Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(o, name);
}
