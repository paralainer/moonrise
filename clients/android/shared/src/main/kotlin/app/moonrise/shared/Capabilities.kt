package app.moonrise.shared

import android.content.Context
import android.hardware.display.DisplayManager
import android.view.Display

// What this device can play, asked for when a playback session starts.

/**
 * Test seams for the player, set by an app's debug builds (the Dev app) from its launch intent.
 */
object PlayerSeams {
    /**
     * Players made from now on find no video decoder for the Original quality, as a device that
     * can't decode a file (the fallback after a switch).
     */
    var refuseOriginalVideo = false
    /** Players made from now on ask for lossless multichannel audio as if an HDMI output took more than 2 channels. */
    var lossless = false
    /** Players made from now on ask for Dolby Vision (`dv=1`) when HDR is on, as if the display showed it and a decoder existed. */
    var dolbyVision = false
}

/**
 * The audio output takes more than 2 channels (HDMI to a TV or receiver that accepts multichannel
 * PCM): lossless multichannel tracks are asked for as FLAC (decoded here, by the device or
 * Media3's FFmpeg decoder).
 */
fun multichannelOutput(context: Context): Boolean {
    if (PlayerSeams.lossless) return true
    val am = context.getSystemService(Context.AUDIO_SERVICE) as android.media.AudioManager
    val hdmi = setOf(
        android.media.AudioDeviceInfo.TYPE_HDMI, android.media.AudioDeviceInfo.TYPE_HDMI_ARC,
        if (android.os.Build.VERSION.SDK_INT >= 31) android.media.AudioDeviceInfo.TYPE_HDMI_EARC else android.media.AudioDeviceInfo.TYPE_HDMI_ARC,
    )
    val counts = am.getDevices(android.media.AudioManager.GET_DEVICES_OUTPUTS).filter { it.type in hdmi }.map { it.channelCounts }
    return app.moonrise.core.ClientRules.wantsLossless(counts)
}

/** The display shows HDR (else the server tone-maps HDR titles to SDR). */
fun displayHdr(context: Context): Boolean {
    val dm = context.getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
    @Suppress("DEPRECATION")
    val types = dm.getDisplay(Display.DEFAULT_DISPLAY)?.hdrCapabilities?.supportedHdrTypes ?: return false
    return types.isNotEmpty()
}

/**
 * Ask for Dolby Vision (`dv=1`): HDR is on, the display lists Dolby Vision, and a
 * `video/dolby-vision` decoder exists ([app.moonrise.core.ClientRules.wantsDolbyVision]).
 */
fun displayDolbyVision(context: Context, hdr: Boolean): Boolean {
    if (PlayerSeams.dolbyVision) return hdr
    if (!hdr) return false
    val dm = context.getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
    @Suppress("DEPRECATION")
    val types = dm.getDisplay(Display.DEFAULT_DISPLAY)?.hdrCapabilities?.supportedHdrTypes ?: return false
    val decoder = runCatching {
        android.media.MediaCodecList(android.media.MediaCodecList.REGULAR_CODECS).codecInfos.any { info ->
            !info.isEncoder && info.supportedTypes.any { it.equals(android.media.MediaFormat.MIMETYPE_VIDEO_DOLBY_VISION, ignoreCase = true) }
        }
    }.getOrDefault(false)
    return app.moonrise.core.ClientRules.wantsDolbyVision(hdr, types, decoder)
}
