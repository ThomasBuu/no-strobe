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
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import java.io.File

/**
 * Preview, status readout, and record button. The shutter is driven by the Rust controller.
 */
class MainActivity : AppCompatActivity() {
    private var camera: NoStrobeCamera? = null
    private var recorder: MediaRecorder? = null
    private lateinit var surfaceView: SurfaceView
    private lateinit var status: TextView
    private lateinit var recordButton: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.main)
        surfaceView = findViewById(R.id.preview)
        status = findViewById(R.id.status)
        recordButton = findViewById(R.id.record)
        recordButton.setOnClickListener { toggleRecord() }

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
        recordButton.text = "record"
        super.onPause()
    }

    private fun startCamera() {
        val manager = getSystemService(CAMERA_SERVICE) as CameraManager
        // Manual exposure is required; fall back to the first camera if none reports it.
        val id = manager.cameraIdList.firstOrNull {
            Camera2Sync.supportsManualSensor(manager.getCameraCharacteristics(it))
        } ?: manager.cameraIdList.first()
        val holder = surfaceView.holder
        // Do not call holder.setFixedSize: resizing recreates the surface buffers and the
        // Surface object read after it is the pre-resize one, which createCaptureSession
        // rejects with "Surface was abandoned".
        camera = NoStrobeCamera(
            manager, id, holder.surface, recorder, pwmMode = true
        ) { cmd ->
            var s = describe(cmd)
            // Live correction is not available: tell the user when the clip will need it.
            if (recorder != null && cmd.state != NsJni.STATE_LOCKED) s += "  [post needed]"
            android.util.Log.d("nostrobe", s)
            runOnUiThread { status.text = s }
        }
        camera?.start()
    }

    private fun toggleRecord() {
        if (recorder != null) {
            val (total, locked) = camera?.recordingStats() ?: (0 to 0)
            camera?.closeMeta()
            recorder?.stop(); recorder?.release(); recorder = null
            recordButton.text = "record"
            notifyPost(total, locked)
            camera?.stop(); startCamera()
            return
        }
        val p = CamcorderProfile.get(CamcorderProfile.QUALITY_HIGH)
        val r = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) MediaRecorder(this) else MediaRecorder()
        r.setVideoEncodingBitRate(p.videoBitRate)
        r.setVideoFrameRate(p.videoFrameRate)
        r.setVideoSize(p.videoFrameWidth, p.videoFrameHeight)
        r.setAudioEncodingBitRate(p.audioBitRate)
        r.setAudioChannels(p.audioChannels)
        r.setAudioSamplingRate(p.audioSampleRate)
        r.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
        r.setVideoEncoder(MediaRecorder.VideoEncoder.H264)
        r.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
        r.setOutputFile(File(externalCacheDir, "clip.mp4").path)
        r.prepare()
        r.start()
        recorder = r
        recordButton.text = "stop"
        // The recorder surface is added by the session, so rebuild it.
        camera?.stop(); startCamera()
        val w = java.io.FileWriter(File(externalCacheDir, "clip.csv"))
        w.write("timestamp_ns,exposure_ns,iso,luma,state,freq_hz,stale\n")
        camera?.meta = w
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
            else -> "?"
        }
        val state = if (cmd.state == NsJni.STATE_LOCKED) "locked" else "search"
        val hz = if (cmd.periodUs > 0) 1e6 / cmd.periodUs else 0.0
        return "%s  %s  %.0f us  fps %.0f  iso %.0f  %.1f Hz  conf %.2f".format(
            state, mode, cmd.exposureUs, cmd.fps, cmd.iso, hz, cmd.confidence
        )
    }
}
