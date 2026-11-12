namespace Moonrise.Core;

// Setting up a new server from the app (Mac: SetupViews.swift), and joining one with an
// invitation (Mac: InviteSignInView): the rules and texts, testable without a window. A network
// share as the library's folder: Shares.cs.

/// <summary>A library as the admin API lists and adds it.</summary>
public sealed record AdminLibrary { public required string Id { get; init; } public required string Name { get; init; } public string? Kind { get; init; } }
public sealed record AdminLibraryList { public IReadOnlyList<AdminLibrary> Items { get; init; } = []; }

/// <summary>A folder of the server's own (<c>/v1/admin/fs/browse</c>).</summary>
public sealed record ServerFolder { public required string Name { get; init; } public required string Path { get; init; } }

public sealed record FolderListing
{
    /// <summary>Null: the list of folders browsing may start at.</summary>
    public string? Path { get; init; }
    public string? Parent { get; init; }
    public IReadOnlyList<ServerFolder> Dirs { get; init; } = [];
    public int VideoFiles { get; init; }
    public bool? Truncated { get; init; }
}

public sealed record ScanProgress
{
    public sealed record IdentifyCount { public int Done { get; init; } public int Total { get; init; } }
    public sealed record LibraryCount
    {
        public required string Id { get; init; }
        public required string Name { get; init; }
        public int Files { get; init; }
        public int Identified { get; init; }
        public int Movies { get; init; }
        public int Series { get; init; }
        public int Episodes { get; init; }
        public int Review { get; init; }
        public int? ReviewEpisodes { get; init; }
        public int Settling { get; init; }
    }
    public sealed record Pass { public string? Error { get; init; } public string? Aborted { get; init; } }

    /// <summary>idle, queued, scanning, identifying, metadata, finishing.</summary>
    public string Phase { get; init; } = "idle";
    public int Walked { get; init; }
    public IdentifyCount Identify { get; init; } = new();
    public int Passes { get; init; }
    public Pass? LastPass { get; init; }
    public IReadOnlyList<LibraryCount> Libraries { get; init; } = [];
}

public static class SetupRules
{
    public const string UnclaimedNote =
        "This server isn't set up yet. You'll need its setup code (in the server's log, or on its page in a browser), then you create its admin account and add your videos.";

    /// <summary>The setup code as typed: digits only (a pasted "123 456" too), at most six.</summary>
    public static string CodeDigits(string typed) => new(typed.Where(char.IsAsciiDigit).Take(6).ToArray());

    /// <summary>"192.168.1.10:8097": where the server's setup page is.</summary>
    public static string PageAddress(Uri server)
    {
        var s = server.ToString().TrimEnd('/');
        return s.StartsWith("http://", StringComparison.Ordinal) ? s[7..] : s;
    }

    public static string CodeMessage(Uri server) =>
        $"This server is new. To show it's yours, enter the 6-digit setup code from the server's log, or open http://{PageAddress(server)} in a browser on your network.";

    /// <summary>Why the account can't be made yet: "" for a field still empty, null when it can.</summary>
    public static string? AccountProblem(string username, string password, string confirm)
    {
        if (username.Trim().Length == 0) return "";
        if (password.Length < 8) return "The password needs at least 8 characters.";
        if (confirm.Length == 0) return "";
        if (confirm != password) return "The passwords don't match.";
        return null;
    }

    /// <summary>Why the invitation can't be redeemed yet: "" for a field still empty, null when it can.</summary>
    public static string? InviteProblem(string code, string username, string password)
    {
        if (code.Count(char.IsLetterOrDigit) != 8) return "";
        if (username.Trim().Length == 0) return "";
        if (password.Length < 8) return "The password needs at least 8 characters.";
        return null;
    }

    /// <summary>A setup code the server turned down (<c>/v1/setup/verify</c>).</summary>
    public static string VerifyError(MoonriseHttpException e) => e.Code switch
    {
        403 => "That isn't the code. Check the server's log or its page, and try again.",
        409 => "This server has been set up already. Go back and sign in.",
        429 => "Too many wrong codes. Wait a few minutes; the server may show a new code.",
        _ => ServerText(e),
    };

