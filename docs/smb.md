# Network shares (SMB)

Videos on a NAS or a Windows PC don't need to be mounted on the server: the Moonrise server reads SMB
shares itself (SMB 2 and 3, guest or with a user name and password).

**Adding a share:** in the app, *Manage Server → Libraries → Add Library* (or the setup's library
step) → **A Network Share**. File servers on your network are listed (Bonjour and NetBIOS); pick
one or type its address (`nas.local`, `192.168.1.20`), then the share and the folder. A user name
and password are asked for only when the share turns guests away; they are stored in the server's
database and never shown again.

Notes

- A share that is offline is left alone for that scan: nothing on it is marked missing, the rest
  of the library is scanned, and it is scanned in full once it answers again.
- Signing: by default the server signs only what the NAS requires (signed reads are several times
  slower on many NAS). `smb.signing = always` (or `MOONRISE_SMB_SIGNING=always`) signs everything.
- Windows 10/11 machines that don't announce themselves over NetBIOS aren't listed: type their
  name or address.
- Moving a library between a mounted folder and a share without losing watch state:
  `moonrise-server migrate-root` (server stopped; see [docs/dev/server.md](dev/server.md#network-shares)).
- Performance: one stream needs a few MB/s; the server keeps a small block cache
  (`MOONRISE_SMB_CACHE_MB`, 384) for the reads every ffmpeg run repeats.
