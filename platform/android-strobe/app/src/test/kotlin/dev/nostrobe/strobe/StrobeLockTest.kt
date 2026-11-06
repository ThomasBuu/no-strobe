package dev.nostrobe.strobe

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.ceil
import kotlin.random.Random

class StrobeLockTest {
    /**
     * steadyClean / steadyFrames : sur les 4 dernières secondes (régime établi).
     * flashKept : images avec flash gardées alors que l'appli disait connaître le strobe (sur toute la durée).
     * lockedAt : premier calage (s).
     */
    class Run(val freq: Double?, val locked: Boolean, val steadyFrames: Int, val steadyClean: Int, val flashKept: Int, val lockedAt: Double?)

    /** Caméra simulée : rolling shutter, pose courte, cadence max 30 i/s, réglages appliqués `latency` images plus tard. */
    private fun simulate(hz: Double?, duty: Double = 0.5, seconds: Double = 12.0, latency: Int = 4, skew: Double = 0.025,
                         trace: Boolean = false, seed: Int = 42): Run {
        val rnd = Random(seed)
        val phase0 = Random(seed + 1000).nextDouble()
        val lock = StrobeLock(bands = 16)
        val exposure = 0.001
        val pending = ArrayDeque(List(latency) { 1.0 / 30 })
        fun lit(t: Double) = hz != null && ((t * hz + phase0) % 1.0) < duty
        var t = 0.0; var frames = 0; var clean = 0; var flashKept = 0; var lockedAt: Double? = null
        while (t < seconds) {
            var frameLit = false
            val lum = DoubleArray(16) { b ->
                var on = 0.0
                for (r in 0 until 4) {            // 4 lignes par bande, 5 instants par pose
                    val row = t + skew * (b * 4 + r) / 64.0
                    for (s in 0 until 5) if (lit(row + exposure * s / 4)) on += 1.0
                }
                on /= 20
                if (on > 0.1) frameLit = true // flash visible : plus de 10 % de la bande éclairée
                0.15 + 0.01 * b + 0.5 * on + rnd.nextDouble(-0.01, 0.01) // relief de la scène + strobe + bruit
            }
            val known = lock.frequency != null
            val res = lock.onFrame(t, skew, exposure, lum)
            pending.addLast(res.nextFrameDuration)
            if (trace) println("TRACE %.3f f=%s lock=%b clean=%b lit=%b next=%.4f phase=%.2f".format(java.util.Locale.ROOT, t, lock.frequency?.let { "%.3f".format(java.util.Locale.ROOT, it) }, lock.locked, res.clean, frameLit, res.nextFrameDuration, hz?.let { ((t + skew / 2) * it + phase0) % 1.0 } ?: 0.0))
            if (lockedAt == null && lock.locked) lockedAt = t
            if (known && res.clean && frameLit) flashKept++
            if (t > seconds - 4) { frames++; if (res.clean) clean++ }
            t += maxOf(1.0 / 30, pending.removeFirst())
        }
        return Run(lock.frequency, lock.locked, frames, clean, flashKept, lockedAt)
    }

    private fun checkLocks(hz: Double, latency: Int = 4, duty: Double = 0.5, skew: Double = 0.025, seed: Int = 42) {
        val r = simulate(hz, duty = duty, latency = latency, skew = skew, seed = seed)
        val name = "$hz Hz (latence $latency, graine $seed)"
        assertEquals("$name : fréquence", hz, r.freq ?: 0.0, 0.05)
        assertTrue("$name : pas calé", r.locked)
        assertTrue("$name : calé seulement à ${r.lockedAt} s", (r.lockedAt ?: 99.0) < 6)
        assertEquals("$name : images avec flash gardées", 0, r.flashKept)
        val cycles = 4 * hz / ceil(hz / 30 - 1e-9) // une image par cycle (ou tous les k cycles) sur 4 s
        // sondes (~7 %) et recentrages coûtent des cycles : au moins 3 cycles sur 4 doivent donner une image propre
        assertTrue("$name : ${r.steadyClean} images propres sur ${r.steadyFrames}, attendu ~${cycles.toInt()}", r.steadyClean >= 0.75 * cycles)
    }

    @Test fun locksOnPoliceStrobe() { for (seed in 1..10) checkLocks(14.3, seed = seed) }
    @Test fun tellsAliasApart() { for (seed in 1..10) checkLocks(15.7, seed = seed) } // à 30 i/s, 15,7 Hz ressemble à 14,3 Hz
    @Test fun locksOnOtherFrequencies() { for (hz in listOf(6.0, 9.0, 12.0)) checkLocks(hz) }
    // au-delà de ~19 Hz, une lecture de 25 ms ne tient plus dans la plage éteinte : il faut un capteur plus rapide
    @Test fun fastSensor() { for (hz in listOf(20.0, 27.0)) checkLocks(hz, skew = 0.010) }
    @Test fun locksAboveFrameRate() = checkLocks(40.0, skew = 0.006) // une image tous les 2 cycles
    // à 1-2 ms de la limite physique (image presque aussi longue que la plage éteinte) : on exige seulement zéro flash
    @Test fun nearPhysicalLimitNeverLeaks() {
        for ((hz, skew) in listOf(18.0 to 0.025, 40.0 to 0.010)) {
            val r = simulate(hz, skew = skew)
            assertEquals("$hz Hz, lecture ${skew * 1000} ms : images avec flash gardées", 0, r.flashKept)
        }
    }
    @Test fun toleratesLongerPipeline() { for (lat in listOf(2, 6)) checkLocks(14.3, latency = lat) }
    @Test fun shortFlashes() = checkLocks(12.0, duty = 0.2)

    @Test fun noStrobeKeepsEverything() {
        val r = simulate(null)
        assertEquals(null, r.freq)
        assertEquals(r.steadyFrames, r.steadyClean)
    }

    @Test fun summary() { // pas d'assertion : tableau lisible dans le rapport de test
        for ((hz, lat, duty) in listOf(Triple(6.0, 4, 0.5), Triple(9.0, 4, 0.5), Triple(12.0, 4, 0.5), Triple(14.3, 4, 0.5), Triple(15.7, 4, 0.5),
                Triple(18.0, 4, 0.5), Triple(20.0, 4, 0.5), Triple(27.0, 4, 0.5), Triple(40.0, 4, 0.5), Triple(14.3, 6, 0.5), Triple(12.0, 4, 0.2))) {
            val r = simulate(hz, duty = duty, latency = lat, skew = if (hz >= 20) 0.010 else 0.025)
            println("RESUME %5.1f Hz lat %d cycle %.1f : f=%s calé à %s s, propres %d/%d, flash %d".format(java.util.Locale.ROOT, hz, lat, duty,
                r.freq?.let { "%.3f".format(java.util.Locale.ROOT, it) }, r.lockedAt?.let { "%.1f".format(java.util.Locale.ROOT, it) }, r.steadyClean, r.steadyFrames, r.flashKept))
        }
    }
}