    /// <summary>The account the server turned down (<c>/v1/setup/claim</c>).</summary>
    public static string ClaimError(MoonriseHttpException e) => e.Code switch
    {
        403 => "The setup code has changed (the server restarted, or it was guessed at). Go back and enter the new one.",
        409 => "This server has just been set up from another device. Go back and sign in.",
        _ => ServerText(e),
    };

    public static string InviteError(MoonriseHttpException e) => e.Code == 429 ? "Too many wrong codes. Try again in a few minutes." : ServerText(e);

    /// <summary>The server's own words for a refusal (capitalised), else the status.</summary>
    public static string ServerText(MoonriseHttpException e) =>
        e.Code < 500 && e.Body.Trim().Length > 0 ? char.ToUpperInvariant(e.Body.Trim()[0]) + e.Body.Trim()[1..] : e.Message;

    /// <summary>"3 videos and 2 folders in here." (Mac: videosText).</summary>
    public static string VideosText(int videos, int folders)
    {
        var v = videos == 1 ? "1 video" : $"{videos} videos";
        var f = folders == 1 ? "1 folder" : $"{folders} folders";
        var here = (videos, folders) switch
        {
            (0, 0) => "Nothing in here yet.",
            (0, _) => $"{f} in here.",
            (_, 0) => $"{v} in here.",
            _ => $"{v} and {f} in here.",
        };
        return here + " Moonrise looks through every folder inside the one you choose.";
    }

    public const string LibraryMessage =
        "Where are your videos? Pick a folder the server's computer can see, or a share on your NAS or another computer. Movies and shows can share a folder: Moonrise tells them apart. You can add more folders later.";

    public const string NoServerFolders =
        "The server can't see any folders. Make your videos available to it (in Docker: mount the folder into the container), or use a network share.";

    /// <summary>A folder's name from its path ("/media/Movies" → "Movies").</summary>
    public static string FolderName(string path)
    {
        var t = path.TrimEnd('/', '\\');
        var i = t.LastIndexOfAny(['/', '\\']);
        return i >= 0 && i < t.Length - 1 ? t[(i + 1)..] : (t.Length == 0 ? path : t);
    }
}

/// <summary>An optional service (<c>/v1/admin/services</c>): its state, and what Moonrise does with it and without it.</summary>
public sealed record Service
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    /// <summary>configured, builtin, not_set, off.</summary>
    public string State { get; init; } = "not_set";
    public string Summary { get; init; } = "";
    public string With { get; init; } = "";
    public string Without { get; init; } = "";
}
public sealed record ServiceList { public IReadOnlyList<Service> Services { get; init; } = []; }

/// <summary>Step 5, Extras (Mac: SetupExtrasView): own keys and accounts, all optional.</summary>
public static class ExtrasRules
{
    public const string Message = "Moonrise works without any of these. Each adds something; add your own keys or accounts now or skip — nothing here is required.";

    /// <summary>The services with fields of their own here; the others are only described.</summary>
    public static bool HasFields(Service s) => s.State != "off" && s.Id is "tmdb" or "opensubtitles" or "llm";

    /// <summary>What to save (<c>PATCH /v1/admin/settings</c>, dotted keys): only what was filled in; an OpenSubtitles account needs both.</summary>
    public static Dictionary<string, string> Changes(string tmdbKey, string osUser, string osPass, string llmKey)
    {
        var c = new Dictionary<string, string>();
        if (tmdbKey.Trim().Length > 0) c["tmdb.api_key"] = tmdbKey.Trim();
        if (osUser.Trim().Length > 0 && osPass.Length > 0) { c["opensubtitles.username"] = osUser.Trim(); c["opensubtitles.password"] = osPass; }
        if (llmKey.Trim().Length > 0) c["llm.api_key"] = llmKey.Trim();
        return c;
    }

    public static string FinishTitle(int changes) => changes == 0 ? "Finish" : "Save and Finish";

    /// <summary>The state's mark: done, off, or not set up.</summary>
    public static string Glyph(string state) => state switch
    {
        "configured" or "builtin" => "",
        "off" => "",
        _ => "",
    };
}

