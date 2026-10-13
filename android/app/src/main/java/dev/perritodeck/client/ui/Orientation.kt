package dev.perritodeck.client.ui

import android.content.pm.ActivityInfo

/**
 * The screen orientation that suits a [cols]×[rows] grid: landscape for a wider grid and
 * portrait for a taller one (either way up, as the person prefers), so the keys fill the
 * screen the way the computer's editor draws them. A square grid leaves it to the tablet.
 * The host sizes key images for the same orientation (`render::key_pixel_size`).
 */
fun orientationFor(cols: Int, rows: Int): Int = when {
    cols > rows -> ActivityInfo.SCREEN_ORIENTATION_USER_LANDSCAPE
    rows > cols -> ActivityInfo.SCREEN_ORIENTATION_USER_PORTRAIT
    else -> ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
}
