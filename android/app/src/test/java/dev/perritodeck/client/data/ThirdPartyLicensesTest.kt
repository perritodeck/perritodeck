package dev.perritodeck.client.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File

class ThirdPartyLicensesTest {
    private val rule = "=".repeat(78)
    private val dashes = "-".repeat(78)

    private val sample = listOf(
        "Open-source software in Perrito",
        "",
        "Intro line one",
        "intro line two.",
        "",
        rule,
        "Libraries",
        dashes,
        "com.example:a 1.0",
        "A",
        "Licence: Apache-2.0",
        "",
        "com.example:b 2.0",
        "Licence: Apache-2.0, BSD-3-Clause (libyuv)",
        "https://example.com/b",
        "",
        rule,
        "Apache License 2.0, used by Perrito and the libraries marked Apache-2.0",
        dashes,
        "                                 Apache License",
        "                           Version 2.0, January 2004",
        "",
        "   1. Definitions.",
        "",
        "      \"License\" shall mean the terms and conditions for use, reproduction,",
        "      and distribution as defined by Sections 1 through 9 of this document.",
        "",
    ).joinToString("\n")

    @Test
    fun readsThePreambleTheLibrariesAndTheTexts() {
        val doc = ThirdPartyLicenses.parse(sample)
        assertEquals(listOf(listOf("Open-source software in Perrito"), listOf("Intro line one", "intro line two.")), doc.preamble)
        assertEquals(
            listOf(
                ThirdPartyLicenses.Library("com.example:a 1.0", listOf("A", "Licence: Apache-2.0")),
                ThirdPartyLicenses.Library(
                    "com.example:b 2.0",
                    listOf("Licence: Apache-2.0, BSD-3-Clause (libyuv)", "https://example.com/b"),
                ),
            ),
            doc.libraries,
        )
        assertEquals(1, doc.texts.size)
        assertEquals("Apache License 2.0, used by Perrito and the libraries marked Apache-2.0", doc.texts[0].title)
        assertEquals(3, doc.texts[0].paragraphs.size)
    }

    @Test
    fun windowsLineEndsReadTheSame() {
        assertEquals(ThirdPartyLicenses.parse(sample), ThirdPartyLicenses.parse(sample.replace("\n", "\r\n")))
    }

    @Test
    fun paragraphsAreJoinedToWrapOnScreenButHeadingsStayCentred() {
        val apache = ThirdPartyLicenses.parse(sample).texts[0].paragraphs
        assertEquals(
            ThirdPartyLicenses.Paragraph("Apache License\nVersion 2.0, January 2004", indent = 0, centered = true),
            ThirdPartyLicenses.reflow(apache[0]),
        )
        assertEquals(ThirdPartyLicenses.Paragraph("1. Definitions.", indent = 3, centered = false), ThirdPartyLicenses.reflow(apache[1]))
        assertEquals(
            ThirdPartyLicenses.Paragraph(
                "\"License\" shall mean the terms and conditions for use, reproduction, and distribution as " +
                    "defined by Sections 1 through 9 of this document.",
                indent = 6,
                centered = false,
            ),
            ThirdPartyLicenses.reflow(apache[2]),
        )
    }

    @Test
    fun aListWithNoSectionsHasNoLibraries() {
        val doc = ThirdPartyLicenses.parse("just text\n")
        assertTrue(doc.libraries.isEmpty())
        assertTrue(doc.texts.isEmpty())
    }

