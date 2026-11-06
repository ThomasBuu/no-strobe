package dev.nostrobe.strobe

import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.cos
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * Mesure un strobe et cale la caméra dessus pour que chaque image soit prise lampe éteinte.
 *
 * Mesure : chaque bande horizontale de l'image est lue à un instant différent (rolling shutter), donc une image
 * à 30 i/s donne plusieurs échantillons répartis dans le temps. Ça lève l'ambiguïté d'une caméra à 30 i/s
 * (14,3 Hz et 15,7 Hz s'y ressemblent, mais la luminosité ne progresse pas dans le même sens d'une bande à l'autre).
 *
 * Accrochage : une image par cycle (tous les k au-delà de 30 Hz), qui glisse lentement dans le cycle jusqu'à tomber
 * sur des images entièrement sombres. Pas besoin d'une fréquence parfaite : le suivi rattrape la dérive.
 *
 * Calé : les images elles-mêmes donnent l'erreur. Si la lampe mord sur les premières bandes, l'image est trop tôt ;
 * sur les dernières, trop tard (comme le « early-late gate » d'un récepteur GPS). Une image « sonde » est prise
 * régulièrement en pleine phase allumée pour vérifier que le strobe est toujours là ; elle n'est pas gardée.
 */
class StrobeLock(private val bands: Int = 16) {
    var minFrameDuration = 1.0 / 30
    var searchFrameDuration = 1 / 29.3 // cadence pendant la mesure : pas un sous-multiple des fréquences rondes (20, 40 Hz…)
    var probeEvery = 30       // une image sonde toutes les N images une fois calé (coûte ~2 cycles)
    var sweep = 0.04          // accrochage : glissement (cycles par image) pour balayer le cycle à la recherche du noir
    var expectedLatency = 4   // images déjà demandées à la caméra quand on décide (ordre de grandeur courant)
    var drift = 0.0007        // calé : glissement volontaire (cycles par image) pour retrouver régulièrement un bord
    var maxGlow = 0.07        // calé : image gardée si aucune bande ne reçoit plus de 7 % du flash (invisible)
    var minSwing = 0.03       // en dessous, pas de strobe exploitable
    var minSNR = 6.0

    var frequency: Double? = null; private set
    var locked = false; private set

    private class Sample(val t: Double, val band: Int, val lum: Double, var v: Double = 0.0)
    private val samples = ArrayDeque<Sample>()      // 2 s de luminance, une par bande ; v = centrée sur la moyenne de sa bande
    private val bandMean = DoubleArray(bands)       // moyenne de chaque bande sur la fenêtre de mesure (structure de la scène)
    private var lastEstimate = -1e9; private var acquiredAt = 0.0
    private var darkLevel = 0.0                     // échantillon centré lampe éteinte
    private var goodFrames = 0
    private var frames = 0

    // calé
    private var dark = DoubleArray(bands)           // luminosité lampe éteinte, par bande
    private var amplitude = 0.0                     // ce qu'ajoute le flash
    private var window = 0.5                        // largeur de la plage éteinte, en cycles
    private var litInARow = 0; private var sinceProbeLit = 0
    private var refractory = 0; private var framesSinceBounce = 0; private var lastBounceSign = 0.0
    private var halfShifts = 0; private var lastStart = 0.0
    private var probeStep = 0                       // 2 puis 1 : deux demi-périodes ajoutées = une image sonde

    class Result(val clean: Boolean, val nextFrameDuration: Double)

    /**
     * start : début d'exposition de la première ligne (s) ; skew : lecture de la première à la dernière ligne (s) ;
     * exposure : temps de pose (s) ; lum : luminance moyenne (0…1) de chaque bande, dans l'ordre de lecture.
     */
    fun onFrame(start: Double, skew: Double, exposure: Double, lum: DoubleArray): Result {
        frames++
        val times = DoubleArray(bands) { start + skew * (it + 0.5) / bands + exposure / 2 }
        for (b in 0 until bands) samples.addLast(Sample(times[b], b, lum[b]))
        while (samples.first().t < start - 2) samples.removeFirst()

        // fréquence mesurée une seule fois ; ensuite c'est la boucle qui l'affine
        if (frequency == null && start - lastEstimate > 0.5) {
            lastEstimate = start
            // moyenne propre à chaque bande sur la fenêtre : retire la structure de la scène (un mur clair, un ciel sombre…) ;
            // figée ensuite, car les images synchronisées sur le strobe sont toutes sombres et la fausseraient
            val sum = DoubleArray(bands); val n = IntArray(bands)
            for (s in samples) { sum[s.band] += s.lum; n[s.band]++ }
            for (b in 0 until bands) bandMean[b] = sum[b] / maxOf(1, n[b])
            for (s in samples) s.v = s.lum - bandMean[s.band]
            dominantFrequency()?.let {
                frequency = it; acquiredAt = start; goodFrames = 0
                // niveau sombre, force du flash et part du cycle allumée, mesurés tant que la caméra tourne librement
                val sorted = samples.map { s -> s.v }.sorted()
                darkLevel = sorted[sorted.size / 20]; amplitude = sorted[sorted.size * 19 / 20] - darkLevel
                window = 1 - samples.count { s -> s.v > darkLevel + amplitude / 2 }.toDouble() / samples.size
            }
        }
        val f = frequency ?: return Result(true, max(minFrameDuration, searchFrameDuration))
        val k = max(1.0, ceil(minFrameDuration * f - 1e-9)) // cycles par image
        val period = k / f
        val span = (skew + exposure) * f                    // durée de lecture d'une image, en cycles
        val interval = start - lastStart; lastStart = start
        return if (locked) tracking(lum, f, period, span, interval) else acquiring(start, lum, f, period, span)
    }

    private fun acquiring(start: Double, lum: DoubleArray, f: Double, period: Double, span: Double): Result {
        if (amplitude < minSwing) { reset(); return Result(true, minFrameDuration) } // trop faible : fausse détection
        // pas du balayage : au moins 3 images doivent tomber dans la zone où une image entière tient dans le noir
        val step = minOf(sweep, ((window - span) / 3).coerceAtLeast(0.005))
        // image vraiment sombre dans toutes ses bandes : on est dans la plage éteinte
        val dark = (0 until bands).all { lum[it] - bandMean[it] < darkLevel + 0.15 * amplitude }
        if (dark) goodFrames++ else goodFrames = 0
        if (goodFrames >= 2) {
            locked = true; this.dark = lum.copyOf(); litInARow = 0; sinceProbeLit = 0; halfShifts = 0
            // on arrête de glisser, en retirant le glissement des images déjà en route dans le pipeline caméra
            return Result(true, max(minFrameDuration, period - expectedLatency * step / f))
        }
        if (start - acquiredAt > 1.5 / step / f + 2) { reset(); return Result(true, minFrameDuration) } // un tour et demi sans rien trouver
        return Result(dark, max(minFrameDuration, period + step / f))
    }

    private fun tracking(lum: DoubleArray, f: Double, period: Double, span: Double, interval: Double): Result {
        // part du flash reçue par chaque bande (0 = éteinte, 1 = pleine lumière)
        val lit = DoubleArray(bands) { ((lum[it] - dark[it]) / amplitude).coerceAtLeast(0.0) }
        val meanLit = lit.average()

        // sonde : reconnue à l'intervalle allongé d'un demi-cycle qui la précède (le suivant la ramène en phase)
        if (abs(interval - period - 0.5 / f) < 0.2 / f) halfShifts++
        if (halfShifts % 2 == 1) {
            if (meanLit > 0.3) { sinceProbeLit = 0; amplitude += 0.2 * (meanLit * amplitude - amplitude) }
            return Result(false, period)
        }
        // pas une seule sonde allumée depuis longtemps : le strobe s'est arrêté
        if (++sinceProbeLit > 3 * probeEvery) { reset(); return Result(true, minFrameDuration) }
        if (meanLit > 0.5) { // image ratée : lampe en plein dedans
            if (++litInARow > 3) reset() // plusieurs de suite : calage perdu
            return Result(false, period)
        }
        litInARow = 0

        // erreur : lumière sur les premières bandes = trop tôt (retarder), sur les dernières = trop tard (avancer)
        val glow = DoubleArray(bands) { (lit[it] - 0.05).coerceAtLeast(0.0) } // zone morte : bruit
        var edge = 0.0
        for (b in 0 until bands) edge += if (b < bands / 2) glow[b] else -glow[b]
        edge *= span / bands
        val clean = lit.max() < maxGlow
        // lampe éteinte : suit lentement la scène (monte doucement, descend tout de suite)
        if (clean) for (b in 0 until bands) dark[b] = if (lum[b] < dark[b]) lum[b] else dark[b] + 0.1 * (lum[b] - dark[b]) * (1 - glow[b] * 4).coerceAtLeast(0.0)

        // décalage de phase à appliquer (en cycles) : par défaut on glisse lentement vers un bord pour le retrouver
        framesSinceBounce++
        var shift = drift
        if (refractory > 0) { refractory--; shift = 0.0 } // les images déjà en route voient encore le bord : on attend
        else if (edge != 0.0) {
            // un bord touche la lampe : on sait où il est, on ramène l'image d'un coup au milieu de la plage éteinte
            val margin = ((window - span) / 2).coerceAtLeast(0.0) * 0.8
            shift = edge + Math.copySign(margin, edge)
            // recentrage qui envoie droit sur le bord opposé : la plage éteinte est plus étroite qu'estimé
            if (Math.signum(edge) == -lastBounceSign && framesSinceBounce <= expectedLatency + 4) window = maxOf(span, window - 0.3 * margin)
            // deux recentrages du même côté : la dérive rattrapée, moins le glissement volontaire, vient de la fréquence
            if (Math.signum(edge) == lastBounceSign) {
                val naturalDrift = -shift / framesSinceBounce - drift // cycles par image, positif = les images arrivent en retard
                frequency = f * (1 + 0.5 * naturalDrift / (period * f))
            }
            lastBounceSign = Math.signum(edge); framesSinceBounce = 0; refractory = expectedLatency + 3
        }

        if (probeStep == 0 && frames % probeEvery == 0) probeStep = 2
        val probe = if (probeStep > 0) { probeStep--; 0.5 / f } else 0.0
        return Result(clean, max(minFrameDuration, period + shift / f + probe))
    }

    private fun reset() { locked = false; frequency = null; goodFrames = 0; probeStep = 0; samples.clear() }

    private fun wrap(x: Double): Double { val y = x - floor(x); return if (y >= 0.5) y - 1 else y }

    // ponytail: DFT directe (~1200 fréquences × ~1000 échantillons, 2 fois par seconde avant calage) ; FFT si ça chauffe.
    private fun dominantFrequency(): Double? {
        if (samples.size < 64) return null
        val t0 = samples.first().t; val t1 = samples.last().t
        if (t1 - t0 < 1.8) return null // 2 s de signal : la précision sur la fréquence conditionne l'accrochage
        val w = DoubleArray(samples.size) { 0.5 - 0.5 * cos(2 * PI * (samples[it].t - t0) / (t1 - t0)) } // Hann
        fun magnitude(f: Double): Double {
            var re = 0.0; var im = 0.0
            for (i in samples.indices) { val s = samples[i]; val a = 2 * PI * f * (s.t - t0); re += w[i] * s.v * cos(a); im += w[i] * s.v * sin(a) }
            return sqrt(re * re + im * im)
        }
        val mags = ArrayList<Double>(); var bestF = 0.0; var bestM = 0.0
        var f = 3.0
        while (f <= 60.0) { val m = magnitude(f); mags.add(m); if (m > bestM) { bestM = m; bestF = f }; f += 0.05 }
        val median = mags.sorted()[mags.size / 2]
        if (bestM <= minSNR * median || bestF < 3.05) return null
        // pic à la cadence de la caméra (ou un multiple) : artefact de l'échantillonnage, pas un strobe
        val frameRate = (samples.size / bands - 1) / (t1 - t0)
        if ((1..3).any { abs(bestF - it * frameRate) < 0.4 }) return null
        // flashs courts : les harmoniques sont presque aussi fortes que la fondamentale ; on préfère f/2 ou f/3 si elle est là
        for (n in listOf(3, 2)) if (bestF / n >= 3 && magnitude(bestF / n) > 0.4 * bestM) { bestF /= n; bestM = magnitude(bestF); break }
        f = bestF - 0.05
        while (f <= bestF + 0.05) { val m = magnitude(f); if (m > bestM) { bestM = m; bestF = f }; f += 0.001 }
        return bestF
    }
}
