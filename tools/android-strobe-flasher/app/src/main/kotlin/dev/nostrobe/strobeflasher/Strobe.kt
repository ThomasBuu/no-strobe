package dev.nostrobe.strobeflasher

import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.os.Process
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.util.concurrent.locks.LockSupport

/** Clignotement de la torche dans un thread dédié, échéances absolues (System.nanoTime) : pas de dérive cumulée. */
class Strobe(private val cm: CameraManager) {
    @Volatile var freqHz = 10.0
    @Volatile var duty = 0.5
    @Volatile var sweep = false              // change de fréquence toutes les 2 s en parcourant les préréglages
    var screenOn by mutableStateOf(false)   // état « allumé » courant, pour l'écran blanc
        private set
    var running by mutableStateOf(false)
        private set
    @Volatile var measuredHz = 0.0           // calculée sur les dernières bascules

    private val cameraId = cm.cameraIdList.firstOrNull {
        cm.getCameraCharacteristics(it).get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true
    }
    val hasTorch get() = cameraId != null
    @Volatile private var thread: Thread? = null

    fun start() {
        if (cameraId == null || thread != null) return
        running = true
        val t = Thread({ loop(cameraId) }, "strobe").apply { start() }
        thread = t
    }

    fun stop() {
        val t = thread ?: return
        thread = null
        t.interrupt()
        t.join(500)
        running = false
    }

    private fun torch(id: String, on: Boolean) {
        try { cm.setTorchMode(id, on) } catch (_: Exception) {}
    }

    private fun waitUntil(deadline: Long) {
        while (true) {
            val left = deadline - System.nanoTime()
            if (left <= 0 || Thread.currentThread().isInterrupted) return
            // ponytail: park puis spin sur la dernière ms ; coûte du CPU mais donne ~µs de précision
            if (left > 1_500_000) LockSupport.parkNanos(left - 1_000_000)
        }
    }

    private fun loop(id: String) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        val me = Thread.currentThread()
        val ring = LongArray(20); var n = 0   // instants des allumages
        var nextOn = System.nanoTime()
        var lastStep = nextOn; var idx = PRESETS.indexOfFirst { it >= freqHz }.coerceAtLeast(0)
        try {
            while (!me.isInterrupted) {
                if (sweep && System.nanoTime() - lastStep >= 2_000_000_000L) {
                    lastStep = System.nanoTime(); idx = (idx + 1) % PRESETS.size; freqHz = PRESETS[idx]; n = 0 // n=0 : mesure repart à zéro
                }
                val period = (1e9 / freqHz).toLong()
                if (System.nanoTime() - nextOn > period) nextOn = System.nanoTime() // trop en retard (latence torche) : on recale
                waitUntil(nextOn)
                if (me.isInterrupted) break
                torch(id, true); screenOn = true
                ring[n++ % ring.size] = System.nanoTime()
                if (n >= 2) {
                    val c = minOf(n, ring.size)
                    val last = ring[(n - 1) % ring.size]; val first = ring[(n - c) % ring.size]
                    measuredHz = (c - 1) * 1e9 / (last - first)
                }
                waitUntil(nextOn + (period * duty).toLong())
                torch(id, false); screenOn = false
                nextOn += period
            }
        } finally {
            torch(id, false); screenOn = false; measuredHz = 0.0
        }
    }

    companion object { val PRESETS = doubleArrayOf(5.0, 8.0, 10.0, 12.0, 15.0, 20.0) }
}
