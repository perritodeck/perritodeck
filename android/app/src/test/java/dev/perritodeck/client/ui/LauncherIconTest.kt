package dev.perritodeck.client.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The launcher icon's vectors are the SVGs in docs/assets on Android's canvas. Each SVG places
 * the paw with translate(512 512) scale(s) translate(-512 -c); the vector's group scales by s
 * about the centre of its viewport, so it has to move the paw down by s × (centre − c), in
 * viewport units, to put it in the same place.
 */
class LauncherIconTest {
    private val repo = File(System.getProperty("perritodeck.repo") ?: "../..")
    private val res = File(repo, "android/app/src/main/res")
    private val assets = File(repo, "docs/assets")

    private fun attribute(xml: String, name: String): Double =
        Regex("""android:$name="([-\d.]+)"""").find(xml)?.groupValues?.get(1)?.toDouble()
            ?: throw AssertionError("no android:$name")

    private fun assertPlacedLikeItsSvg(vector: String, svg: String) {
        val source = File(assets, svg).readText()
        val transform = Regex("""translate\(512 512\) scale\(([\d.]+)\) translate\(-512 -([\d.]+)\)""").find(source)
            ?: throw AssertionError("$svg has no paw transform")
        val scale = transform.groupValues[1].toDouble()
        val centreY = transform.groupValues[2].toDouble()

        val xml = File(res, vector).readText()
        val viewport = attribute(xml, "viewportHeight")
        val group = xml.substring(xml.indexOf("<group"))
        assertEquals("$vector pivot", viewport / 2, attribute(group, "pivotY"), 0.0)
        assertEquals("$vector scale", scale, attribute(group, "scaleY"), 0.0)
        assertEquals("$vector translateY", scale * (viewport / 2 - centreY * viewport / 1024), attribute(group, "translateY"), 0.001)
    }

    @Test
    fun theAdaptiveForegroundSitsWhereItsSvgPutsIt() {
        assertPlacedLikeItsSvg("drawable/ic_launcher_foreground.xml", "icon-android-foreground.svg")
    }

    @Test
    fun theThemedIconSitsWhereItsSvgPutsIt() {
        assertPlacedLikeItsSvg("drawable/ic_launcher_monochrome.xml", "icon-monochrome.svg")
    }

    @Test
    fun theFallbackIconSitsWhereItsSvgPutsIt() {
        assertPlacedLikeItsSvg("drawable/ic_launcher.xml", "icon-master-rounded.svg")
    }

    @Test
    fun theAppHasTheFinishedSetFromDocsAssets() {
        val finished = File(assets, "android").walk().filter { it.isFile }.toList()
        assertTrue("nothing in ${File(assets, "android")}", finished.isNotEmpty())
        for (file in finished) {
            val path = file.relativeTo(File(assets, "android")).invariantSeparatorsPath
            assertEquals("res/$path differs from docs/assets/android", file.readText(), File(res, path).readText())
        }
    }
}
