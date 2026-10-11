package dev.perritodeck.client.net

import android.graphics.BitmapFactory
import android.util.Base64
import androidx.compose.ui.graphics.asImageBitmap

/** Decodes host-rendered key images (`data:image/png;base64,…` or WebP). */
object KeyImages {
    private val prefix = Regex("^data:image/(png|webp);base64,")

    fun decode(dataUrl: String): Any? {
        val match = prefix.find(dataUrl) ?: return null
        return runCatching {
            val bytes = Base64.decode(dataUrl.substring(match.range.last + 1), Base64.DEFAULT)
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap()
        }.getOrNull()
    }
}
