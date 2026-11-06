package dev.nostrobe.strobe

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import kotlin.math.ln
import kotlin.math.exp

class MainActivity : ComponentActivity() {
    private var status by mutableStateOf(StrobeCamera.Status())
    private var granted by mutableStateOf(false)
    private lateinit var camera: StrobeCamera

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        camera = StrobeCamera(this) { status = it }
        val needed = arrayOf(Manifest.permission.CAMERA, Manifest.permission.RECORD_AUDIO)
        granted = needed.all { checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }
        if (!granted) registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { r ->
            granted = r[Manifest.permission.CAMERA] == true && r[Manifest.permission.RECORD_AUDIO] == true
        }.launch(needed)
        setContent { MaterialTheme(colorScheme = darkColorScheme()) { Screen() } }
    }

    @Composable
    private fun Screen() {
        val exposureMs = status.exposureMs; val iso = status.iso

        Box(Modifier.fillMaxSize().background(Color.Black)) {
            if (granted) AndroidView(
                modifier = Modifier.fillMaxWidth().aspectRatio(9f / 16f).align(Alignment.TopCenter),
                factory = { ctx ->
                    SurfaceView(ctx).apply {
                        holder.setFixedSize(camera.width, camera.height)
                        holder.addCallback(object : SurfaceHolder.Callback {
                            override fun surfaceCreated(h: SurfaceHolder) { camera.open(h.surface) }
                            override fun surfaceChanged(h: SurfaceHolder, format: Int, w: Int, ht: Int) {}
                            override fun surfaceDestroyed(h: SurfaceHolder) { camera.close() }
                        })
                    }
                },
            ) else Text("Autorise la caméra et le micro pour filmer.", color = Color.White, modifier = Modifier.align(Alignment.Center).padding(24.dp))

            Column(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(statusText(), color = if (status.frequency == null) Color.White else Color(0xFFFFD54F),
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.clip(RoundedCornerShape(10.dp)).background(Color(0x99000000)).padding(10.dp))
                Spacer(Modifier.weight(1f))
                Column(Modifier.clip(RoundedCornerShape(14.dp)).background(Color(0x99000000)).padding(12.dp)) {
                    Text("Pose %.1f ms".format(exposureMs), color = Color.White)
                    // échelle logarithmique : 0,1 à 20 ms
                    Slider(value = ((ln(exposureMs) - ln(0.1)) / (ln(20.0) - ln(0.1))).toFloat(),
                        onValueChange = { val ms = exp(ln(0.1) + it * (ln(20.0) - ln(0.1))); status = status.copy(exposureMs = ms); camera.setExposure(ms, iso) })
                    Text("ISO $iso", color = Color.White)
                    Slider(value = iso.toFloat(), valueRange = status.isoRange.first.toFloat()..status.isoRange.last.toFloat(),
                        onValueChange = { status = status.copy(iso = it.toInt()); camera.setExposure(exposureMs, it.toInt()) })
                }
                Box(Modifier.align(Alignment.CenterHorizontally).size(76.dp).border(4.dp, Color.White, CircleShape)
                    .semantics { contentDescription = if (status.recording) "Arrêter l'enregistrement" else "Enregistrer" }
                    .clickable { camera.toggleRecording() }, contentAlignment = Alignment.Center) {
                    Box(Modifier.size(if (status.recording) 36.dp else 62.dp).clip(if (status.recording) RoundedCornerShape(6.dp) else CircleShape).background(Color.Red))
                }
                Spacer(Modifier.height(8.dp))
            }
        }
    }

    private fun statusText(): String {
        if (status.metering) return "Mesure de la lumière…"
        if (status.timingIgnored) return "Ce téléphone ne laisse pas régler la cadence image par image : impossible de se caler sur le strobe."
        val fps = "%.0f i/s".format(status.fps)
        val f = status.frequency ?: return "Aucun strobe détecté · $fps"
        val state = if (status.locked) "calé" else "recherche du noir…"
        return "Strobe %.2f Hz · %s · %d %% propres · %s".format(f, state, (status.cleanRatio * 100).toInt(), fps)
    }
}
