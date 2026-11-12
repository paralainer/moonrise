using System.Text.Json;

namespace Moonrise.Core;

// A library's folder in a network share, picked through the server (it reads the share itself;
// the app never talks to the file server): Mac SharePicker in SetupViews.swift and
// MoonriseSetupAPI.swift.

/// <summary>A file server the Moonrise server sees on its network (<c>/v1/admin/smb/discover</c>).</summary>
public sealed record FileServer { public required string Name { get; init; } public required string Host { get; init; } }
public sealed record FileServerList { public IReadOnlyList<FileServer> Servers { get; init; } = []; }

/// <summary>A share of a file server (<c>/v1/admin/smb/shares</c>).</summary>
public sealed record Share { public required string Name { get; init; } public string? Comment { get; init; } }
public sealed record ShareList { public IReadOnlyList<Share> Shares { get; init; } = []; }

/// <summary>A folder in a share (<c>/v1/admin/smb/browse</c>); paths are inside the share, "" its top.</summary>
public sealed record ShareListing
{
    public string Path { get; init; } = "";
    public string? Parent { get; init; }
    public IReadOnlyList<ServerFolder> Dirs { get; init; } = [];
    public int VideoFiles { get; init; }
}

/// <summary>Whether a share's folder works as a library folder (<c>/v1/admin/smb/test</c>).</summary>
public sealed record ShareTest
{
    public bool Ok { get; init; }
    public string Url { get; init; } = "";
    public int Entries { get; init; }
    public int VideoFiles { get; init; }
    public string? Error { get; init; }
}

/// <summary>A user name and password for a share, only asked for when the share needs one.</summary>
public sealed record ShareLogin(string Username, string Password);

public enum RootSource { Local, Smb }

/// <summary>A library's folder: on the server's computer, or in a network share.</summary>
public sealed record LibraryRoot(RootSource Source, string Path, string Host = "", string Share = "", ShareLogin? Login = null, int VideoFiles = 0)
{
    public static LibraryRoot Local(string path) => new(RootSource.Local, path);

    /// <summary>As <c>POST /v1/admin/libraries</c> takes it in <c>roots</c>.</summary>
    public Dictionary<string, object> Body()
    {
        if (Source == RootSource.Local) return new() { ["source"] = "local", ["path"] = Path };
        var b = new Dictionary<string, object> { ["source"] = "smb", ["host"] = Host, ["share"] = Share, ["path"] = Path };
        if (Login != null) { b["username"] = Login.Username; b["password"] = Login.Password; }
        return b;
    }

    /// <summary>As shown: "/media/movies", "nas › Media › movies".</summary>
    public string Display => Source == RootSource.Local ? Path : ShareRules.Breadcrumb(Host, Share, Path);
}

/// <summary>A network-share failure the server explains: <c>problem</c> (auth_required, wrong_password, access_denied, not_found, unreachable, other) and its message.</summary>
public sealed record ShareProblem(string Problem, string Message);

/// <summary>What to do after a failed request: ask for an account, and what to say.</summary>
public sealed record ShareOutcome(bool AskLogin, string? Error);

public static class ShareRules
{
    public const string ServersMessage = "File servers the Moonrise server sees on your network. Pick yours, or type its name or address.";
    public const string LoginMessage = "This file server doesn't let guests in here. Enter an account on it: the Moonrise server keeps it, only to read your videos.";
    public const string SharesMessage = "Shares on this file server. Pick the one with your videos.";

    /// <summary>The server's JSON answer to a share request that failed; null for any other answer.</summary>
    public static ShareProblem? Problem(MoonriseHttpException e)
    {
        try
        {
            using var doc = JsonDocument.Parse(e.Body);
            if (doc.RootElement.ValueKind != JsonValueKind.Object) return null;
            if (!doc.RootElement.TryGetProperty("problem", out var p) || p.ValueKind != JsonValueKind.String) return null;
            var problem = p.GetString()!;
            var message = doc.RootElement.TryGetProperty("error", out var m) && m.ValueKind == JsonValueKind.String ? m.GetString()! : problem;
            return new ShareProblem(problem, message);
        }
        catch (JsonException) { return null; }
    }

    private static string Capitalised(string s) => s.Length == 0 ? s : char.ToUpperInvariant(s[0]) + s[1..];

    /// <summary>The server's words for a share failure, else its plain answer.</summary>
    public static string Text(MoonriseHttpException e) => Problem(e) is { } p ? Capitalised(p.Message) : SetupRules.ServerText(e);

    /// <summary>Listing a file server's shares failed (Connect).</summary>
    public static ShareOutcome ConnectFailure(MoonriseHttpException e, string host) => Problem(e)?.Problem switch
    {
        "auth_required" => new(true, null),
        "wrong_password" => new(true, "The user name or password is wrong."),
        "unreachable" => new(false, $"The Moonrise server can't reach {host}. Check the name or address."),
        _ => new(false, Text(e)),
    };

    /// <summary>Opening a share's folder failed: shares can be listed as a guest while the share itself wants an account.</summary>
    public static ShareOutcome BrowseFailure(MoonriseHttpException e, bool hadLogin)
    {
        var p = Problem(e)?.Problem;
        if (p is "auth_required" or "wrong_password" or "access_denied")
            return new(true, !hadLogin ? null : p == "access_denied" ? "This account may not open that share." : "The user name or password is wrong.");
        return new(false, Text(e));
    }

    /// <summary>"Use This Folder": why the folder can't be a library folder, or null when it can.</summary>
    public static string? TestProblem(ShareTest t) => t.Ok ? null : "This folder is empty.";

    /// <summary>The folder above a share's folder; null at the share's top (back to the shares).</summary>
    public static string? Up(string path)
    {
        var t = path.Trim('/');
        if (t.Length == 0) return null;
        var i = t.LastIndexOf('/');
        return i < 0 ? "" : t[..i];
    }

    /// <summary>"nas › Media › Video › Movies".</summary>
    public static string Breadcrumb(string host, string share, string path) =>
        string.Join(" › ", new[] { host, share }.Concat(path.Split('/', StringSplitOptions.RemoveEmptyEntries)));

    /// <summary>A share folder's title: its name, or the share's at the top.</summary>
    public static string FolderTitle(string share, string path) => path.Trim('/').Length == 0 ? share : SetupRules.FolderName(path);
}
