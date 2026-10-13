package dev.perritodeck.client.ui

import android.content.pm.ActivityInfo
import org.junit.Assert.assertEquals
import org.junit.Test

class OrientationTest {
    @Test
    fun theScreenTurnsToTheGridsShape() {
        // The starter profile's 5×3 fills a Fire 7 only sideways.
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_USER_LANDSCAPE, orientationFor(5, 3))
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_USER_LANDSCAPE, orientationFor(8, 5))
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_USER_PORTRAIT, orientationFor(3, 5))
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED, orientationFor(4, 4))
    }
}
