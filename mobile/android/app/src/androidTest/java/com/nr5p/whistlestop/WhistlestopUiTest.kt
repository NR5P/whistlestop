package com.nr5p.whistlestop

import android.content.Intent
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
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
        rule.waitUntilAtLeastOneExists(hasText("Copy: \"hello from Android\"", substring = true), 90_000)
        // And our own line is in the chat. (Above the demo station's
        // greeting, which comes after its encryption handshake: with the
        // keyboard up it can be scrolled out of view, so scroll to it.)
        rule.onNode(androidx.compose.ui.test.hasScrollToIndexAction())
            .performScrollToNode(hasText("hello from Android"))
        rule.onNode(hasText("hello from Android")).assertExists()
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

    /** Tap a button by its text, scrolling to it when it's in a scrolling screen. */
    private fun tap(text: String) {
        val n = rule.onNode(hasText(text) and androidx.compose.ui.test.hasClickAction())
        runCatching { n.performScrollTo() }
        n.performClick()
    }

    @Test
    fun roundtableJoinTalkAndLeave() {
        tap("Chat")
        tap("Roundtable")
        field("Roundtable name").performTextInput("Evening Table")
        tap("Join here")
        rule.waitUntilAtLeastOneExists(hasText("Evening Table"), 30_000)
        // K7BOT joins and greets the table.
        rule.waitUntilAtLeastOneExists(hasText("Good evening all", substring = true), 90_000)
        field("Say something to the table").performTextInput("good evening from Android")
        rule.onNode(hasContentDescription("Send to the table")).performClick()
        rule.waitUntilAtLeastOneExists(hasText("good evening from Android", substring = true), 10_000)
        rule.waitUntilAtLeastOneExists(hasText("solid copy here", substring = true), 90_000)
        rule.onNode(hasText("Leave")).performClick()
        rule.waitUntilAtLeastOneExists(hasText("Join here"), 30_000)
    }

    @Test
    fun hostRoundtableByCq() {
        rule.onAllNodes(hasText("Roundtable") and androidx.compose.ui.test.hasClickAction()).onFirst().performScrollTo().performClick()
        field("Roundtable name").performScrollTo().performTextInput("Phone Table")
        tap("Whistle and host the table")
        // Hosting switches to the table; K7BOT hears the call and joins.
        rule.waitUntilAtLeastOneExists(hasText("you host"), 30_000)
        rule.waitUntilAtLeastOneExists(hasText("Good evening all", substring = true), 150_000)
        // At the table, nothing on Operate may take the station away.
        tap("Operate")
        rule.onNode(hasText("Search current band")).performScrollTo().assertIsNotEnabled()
    }

    @Test
    fun encryptionShowsCheckCode() {
        tap("Settings")
        rule.onNode(hasContentDescription("Encryption Prefer")).performScrollTo().performClick()
        // Leaving Off asks to confirm the legal warning.
        rule.waitUntilAtLeastOneExists(hasText("Turn on encryption?"), 5_000)
        rule.onNode(hasText("Turn on")).performClick()
        tap("Save")
        tap("Operate")
        field("Callsign").performScrollTo().performTextInput("K7BOT")
        rule.onNode(hasText("Connect") and androidx.compose.ui.test.hasClickAction()).performScrollTo().performClick()
        rule.waitUntilAtLeastOneExists(hasText("Linked with K7BOT"), 60_000)
        // K7BOT prefers encryption too: the link is sealed and shows its check code.
        rule.waitUntilAtLeastOneExists(hasText("🔒 Encrypted"), 120_000)
        rule.waitUntilAtLeastOneExists(hasContentDescription("check code", substring = true), 5_000)
    }
}
