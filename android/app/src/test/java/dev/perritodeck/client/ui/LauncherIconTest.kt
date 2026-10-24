package dev.perritodeck.client.ui

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.File
import java.util.zip.Inflater

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

    /**
     * Android 5 to 7 (API 22 to 25) get bitmaps, one per density, made from icon-rounded-1024.png
     * (icon-master-rounded.svg). A vector there is drawn at the screen's own density whatever
     * density the launcher asks for (on API 22 and 23), and launchers don't scale icons up: a
     * 160 dpi tablet's launcher asks for 240 dpi icons, so a 48 dp vector came out a third
     * smaller than the other apps' icons.
     */
    @Test
    fun theFallbackIconIsABitmapForEveryDensity() {
        assertFalse(
            "drawable/ic_launcher.xml would be drawn too small on Android 5",
            File(res, "drawable/ic_launcher.xml").exists(),
        )
        for ((density, size) in listOf("mdpi" to 48, "hdpi" to 72, "xhdpi" to 96, "xxhdpi" to 144, "xxxhdpi" to 192)) {
            val file = File(res, "drawable-$density/ic_launcher.png")
            assertTrue("no $file", file.isFile)
            val image = Rgba.read(file)
            assertEquals("$file width", size, image.width)
            assertEquals("$file height", size, image.height)
            // The tile covers 72 to 952 of the SVG's 1024, with a transparent margin.
            var (left, top, right, bottom) = listOf(size, size, -1, -1)
            for (y in 0 until size) for (x in 0 until size) {
                if (image.alpha(x, y) >= 128) {
                    left = minOf(left, x); top = minOf(top, y); right = maxOf(right, x); bottom = maxOf(bottom, y)
                }
            }
            val from = size * 72.0 / 1024
            val to = size * 952.0 / 1024
            for ((name, edge, expected) in listOf(
                Triple("left", left, from), Triple("top", top, from),
                Triple("right", right + 1, to), Triple("bottom", bottom + 1, to),
            )) {
                assertEquals("$file tile $name edge", expected, edge.toDouble(), 1.0)
            }
        }
    }

    @Test
    fun theAppHasTheFinishedSetFromDocsAssets() {
        val finished = File(assets, "android").walk().filter { it.isFile }.toList()
        assertTrue("nothing in ${File(assets, "android")}", finished.isNotEmpty())
        for (file in finished) {
            val path = file.relativeTo(File(assets, "android")).invariantSeparatorsPath
            assertArrayEquals("res/$path differs from docs/assets/android", file.readBytes(), File(res, path).readBytes())
        }
    }

    /** An 8-bit RGBA PNG, as the icons are saved (unit tests have no image decoder). */
    private class Rgba(val width: Int, val height: Int, private val pixels: ByteArray) {
        fun alpha(x: Int, y: Int): Int = pixels[(y * width + x) * 4 + 3].toInt() and 0xff

        companion object {
            fun read(file: File): Rgba {
                val input = DataInputStream(file.inputStream().buffered())
                val signature = ByteArray(8).also { input.readFully(it) }
                assertArrayEquals("$file isn't a PNG", byteArrayOf(-119, 80, 78, 71, 13, 10, 26, 10), signature)
                var width = 0
                var height = 0
                val data = ByteArrayOutputStream()
                input.use {
                    while (true) {
                        val length = it.readInt()
                        val type = ByteArray(4).also { t -> it.readFully(t) }.toString(Charsets.US_ASCII)
                        val body = ByteArray(length).also { b -> it.readFully(b) }
                        it.readInt() // CRC
                        when (type) {
                            "IHDR" -> {
                                val header = DataInputStream(body.inputStream())
                                width = header.readInt()
                                height = header.readInt()
                                assertEquals("$file bit depth", 8, header.read())
                                assertEquals("$file colour type (RGBA)", 6, header.read())
                                header.skipBytes(2)
                                assertEquals("$file interlace", 0, header.read())
                            }
                            "IDAT" -> data.write(body)
                            "IEND" -> break
                        }
                    }
                }
                val stride = width * 4
                val raw = ByteArray((stride + 1) * height)
                Inflater().run {
                    setInput(data.toByteArray())
                    var done = 0
                    while (done < raw.size && !finished()) done += inflate(raw, done, raw.size - done)
                    end()
                }
                val pixels = ByteArray(stride * height)
                for (y in 0 until height) {
                    val filter = raw[y * (stride + 1)].toInt()
                    for (i in 0 until stride) {
                        val x = raw[y * (stride + 1) + 1 + i].toInt() and 0xff
                        val a = if (i >= 4) pixels[y * stride + i - 4].toInt() and 0xff else 0
                        val b = if (y > 0) pixels[(y - 1) * stride + i].toInt() and 0xff else 0
                        val c = if (i >= 4 && y > 0) pixels[(y - 1) * stride + i - 4].toInt() and 0xff else 0
                        val predicted = when (filter) {
                            0 -> 0
                            1 -> a
                            2 -> b
                            3 -> (a + b) / 2
                            4 -> {
                                val p = a + b - c
                                val pa = Math.abs(p - a)
                                val pb = Math.abs(p - b)
                                val pc = Math.abs(p - c)
                                if (pa <= pb && pa <= pc) a else if (pb <= pc) b else c
                            }
                            else -> throw AssertionError("$file: unknown PNG filter $filter")
                        }
                        pixels[y * stride + i] = (x + predicted).toByte()
                    }
                }
                return Rgba(width, height, pixels)
            }
        }
    }
}
