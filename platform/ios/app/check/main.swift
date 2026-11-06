// Vérification du tri : swiftc ../AntiStrobe/StrobeAnalyzer.swift main.swift -o /tmp/strobecheck && /tmp/strobecheck
import Foundation

struct Run { var freq: Double?; var kept = 0, total = 0, flashKept = 0, firstDetect: Double?; var out: [(t: Double, slot: Double)] = [] }

/// 240 i/s, 8 bandes décalées de 0,5 ms (rolling shutter), bruit, et en option une caméra qui bouge.
func simulate(strobe hz: Double?, duty: Double = 0.5, seconds: Double = 5, moving: Bool = false) -> Run {
    var a = StrobeAnalyzer(bands: 8), pacer = FramePacer<Double>(interval: 1 / 30), r = Run()
    for i in 0..<Int(seconds * 240) {
        let t = Double(i) / 240
        var lit = false, lums = [Float]()
        for b in 0..<8 {
            let tb = t + Double(b) * 0.0005
            let on = hz.map { (tb * $0).truncatingRemainder(dividingBy: 1) < duty } ?? false
            lit = lit || on
            let scene: Float = moving ? 0.2 + 0.12 * Float(sin(2 * .pi * 0.7 * t + Double(b))) : 0.2 // main/caméra qui bouge
            lums.append(scene + (on ? 0.4 : 0) + Float.random(in: -0.01...0.01))
        }
        let ok = a.add(time: t, bands: lums)
        if r.firstDetect == nil, a.frequency != nil { r.firstDetect = t }
        if t > 1 {
            r.total += 1
            if ok { r.kept += 1; if lit { r.flashKept += 1 } }
            r.out += pacer.push(time: t, clean: ok, frame: t).map { ($0.frame, $0.time) }
        }
    }
    r.freq = a.frequency
    return r
}

func check(_ name: String, _ r: Run, hz: Double?, duty: Double = 0.5, minKept: Double) {
    if let hz { precondition(r.freq.map { abs($0 - hz) < 0.1 } == true, "\(name) : fréquence \(String(describing: r.freq))") }
    else { precondition(r.freq == nil, "\(name) : strobe fantôme \(r.freq!)") }
    precondition(r.flashKept == 0, "\(name) : \(r.flashKept) images avec flash gardées")
    precondition(Double(r.kept) >= minKept * Double(r.total), "\(name) : \(r.kept)/\(r.total) gardées")
    precondition((r.firstDetect ?? 0) < 0.8, "\(name) : détecté seulement à \(r.firstDetect!) s")
    // sortie : un créneau tous les 1/30 s, sans trou, image au plus à une demi-durée de flash de son créneau
    let gaps = zip(r.out.dropFirst(), r.out).map { $0.slot - $1.slot }
    precondition(gaps.allSatisfy { abs($0 - 1.0 / 30) < 1e-6 }, "\(name) : cadence irrégulière")
    let err = r.out.map { abs($0.t - $0.slot) }.max() ?? 0
    let limit = hz.map { duty / $0 / 2 + 0.005 } ?? 0.005 // au pire, créneau au milieu d'un flash
    precondition(err <= limit, "\(name) : image à \(err * 1000) ms de son créneau")
    print("\(name) → \(r.freq.map { String(format: "%.2f Hz", $0) } ?? "aucun strobe"), \(r.kept)/\(r.total) gardées, sortie \(r.out.count) images régulières, écart max \(Int(err * 1000)) ms")
}

for hz in [5.0, 12.0, 17.5, 30.0] { check("\(hz) Hz", simulate(strobe: hz), hz: hz, minKept: 0.35) }
check("12 Hz caméra qui bouge", simulate(strobe: 12, moving: true), hz: 12, minKept: 0.35)
check("10 Hz cycle 20 %", simulate(strobe: 10, duty: 0.2), hz: 10, duty: 0.2, minKept: 0.6)
check("sans strobe", simulate(strobe: nil), hz: nil, minKept: 1)
check("sans strobe, caméra qui bouge", simulate(strobe: nil, moving: true), hz: nil, minKept: 1)
print("OK")
