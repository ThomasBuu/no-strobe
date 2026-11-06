package dev.nostrobe.strobeflasher

import android.hardware.camera2.CameraManager
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlin.math.roundToInt

class MainActivity : ComponentActivity() {
    private lateinit var strobe: Strobe

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        strobe = Strobe(getSystemService(CameraManager::class.java))
        setContent { MaterialTheme(colorScheme = darkColorScheme()) { Screen(strobe) { keep ->
            if (keep) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } } }
    }

    override fun onPause() { strobe.stop(); super.onPause() }
    override fun onStop() { strobe.stop(); super.onStop() }
}

@Composable
private fun Screen(strobe: Strobe, keepOn: (Boolean) -> Unit) {
    var freq by remember { mutableFloatStateOf(10f) }
    var duty by remember { mutableFloatStateOf(0.5f) }
    var white by remember { mutableStateOf(false) }
    var sweep by remember { mutableStateOf(false) }
    var measured by remember { mutableDoubleStateOf(0.0) }
    val running = strobe.running

    LaunchedEffect(running) { keepOn(running) }
    LaunchedEffect(running) { while (running) { measured = strobe.measuredHz; if (sweep) freq = strobe.freqHz.toFloat(); delay(250) } ; measured = 0.0 }

    val flash = white && running && strobe.screenOn
    Box(Modifier.fillMaxSize().background(if (flash) Color.White else MaterialTheme.colorScheme.background)) {
        Column(Modifier.fillMaxSize().systemBarsPadding().padding(16.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp)) {
            // les contrôles restent visibles en noir sur l'écran blanc : on les met sur un fond opaque
            Surface(shape = MaterialTheme.shapes.medium, modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Fréquence demandée : ${"%.1f".format(freq)} Hz", style = MaterialTheme.typography.titleMedium)
                    Text("Mesurée : " + if (running && measured > 0) "%.2f Hz".format(measured) else "—")
                    Slider(freq, { freq = it; strobe.freqHz = it.toDouble() }, valueRange = 1f..30f, steps = 57)
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        for (p in listOf(5, 8, 10, 12, 15, 20)) OutlinedButton(
                            { freq = p.toFloat(); strobe.freqHz = p.toDouble() },
                            Modifier.weight(1f), contentPadding = PaddingValues(0.dp)) { Text("$p") }
                    }
                    Text("Rapport cyclique : ${(duty * 100).roundToInt()} %")
                    Slider(duty, { duty = (it * 100).roundToInt() / 100f; strobe.duty = duty.toDouble() }, valueRange = 0.1f..0.9f)
                    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Switch(sweep, { sweep = it; strobe.sweep = it }); Spacer(Modifier.width(8.dp)); Text("Balayage : 5→20 Hz, change toutes les 2 s")
                    }
                    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Switch(white, { white = it }); Spacer(Modifier.width(8.dp)); Text("Écran blanc (secours)")
                    }
                    if (!strobe.hasTorch) Text("Pas de flash sur cet appareil.", color = MaterialTheme.colorScheme.error)
                    Button({ if (running) strobe.stop() else { strobe.freqHz = freq.toDouble(); strobe.duty = duty.toDouble(); strobe.start() } },
                        Modifier.fillMaxWidth(), enabled = strobe.hasTorch) {
                        Text(if (running) "Arrêter" else "Démarrer")
                    }
                }
            }
        }
    }
}
