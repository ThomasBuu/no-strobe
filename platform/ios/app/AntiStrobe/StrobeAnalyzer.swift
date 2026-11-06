import Foundation

/// Détecte un strobe dans la luminance des images (une valeur par bande horizontale, pour le rolling shutter)
/// et dit si une image a été prise lampe éteinte.
///
/// Une fois la fréquence connue, chaque image est placée dans le cycle du strobe (phase), et c'est la luminance
/// moyenne de cette phase sur les derniers cycles qui décide : un mouvement de caméra ou de main change toutes
/// les phases pareil, donc ne fausse pas le tri.
struct StrobeAnalyzer {
    let bands: Int
    var phaseBins = 24
    var smoothing: Float = 0.2      // poids d'une nouvelle image dans la moyenne de sa phase (~5 cycles de mémoire)
    var cleanFraction: Float = 0.25 // phase propre si, dans chaque bande, elle est dans le quart bas du profil
    var minSwing: Float = 0.03      // bande ignorée si le strobe la change moins que ça (zone jamais éclairée)
    var minSNR: Float = 6           // pic de la DFT / magnitude médiane au-delà duquel on parle de strobe
    var hold = 1.5                  // s pendant lesquelles on garde la fréquence si la détection décroche

    private var history: [(t: Double, m: Float)] = []  // 2 s de luminance moyenne pour la DFT
    private var lastEstimate = -Double.infinity, lastSeen = -Double.infinity
    private var phase = 0.0, lastT: Double?
    private var profile: [[Float]] = []                // [phase][bande], NaN tant que la phase n'a pas été vue
    private var profileT: [Double] = []                // dernière mise à jour de chaque phase
    private var lastClean: (t: Double, l: [Float])?    // dernière image propre : référence qui suit le mouvement
    private(set) var frequency: Double?

    init(bands: Int) { self.bands = bands }

    /// Ajoute une image ; renvoie true si elle est exploitable (prise lampe éteinte, ou pas de strobe).
    mutating func add(time t: Double, bands l: [Float]) -> Bool {
        history.append((t, l.reduce(0, +) / Float(l.count)))
        while let f = history.first, t - f.t > 2 { history.removeFirst() }
        if t - lastEstimate > 0.25 {
            lastEstimate = t
            if let f = Self.dominantFrequency(history, minSNR: minSNR) {
                if frequency.map({ abs($0 - f) > 0.3 }) ?? true { profile = [] } // nouveau strobe : on réapprend
                frequency = f; lastSeen = t
            } else if t - lastSeen > hold { frequency = nil; profile = [] }
        }
        defer { lastT = t }
        guard let f = frequency else { return true }

        // phase continue : un changement de fréquence ne la fait pas sauter
        phase = (phase + f * (t - (lastT ?? t))).truncatingRemainder(dividingBy: 1)
        if profile.isEmpty {
            profile = Array(repeating: Array(repeating: .nan, count: bands), count: phaseBins)
            profileT = Array(repeating: -.infinity, count: phaseBins)
        }
        let bin = min(Int(phase * Double(phaseBins)), phaseBins - 1)
        profileT[bin] = t
        for b in 0..<bands {
            let p = profile[bin][b]
            profile[bin][b] = p.isNaN ? l[b] : p + smoothing * (l[b] - p)
        }
        // référence récente (< 0,2 s) pour juger l'image elle-même ; sinon la moyenne de sa phase
        let ref = lastClean.flatMap { t - $0.t < 0.2 ? $0.l : nil } ?? profile[bin]
        for b in 0..<bands {
            var lo = Float.infinity, hi = -Float.infinity
            // phases vues récemment seulement : une phase périmée garde la luminosité d'une scène qui a changé
            for (row, seen) in zip(profile, profileT) where t - seen < 0.3 && !row[b].isNaN { lo = min(lo, row[b]); hi = max(hi, row[b]) }
            guard hi - lo >= minSwing else { continue }
            // phase sale, ou image elle-même éclairée (phase à cheval sur un front du strobe)
            if profile[bin][b] > lo + cleanFraction * (hi - lo) || l[b] > ref[b] + 0.5 * (hi - lo) { return false }
        }
        lastClean = (t, l)
        return true
    }

    // ponytail: DFT directe (~600 fréquences × ≤480 échantillons, 4 fois par seconde) ; vDSP si ça chauffe.
    static func dominantFrequency(_ h: [(t: Double, m: Float)], minSNR: Float) -> Double? {
        guard h.count > 32, let t0 = h.first?.t, let t1 = h.last?.t, t1 - t0 > 0.5 else { return nil }
        // passe-haut : on retire la moyenne glissante sur 0,25 s (mouvements de main, panoramiques, changements de scène)
        var v = [Float](repeating: 0, count: h.count), lo = 0, hi = 0, sum: Float = 0
        for i in h.indices {
            while hi < h.count, h[hi].t <= h[i].t + 0.125 { sum += h[hi].m; hi += 1 }
            while h[lo].t < h[i].t - 0.125 { sum -= h[lo].m; lo += 1 }
            v[i] = h[i].m - sum / Float(hi - lo)
            v[i] *= Float(0.5 - 0.5 * cos(2 * Double.pi * (h[i].t - t0) / (t1 - t0))) // fenêtre de Hann : pas de fuite spectrale
        }
        func magnitude(_ f: Double) -> Float {
            var re: Float = 0, im: Float = 0
            let w = 2 * Double.pi * f
            for (s, x) in zip(h, v) {
                let a = w * (s.t - t0)
                re += x * Float(cos(a)); im += x * Float(sin(a))
            }
            return (re * re + im * im).squareRoot()
        }
        var mags: [Float] = [], best = (f: 0.0, m: Float(0))
        for f in stride(from: 2.0, through: 60.0, by: 0.1) {
            let m = magnitude(f)
            mags.append(m)
            if m > best.m { best = (f, m) }
        }
        let median = mags.sorted()[mags.count / 2]
        // pic au bord bas = fuite d'un mouvement lent (main, panoramique), pas un strobe
        guard best.m > minSNR * median, best.f > 2.05 else { return nil }
        // affinage : la phase dérive de (erreur × temps), donc la précision compte
        for f in stride(from: best.f - 0.1, through: best.f + 0.1, by: 0.005) {
            let m = magnitude(f)
            if m > best.m { best = (f, m) }
        }
        return best.f
    }
}

/// Transforme des images propres irrégulières en sortie à cadence fixe : pour chaque créneau,
/// l'image propre la plus proche (dupliquée si besoin), horodatée au créneau.
struct FramePacer<Frame> {
    let interval: Double
    var maxGap = 0.1 // au-delà, créneau sauté (le lecteur garde l'image précédente)
    private var next: Double?
    private var candidate: (t: Double, f: Frame)? // dernière image propre vue

    init(interval: Double) { self.interval = interval }

    mutating func push(time t: Double, clean: Bool, frame: Frame) -> [(frame: Frame, time: Double)] {
        if next == nil { guard clean else { return [] }; next = t }
        var out: [(frame: Frame, time: Double)] = []
        while let s = next, t >= s {
            let before = candidate.map { s - $0.t } ?? .infinity
            if clean {
                if t - s <= before, t - s <= maxGap { out.append((frame, s)) }
                else if let c = candidate, before <= maxGap { out.append((c.f, s)) }
            } else {
                if t - s < min(before, maxGap) { break } // une image propre plus proche peut encore arriver
                if let c = candidate, before <= maxGap { out.append((c.f, s)) }
            }
            next = s + interval
        }
        if clean { candidate = (t, frame) }
        return out
    }
}