/// <summary>
/// The scan step's polling: on while the step shows, off once it's left. A poll started before
/// leaving finds it off (or a newer round on) when its answer comes, and doesn't go on.
/// </summary>
public sealed class PollGate
{
    private int _round;

    public bool On { get; private set; }

    /// <summary>Turns polling on; the round a poll carries.</summary>
    public int Start() { On = true; return ++_round; }

    public void Stop() { On = false; _round++; }

    /// <summary>The current round, for a poll to carry over its request.</summary>
    public int Round => _round;

    /// <summary>After a poll's answer: may this round go on (draw it, poll again)?</summary>
    public bool Continues(int round) => On && round == _round;
}

/// <summary>The first scan as the setup shows it (Mac: SetupScanView), from the polled progress.</summary>
public sealed class ScanWatch(int passesBefore, string? libraryId)
{
    private int _idlePolls;

    public ScanProgress? Progress { get; private set; }
    public bool Done { get; private set; }

    public ScanProgress.LibraryCount? Library =>
        Progress?.Libraries.FirstOrDefault(l => l.Id == libraryId) ?? Progress?.Libraries.FirstOrDefault();

    /// <summary>A poll's answer. Done once a pass after the library was added is over, and it found files (or three idle polls).</summary>
    public void Update(ScanProgress p)
    {
        Progress = p;
        var finished = p.Phase == "idle" && p.Passes > passesBefore;
        _idlePolls = finished ? _idlePolls + 1 : 0;
        if (finished && ((Library?.Files ?? 0) > 0 || _idlePolls >= 3)) Done = true;
    }

    public string PhaseText => Done ? "Your library is ready." : Progress?.Phase switch
    {
        "scanning" => "Looking for videos…",
        "identifying" => Progress.Identify.Total > 0 ? $"Recognizing titles… {Progress.Identify.Done} of {Progress.Identify.Total}" : "Recognizing titles…",
        "metadata" => "Getting artwork and details…",
        "finishing" => "Finishing…",
        _ => "Starting the scan…",
    };

    /// <summary>Movies, shows and episodes as they're recognized; the files found until then.</summary>
    public IReadOnlyList<(int Count, string Label)> Stats
    {
        get
        {
            var l = Library;
            var files = Math.Max(l?.Files ?? 0, Progress?.Phase == "scanning" ? Progress.Walked : 0);
            if (l == null || l.Movies + l.Series == 0) return [(files, files == 1 ? "File" : "Files"), (0, "Movies"), (0, "Shows")];
            return [(l.Movies, l.Movies == 1 ? "Movie" : "Movies"), (l.Series, l.Series == 1 ? "Show" : "Shows"), (l.Episodes, l.Episodes == 1 ? "Episode" : "Episodes")];
        }
    }

    public IReadOnlyList<string> Notes
    {
        get
        {
            var o = new List<string>();
            var l = Library;
            if (Done && (Progress?.LastPass?.Aborted ?? Progress?.LastPass?.Error) is { } e) o.Add("The scan had a problem: " + e);
            if (l is { Settling: > 0 })
                o.Add(l.Settling == 1 ? "1 file is still being copied: it's added a few minutes after it stops changing."
                    : $"{l.Settling} files are still being copied: they're added a few minutes after they stop changing.");
            if (l != null && Done)
            {
                var eps = l.ReviewEpisodes ?? 0;
                var what = l.Review == 1 ? "1 title" : $"{l.Review} titles";
                var epText = eps == 1 ? "1 episode" : $"{eps} episodes";
                if (l.Review > 0 && eps == 0) o.Add($"{what} couldn't be recognized for sure: {(l.Review == 1 ? "it's" : "they're")} in the library under the names of the files and folders.");
                else if (l.Review == 0 && eps > 0) o.Add($"{epText} couldn't be matched for sure: {(eps == 1 ? "it's" : "they're")} in the library under the file names.");
                else if (l.Review > 0) o.Add($"{what} and {epText} couldn't be recognized for sure: they're in the library under the names of the files and folders.");
            }
            if (Done && l is { Files: 0 }) o.Add("No videos were found in that folder.");
            if (!Done) o.Add("You can continue: the scan goes on in the background.");
            return o;
        }
    }
}
