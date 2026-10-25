package dev.perritodeck.client.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The debug build installs next to the release build with the same icon, so the launcher has
 * to name them apart: the manifest labels the app with launcher_name, which debug overrides.
 */
class LauncherNameTest {
    private val app = File(File(System.getProperty("perritodeck.repo") ?: "../.."), "android/app/src")

    private fun launcherName(sourceSet: String): String? =
        Regex("""<string name="launcher_name">([^<]*)</string>""")
            .find(File(app, "$sourceSet/res/values/strings.xml").readText())
            ?.groupValues?.get(1)

    @Test
    fun theLauncherShowsLauncherName() {
        val manifest = File(app, "main/AndroidManifest.xml").readText()
        val application = manifest.substring(manifest.indexOf("<application"))
        assertTrue(application.substringBefore(">").contains("""android:label="@string/launcher_name""""))
    }

    @Test
    fun theDebugBuildIsNamedApart() {
        assertEquals("Perrito", launcherName("main"))
        val debug = launcherName("debug")
        assertNotEquals("Perrito", debug)
        assertTrue("debug name $debug", debug!!.startsWith("Perrito") && debug.contains("dbg"))
    }
}
