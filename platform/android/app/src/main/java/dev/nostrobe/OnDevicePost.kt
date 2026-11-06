package dev.nostrobe

import android.media.Image
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.media.MediaMuxer
import java.io.File
import java.nio.ByteBuffer

/**
 * Post-processing on the phone: flattens whole-frame brightness pumping (the same idea as the
 * restorer's temporal gain). Two passes: decode to measure each frame's luma, then decode again,
 * scale the luma by the gain and encode to H.264. The row-banding gain is left to the desktop
 * tool (it needs the sensor row rate and costs far more on a phone).
 *
 * Runs on the calling thread; the clip must be H.264/HEVC video the device can decode.
 */
object OnDevicePost {
    private const val TIMEOUT_US = 10_000L
    private const val WINDOW = 15
    private const val MAX_GAIN = 4.0

    class PostException(message: String) : Exception(message)

    fun fix(src: File, dst: File, lineRateHz: Double = 0.0, onProgress: (Int) -> Unit) {
        val means = ArrayList<Double>()
        decode(src) { image, _ -> means.add(meanLuma(image)); onProgress(0) }
        if (means.isEmpty()) throw PostException("no frames decoded")
        val gains = temporalGain(means.toDoubleArray(), WINDOW, MAX_GAIN)
        encodeWithGains(src, dst, gains, lineRateHz, onProgress)
    }

    /** Same formula as `ns_core::restorer::temporal_gain`. */
    fun temporalGain(means: DoubleArray, window: Int, maxGain: Double): DoubleArray {
        val w = maxOf(window, 3) or 1
        val half = w / 2
        val n = means.size
        return DoubleArray(n) { i ->
            val lo = maxOf(0, i - half)
            val hi = minOf(n, i + half + 1)
            var sum = 0.0
            for (j in lo until hi) sum += means[j]
            val local = sum / (hi - lo)
            if (means[i] <= 0.0) 1.0 else (local / means[i]).coerceIn(1.0 / maxGain, maxGain)
        }
    }

    private fun meanLuma(image: Image): Double {
        val plane = image.planes[0]
        val buf = plane.buffer
        if (buf.isDirect) {
            return NsJni.meanLuma(buf, image.width, image.height, plane.rowStride, plane.pixelStride)
        }
        val rs = plane.rowStride
        val ps = plane.pixelStride
        var sum = 0L
        var count = 0
        var y = 0
        while (y < image.height) {
            var x = 0
            while (x < image.width) {
                sum += buf.get(y * rs + x * ps).toInt() and 0xFF
                count++
                x += 4
            }
            y += 2
        }
        return if (count == 0) 0.0 else sum / (count * 255.0)
    }

    private class Source(file: File) {
        val extractor = MediaExtractor().apply { setDataSource(file.path) }
        val track: Int
        val format: MediaFormat
        init {
            var t = -1
            for (i in 0 until extractor.trackCount) {
                if (extractor.getTrackFormat(i).getString(MediaFormat.KEY_MIME)?.startsWith("video/") == true) { t = i; break }
            }
            if (t < 0) throw PostException("no video track")
            track = t
            format = extractor.getTrackFormat(t)
            extractor.selectTrack(t)
        }
    }

    /** Decodes every frame in presentation order and hands each Image to [onFrame]. */
    private fun decode(file: File, onFrame: (Image, Long) -> Unit) {
        val src = Source(file)
        val mime = src.format.getString(MediaFormat.KEY_MIME)!!
        val codec = MediaCodec.createDecoderByType(mime)
        try {
            src.format.setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible)
            codec.configure(src.format, null, null, 0)
            codec.start()
            val info = MediaCodec.BufferInfo()
            var inputDone = false
            var outputDone = false
            while (!outputDone) {
                if (!inputDone) {
                    val i = codec.dequeueInputBuffer(TIMEOUT_US)
                    if (i >= 0) {
                        val buf = codec.getInputBuffer(i)!!
                        val n = src.extractor.readSampleData(buf, 0)
                        if (n < 0) {
                            codec.queueInputBuffer(i, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                            inputDone = true
                        } else {
                            codec.queueInputBuffer(i, 0, n, src.extractor.sampleTime, 0)
                            src.extractor.advance()
                        }
                    }
                }
                val o = codec.dequeueOutputBuffer(info, TIMEOUT_US)
                if (o >= 0) {
                    if (info.size > 0) {
                        val image = codec.getOutputImage(o)
                        if (image != null) { onFrame(image, info.presentationTimeUs); image.close() }
                    }
                    codec.releaseOutputBuffer(o, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) outputDone = true
                }
            }
        } finally {
            try { codec.stop() } catch (_: Exception) {}
            codec.release()
            src.extractor.release()
        }
    }

    private fun encodeWithGains(srcFile: File, dstFile: File, gains: DoubleArray, lineRateHz: Double, onProgress: (Int) -> Unit) {
        val rotation = MediaMetadataRetriever().use { r ->
            r.setDataSource(srcFile.path)
            r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)?.toIntOrNull() ?: 0
        }
        // The audio track (if any) is copied as is, so the fixed clip keeps its sound.
        val audioEx = MediaExtractor().apply { setDataSource(srcFile.path) }
        var audioFormat: MediaFormat? = null
        for (i in 0 until audioEx.trackCount) {
            val f = audioEx.getTrackFormat(i)
            if (f.getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true) {
                audioEx.selectTrack(i); audioFormat = f; break
            }
        }
        var audioTrackOut = -1
        var encoder: MediaCodec? = null
        var muxer: MediaMuxer? = null
        var track = -1
        var muxerStarted = false
        var frame = 0
        val info = MediaCodec.BufferInfo()

