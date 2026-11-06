package dev.nostrobe

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.media.CamcorderProfile
import android.media.MediaRecorder
import android.os.Build
import android.os.Bundle
import android.util.Size
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.widget.Button
import android.widget.SeekBar
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import java.io.File
import android.content.ContentValues
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import kotlin.math.ln
import kotlin.math.min
import kotlin.math.pow

/**
 * Preview, status readout, and record button. The shutter is driven by the Rust controller.
 */
class MainActivity : AppCompatActivity() {
    private companion object {
        const val MAX_VIDEO_BITRATE = 30_000_000
    }

    private var camera: NoStrobeCamera? = null
    private var recorder: MediaRecorder? = null
    private lateinit var surfaceView: SurfaceView
    private lateinit var focusRing: android.widget.ImageView
    private lateinit var status: TextView
    private lateinit var recordButton: android.view.View
    private lateinit var lensRow: android.widget.LinearLayout
    private lateinit var flipButton: Button
    private lateinit var postButton: Button
    private lateinit var micButton: Button
    private lateinit var gridButton: Button
    private lateinit var timerButton: Button
    private lateinit var gridView: android.view.View
    private lateinit var countdownText: TextView
    private var micOn = true
    private var timerSecs = 0
    // The last clip recorded (in the cache) and the row rate it was shot with.
    private var lastClip: File? = null
    private var lastRowRate = 0.0
    private var recFps = 30
    // The flicker lock in force when the recording starts: the camera is reopened with the
    // recorder surface and must start on it, not search (probing exposures ruin a clip).
    private var carryExposureUs = 0.0
    private var carryHz = 0.0
    private lateinit var zoomBar: SeekBar
    private lateinit var zoomText: TextView
    private lateinit var exposureBar: SeekBar
    private lateinit var exposureText: TextView

