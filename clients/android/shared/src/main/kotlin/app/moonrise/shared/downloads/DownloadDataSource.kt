package app.moonrise.shared.downloads

import android.net.Uri
import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.BaseDataSource
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSourceException
import androidx.media3.datasource.DataSpec
import androidx.media3.common.PlaybackException
import app.moonrise.core.DownloadRules
import java.io.ByteArrayInputStream
import java.io.File
import java.io.FileNotFoundException
import java.io.InputStream
import java.io.RandomAccessFile

/**
 * Plays a download from the device (`LocalHLSServer`): `moonrise-dl://<id>/<mode>/master.m3u8`
 * and everything under it comes from its `hls/` folder. A segment that isn't there is fetched
 * from the server right then (the request waits for it). Mode `l` (no server, download
 * incomplete): the playlists end where the downloaded part stops.
 */
@OptIn(UnstableApi::class)
class DownloadDataSource : BaseDataSource(false) {
    private var uri: Uri? = null
    private var file: RandomAccessFile? = null
    private var memory: InputStream? = null
    private var left = 0L

    override fun open(spec: DataSpec): Long {
        transferInitializing(spec)
        uri = spec.uri
        val id = spec.uri.authority ?: throw FileNotFoundException("no download")
        val path = spec.uri.pathSegments
        if (path.size < 2 || path.any { it == ".." }) throw FileNotFoundException(spec.uri.toString())
        val mode = path[0]
        val relative = path.drop(1).joinToString("/")
        Downloads.playerAsked(id, relative)
        val f = Downloads.file(id, "hls/$relative")
        if (relative.endsWith(".m3u8") && mode == "l") {
            val text = Downloads.growingPlaylist(id, relative) ?: throw FileNotFoundException(relative)
            memory = ByteArrayInputStream(text).also { it.skip(spec.position) }
            left = if (spec.length == C.LENGTH_UNSET.toLong()) text.size - spec.position else spec.length
        } else {
            if (!f.exists()) {
                val ok = runCatching { Downloads.fetchNow(id, relative, DownloadRules.FILE_WAIT_SECONDS) }.getOrElse { e ->
                    if (e is InterruptedException) throw java.io.InterruptedIOException() else false
                }
                if (!ok) throw FileNotFoundException("$relative isn't on the device")
            }
            val length = f.length()
            if (spec.position > length) throw DataSourceException(PlaybackException.ERROR_CODE_IO_READ_POSITION_OUT_OF_RANGE)
            file = RandomAccessFile(f, "r").also { it.seek(spec.position) }
            left = if (spec.length == C.LENGTH_UNSET.toLong()) length - spec.position else spec.length
        }
        transferStarted(spec)
        return left
    }

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
        if (length == 0) return 0
        if (left == 0L) return C.RESULT_END_OF_INPUT
        val wanted = minOf(left, length.toLong()).toInt()
        val n = file?.read(buffer, offset, wanted) ?: memory?.read(buffer, offset, wanted) ?: -1
        if (n == -1) return C.RESULT_END_OF_INPUT
        left -= n
        bytesTransferred(n)
        return n
    }

    override fun getUri(): Uri? = uri

    override fun close() {
        runCatching { file?.close() }
        file = null
        memory = null
        uri = null
        transferEnded()
    }

    class Factory : DataSource.Factory {
        override fun createDataSource(): DataSource = DownloadDataSource()
    }

    companion object {
        fun masterUri(id: String, growing: Boolean): String = "moonrise-dl://$id/${if (growing) "l" else "v"}/master.m3u8"
    }
}
