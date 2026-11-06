package dev.nostrobe

import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * Offload to a computer running `no-strobe-post serve`: the clip is POSTed to /fix and the
 * corrected clip comes back in the response. Runs on the calling thread (use a worker).
 */
object DesktopPost {
    class PostException(message: String) : Exception(message)

    /**
     * @param base server address, e.g. "http://192.168.1.20:8765"
     * @param lineRateHz sensor row readout rate (0 when unknown: only whole-frame pumping is fixed)
     * @param onProgress 0..100, first the upload then the download
     */
    fun send(
        clip: File, out: File, base: String, token: String, lineRateHz: Double,
        onProgress: (Int) -> Unit,
    ) {
        val root = base.trim().trimEnd('/').let { if (it.contains("://")) it else "http://$it" }
        val conn = URL("$root/fix?line_rate=${"%.0f".format(java.util.Locale.US, lineRateHz)}")
            .openConnection() as HttpURLConnection
        try {
            conn.requestMethod = "POST"
            conn.doOutput = true
            conn.connectTimeout = 10_000
            conn.readTimeout = 30 * 60_000  // the computer re-encodes the whole clip
            conn.setFixedLengthStreamingMode(clip.length())
            conn.setRequestProperty("Content-Type", "video/mp4")
            if (token.isNotEmpty()) conn.setRequestProperty("Authorization", "Bearer $token")

            val total = clip.length().coerceAtLeast(1)
            conn.outputStream.use { o ->
                clip.inputStream().use { input ->
                    val buf = ByteArray(1 shl 16)
                    var sent = 0L
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        o.write(buf, 0, n)
                        sent += n
                        onProgress((50 * sent / total).toInt())
                    }
                }
            }
            val code = conn.responseCode
            if (code != 200) {
                val msg = (conn.errorStream ?: conn.inputStream).bufferedReader().use { it.readText() }
                throw PostException("server answered $code: ${msg.take(200)}")
            }
            val size = conn.contentLengthLong.coerceAtLeast(1)
            conn.inputStream.use { input ->
                out.outputStream().use { o ->
                    val buf = ByteArray(1 shl 16)
                    var got = 0L
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        o.write(buf, 0, n)
                        got += n
                        onProgress(50 + (50 * got / size).toInt())
                    }
                }
            }
        } finally {
            conn.disconnect()
        }
    }
}
