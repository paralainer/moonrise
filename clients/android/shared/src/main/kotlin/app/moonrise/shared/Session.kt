package app.moonrise.shared

import android.content.Context
import android.os.Build
import android.provider.Settings
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import app.moonrise.core.MoonriseApi
import app.moonrise.shared.downloads.ServerReach
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import java.util.UUID

/**
 * What an app does when watch state may have changed (the TV app's Watch Next row follows the
 * server): set once when the app starts. Nothing by default.
 */
object WatchHooks {
    /** Watch state may have changed (signed in, a mark, playback stopped): `why` for the log. */
    var changed: (why: String) -> Unit = {}

    /** Signed out: what was shown for this account goes. */
    var signedOut: (why: String) -> Unit = {}
}

/**
 * The signed-in server: its address and this device's token, kept in the app's private
 * preferences. The token is encrypted with a key in the Android Keystore, which never leaves the
 * device (as the Apple apps keep theirs in the keychain).
 */
object Session {
    private const val PREFS = "session"
    private lateinit var app: Context

    /** This install's id, sent with sign-ins so the server lists the device once. */
    lateinit var deviceId: String
        private set

    /** The name the server shows for this device: the TV's own name. */
    val deviceName: String
        get() = runCatching { Settings.Global.getString(app.contentResolver, Settings.Global.DEVICE_NAME) }.getOrNull()
            ?.takeIf { it.isNotBlank() } ?: Build.MODEL

    var api: MoonriseApi? = null
        private set(v) {
            v?.awaitRoute = routeGate
            field = v
        }

    /** What the server's client waits for before each request: the route to it (`ServerReach`). */
    var routeGate: (suspend () -> Unit)? = null
        set(v) {
            field = v
            api?.awaitRoute = v
        }

    val main = android.os.Handler(android.os.Looper.getMainLooper())

    /** For work that outlives a screen (closing a playback session when the player goes). */
    val background = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** For UI work that outlives a menu (a mark chosen in a menu that closes at once). */
    val ui = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    fun init(context: Context) {
        app = context.applicationContext
        val p = prefs()
        deviceId = p.getString("device_id", null) ?: UUID.randomUUID().toString().also { p.edit().putString("device_id", it).apply() }
        val address = p.getString("address", null)
        // A token stored in the clear by an earlier build moves to the encrypted entry.
        p.getString("token", null)?.let { plain ->
            p.edit().putString("token_sealed", TokenCipher.seal(plain)).remove("token").apply()
        }
        val token = p.getString("token_sealed", null)?.let { TokenCipher.open(it) }
        if (address != null && token != null) {
            api = MoonriseApi.normalize(address)?.let { MoonriseApi(it, token, deviceId, deviceName) }
        }
    }

    /** A client for `address` without a token, for checking the server and signing in. */
    fun client(address: String): MoonriseApi? = MoonriseApi.normalize(address)?.let { MoonriseApi(it, null, deviceId, deviceName) }

    fun signedIn(client: MoonriseApi) {
        val token = client.token ?: return
        prefs().edit().putString("address", client.baseUrl.toString()).putString("token_sealed", TokenCipher.seal(token)).apply()
        api = client
        ServerReach.signedIn()
        WatchHooks.changed("signed in")
    }

    /** Forgets the token here and revokes it on the server (`POST /v1/auth/logout`), best effort. */
    fun signOut() {
        val old = api
        prefs().edit().remove("token_sealed").remove("token").apply()
        api = null
        WatchHooks.signedOut("signed out")
        if (old != null) background.launch { old.logout() }
    }

    val lastAddress: String? get() = prefs().getString("address", null)

    /**
     * The server's address on the home network, as signed in. `api.baseUrl` is where requests go
     * now, which away from home is a local forwarder to the server on the tailnet.
     */
    val homeAddress: okhttp3.HttpUrl? get() = lastAddress?.let { MoonriseApi.normalize(it) }

    /** Change Server: signs out and forgets the address, so sign-in starts with the servers found. */
    fun changeServer() {
        signOut()
        prefs().edit().remove("address").apply()
    }

    private fun prefs() = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}

/** AES-GCM with a key kept in the Android Keystore: "iv:ciphertext" in Base64. */
private object TokenCipher {
    private const val ALIAS = "moonrise-token"
    private const val KEYSTORE = "AndroidKeyStore"

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return gen.generateKey()
    }

    fun seal(plain: String): String {
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, key())
        val b64 = { b: ByteArray -> Base64.encodeToString(b, Base64.NO_WRAP) }
        return b64(c.iv) + ":" + b64(c.doFinal(plain.toByteArray(Charsets.UTF_8)))
    }

    /** null if it can't be read (say the key is gone): the app then asks to sign in again. */
    fun open(sealed: String): String? = runCatching {
        val (iv, data) = sealed.split(":", limit = 2).map { Base64.decode(it, Base64.NO_WRAP) }
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, iv))
        String(c.doFinal(data), Charsets.UTF_8)
    }.onFailure { Log.w("Moonrise", "stored sign-in can't be read: ${it.javaClass.simpleName}") }.getOrNull()
}
