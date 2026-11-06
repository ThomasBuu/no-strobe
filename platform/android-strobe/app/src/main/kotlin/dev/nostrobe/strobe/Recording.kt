package dev.nostrobe.strobe

import android.annotation.SuppressLint
import android.content.ContentValues
import android.content.Context
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.Image
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import android.media.MediaRecorder
import android.net.Uri
import android.os.SystemClock
import android.provider.MediaStore
import android.util.Log
import java.nio.ByteBuffer
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Un enregistrement = deux vidéos dans Films/AntiStrobe : la traitée (seulement les images propres) et la brute
 * (toutes les images, pour ne rien perdre), avec le même son.
 */
class Recording(context: Context, width: Int, height: Int, orientation: Int, private val realtimeClock: Boolean) {
    private val stamp = SimpleDateFormat("yyyyMMdd-HHmmss", Locale.ROOT).format(Date())
    private val sinks = listOf(Mp4(context, "antistrobe-$stamp-traitee", orientation), Mp4(context, "antistrobe-$stamp-brute", orientation))
    val processed = VideoEncoder(width, height, sinks[0], ::relative)
    val raw = VideoEncoder(width, height, sinks[1], ::relative)
    private val audio = AudioEncoder(sinks, ::relative, ::now)
    @Volatile private var baseNs = -1L // premier instant enregistré, dans l'horloge des horodatages caméra

    /** Horodatage relatif au début de l'enregistrement (µs), ou -1 avant le début. */
    private fun relative(ns: Long): Long {
        if (baseNs < 0) baseNs = ns
        return if (ns < baseNs) -1 else (ns - baseNs) / 1000
    }

    /** Même horloge que les horodatages du capteur. */
    private fun now() = if (realtimeClock) SystemClock.elapsedRealtimeNanos() else System.nanoTime()

    fun finish() {
        audio.finish(); processed.finish(); raw.finish()
        sinks.forEach { it.finish() }
    }
}

/** Fichier MP4 dans la galerie ; démarre quand les pistes vidéo et son sont connues. */
class Mp4(context: Context, name: String, orientation: Int) {
    private val resolver = context.contentResolver
    private val uri: Uri = resolver.insert(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, ContentValues().apply {
        put(MediaStore.Video.Media.DISPLAY_NAME, "$name.mp4")
        put(MediaStore.Video.Media.MIME_TYPE, "video/mp4")
        put(MediaStore.Video.Media.RELATIVE_PATH, "Movies/AntiStrobe")
        put(MediaStore.Video.Media.IS_PENDING, 1)
    })!!
    private val fd = resolver.openFileDescriptor(uri, "rw")!!
    private val muxer = MediaMuxer(fd.fileDescriptor, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4).apply { setOrientationHint(orientation) }
    private var videoFormat: MediaFormat? = null; private var audioFormat: MediaFormat? = null
    private var videoTrack = -1; private var audioTrack = -1; private var started = false
    private val pending = ArrayList<Triple<Boolean, ByteArray, MediaCodec.BufferInfo>>() // avant démarrage (dont la 1re image clé)

    @Synchronized fun setFormat(video: Boolean, format: MediaFormat) {
        if (video) videoFormat = format else audioFormat = format
        val v = videoFormat; val a = audioFormat
        if (started || v == null || a == null) return
        videoTrack = muxer.addTrack(v); audioTrack = muxer.addTrack(a)
        muxer.start(); started = true
        pending.forEach { (isVideo, bytes, info) -> muxer.writeSampleData(if (isVideo) videoTrack else audioTrack, ByteBuffer.wrap(bytes), info) }
        pending.clear()
    }

    @Synchronized fun write(video: Boolean, buffer: ByteBuffer, info: MediaCodec.BufferInfo) {
        if (started) { muxer.writeSampleData(if (video) videoTrack else audioTrack, buffer, info); return }
        if (pending.size > 300) return
        val bytes = ByteArray(info.size); buffer.position(info.offset); buffer.get(bytes)
        pending.add(Triple(video, bytes, MediaCodec.BufferInfo().apply { set(0, info.size, info.presentationTimeUs, info.flags) }))
    }

    @Synchronized fun finish() {
        if (started) runCatching { muxer.stop() }.onFailure { Log.w("AntiStrobe", "mp4", it) }
        muxer.release(); fd.close()
        if (started) resolver.update(uri, ContentValues().apply { put(MediaStore.Video.Media.IS_PENDING, 0) }, null, null)
        else resolver.delete(uri, null, null)
    }
}

