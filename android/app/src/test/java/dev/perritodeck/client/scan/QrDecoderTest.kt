package dev.perritodeck.client.scan

import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class QrDecoderTest {
    private val link =
        "perritodeck://pair?v=1&n=Dev%20Laptop&a=192.168.1.20%3A46710&fp=q1w2e3r4t5y6u7i8o9p0a1s2d3f4g5h6j7k8l9z0x1c&c=042913"

    /**
     * Renders [text] as a QR code into a luminance frame, centred, with camera-like padding
     * and stride. [inverted] draws it light on dark, like a terminal with a dark background.
     */
    private fun frame(
        text: String,
        width: Int = 640,
        height: Int = 480,
        stride: Int = 704,
        inverted: Boolean = false,
    ): ByteArray {
        val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 300, 300)
        val (dark, light) = if (inverted) 200.toByte() to 20.toByte() else 20.toByte() to 200.toByte()
        val out = ByteArray(stride * height) { light } // background, same as the quiet zone
        val x0 = (width - matrix.width) / 2
        val y0 = (height - matrix.height) / 2
        for (y in 0 until matrix.height) for (x in 0 until matrix.width) {
            if (matrix[x, y]) out[(y0 + y) * stride + x0 + x] = dark
        }
        return out
    }

    @Test
    fun decodesThePairingLinkFromACameraFrame() {
        assertEquals(link, QrDecoder().decode(frame(link), 640, 480, 704))
    }

    @Test
    fun decodesACodeDrawnLightOnDark() {
        // What `perritodeck-host --pair` prints in a dark terminal.
        assertEquals(link, QrDecoder().decode(frame(link, inverted = true), 640, 480, 704))
    }

    @Test
    fun emptyFramesDecodeToNothing() {
        assertNull(QrDecoder().decode(ByteArray(640 * 480) { 128.toByte() }, 640, 480))
    }

    @Test
    fun decoderIsReusable() {
        val d = QrDecoder()
        assertNull(d.decode(ByteArray(320 * 240), 320, 240))
        assertEquals(link, d.decode(frame(link), 640, 480, 704))
    }
}