    /** The file the build generated for this variant (see app/build.gradle.kts). */
    @Test
    fun theGeneratedListCoversWhatTheAppShips() {
        val path = System.getProperty("perritodeck.licenses")
        assumeTrue("run through Gradle, which generates the list", path != null)
        val file = File(path!!)
        assertTrue("$file doesn't exist", file.isFile)
        val text = file.readText(Charsets.UTF_8)
        assertFalse("the list should use \\n line ends on every OS", '\r' in text)
        assertTrue(text.endsWith("\n"))

        val doc = ThirdPartyLicenses.parse(text)
        val titles = doc.libraries.map { it.title }
        val modules = titles.map { it.substringBefore(' ') }
        for (shipped in listOf(
            "com.squareup.okhttp3:okhttp",
            "com.google.zxing:core",
            "org.jetbrains.kotlin:kotlin-stdlib",
            "androidx.camera:camera-core",
            "androidx.compose.material3:material3-android",
        )) {
            assertTrue("$shipped is in the app but not in the list", shipped in modules)
        }
        for (testOnly in listOf("junit:junit", "com.squareup.okhttp3:mockwebserver", "org.jetbrains.kotlinx:kotlinx-coroutines-test")) {
            assertFalse("$testOnly is only for tests and shouldn't be listed", testOnly in modules)
        }
        assertEquals("each library once", modules.distinct(), modules)
        val ordered = modules.sortedWith(compareBy({ it.substringBefore(':') }, { it.substringAfter(':') }))
        assertEquals("libraries sorted by group, then name", ordered, modules)

        // Every licence named in the list has its text further down.
        val labels = doc.libraries.flatMap { lib ->
            val line = lib.details.singleOrNull { it.startsWith("Licence: ") }
            assertTrue("${lib.title} names no licence", line != null)
            line!!.removePrefix("Licence: ").split(", ")
        }.toSet()
        val textTitles = doc.texts.map { it.title }
        for (label in labels) {
            val title = if (label == "Apache-2.0") "Apache License 2.0" else label
            assertTrue("no text for $label", textTitles.any { it.startsWith(title) })
        }
        val apache = doc.texts.single { it.title.startsWith("Apache License 2.0") }
        assertTrue(apache.paragraphs.flatten().any { "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION" in it })
        // camera-core's native code includes libyuv, and OkHttp carries the Public Suffix List.
        assertTrue(textTitles.any { it.startsWith("BSD-3-Clause (libyuv), used by androidx.camera:camera-core") })
        assertTrue(textTitles.any { it.startsWith("Notice in com.squareup.okhttp3:okhttp") })
        val okhttp = doc.libraries.single { it.title.startsWith("com.squareup.okhttp3:okhttp ") }
        assertTrue(okhttp.details.toString(), "Licence: Apache-2.0, MPL-2.0 (Public Suffix List data)" in okhttp.details)
        val mpl = doc.texts.single { it.title.startsWith("MPL-2.0 (Public Suffix List data), used by com.squareup.okhttp3:okhttp ") }
        assertEquals(listOf("Mozilla Public License Version 2.0", "=================================="), mpl.paragraphs.first())
        // ZXing's POM names no licence: it comes from its parent POM, zxing-parent.
        val zxing = doc.libraries.single { it.title.startsWith("com.google.zxing:core ") }
        assertTrue(zxing.details.toString(), "Licence: Apache-2.0" in zxing.details)
    }

    @Test
    fun anUnderlinedLineIsAHeading() {
        assertEquals(
            ThirdPartyLicenses.Paragraph("1. Definitions", indent = 0, centered = false, heading = true),
            ThirdPartyLicenses.reflow(listOf("1. Definitions", "--------------")),
        )
        assertEquals(
            ThirdPartyLicenses.Paragraph("Mozilla Public License Version 2.0", indent = 0, centered = false, heading = true),
            ThirdPartyLicenses.reflow(listOf("Mozilla Public License Version 2.0", "==================================")),
        )
        assertFalse(ThirdPartyLicenses.reflow(listOf("A sentence that", "goes on.")).heading)
    }

    @Test
    fun aBoxDrawnWithStarsIsLeftOut() {
        val boxed = listOf(
            "*".repeat(20),
            "*" + " ".repeat(18) + "*",
            "*  6. Disclaimer   *",
            "*  -------------   *",
            "*" + " ".repeat(18) + "*",
            "*  Covered Software *",
            "*  is provided.     *",
            "*" + " ".repeat(18) + "*",
            "*".repeat(20),
        )
        val paragraphs = ThirdPartyLicenses.paragraphs(boxed)
        assertEquals(listOf(listOf("  6. Disclaimer", "  -------------"), listOf("  Covered Software", "  is provided.")), paragraphs)
        assertEquals(
            ThirdPartyLicenses.Paragraph("6. Disclaimer", indent = 0, centered = false, heading = true),
            ThirdPartyLicenses.reflow(paragraphs[0]),
        )
        assertEquals(
            ThirdPartyLicenses.Paragraph("Covered Software is provided.", indent = 2, centered = false),
            ThirdPartyLicenses.reflow(paragraphs[1]),
        )
    }
}