        fun drain(endOfStream: Boolean) {
            val enc = encoder ?: return
            while (true) {
                val o = enc.dequeueOutputBuffer(info, if (endOfStream) TIMEOUT_US else 0)
                when {
                    o == MediaCodec.INFO_TRY_AGAIN_LATER -> if (!endOfStream) return
                    o == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                        track = muxer!!.addTrack(enc.outputFormat)
                        audioFormat?.let { audioTrackOut = muxer!!.addTrack(it) }
                        muxer!!.start(); muxerStarted = true
                    }
                    o >= 0 -> {
                        val data = enc.getOutputBuffer(o)!!
                        if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) info.size = 0
                        if (info.size > 0 && muxerStarted) {
                            data.position(info.offset); data.limit(info.offset + info.size)
                            muxer!!.writeSampleData(track, data, info)
                        }
                        enc.releaseOutputBuffer(o, false)
                        if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return
                    }
                }
            }
        }

        try {
            decode(srcFile) { image, pts ->
                if (encoder == null) {
                    val w = image.width and 1.inv()
                    val h = image.height and 1.inv()
                    val fmt = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h).apply {
                        setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible)
                        setInteger(MediaFormat.KEY_BIT_RATE, 17_000_000)
                        setInteger(MediaFormat.KEY_FRAME_RATE, 30)
                        setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
                        // No B-frames: the muxer takes the input timestamps as decode timestamps.
                        if (android.os.Build.VERSION.SDK_INT >= 29) setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
                    }
                    encoder = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC).also {
                        it.configure(fmt, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                        it.start()
                    }
                    muxer = MediaMuxer(dstFile.path, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4).also {
                        it.setOrientationHint(rotation)
                    }
                }
                val enc = encoder!!
                var idx = enc.dequeueInputBuffer(-1)
                while (idx < 0) { drain(false); idx = enc.dequeueInputBuffer(TIMEOUT_US) }
                val dst = enc.getInputImage(idx) ?: throw PostException("encoder gives no input image")
                val g = gains.getOrElse(frame) { 1.0 }

                val rowGains: DoubleArray? = if (lineRateHz > 0.0) {
                    val p0 = image.planes[0]
                    val prof = NsJni.extractProfile(p0.buffer, image.width, image.height, p0.rowStride, p0.pixelStride)
                    NsJni.rowGain(prof, lineRateHz)
                } else null

                copyPlane(image.planes[0], image.width, image.height, dst.planes[0], dst.width, dst.height, g, rowGains)
                copyPlane(image.planes[1], image.width / 2, image.height / 2, dst.planes[1], dst.width / 2, dst.height / 2, 1.0, null)
                copyPlane(image.planes[2], image.width / 2, image.height / 2, dst.planes[2], dst.width / 2, dst.height / 2, 1.0, null)
                val size = dst.width * dst.height * 3 / 2
                dst.close()
                enc.queueInputBuffer(idx, 0, size, pts, 0)
                frame++
                if (frame % 10 == 0) onProgress(100 * frame / gains.size)
                drain(false)
            }
            val enc = encoder ?: throw PostException("no frames to encode")
            val eos = enc.dequeueInputBuffer(-1)
            enc.queueInputBuffer(eos, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
            drain(true)
            if (audioFormat != null && audioTrackOut >= 0 && muxerStarted) {
                val buf = ByteBuffer.allocate(1 shl 20)
                val bi = MediaCodec.BufferInfo()
                while (true) {
                    val n = audioEx.readSampleData(buf, 0)
                    if (n < 0) break
                    bi.set(0, n, audioEx.sampleTime, audioEx.sampleFlags)
                    muxer!!.writeSampleData(audioTrackOut, buf, bi)
                    audioEx.advance()
                }
            }
            onProgress(100)
        } finally {
            audioEx.release()
            try { encoder?.stop() } catch (_: Exception) {}
            encoder?.release()
            try { if (muxerStarted) muxer?.stop() } catch (_: Exception) {}
            muxer?.release()
        }
    }

    /** Copies a plane honouring both sides' row and pixel strides; applies [gain] and optional [rowGains]. */
    private fun copyPlane(
        s: Image.Plane,
        sw: Int,
        sh: Int,
        d: Image.Plane,
        dw: Int,
        dh: Int,
        gain: Double = 1.0,
        rowGains: DoubleArray? = null,
    ) {
        val w = minOf(sw, dw)
        val h = minOf(sh, dh)
        val sb = s.buffer
        val db = d.buffer
        if (sb.isDirect && db.isDirect) {
            NsJni.applyPlaneGains(
                sb,
                db,
                w,
                h,
                s.rowStride,
                s.pixelStride,
                d.rowStride,
                d.pixelStride,
                rowGains,
                gain,
            )
            return
        }
        val lut = if (gain != 1.0) IntArray(256) { v -> Math.round(v * gain).toInt().coerceIn(0, 255) } else null
        val row = ByteArray(maxOf(s.rowStride, d.rowStride))
        for (y in 0 until h) {
            val effLut = if (rowGains != null && y < rowGains.size) {
                val combined = gain * rowGains[y]
                IntArray(256) { v -> Math.round(v * combined).toInt().coerceIn(0, 255) }
            } else lut

            if (s.pixelStride == 1 && d.pixelStride == 1) {
                sb.position(y * s.rowStride)
                sb.get(row, 0, w)
                if (effLut != null) for (x in 0 until w) row[x] = effLut[row[x].toInt() and 0xFF].toByte()
                db.position(y * d.rowStride)
                db.put(row, 0, w)
            } else {
                for (x in 0 until w) {
                    var v = sb.get(y * s.rowStride + x * s.pixelStride).toInt() and 0xFF
                    if (effLut != null) v = effLut[v]
                    db.put(y * d.rowStride + x * d.pixelStride, v.toByte())
                }
            }
        }
    }
}
