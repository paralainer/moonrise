using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Moonrise.Core;

namespace Moonrise.App;

/// <summary>The server and sign-in this app uses, kept per Windows user (the token encrypted with DPAPI).</summary>
public sealed class Session
{
    private sealed record Stored(string Server, string? ProtectedToken, string DeviceId, string? UserName);

    private static string FilePath => Path.Combine(AppInfo.DataDir, "session.json");

    public MoonriseApi? Api { get; private set; }
    public string? UserName { get; private set; }
    public string DeviceId { get; private set; } = Guid.NewGuid().ToString("N");

    public static Session Load()
    {
        var s = new Session();
        try
        {
            if (!File.Exists(FilePath)) return s;
            var st = JsonSerializer.Deserialize<Stored>(File.ReadAllText(FilePath))!;
            s.DeviceId = st.DeviceId;
            s.UserName = st.UserName;
            string? token = null;
            if (st.ProtectedToken != null)
                token = Encoding.UTF8.GetString(ProtectedData.Unprotect(Convert.FromBase64String(st.ProtectedToken), null, DataProtectionScope.CurrentUser));
            if (token != null) s.Api = new MoonriseApi(new Uri(st.Server), token, s.DeviceId, Environment.MachineName);
        }
        catch { /* start signed out */ }
        return s;
    }

    public void SignedIn(MoonriseApi api, AuthResult auth)
    {
        Api = api;
        UserName = auth.User.Name;
        Directory.CreateDirectory(AppInfo.DataDir);
        var enc = Convert.ToBase64String(ProtectedData.Protect(Encoding.UTF8.GetBytes(auth.Token), null, DataProtectionScope.CurrentUser));
        File.WriteAllText(FilePath, JsonSerializer.Serialize(new Stored(api.BaseUrl.ToString(), enc, DeviceId, auth.User.Name)));
    }

    public async Task SignOut()
    {
        if (Api != null) await Api.Logout();
        Api = null;
        try { File.Delete(FilePath); } catch { }
    }

    public MoonriseApi NewApi(Uri server) => new(server, null, DeviceId, Environment.MachineName);
}
