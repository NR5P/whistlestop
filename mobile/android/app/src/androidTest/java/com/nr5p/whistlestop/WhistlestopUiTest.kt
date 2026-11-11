package com.nr5p.whistlestop

import android.content.Intent
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performScrollTo
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Real taps on the app, with the engine's demo station (K7BOT on a
 * simulated band): link, chat, Depot, search, Help.
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(AndroidJUnit4::class)
class WhistlestopUiTest {
    @get:Rule
    val rule = createEmptyComposeRule()
    private var scenario: ActivityScenario<MainActivity>? = null

    @Before
    fun launch() {
        val ctx = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = Intent(ctx, MainActivity::class.java).putExtra("ws_uitest", true)
        scenario = ActivityScenario.launch(intent)
        rule.waitUntilAtLeastOneExists(hasText("N0PHN"), 10_000)
        rule.waitUntilAtLeastOneExists(hasContentDescription("Modem ready"), 20_000)
    }

    @After
    fun close() {
        scenario?.close()
        Core.stop()
    }

    private fun field(placeholder: String) = rule.onAllNodes(hasSetTextAction() and hasText(placeholder)).onFirst()

    @Test
    fun connectChatAndDisconnect() {
        field("Callsign").performScrollTo().performTextInput("K7BOT")
        rule.onNode(hasText("Connect") and androidx.compose.ui.test.hasClickAction()).performScrollTo().performClick()
        // Linking switches to Chat.
        rule.waitUntilAtLeastOneExists(hasText("Linked with K7BOT"), 60_000)
        field("Type a message").performTextInput("hello from Android")
        rule.onNode(hasContentDescription("Send")).performClick()
        // The demo station answers, quoting us.
        rule.waitUntil(90_000) {
            rule.onAllNodes(hasText("hello from Android", substring = true)).fetchSemanticsNodes().size >= 2
        }
        rule.onNode(hasText("Disconnect")).performClick()
        rule.waitUntilAtLeastOneExists(hasText("Not linked"), 60_000)
    }

    @Test
    fun writeDepotMail() {
        rule.onNode(hasText("Depot")).performClick()
        rule.onNode(hasContentDescription("Write mail")).performClick()
        field("To (callsign)").performTextInput("K7BOT")
        field("Message").performTextInput("See you on 20m tonight.")
        rule.onNode(hasText("Post")).performClick()
        rule.onNode(hasText("Departures")).performClick()
        rule.waitUntilAtLeastOneExists(hasText("To K7BOT"), 10_000)
    }

    @Test
    fun searchFindsTheDemoStation() {
        rule.onNode(hasText("Search current band")).performScrollTo().performClick()
        rule.waitUntilAtLeastOneExists(hasText("Stop search"), 5_000)
        // Search locks onto K7BOT's whistle, then its CQ is decoded.
        rule.waitUntilAtLeastOneExists(hasText("Answer"), 150_000)
        rule.onAllNodes(hasText("Answer")).onFirst().performScrollTo().performClick()
        rule.waitUntilAtLeastOneExists(hasText("Linked with K7BOT"), 90_000)
    }

    @Test
    fun helpModeExplains() {
        rule.onNode(hasContentDescription("Help mode")).performClick()
        rule.onNode(hasContentDescription("Help: Search")).performScrollTo().performClick()
        rule.waitUntilAtLeastOneExists(hasText("Steps the radio", substring = true), 5_000)
    }
}