/** Encodeur H.264 alimenté image par image (copie des plans YUV de la caméra). */
class VideoEncoder(private val width: Int, private val height: Int, private val sink: Mp4, private val time: (Long) -> Long) {
    private val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC).apply {
        configure(MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible)
            setInteger(MediaFormat.KEY_BIT_RATE, 6_000_000)
            setInteger(MediaFormat.KEY_FRAME_RATE, 30)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
        }, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        start()
    }
    private val info = MediaCodec.BufferInfo()
    private var lastPts = -1L
    var dropped = 0; private set
    init { Log.i("AntiStrobe", "encodeur vidéo : ${codec.name}") }

    fun encode(image: Image, timestampNs: Long) {
        val pts = time(timestampNs)
        if (pts < 0 || pts <= lastPts) return
        val index = codec.dequeueInputBuffer(0)
        if (index < 0) { dropped++; drain(false); return } // encodeur saturé : image sautée
        val input = codec.getInputImage(index) ?: return
        copy(image, input)
        codec.queueInputBuffer(index, 0, width * height * 3 / 2, pts, 0)
        lastPts = pts
        drain(false)
    }

    fun finish() {
        Log.i("AntiStrobe", "encodeur ${codec.name} : $dropped images sautées")
        val index = codec.dequeueInputBuffer(10_000)
        if (index >= 0) codec.queueInputBuffer(index, 0, 0, maxOf(lastPts + 1, 0), MediaCodec.BUFFER_FLAG_END_OF_STREAM)
        drain(true)
        codec.stop(); codec.release()
    }

    private fun drain(untilEnd: Boolean) {
        var waits = 0
        while (true) {
            val i = codec.dequeueOutputBuffer(info, if (untilEnd) 10_000 else 0)
            when {
                i == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> sink.setFormat(true, codec.outputFormat)
                i >= 0 -> {
                    val out = codec.getOutputBuffer(i)!!
                    if (info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) sink.write(true, out, info)
                    codec.releaseOutputBuffer(i, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return
                }
                else -> if (!untilEnd || ++waits > 100) return // fin : 1 s au plus pour vider l'encodeur
            }
        }
    }

    // ponytail: copie pixel par pixel des plans de chrominance (dispositions caméra et encodeur souvent différentes) ;
    // ~5 ms par image en 720p. Copie par lignes si ça rame.
    private fun copy(src: Image, dst: Image) {
        for (p in 0..2) {
            val s = src.planes[p]; val d = dst.planes[p]
            val w = if (p == 0) width else width / 2; val h = if (p == 0) height else height / 2
            val sb = s.buffer; val db = d.buffer
            if (s.pixelStride == 1 && d.pixelStride == 1) {
                val row = ByteArray(w)
                for (y in 0 until h) { sb.position(y * s.rowStride); sb.get(row); db.position(y * d.rowStride); db.put(row) }
            } else {
                for (y in 0 until h) {
                    val so = y * s.rowStride; val dOff = y * d.rowStride
                    for (x in 0 until w) db.put(dOff + x * d.pixelStride, sb.get(so + x * s.pixelStride))
                }
            }
        }
    }
}

/** Micro → AAC, envoyé aux deux fichiers. */
class AudioEncoder(private val sinks: List<Mp4>, private val time: (Long) -> Long, private val now: () -> Long) {
    private val rate = 44_100
    private val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).apply {
        configure(MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, rate, 1).apply {
            setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
            setInteger(MediaFormat.KEY_BIT_RATE, 128_000)
        }, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        start()
    }
    @Volatile private var running = true
    private val thread = Thread(::loop, "micro").apply { start() }

    @SuppressLint("MissingPermission") // demandée par l'activité
    private fun loop() {
        val size = maxOf(AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT), 4096)
        val record = runCatching { AudioRecord(MediaRecorder.AudioSource.CAMCORDER, rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, size) }.getOrNull()
        if (record == null || record.state != AudioRecord.STATE_INITIALIZED) { Log.w("AntiStrobe", "micro indisponible"); return }
        record.startRecording()
        val info = MediaCodec.BufferInfo()
        var lastPts = -1L
        while (running) {
            val index = codec.dequeueInputBuffer(10_000)
            if (index < 0) { drain(info); continue }
            val buf = codec.getInputBuffer(index)!!.apply { clear() }
            val n = record.read(buf, minOf(buf.remaining(), size)) // lu directement dans le tampon de l'encodeur, à sa taille
            val pts = time(now() - maxOf(n, 0) / 2 * 1_000_000_000L / rate) // début du morceau lu
            if (n <= 0 || pts < 0 || pts <= lastPts) codec.queueInputBuffer(index, 0, 0, 0, 0)
            else { codec.queueInputBuffer(index, 0, n, pts, 0); lastPts = pts }
            drain(info)
        }
        record.stop(); record.release()
        val index = codec.dequeueInputBuffer(10_000)
        if (index >= 0) codec.queueInputBuffer(index, 0, 0, maxOf(lastPts + 1, 0), MediaCodec.BUFFER_FLAG_END_OF_STREAM)
        drain(info, untilEnd = true)
        codec.stop(); codec.release()
    }

    private fun drain(info: MediaCodec.BufferInfo, untilEnd: Boolean = false) {
        var waits = 0
        while (true) {
            val i = codec.dequeueOutputBuffer(info, if (untilEnd) 10_000 else 0)
            when {
                i == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> sinks.forEach { it.setFormat(false, codec.outputFormat) }
                i >= 0 -> {
                    val out = codec.getOutputBuffer(i)!!
                    if (info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) sinks.forEach { out.position(info.offset); it.write(false, out, info) }
                    codec.releaseOutputBuffer(i, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return
                }
                else -> if (!untilEnd || ++waits > 100) return
            }
        }
    }

    fun finish() { running = false; thread.join(2000) }
}
