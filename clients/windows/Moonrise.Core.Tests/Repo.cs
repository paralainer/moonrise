namespace Moonrise.Core.Tests;

internal static class Repo
{
    /// <summary>The repository's api/ folder, found from where the tests run.</summary>
    public static string ApiDir { get; } = Find();

    private static string Find()
    {
        for (var d = new DirectoryInfo(AppContext.BaseDirectory); d != null; d = d.Parent)
        {
            var p = Path.Combine(d.FullName, "api", "segment-rules.json");
            if (File.Exists(p)) return Path.Combine(d.FullName, "api");
        }
        throw new InvalidOperationException("api/ not found above " + AppContext.BaseDirectory);
    }
}