    // Lens and zoom. The default is a pinned physical lens: the phone's logical camera
    // switches lenses by itself, which moves the sensor under the flicker measurement.
    private var lensOptions: List<LensOption> = emptyList()
    private var lensIndex = -1
    private var zoomRatio = 1f
    private var zoomMin = 1f
    private var zoomMax = 1f
    // Exposure compensation: index range and step come from the chosen camera.
    private var evBias = 0f
    private var evStep = 0f
    private var evMinIndex = 0
    private var pendingMeta: java.io.Writer? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.main)
        surfaceView = findViewById(R.id.preview)
        focusRing = findViewById(R.id.focusRing)
        surfaceView.setOnTouchListener { v, e ->
            if (e.action == android.view.MotionEvent.ACTION_UP) {
                camera?.focusAt(e.x / v.width, e.y / v.height)
                showFocusRing(e.x, e.y)
                v.performClick()
            }
            true
        }
        status = findViewById(R.id.status)
        recordButton = findViewById(R.id.record)
        recordButton.setOnClickListener {
            if (recorder == null && timerSecs > 0) startCountdown(timerSecs) { toggleRecord() } else toggleRecord()
        }
        micButton = findViewById(R.id.mic)
        gridButton = findViewById(R.id.gridToggle)
        timerButton = findViewById(R.id.timer)
        gridView = findViewById(R.id.grid)
        countdownText = findViewById(R.id.countdown)
        micOn = prefs().getBoolean("mic_on", true)
        timerSecs = prefs().getInt("timer_secs", 0)
        gridView.visibility = if (prefs().getBoolean("grid_on", false)) android.view.View.VISIBLE else android.view.View.GONE
        refreshToggles()
        micButton.setOnClickListener {
            micOn = !micOn
            prefs().edit().putBoolean("mic_on", micOn).apply()
            refreshToggles()
        }
        gridButton.setOnClickListener {
            val on = gridView.visibility != android.view.View.VISIBLE
            gridView.visibility = if (on) android.view.View.VISIBLE else android.view.View.GONE
            prefs().edit().putBoolean("grid_on", on).apply()
            refreshToggles()
        }
        timerButton.setOnClickListener {
            timerSecs = when (timerSecs) { 0 -> 3; 3 -> 10; else -> 0 }
            prefs().edit().putInt("timer_secs", timerSecs).apply()
            refreshToggles()
        }
        lensRow = findViewById(R.id.lensRow)
        flipButton = findViewById(R.id.flip)
        zoomBar = findViewById(R.id.zoom)
        zoomText = findViewById(R.id.zoomText)
        flipButton.setOnClickListener { flipCamera() }
        postButton = findViewById(R.id.post)
        postButton.setOnClickListener { showPostMenu() }
        exposureBar = findViewById(R.id.exposureBar)
        exposureText = findViewById(R.id.exposureText)
        exposureBar.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(bar: SeekBar, progress: Int, fromUser: Boolean) {
                if (!fromUser || evStep <= 0f) return
                evBias = (evMinIndex + progress) * evStep
                exposureText.text = "%+.1f EV".format(evBias)
                camera?.setExposureBias(evBias)
            }
            override fun onStartTrackingTouch(bar: SeekBar) {}
            override fun onStopTrackingTouch(bar: SeekBar) {}
        })
        // A tap on the value resets the bias.
        exposureText.setOnClickListener {
            if (evStep <= 0f) return@setOnClickListener
            evBias = 0f
            exposureBar.progress = -evMinIndex
            exposureText.text = "%+.1f EV".format(0f)
            camera?.setExposureBias(0f)
        }
        findViewById<Button>(R.id.zoomOut).setOnClickListener { stepZoom(1f / 1.25f) }
        findViewById<Button>(R.id.zoomIn).setOnClickListener { stepZoom(1.25f) }
        zoomBar.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(bar: SeekBar, progress: Int, fromUser: Boolean) {
                if (!fromUser || zoomMax <= zoomMin) return
                zoomRatio = zoomMin * (zoomMax / zoomMin).pow(progress / 100f)
                zoomText.text = "%.1fx".format(zoomRatio)
                camera?.setZoom(zoomRatio)
            }
            override fun onStartTrackingTouch(bar: SeekBar) {}
            override fun onStopTrackingTouch(bar: SeekBar) {}
        })

        // The SurfaceView surface is created and destroyed with the window, so the camera
        // session must follow the surface, not the activity lifecycle. A Surface cached
        // across pause/resume is abandoned and createCaptureSession throws.
        surfaceView.holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(h: SurfaceHolder) = startCamera()
            override fun surfaceChanged(h: SurfaceHolder, format: Int, w: Int, hgt: Int) {}
            override fun surfaceDestroyed(h: SurfaceHolder) {
                camera?.stop(); camera = null
            }
        })

        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA)
            != PackageManager.PERMISSION_GRANTED
        ) {
            ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.CAMERA), 1)
        }
    }

    override fun onPause() {
        camera?.closeMeta()
        recorder?.stop(); recorder?.release(); recorder = null
        recordButton.isSelected = false
        super.onPause()
    }

    private fun startCamera() {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA)
            != PackageManager.PERMISSION_GRANTED
        ) return  // started again from onRequestPermissionsResult
        val manager = getSystemService(CAMERA_SERVICE) as CameraManager
        if (lensOptions.isEmpty()) {
            lensOptions = LensOptions.list(manager)
            lensIndex = LensOptions.defaultIndex(lensOptions)
        }
        val option = lensOptions[lensIndex]
        configureZoom(manager, option)
        configureExposure(manager, option)
        val holder = surfaceView.holder
        // Do not call holder.setFixedSize: resizing recreates the surface buffers and the
        // Surface object read after it is the pre-resize one, which createCaptureSession
        // rejects with "Surface was abandoned".
        camera = NoStrobeCamera(
            manager, option.logicalId, holder.surface, recorder, pwmMode = !intent.getBooleanExtra("strobe_mode", false),
            // While recording, do not pin a physical lens: on a Pixel 11 Pro Fold the recorder
            // surface of a pinned-lens stream received garbage frames (pure noise). The lock
            // (an exposure that is a multiple of the period) does not depend on the sensor, so
            // the logical camera is fine; the lens chip becomes a zoom ratio instead.
            physicalId = if (recorder != null) null else option.physicalId,
            zoomRatio = if (recorder != null && option.physicalId != null) logicalZoomFor(option) else zoomRatio,
            evBias = evBias,
            simFlickerHz = intent.getDoubleExtra("sim_hz", 0.0),
            seedExposureUs = if (recorder != null) carryExposureUs else 0.0,
            seedHz = if (recorder != null) carryHz else 0.0,
            holdAutoNoSeed = recorder != null,
            forceAutoOnly = intent.getBooleanExtra("force_auto_only", false),
            // Recording: no analysis stream (preview + recorder only). With the third stream a
            // Pixel 11 Pro Fold wrote pure noise into the clip; the lock is applied open loop.
            dumpProfiles = intent.getBooleanExtra("dump_profiles", false),
            outDir = externalCacheDir ?: cacheDir,
            startLevel = if (recorder != null) 2 else intent.getIntExtra("session_level", 0),
            recordFps = recFps,
            onReady = {
                // Session configured with the recorder surface: start recording now.
                val r = recorder
                if (r != null) {
                    try {
                        r.start()
                    } catch (e: RuntimeException) {
                        android.util.Log.w("nostrobe", "recorder did not start: ${e.message}")
                    }
                }
            },
        ) { cmd ->
            var s = describe(cmd)
            // Live correction is not available: tell the user when the clip will need it.
            if (recorder != null && cmd.state != NsJni.STATE_LOCKED) s += "  [post needed]"
            android.util.Log.d("nostrobe", s)
            runOnUiThread { status.text = s }
        }
        camera?.meta = pendingMeta
        pendingMeta = null
        camera?.start()
    }

    /** Range and step of the exposure compensation of the chosen camera. */
    private fun configureExposure(manager: CameraManager, option: LensOption) {
        val chars = manager.getCameraCharacteristics(option.logicalId)
        val range = chars.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE)
        val step = chars.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_STEP)?.toFloat() ?: 0f
        if (range == null || step <= 0f || range.upper <= range.lower) {
            evStep = 0f
            exposureBar.isEnabled = false
            exposureBar.max = 0
            exposureText.text = "n/a"
            return
        }
        evStep = step
        evMinIndex = range.lower
        exposureBar.isEnabled = true
        exposureBar.max = range.upper - range.lower
        evBias = evBias.coerceIn(range.lower * step, range.upper * step)
        exposureBar.progress = Math.round(evBias / step) - evMinIndex
        exposureText.text = "%+.1f EV".format(evBias)
    }

    /** Zoom ratio on the logical camera that frames like [option]'s lens times the user zoom. */
    private fun logicalZoomFor(option: LensOption): Float {
        val main = lensOptions.getOrNull(LensOptions.defaultIndex(lensOptions))?.equivMm ?: 0.0
        val ratio = if (main > 0.0 && option.equivMm > 0.0) (option.equivMm / main).toFloat() else 1f
        var lo = 0.5f
        var hi = 8f
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val manager = getSystemService(CAMERA_SERVICE) as CameraManager
            manager.getCameraCharacteristics(option.logicalId)
                .get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)?.let { lo = it.lower; hi = it.upper }
        }
        return (ratio * zoomRatio).coerceIn(lo, hi)
    }

    /** Zoom range of the chosen camera; a pinned lens starts at its own 1x. */
    private fun configureZoom(manager: CameraManager, option: LensOption) {
        refreshLensChips()
        var lo = 1f
        var hi = 1f
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val r = manager.getCameraCharacteristics(option.logicalId)
                .get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)
            if (r != null) { lo = r.lower; hi = r.upper }
        }
        if (option.physicalId != null) { lo = maxOf(lo, 1f); hi = min(hi, 8f) }
        zoomMin = lo
        zoomMax = maxOf(hi, lo)
        zoomRatio = zoomRatio.coerceIn(zoomMin, zoomMax)
        zoomBar.isEnabled = zoomMax > zoomMin
        android.util.Log.i("nostrobe", "zoom range %.2f..%.2f (logical %.2f..%.2f) lens=%s".format(zoomMin, zoomMax, lo, hi, option.label))
        zoomBar.progress = if (zoomMax > zoomMin) {
            (100f * ln(zoomRatio / zoomMin) / ln(zoomMax / zoomMin)).toInt()
        } else 0
        zoomText.text = "%.1fx".format(zoomRatio)
    }

    override fun onRequestPermissionsResult(
        requestCode: Int, permissions: Array<out String>, grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == 1 && grantResults.firstOrNull() != PackageManager.PERMISSION_GRANTED) {
            Toast.makeText(this, "The camera permission is required.", Toast.LENGTH_LONG).show()
            return
        }
        if (requestCode == 1 && grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED &&
            surfaceView.holder.surface.isValid && camera == null
        ) startCamera()
    }

    /** The ring appears at the tap, shrinks onto it and fades out. */
    private fun showFocusRing(x: Float, y: Float) {
        focusRing.animate().cancel()
        // The ring is GONE between taps, so its measured size can be 0: use the layout size.
        val size = 76f * resources.displayMetrics.density
        focusRing.translationX = x - size / 2f
        focusRing.translationY = y - size / 2f
        focusRing.visibility = android.view.View.VISIBLE
        focusRing.alpha = 1f
        focusRing.scaleX = 1.5f
        focusRing.scaleY = 1.5f
        focusRing.animate().scaleX(1f).scaleY(1f).setDuration(200).withEndAction {
            focusRing.animate().alpha(0f).setStartDelay(900).setDuration(300).withEndAction {
                focusRing.visibility = android.view.View.GONE
            }.start()
        }.start()
    }

    /** First camcorder profile the device has, 1080p preferred; null when it has none. */
    private fun pickProfile(): CamcorderProfile? {
        for (q in intArrayOf(CamcorderProfile.QUALITY_1080P, CamcorderProfile.QUALITY_720P,
            CamcorderProfile.QUALITY_HIGH, CamcorderProfile.QUALITY_480P)) {
            try {
                if (CamcorderProfile.hasProfile(q)) return CamcorderProfile.get(q)
            } catch (_: RuntimeException) {}
        }
        return null
    }

    /**
     * Rotation to store in the clip so it plays upright: the sensor's mounting angle corrected
     * by how the screen is turned (a foldable held sideways records landscape).
     */
    private fun sensorOrientation(): Int {
        val manager = getSystemService(CAMERA_SERVICE) as CameraManager
        val option = lensOptions.getOrNull(lensIndex) ?: return 90
        val chars = manager.getCameraCharacteristics(option.logicalId)
        val sensor = chars.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
        @Suppress("DEPRECATION")
        val screen = when (windowManager.defaultDisplay.rotation) {
            android.view.Surface.ROTATION_90 -> 90
            android.view.Surface.ROTATION_180 -> 180
            android.view.Surface.ROTATION_270 -> 270
            else -> 0
        }
        val front = chars.get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_FRONT
        return if (front) (sensor + screen) % 360 else (sensor - screen + 360) % 360
    }

    private fun selectLens(index: Int) {
        if (index == lensIndex || index !in lensOptions.indices) return
        lensIndex = index
        zoomRatio = 1f
        camera?.stop(); startCamera()
    }

    /** Back <-> front. Going back, return to the lens used last on the back. */
    private var lastBackIndex = -1
    private fun flipCamera() {
        if (lensOptions.isEmpty()) return
        val isFront = lensOptions[lensIndex].label.startsWith("front")
        if (isFront) {
            selectLens(if (lastBackIndex in lensOptions.indices) lastBackIndex else LensOptions.defaultIndex(lensOptions))
        } else {
            lastBackIndex = lensIndex
            val front = lensOptions.indexOfFirst { it.label.startsWith("front") }
            if (front >= 0) selectLens(front)
        }
    }

    /** One chip per lens, labelled by magnification relative to the main lens (0.5x, 1x, 5x). */
    private fun refreshLensChips() {
        lensRow.removeAllViews()
        val main = lensOptions.getOrNull(LensOptions.defaultIndex(lensOptions))?.equivMm ?: 0.0
        for ((i, o) in lensOptions.withIndex()) {
            if (o.label.startsWith("front")) continue
            val text = when {
                o.physicalId == null && o.equivMm == 0.0 -> "auto"
                main > 0 && o.equivMm > 0 -> {
                    val m = o.equivMm / main
                    (if (m >= 10 || Math.abs(m - Math.round(m)) < 0.05) "%.0f" else "%.1f").format(m) + "x"
                }
                else -> o.label
            }
            val b = Button(this).apply {
                this.text = text
                minWidth = 0
                minimumWidth = 0
                isAllCaps = false
                alpha = if (i == lensIndex) 1f else 0.55f
                setOnClickListener { selectLens(i) }
            }
            lensRow.addView(b)
        }
        flipButton.text = if (lensOptions[lensIndex].label.startsWith("front")) "back" else "front"
    }

    private fun stepZoom(factor: Float) {
        if (zoomMax <= zoomMin) return
        zoomRatio = (zoomRatio * factor).coerceIn(zoomMin, zoomMax)
        zoomBar.progress = (100f * ln(zoomRatio / zoomMin) / ln(zoomMax / zoomMin)).toInt()
        zoomText.text = "%.1fx".format(zoomRatio)
        camera?.setZoom(zoomRatio)
    }

    private fun toggleRecord() {
        if (recorder != null) {
            val (total, locked) = camera?.recordingStats() ?: (0 to 0)
            lastRowRate = camera?.rowRateHz ?: 0.0
            camera?.closeMeta()
            try { recorder?.stop() } catch (e: RuntimeException) {
                android.util.Log.w("nostrobe", "recorder stopped without data: ${e.message}")
            }
            recorder?.release(); recorder = null
            recordButton.isSelected = false
            requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
            finishClip()
            notifyPost(total, locked)
            camera?.stop(); startCamera()
            return
        }
        // The lock in force now, with the frame rate it was found at (a dim scene runs at a lower
        // rate): the recording must use that rate, or the exposure would not fit in the frame.
        val carriedLock = camera?.carryLock() ?: Triple(0.0, 0.0, 0.0)
        // 1080p keeps the encoder and the lens stream manageable; QUALITY_HIGH can be 4K.
        val p = pickProfile()
        val r = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) MediaRecorder(this) else MediaRecorder()
        // The call order matters: the source first, then the format, then the encoders.
        // Video only for now: audio would need the RECORD_AUDIO permission.
        val withAudio = micOn && ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        if (micOn && !withAudio) {
            // Ask once; this clip is recorded without sound, the next one will have it.
            ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.RECORD_AUDIO), 2)
            Toast.makeText(this, "Allow the microphone to record sound.", Toast.LENGTH_SHORT).show()
        }
        if (withAudio) r.setAudioSource(MediaRecorder.AudioSource.CAMCORDER)
        r.setVideoSource(MediaRecorder.VideoSource.SURFACE)
        r.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
        // Cap the requested rate (the profile asks 33 Mbit/s at 1080p60). The Pixel's encoder
        // still overshoots on very noisy footage: measured ~143 Mbit/s in a dark scene.
        // A device with no camcorder profile at all gets plain 720p30.
        val rate = p?.videoBitRate ?: 8_000_000
        val fps = intent.getIntExtra("rec_fps", 0).takeIf { it > 0 }
            ?: carriedLock.third.toInt().takeIf { it > 0 }
            ?: (p?.videoFrameRate ?: 30)
        val vw = p?.videoFrameWidth ?: 1280
        val vh = p?.videoFrameHeight ?: 720
        r.setVideoEncodingBitRate(minOf(rate, MAX_VIDEO_BITRATE))
        android.util.Log.i("nostrobe", "recorder: profile %d bps -> asked %d bps, %dx%d @ %d".format(rate, minOf(rate, MAX_VIDEO_BITRATE), vw, vh, fps))
        r.setVideoFrameRate(fps)
        recFps = fps
        r.setVideoSize(vw, vh)
        r.setVideoEncoder(MediaRecorder.VideoEncoder.H264)
        if (withAudio) {
            r.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
            r.setAudioEncodingBitRate(128_000)
            r.setAudioSamplingRate(44_100)
        }
        r.setOrientationHint(sensorOrientation())
        r.setOutputFile(File(externalCacheDir ?: cacheDir, "clip.mp4").path)
        r.prepare()
        recorder = r
        // A rotation would recreate the activity and cut the recording.
        requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_LOCKED
        carryExposureUs = carriedLock.first
        carryHz = carriedLock.second
        recordButton.isSelected = true
        val w = java.io.FileWriter(File(externalCacheDir ?: cacheDir, "clip.csv"))
        w.write("timestamp_ns,exposure_ns,iso,luma,state,freq_hz,stale\n")
        pendingMeta = w
        // The recorder surface is added by the session, so rebuild it; the recorder starts
        // once the session is configured (MediaRecorder needs frames to flow to its surface).
        camera?.stop(); startCamera()
    }

    /** The clip is in the cache; copy it to Movies/no-strobe so the gallery sees it. */
    private fun finishClip() {
        val clip = File(externalCacheDir ?: cacheDir, "clip.mp4")
        if (!clip.exists() || clip.length() == 0L) return
        lastClip = clip
        postButton.visibility = android.view.View.VISIBLE
        val name = "nostrobe-" + java.text.SimpleDateFormat("yyyyMMdd-HHmmss", java.util.Locale.US)
            .format(java.util.Date()) + ".mp4"
        Thread {
            val where = saveToGallery(clip, name)
            runOnUiThread {
                Toast.makeText(this, if (where != null) "Saved: $where" else "Could not save the clip to the gallery",
                    Toast.LENGTH_LONG).show()
            }
        }.start()
    }

    /** Movies/no-strobe through MediaStore (no permission needed from Android 10). */
    private fun saveToGallery(src: File, displayName: String): String? {
        return try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                val values = ContentValues().apply {
                    put(MediaStore.Video.Media.DISPLAY_NAME, displayName)
                    put(MediaStore.Video.Media.MIME_TYPE, "video/mp4")
                    put(MediaStore.Video.Media.RELATIVE_PATH, Environment.DIRECTORY_MOVIES + "/no-strobe")
                    put(MediaStore.Video.Media.IS_PENDING, 1)
                }
                val uri: Uri = contentResolver.insert(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, values) ?: return null
                contentResolver.openOutputStream(uri)?.use { o -> src.inputStream().use { it.copyTo(o) } }
                values.clear()
                values.put(MediaStore.Video.Media.IS_PENDING, 0)
                contentResolver.update(uri, values, null, null)
                "Movies/no-strobe/$displayName"
            } else {
                // Before Android 10 a shared-storage write needs a runtime permission: keep the
                // copy in the app's own movies folder instead.
                val dir = getExternalFilesDir(Environment.DIRECTORY_MOVIES) ?: filesDir
                src.copyTo(File(dir, displayName), overwrite = true).path
            }
        } catch (e: Exception) {
            android.util.Log.w("nostrobe", "gallery save failed: ${e.message}")
            null
        }
    }

    private fun refreshToggles() {
        micButton.text = if (micOn) "mic on" else "mic off"
        gridButton.text = if (gridView.visibility == android.view.View.VISIBLE) "grid on" else "grid off"
        timerButton.text = if (timerSecs == 0) "timer off" else "timer ${timerSecs}s"
    }

    /** Counts down on the preview, then runs [then] (starting the recording). */
    private fun startCountdown(seconds: Int, then: () -> Unit) {
        recordButton.isEnabled = false
        countdownText.visibility = android.view.View.VISIBLE
        val handler = android.os.Handler(android.os.Looper.getMainLooper())
        var left = seconds
        val tick = object : Runnable {
            override fun run() {
                if (left > 0) {
                    countdownText.text = left.toString()
                    left -= 1
                    handler.postDelayed(this, 1000)
                } else {
                    countdownText.visibility = android.view.View.GONE
                    recordButton.isEnabled = true
                    then()
                }
            }
        }
        handler.post(tick)
    }

    private fun prefs() = getSharedPreferences("nostrobe", MODE_PRIVATE)

    private fun showPostMenu() {
        val items = arrayOf("Fix on this phone", "Send to the desktop and fix", "Desktop server settings")
        androidx.appcompat.app.AlertDialog.Builder(this)
            .setTitle("Post-process the last clip")
            .setItems(items) { _, which ->
                when (which) {
                    0 -> fixOnPhone()
                    1 -> sendToDesktop()
                    else -> showServerSettings()
                }
            }
            .show()
    }

    private fun showServerSettings() {
        val pad = (16 * resources.displayMetrics.density).toInt()
        val box = android.widget.LinearLayout(this).apply {
            orientation = android.widget.LinearLayout.VERTICAL
            setPadding(pad, pad, pad, 0)
        }
        val url = android.widget.EditText(this).apply {
            hint = "http://192.168.1.20:8765"
            setText(prefs().getString("post_url", ""))
            setSingleLine()
        }
        val token = android.widget.EditText(this).apply {
            hint = "token (NO_STROBE_TOKEN on the computer)"
            setText(prefs().getString("post_token", ""))
            setSingleLine()
        }
        box.addView(url); box.addView(token)
        androidx.appcompat.app.AlertDialog.Builder(this)
            .setTitle("Desktop server")
            .setMessage("Run: no-strobe-post serve --listen 0.0.0.0:8765 --token ...")
            .setView(box)
            .setPositiveButton("Save") { _, _ ->
                prefs().edit().putString("post_url", url.text.toString().trim())
                    .putString("post_token", token.text.toString().trim()).apply()
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    /** Brightness-pumping fix on the phone itself (no network); slower than the desktop. */
    private fun fixOnPhone() {
        val clip = lastClip
        if (clip == null || !clip.exists()) {
            Toast.makeText(this, "No clip to process yet.", Toast.LENGTH_SHORT).show(); return
        }
        val out = File(clip.parentFile, "clip-fixed.mp4")
        postButton.isEnabled = false
        Thread {
            try {
                val t0 = System.currentTimeMillis()
                android.util.Log.i("nostrobe", "phone post: start (${clip.length()} bytes)")
                OnDevicePost.fix(clip, out) { pct ->
                    runOnUiThread { status.text = "post-processing on the phone: $pct %" }
                }
                val name = "nostrobe-" + java.text.SimpleDateFormat("yyyyMMdd-HHmmss", java.util.Locale.US)
                    .format(java.util.Date()) + "-fixed.mp4"
                android.util.Log.i("nostrobe", "phone post: done in ${System.currentTimeMillis() - t0} ms, ${out.length()} bytes")
                val where = saveToGallery(out, name)
                runOnUiThread {
                    status.text = "post-processing done in ${(System.currentTimeMillis() - t0) / 1000} s"
                    Toast.makeText(this, if (where != null) "Fixed clip saved: $where" else "Fixed clip ready",
                        Toast.LENGTH_LONG).show()
                }
            } catch (e: Exception) {
                android.util.Log.w("nostrobe", "phone post failed: ${e.javaClass.simpleName}: ${e.message}")
                runOnUiThread {
                    status.text = "post-processing failed"
                    Toast.makeText(this, "Post-processing failed: ${e.message}", Toast.LENGTH_LONG).show()
                }
            } finally {
                runOnUiThread { postButton.isEnabled = true }
            }
        }.start()
    }

    private fun sendToDesktop() {
        val clip = lastClip
        if (clip == null || !clip.exists()) {
            Toast.makeText(this, "No clip to process yet.", Toast.LENGTH_SHORT).show(); return
        }
        val base = prefs().getString("post_url", "") ?: ""
        if (base.isBlank()) { showServerSettings(); return }
        val token = prefs().getString("post_token", "") ?: ""
        val out = File(clip.parentFile, "clip-fixed.mp4")
        postButton.isEnabled = false
        val rate = lastRowRate
        Thread {
            try {
                DesktopPost.send(clip, out, base, token, rate) { pct ->
                    runOnUiThread { status.text = "post-processing on the desktop: $pct %" }
                }
                val name = "nostrobe-" + java.text.SimpleDateFormat("yyyyMMdd-HHmmss", java.util.Locale.US)
                    .format(java.util.Date()) + "-fixed.mp4"
                val where = saveToGallery(out, name)
                runOnUiThread {
                    status.text = "post-processing done"
                    Toast.makeText(this, if (where != null) "Fixed clip saved: $where" else "Fixed clip ready",
                        Toast.LENGTH_LONG).show()
                }
            } catch (e: Exception) {
                android.util.Log.w("nostrobe", "desktop post failed: ${e.message}")
                runOnUiThread {
                    status.text = "post-processing failed"
                    Toast.makeText(this, "Desktop post-processing failed: ${e.message}", Toast.LENGTH_LONG).show()
                }
            } finally {
                runOnUiThread { postButton.isEnabled = true }
            }
        }.start()
    }

    /** Live correction is not available (MediaRecorder), so say when post-processing is needed. */
    private fun notifyPost(total: Int, locked: Int) {
        val pct = if (total > 0) 100 * locked / total else 0
        val msg = when {
            total == 0 -> "Recorded without analysis: post-processing needed (tools/postprocess.sh)."
            pct >= 95 -> "Flicker locked on $pct% of the frames: no post-processing needed."
            else -> "Flicker locked on only $pct% of the frames. Live correction is not available: " +
                "post-processing is needed (tools/postprocess.sh)."
        }
        Toast.makeText(this, msg, Toast.LENGTH_LONG).show()
    }

    private fun describe(cmd: NsCommand): String {
        val mode = when (cmd.mode) {
            NsJni.MODE_INTEGRATE -> "integrate"
            NsJni.MODE_GAP -> "gap"
            NsJni.MODE_FLASH -> "flash"
            NsJni.MODE_PWM -> "pwm"
            NsJni.MODE_AUTO -> "auto (no flicker found)"
            NsJni.MODE_UNSUPPORTED -> "auto (no manual exposure on this camera: post-processing only)"
            else -> "?"
        }
        val state = if (cmd.state == NsJni.STATE_LOCKED) "locked" else "search"
        val hz = if (cmd.periodUs > 0) 1e6 / cmd.periodUs else 0.0
        return "%s  %s  %.0f us  fps %.0f  iso %.0f  %.1f Hz  conf %.2f".format(
            state, mode, cmd.exposureUs, cmd.fps, cmd.iso, hz, cmd.confidence
        )
    }
}
