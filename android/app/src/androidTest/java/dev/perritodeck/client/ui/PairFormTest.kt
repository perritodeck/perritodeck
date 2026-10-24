package dev.perritodeck.client.ui

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performImeAction
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.perritodeck.client.PairState
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class PairFormTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    /**
     * The keyboard's Next key on the address goes to the code. Typed keys take the window out of
     * touch mode, and on Android 5 a tap on the on-screen keyboard doesn't bring it back, so the
     * preset buttons between the two fields can take focus: plain "next" stopped on the first of
     * them and closed the keyboard.
     */
    @Test
    fun nextOnTheAddressGoesToTheCode() {
        compose.setContent {
            PerritoTheme {
                PairScreen(PairState.Form(""), onSubmit = { _, _ -> }, onConfirm = {}, onCancel = {})
            }
        }
        compose.onNodeWithTag("address").performClick()
        compose.waitForIdle()
        // Keys, as a keyboard (or adb's "input text") types them.
        InstrumentationRegistry.getInstrumentation().sendStringSync("192.168.1.20")
        compose.waitForIdle()
        compose.onNodeWithTag("address").assertTextContains("192.168.1.20")
        compose.onNodeWithTag("address").performImeAction()
        compose.onNodeWithTag("code").assertIsFocused()
    }
}
