package dev.nostrobe

import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager

/**
 * A camera the user can pick. `physicalId` pins one lens of a logical multi-camera: the
 * logical camera of a phone switches lenses by itself (zoom, low light), which changes the
 * sensor and its row readout under the flicker measurement.
 */
data class LensOption(
    val label: String,
    val logicalId: String,
    val physicalId: String?,
    /** 35 mm equivalent focal length, 0 when unknown. */
    val equivMm: Double,
)

object LensOptions {
    fun list(manager: CameraManager): List<LensOption> {
        val back = ArrayList<LensOption>()
        val front = ArrayList<LensOption>()
        val auto = ArrayList<LensOption>()
        for (id in manager.cameraIdList) {
            val chars = manager.getCameraCharacteristics(id)
            val facing = chars.get(CameraCharacteristics.LENS_FACING)
            val isFront = facing == CameraCharacteristics.LENS_FACING_FRONT
            val physical = chars.physicalCameraIds
            if (physical.isNotEmpty()) {
                for (pid in physical) {
                    val mm = equivMm(manager.getCameraCharacteristics(pid))
                    (if (isFront) front else back).add(LensOption(name(mm, isFront), id, pid, mm))
                }
                if (!isFront) auto.add(LensOption("auto (phone switches lens)", id, null, 0.0))
            } else {
                val mm = equivMm(chars)
                (if (isFront) front else back).add(LensOption(name(mm, isFront), id, null, mm))
            }
        }
        return back.sortedBy { it.equivMm } + front.sortedBy { it.equivMm } + auto
    }

    /** The lens to start on: the main one (closest to 24-28 mm), never the auto-switching one. */
    fun defaultIndex(options: List<LensOption>): Int {
        val backPinned = options.withIndex().filter { it.value.equivMm > 0 && !it.value.label.startsWith("front") }
        return backPinned.minByOrNull { kotlin.math.abs(it.value.equivMm - 26.0) }?.index ?: 0
    }

    private fun equivMm(chars: CameraCharacteristics): Double {
        val f = chars.get(CameraCharacteristics.LENS_INFO_AVAILABLE_FOCAL_LENGTHS)?.firstOrNull()
        val size = chars.get(CameraCharacteristics.SENSOR_INFO_PHYSICAL_SIZE)
        if (f == null || size == null || size.width <= 0f) return 0.0
        return f.toDouble() * 36.0 / size.width
    }

    private fun name(mm: Double, front: Boolean): String {
        if (front) return "front"
        if (mm <= 0.0) return "back"
        val kind = when {
            mm < 20 -> "ultra-wide"
            mm < 40 -> "wide"
            else -> "tele"
        }
        return "$kind %.0f mm".format(mm)
    }
}
