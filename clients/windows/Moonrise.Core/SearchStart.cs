namespace Moonrise.Core;

/// <summary>What Search offers before anything is typed (capability search-start): Recent and Suggestions.</summary>
public sealed record SearchStart
{
    public const int RecentKept = 20;

    public IReadOnlyList<Item> Recent { get; init; } = [];
    public IReadOnlyList<Item> Suggestions { get; init; } = [];

    public bool IsEmpty => Recent.Count == 0 && Suggestions.Count == 0;

    /// <summary>A title was opened from Search: first in Recent at once, and no longer suggested.</summary>
    public SearchStart Opened(Item item) => this with
    {
        Recent = new[] { item }.Concat(Recent.Where(i => i.Id != item.Id)).Take(RecentKept).ToList(),
        Suggestions = Suggestions.Where(i => i.Id != item.Id).ToList(),
    };

    public SearchStart RecentCleared() => this with { Recent = [] };

    /// <summary>The rows are shown only while nothing is typed.</summary>
    public static bool Shows(string query) => string.IsNullOrWhiteSpace(query);
}
