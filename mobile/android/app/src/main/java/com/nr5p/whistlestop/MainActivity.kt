package com.nr5p.whistlestop

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.SystemBarStyle
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Inbox
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.SettingsInputAntenna
import androidx.compose.material.icons.outlined.HelpOutline
import androidx.compose.material.icons.filled.Help
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.RadioButton
import androidx.compose.material3.RadioButtonDefaults
import androidx.compose.material3.SecondaryTabRow
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonObject

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Light clock and icons over the dark header and tab bar.
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(android.graphics.Color.parseColor("#131921")),
            navigationBarStyle = SystemBarStyle.dark(android.graphics.Color.parseColor("#131921")),
        )
        val uiTest = intent?.getBooleanExtra("ws_uitest", false) == true
        setContent {
            WhistlestopTheme {
                val scope = rememberCoroutineScope()
                val station = remember { Station(scope) }
                // The radio's audio needs the microphone permission before
                // Mercury and the whistle receiver open it; start either way
                // (without it the demo station and remote modems still work).
                val askMic = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
                    station.start(applicationContext, uiTest)
                }
                LaunchedEffect(Unit) {
                    val granted = checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
                    if (granted || uiTest) station.start(applicationContext, uiTest)
                    else askMic.launch(Manifest.permission.RECORD_AUDIO)
                }
                // Like the desktop: Help mode is off every time Whistlestop starts.
                val help = remember { mutableStateOf(false) }
                CompositionLocalProvider(LocalHelp provides help) {
                    App(station)
                }
            }
        }
    }
}

private enum class Tab(val label: String) { Operate("Operate"), Chat("Chat"), Depot("Depot"), Log("Log"), Settings("Settings") }

@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
private fun App(station: Station) {
    val s by station.state.collectAsState()
    var tab by rememberSaveable { mutableStateOf(Tab.Operate) }
    // Switch to Chat when a link or a Roundtable starts, not when the
    // activity is recreated (rotation) with one already up: what was seen
    // survives recreation, and the empty state before the first poll is
    // ignored.
    var lastPeer by rememberSaveable { mutableStateOf<String?>(null) }
    var lastTable by rememberSaveable { mutableStateOf<String?>(null) }
    // The Chat tab shows the 1:1 link's chat or the Roundtable.
    var tablePane by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(s.loaded, s.link?.peer) {
        if (!s.loaded) return@LaunchedEffect
        if (s.link?.peer != null && s.link?.peer != lastPeer) { tab = Tab.Chat; tablePane = false }
        lastPeer = s.link?.peer
    }
    val tableName = s.table?.name ?: s.tablePending
    LaunchedEffect(s.loaded, tableName) {
        if (!s.loaded) return@LaunchedEffect
        if (tableName != null && tableName != lastTable) { tab = Tab.Chat; tablePane = true }
        lastTable = tableName
    }
    Column(Modifier.fillMaxSize().background(C.bg).systemBarsPadding()) {
        StationHeader(station, s)
        Box(Modifier.weight(1f)) {
            when (tab) {
                Tab.Operate -> OperateScreen(station, s)
                Tab.Chat -> Column {
                    SecondaryTabRow(selectedTabIndex = if (tablePane) 1 else 0, containerColor = C.panel, contentColor = C.brass) {
                        Tab(selected = !tablePane, onClick = { tablePane = false }, text = { Text("1:1 chat") })
                        Tab(selected = tablePane, onClick = { tablePane = true }, text = { Text(if (s.table != null) "Roundtable ●" else "Roundtable") })
                    }
                    if (tablePane) TableScreen(station, s) else ChatScreen(station, s)
                }
                Tab.Depot -> DepotScreen(station, s)
                Tab.Log -> LogScreen(s)
                Tab.Settings -> SettingsScreen(station, s)
            }
        }
        NavigationBar(containerColor = C.panel) {
            Tab.entries.forEach { t ->
                val unread = s.depot.arrivals.count { !it.read }
                NavigationBarItem(
                    selected = tab == t,
                    onClick = { tab = t },
                    icon = {
                        val icon = when (t) {
                            Tab.Operate -> Icons.Filled.SettingsInputAntenna
                            Tab.Chat -> Icons.AutoMirrored.Filled.Chat
                            Tab.Depot -> Icons.Filled.Inbox
                            Tab.Log -> Icons.AutoMirrored.Filled.List
                            Tab.Settings -> Icons.Filled.Settings
                        }
                        if (t == Tab.Depot && unread > 0) {
                            BadgedBox(badge = { Badge { Text("$unread") } }) { Icon(icon, null) }
                        } else Icon(icon, null)
                    },
                    label = { Text(t.label, fontSize = 11.sp) },
                    colors = NavigationBarItemDefaults.colors(
                        selectedIconColor = C.brass, selectedTextColor = C.brass, indicatorColor = C.cardHi,
                        unselectedIconColor = C.text, unselectedTextColor = C.dim,
                    ),
                )
            }
        }
    }
}

@Composable
private fun StationHeader(station: Station, s: EngineState) {
    val help = LocalHelp.current
    Column(Modifier.fillMaxWidth().background(C.panel).padding(horizontal = 16.dp, vertical = 10.dp)) {
        Row(verticalAlignment = Alignment.Bottom) {
            Column(Modifier.weight(1f)) {
                Text(s.mycall.ifEmpty { "NOCALL" }, color = C.brass, fontWeight = FontWeight.Bold, fontSize = 17.sp)
                Text(if (s.demo) "Demo station" else s.rigDesc, color = C.dim, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    Fmt.mhz(s.freqHz), fontFamily = FontFamily.Monospace, fontSize = 28.sp, fontWeight = FontWeight.SemiBold,
                    color = if (s.tx) C.red else C.text,
                    modifier = Modifier.semantics { contentDescription = "Dial ${Fmt.dial(s.freqHz)} kilohertz" },
                )
                Text("MHz · ${s.mode.ifEmpty { "—" }}", color = C.dim, fontSize = 12.sp)
            }
        }
        Spacer(Modifier.height(8.dp))
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Pill("Modem", s.mercuryOk)
            Pill("Radio", s.rigOk)
            Pill("Audio", s.audioOk || s.demo)
            if (s.tx) Text("TX", color = C.text, fontWeight = FontWeight.Black, fontSize = 12.sp,
                modifier = Modifier.background(C.red, CircleShape).padding(horizontal = 8.dp, vertical = 3.dp))
            Spacer(Modifier.weight(1f))
            HelpButton(HelpTopic.status)
            HelpButton(HelpTopic.away)
            Text(
                if (s.away) "Away" else "Here",
                color = if (s.away) C.ink else C.text, fontWeight = FontWeight.SemiBold, fontSize = 13.sp,
                textAlign = TextAlign.Center,
                modifier = Modifier
                    .width(56.dp)
                    .background(if (s.away) C.brass else C.cardHi, CircleShape)
                    .clickable { station.setAway(!s.away) }
                    .padding(vertical = 4.dp)
                    .semantics { contentDescription = if (s.away) "Away. Tap when you're back" else "Here. Tap when you go away" },
            )
            IconButton(
                onClick = { help.value = !help.value },
                modifier = Modifier.size(36.dp).semantics {
                    contentDescription = "Help mode"
                    stateDescription = if (help.value) "on" else "off"
                },
            ) {
                Icon(if (help.value) Icons.Filled.Help else Icons.Outlined.HelpOutline, null, tint = if (help.value) C.brass else C.dim)
            }
        }
    }
}

// ------------------------------------------------------------------ Operate

/** A kHz number as typed: "14103.5" or, with a comma decimal, "14103,5". */
private fun kHz(s: String): Double? = s.trim().replace(',', '.').toDoubleOrNull()

/** Tabular digits: readouts that change don't jiggle the layout. */
private val TNUM = androidx.compose.ui.text.TextStyle(fontFeatureSettings = "tnum")

@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
private fun OperateScreen(station: Station, s: EngineState) {
    val rows by station.rows.collectAsState()
    val cfg by station.config.collectAsState()
    var khz by rememberSaveable { mutableStateOf("") }
    var call by rememberSaveable { mutableStateOf("") }
    var band by rememberSaveable { mutableStateOf("20m") }
    var bandMenu by remember { mutableStateOf(false) }
    // The bands the station has digital segments for, so Search never
    // offers one it would refuse.
    val bands = ((cfg["segments"] as? kotlinx.serialization.json.JsonArray) ?: emptyList())
        .mapNotNull { ((it as? JsonObject)?.get("band") as? JsonPrimitive)?.content }
        .distinct().ifEmpty { listOf("40m", "30m", "20m") }
    // Linked, or a call going out: the engine refuses another call, a
    // retune, a search or a join until it's over.
    val busy = s.link != null || s.calling != null
    var cqTable by rememberSaveable { mutableStateOf(false) }
    var tableName by rememberSaveable { mutableStateOf("") }
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).imePadding().padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        WCard("Waterfall", HelpTopic.waterfall) {
            Waterfall(rows, Modifier.fillMaxWidth().height(150.dp))
        }
        s.link?.let { LinkCard(station, it, s.qsyNote) }
        WCard("Dial", HelpTopic.dial) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Field(khz, { khz = it }, "kHz, e.g. 14103", Modifier.weight(1f), caps = false, number = true)
                QuietButton("Tune", Modifier.width(96.dp), enabled = kHz(khz) != null && !busy) {
                    kHz(khz)?.let { station.setFreq(it); khz = "" }
                }
            }
        }
        WCard("Search", HelpTopic.search) {
            if (s.search.active) {
                LinearProgressIndicator(
                    progress = { s.search.index.toFloat() / maxOf(s.search.total, 1) },
                    modifier = Modifier.fillMaxWidth(), color = C.teal, trackColor = C.cardHi,
                )
                Text(
                    s.search.note.ifEmpty { "Channel ${s.search.index + 1} of ${s.search.total} · ${Fmt.dial(s.search.channelHz)} kHz" },
                    color = C.text, minLines = 2, maxLines = 2, style = TNUM,
                )
                QuietButton("Stop search") { station.stopSearch() }
            } else {
                // A search retunes the radio away from a Roundtable.
                val free = s.table == null && !busy && !s.cq.active
                BrassButton("Search current band", enabled = free) { station.searchCurrentBand() }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.weight(1f)) {
                        QuietButton("$band ▾") { bandMenu = true }
                        DropdownMenu(expanded = bandMenu, onDismissRequest = { bandMenu = false }) {
                            bands.forEach { b -> DropdownMenuItem(text = { Text(b) }, onClick = { band = b; bandMenu = false }) }
                        }
                    }
                    QuietButton("Search $band", Modifier.weight(1f), enabled = free) { station.searchBand(band) }
                }
                QuietButton("Search all bands", enabled = free) { station.searchAllBands() }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Answer automatically", color = C.text, modifier = Modifier.weight(1f))
                HelpButton(HelpTopic.autoAnswer)
                Switch(
                    checked = (cfg["auto_answer"] as? JsonPrimitive)?.content == "true",
                    onCheckedChange = { on -> station.saveOne("auto_answer", JsonPrimitive(on)) },
                    colors = SwitchDefaults.colors(checkedTrackColor = C.teal),
                    modifier = Modifier.semantics { contentDescription = "Answer automatically" },
                )
            }
        }
        WCard("Whistle", HelpTopic.whistle) {
            val w = s.whistle
            if (w == null) Text("None heard yet.", color = C.dim)
            else Column {
                Text("${Fmt.dial(w.dialHz + w.offsetHz)} kHz", color = C.text, fontWeight = FontWeight.SemiBold)
                Text("score ${"%.1f".format(w.score)} · ${Fmt.ago(w.agoS)} ago", color = C.dim, fontSize = 12.sp, style = TNUM)
            }
        }
        WCard("Heard", HelpTopic.heard) {
            if (s.heard.isEmpty()) Text("No CQs decoded yet.", color = C.dim)
            s.heard.reversed().take(8).forEach { h ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(h.call, color = C.text, fontWeight = FontWeight.SemiBold)
                        Text("${Fmt.dial(h.dialHz)} kHz · ${h.what} · ${Fmt.utc(h.at)}", color = C.dim, fontSize = 12.sp, maxLines = 1)
                    }
                    val table = h.table
                    if (table != null) {
                        BrassButton("Join", Modifier.width(110.dp).semantics { contentDescription = "Join $table" }, color = C.teal,
                            enabled = !busy && s.table == null) {
                            station.setFreq(h.dialHz / 1000)
                            station.joinTable(table)
                        }
                    } else BrassButton("Answer", Modifier.width(110.dp), enabled = !busy && s.table == null) {
                        station.setFreq(h.dialHz / 1000)
                        station.connect(h.call)
                    }
                }
            }
        }
        WCard("Call CQ", HelpTopic.cq) {
            if (s.cq.active) {
                Text("Cycle ${s.cq.cycle} of ${s.cq.cycles} · ${s.cq.phase}", color = C.text, maxLines = 1, minLines = 1, style = TNUM)
                BrassButton("Stop CQ", color = C.red) { station.stopCq() }
            } else {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    SecondaryTabRow(selectedTabIndex = if (cqTable) 1 else 0, containerColor = C.card, contentColor = C.brass, modifier = Modifier.weight(1f)) {
                        Tab(selected = !cqTable, onClick = { cqTable = false }, text = { Text("Chat (1:1)") })
                        Tab(selected = cqTable, onClick = { cqTable = true }, text = { Text("Roundtable") })
                    }
                    HelpButton(HelpTopic.cqKind)
                }
                if (cqTable) Field(tableName, { tableName = it }, "Roundtable name", Modifier.fillMaxWidth(), caps = false)
                BrassButton(
                    if (cqTable) "Whistle and host the table" else "Whistle and call CQ",
                    // At a Roundtable: only a CQ for that table, as its host.
                    enabled = !busy && s.mercuryOk && (!cqTable || tableName.isNotBlank())
                        && s.table.let { t -> t == null || (cqTable && t.host && t.name.equals(tableName.trim(), ignoreCase = true)) },
                ) { station.callCq(if (cqTable) tableName else null) }
            }
        }
        WCard("Connect", HelpTopic.connect) {
            val calling = s.calling
            if (calling != null && s.link == null) {
                // A call going out: say so, and let it be cancelled.
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Calling $calling…", color = C.brass, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                    QuietButton("Cancel", Modifier.width(120.dp)) { station.disconnect() }
                }
            } else Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Field(call, { call = it }, "Callsign", Modifier.weight(1f))
                QuietButton("Connect", Modifier.width(120.dp), enabled = call.isNotBlank() && !busy && s.table == null) {
                    station.connect(call); call = ""
                }
            }
        }
    }
}

@Composable
private fun LinkCard(station: Station, link: Link, qsyNote: String?) {
    var qsy by rememberSaveable { mutableStateOf("") }
    WCard("Linked", HelpTopic.link) {
        Row(verticalAlignment = Alignment.Top) {
            Column(Modifier.weight(1f)) {
                Text(link.peer, color = C.teal, fontWeight = FontWeight.Bold, fontSize = 20.sp)
                Text(listOf(link.peerName, link.peerGrid).filter { it.isNotEmpty() }.joinToString(" · "), color = C.dim, fontSize = 12.sp)
                Text("${link.mode} · ${link.bps.toInt()} bps · ${Fmt.ago(link.since)}", color = C.dim, fontSize = 12.sp)
            }
            QuietButton("Disconnect", Modifier.width(130.dp)) { station.disconnect() }
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Field(qsy, { qsy = it }, "Move to kHz", Modifier.weight(1f), caps = false, number = true)
            QuietButton("Ask", Modifier.width(80.dp), enabled = kHz(qsy) != null) {
                kHz(qsy)?.let { station.qsy(it); qsy = "" }
            }
            HelpButton(HelpTopic.qsy)
        }
        if (link.e2e != E2e.Off) E2eBadge(link.e2e)
        if (!qsyNote.isNullOrEmpty()) Text(qsyNote, color = C.brass, fontSize = 12.sp)
    }
}

/** Encryption on the link: setting up, the check code, or plain text. */
@Composable
private fun E2eBadge(e2e: E2e) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
        when (e2e) {
            E2e.Off -> {}
            E2e.Negotiating -> Text("⏳ Setting up encryption…", color = C.brass, fontSize = 14.sp, modifier = Modifier.weight(1f))
            is E2e.Encrypted -> {
                Text("🔒 Encrypted", color = C.teal, fontWeight = FontWeight.SemiBold, fontSize = 14.sp)
                Text(
                    "code ${e2e.sas}", color = C.text, fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold, fontSize = 16.sp,
                    modifier = Modifier.weight(1f).semantics { contentDescription = "check code ${e2e.sas.toList().joinToString(" ")}" },
                )
            }
            is E2e.Plain -> Text("🔓 Not encrypted: ${e2e.why}", color = C.dim, fontSize = 12.sp, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        }
        HelpButton(HelpTopic.linkE2e)
    }
}

// --------------------------------------------------------------------- Chat

@Composable
private fun ChatScreen(station: Station, s: EngineState) {
    var text by rememberSaveable { mutableStateOf("") }
    val list = rememberLazyListState()
    // Not the size alone: it stays at the engine's cap (500) once old
    // lines scroll away.
    LaunchedEffect(s.chat.size, s.chat.lastOrNull()) { if (s.chat.isNotEmpty()) list.animateScrollToItem(s.chat.size - 1) }
    Column(Modifier.fillMaxSize().imePadding()) {
        Row(Modifier.fillMaxWidth().background(C.panel).padding(horizontal = 16.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(8.dp).background(if (s.link != null) C.teal else C.dim, CircleShape))
            Spacer(Modifier.width(8.dp))
            Text(s.link?.let { "Linked with ${it.peer}" } ?: "Not linked", color = if (s.link != null) C.text else C.dim, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
            HelpButton(HelpTopic.chat)
            if (s.link != null) TextButton(onClick = { station.disconnect() }) { Text("Disconnect") }
        }
        s.link?.e2e?.takeIf { it != E2e.Off }?.let {
            Box(Modifier.fillMaxWidth().background(C.panel).padding(horizontal = 16.dp, vertical = 4.dp)) { E2eBadge(it) }
        }
        LazyColumn(Modifier.weight(1f).fillMaxWidth(), state = list, contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            itemsIndexed(s.chat) { _, line -> Bubble(line) }
        }
        Row(Modifier.fillMaxWidth().background(C.panel).padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Field(text, { text = it }, if (s.link == null) "Link with a station to chat" else "Type a message", Modifier.weight(1f), caps = false)
            IconButton(
                onClick = { if (text.isNotBlank()) { station.chat(text.trim()); text = "" } },
                enabled = s.link != null && text.isNotBlank(),
            ) { Icon(Icons.AutoMirrored.Filled.Send, "Send", tint = if (s.link != null && text.isNotBlank()) C.brass else C.dim) }
        }
    }
}

// --------------------------------------------------------------- Roundtable

@Composable
private fun TableScreen(station: Station, s: EngineState) {
    val t = s.table
    if (t == null) {
        var name by rememberSaveable { mutableStateOf("") }
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).imePadding().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            WCard("Roundtable", HelpTopic.roundtable) {
                Text("A group conversation on one frequency. No fixed channel: it's wherever its host calls it.", color = C.text)
                Text("The host sets the speed: it ramps up to the fastest mode every member can copy, and drops back if someone fades.", color = C.dim, fontSize = 12.sp)
            }
            val pending = s.tablePending
            WCard("Join", if (pending != null) HelpTopic.tableJoinWait else HelpTopic.tableJoin) {
                if (pending != null) {
                    Text("Joining “$pending” as soon as Mercury is free (the last link is still closing)…", color = C.brass, minLines = 3, maxLines = 3)
                    QuietButton("Cancel") { station.leaveTable() }
                } else {
                    Field(name, { name = it }, "Roundtable name", Modifier.fillMaxWidth(), caps = false)
                    BrassButton("Join here", enabled = s.link == null && name.isNotBlank()) { station.joinTable(name); name = "" }
                    Text(
                        if (s.link != null) "Disconnect the 1:1 link first."
                        else "Tune to the host's frequency first, or tap Join under Heard on Operate.",
                        color = C.dim, fontSize = 12.sp,
                    )
                }
            }
            Text("To host one, use Call CQ → Roundtable on the Operate tab.", color = C.dim, fontSize = 12.sp)
        }
        return
    }
    var text by rememberSaveable { mutableStateOf("") }
    val list = rememberLazyListState()
    LaunchedEffect(t.lines.size, t.lines.lastOrNull()) { if (t.lines.isNotEmpty()) list.animateScrollToItem(t.lines.size - 1) }
    Column(Modifier.fillMaxSize().imePadding()) {
        Column(Modifier.fillMaxWidth().background(C.panel).padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(t.name, color = C.brass, fontWeight = FontWeight.Bold, fontSize = 20.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.width(8.dp))
                Text(if (t.host) "you host" else "member", color = C.dim, fontSize = 12.sp, modifier = Modifier.weight(1f))
                HelpButton(HelpTopic.tableMembers)
                TextButton(onClick = { station.leaveTable() }) { Text("Leave") }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(t.mode, color = C.teal, fontSize = 12.sp, fontFamily = FontFamily.Monospace)
                t.floor?.takeIf { it.isNotEmpty() }?.let { Text("floor: $it", color = C.brass, fontSize = 12.sp) }
            }
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("${s.mycall} (you)", color = C.text, fontSize = 12.sp, fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.background(C.cardHi, CircleShape).padding(horizontal = 10.dp, vertical = 5.dp))
                t.members.keys.sorted().forEach { call ->
                    val snr = t.members[call]?.let { " %+d dB".format(it) } ?: ""
                    Text("$call$snr", color = C.text, fontSize = 12.sp, fontFamily = FontFamily.Monospace,
                        modifier = Modifier
                            .background(if (t.floor == call) C.brass.copy(alpha = 0.3f) else C.cardHi, CircleShape)
                            .clickable { station.passFloor(call) }
                            .padding(horizontal = 10.dp, vertical = 5.dp)
                            .semantics { contentDescription = "$call$snr. Tap to pass the floor" })
                }
            }
        }
        LazyColumn(Modifier.weight(1f).fillMaxWidth(), state = list, contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            itemsIndexed(t.lines) { _, l ->
                Row {
                    Text(Fmt.utc(l.at), color = C.dim, fontSize = 11.sp, fontFamily = FontFamily.Monospace, modifier = Modifier.width(52.dp))
                    if (l.from.isEmpty()) Text(l.text, color = C.dim, fontSize = 14.sp, fontStyle = androidx.compose.ui.text.font.FontStyle.Italic)
                    else Text(
                        androidx.compose.ui.text.buildAnnotatedString {
                            pushStyle(androidx.compose.ui.text.SpanStyle(color = if (l.from == s.mycall) C.brass else C.teal, fontWeight = FontWeight.Bold))
                            append(l.from)
                            pop()
                            append("  ")
                            append(l.text)
                        },
                        color = C.text, fontSize = 14.sp,
                    )
                }
            }
        }
        Column(Modifier.fillMaxWidth().background(C.panel).padding(horizontal = 12.dp, vertical = 6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Field(text, { text = it }, "Say something to the table", Modifier.weight(1f), caps = false)
                IconButton(
                    onClick = { if (text.isNotBlank()) { station.tableSay(text.trim()); text = "" } },
                    enabled = text.isNotBlank(),
                ) { Icon(Icons.AutoMirrored.Filled.Send, "Send to the table", tint = if (text.isNotBlank()) C.brass else C.dim) }
                HelpButton(HelpTopic.tableSay)
            }
            Text(
                if (t.sendingEtaS > 0) "On air: ${Fmt.ago(t.sendingEtaS)} left" else "Broadcast to everyone at the table",
                color = C.dim, fontSize = 11.sp, maxLines = 1,
            )
        }
    }
}

@Composable
private fun Bubble(line: ChatLine) {
    if (line.system) {
        Text(line.text, color = C.dim, fontSize = 12.sp, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth())
        return
    }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = if (line.mine) Arrangement.End else Arrangement.Start) {
        Column(horizontalAlignment = if (line.mine) Alignment.End else Alignment.Start, modifier = Modifier.widthIn(max = 300.dp)) {
            Text(
                line.text, color = if (line.mine) C.ink else C.text,
                modifier = Modifier.background(if (line.mine) C.brass else C.cardHi, RoundedCornerShape(14.dp)).padding(horizontal = 12.dp, vertical = 8.dp),
            )
            val meta = buildList {
                if (!line.mine && line.from.isNotEmpty()) add(line.from)
                add(Fmt.utc(line.at))
                if (line.mine) add(if (line.delivered) "✓" else "…")
            }.joinToString(" ")
            Text(meta, color = C.dim, fontSize = 11.sp)
        }
    }
}

// -------------------------------------------------------------------- Depot

private enum class Box4(val label: String) { Arrivals("Arrivals"), Departures("Departures"), Transit("In Transit"), Delivered("Delivered") }

@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
private fun DepotScreen(station: Station, s: EngineState) {
    var box by rememberSaveable { mutableStateOf(Box4.Arrivals) }
    var open by remember { mutableStateOf<Mail?>(null) }
    var writeTo by remember { mutableStateOf<String?>(null) }
    val mail = when (box) {
        Box4.Arrivals -> s.depot.arrivals
        Box4.Departures -> s.depot.departures
        Box4.Transit -> s.depot.transit
        Box4.Delivered -> s.depot.delivered
    }.sortedByDescending { it.created }
    androidx.compose.foundation.layout.Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                SecondaryTabRow(selectedTabIndex = box.ordinal, containerColor = C.panel, contentColor = C.brass, modifier = Modifier.weight(1f)) {
                    Box4.entries.forEach { b ->
                        Tab(selected = box == b, onClick = { box = b }, text = { Text(b.label, fontSize = 12.sp, maxLines = 1) })
                    }
                }
                HelpButton(HelpTopic.depotLists)
            }
            if (mail.isEmpty()) {
                Text(
                    when (box) {
                        Box4.Arrivals -> "No mail. Mail for you lands here."
                        Box4.Departures -> "No mail. Mail you write waits here for its station."
                        Box4.Transit -> "No mail. Mail you carry for others waits here."
                        Box4.Delivered -> "No mail. Confirmed mail is kept here."
                    },
                    color = C.dim, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(32.dp),
                )
            }
            LazyColumn(Modifier.weight(1f), contentPadding = androidx.compose.foundation.layout.PaddingValues(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                itemsIndexed(mail) { _, m ->
                    Column(
                        Modifier.fillMaxWidth().background(C.card, RoundedCornerShape(10.dp)).clickable {
                            open = m
                            if (box == Box4.Arrivals && !m.read) station.depotRead(m.id)
                        }.padding(12.dp),
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(if (box == Box4.Arrivals) m.from else "To ${m.to}", color = C.text, fontWeight = FontWeight.SemiBold)
                            m.via?.takeIf { it.isNotEmpty() }?.let { Text("  via $it", color = C.dim, fontSize = 12.sp) }
                            Spacer(Modifier.weight(1f))
                            if (box == Box4.Arrivals && !m.read) Text("NEW", color = C.ink, fontSize = 10.sp, fontWeight = FontWeight.Black,
                                modifier = Modifier.background(C.brass, CircleShape).padding(horizontal = 6.dp, vertical = 1.dp))
                            Spacer(Modifier.width(8.dp))
                            Text(Fmt.utc(m.created), color = C.dim, fontSize = 12.sp)
                        }
                        Text(m.text, color = C.dim, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
        Row(Modifier.align(Alignment.BottomEnd).padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
            HelpButton(HelpTopic.depot)
            FloatingActionButton(onClick = { writeTo = "" }, containerColor = C.brass, contentColor = C.ink) {
                Icon(Icons.Filled.Edit, "Write mail")
            }
        }
    }
    open?.let { m ->
        Dialog(onDismissRequest = { open = null }) {
            Column(Modifier.background(C.cardHi, RoundedCornerShape(14.dp)).padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text("From ${m.from} to ${m.to}", color = C.text, fontWeight = FontWeight.SemiBold)
                m.via?.takeIf { it.isNotEmpty() }?.let { Text("Via $it", color = C.dim) }
                Text(Fmt.utc(m.created), color = C.dim, fontSize = 12.sp)
                Text(m.text, color = C.text, modifier = Modifier.fillMaxWidth().background(C.card, RoundedCornerShape(8.dp)).padding(10.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (box == Box4.Arrivals) BrassButton("Reply", Modifier.weight(1f)) { writeTo = m.from; open = null }
                    if (box == Box4.Departures || box == Box4.Transit) BrassButton("Connect now", Modifier.weight(1f)) {
                        station.depotConnect(m.via ?: m.to); open = null
                    }
                    QuietButton("Delete", Modifier.weight(1f)) { station.depotDelete(m.id); open = null }
                }
            }
        }
    }
    writeTo?.let { to0 ->
        var to by remember { mutableStateOf(to0) }
        var via by remember { mutableStateOf("") }
        var text by remember { mutableStateOf("") }
        Dialog(onDismissRequest = { writeTo = null }) {
            Column(Modifier.background(C.cardHi, RoundedCornerShape(14.dp)).padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Write mail", color = C.text, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                    HelpButton(HelpTopic.depotPost)
                }
                Field(to, { to = it }, "To (callsign)", Modifier.fillMaxWidth())
                Field(via, { via = it }, "Via (optional relay)", Modifier.fillMaxWidth())
                androidx.compose.material3.OutlinedTextField(
                    value = text, onValueChange = { text = it }, placeholder = { Text("Message", color = C.dim) },
                    modifier = Modifier.fillMaxWidth().height(160.dp),
                )
                Text("${text.toByteArray().size} bytes", color = C.dim, fontSize = 12.sp)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    QuietButton("Cancel", Modifier.weight(1f)) { writeTo = null }
                    BrassButton("Post", Modifier.weight(1f), enabled = to.isNotBlank() && text.isNotEmpty()) {
                        station.depotPost(to, via.ifBlank { null }, text); writeTo = null
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------- Log

@Composable
private fun LogScreen(s: EngineState) {
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("LOG", color = C.dim, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
            HelpButton(HelpTopic.log)
        }
        LazyColumn(contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp)) {
            itemsIndexed(s.log.reversed()) { _, (t, x) ->
                Row(Modifier.padding(vertical = 4.dp)) {
                    Text(Fmt.utc(t), color = C.dim, fontSize = 12.sp, fontFamily = FontFamily.Monospace, modifier = Modifier.width(56.dp))
                    Text(x, color = C.text, fontSize = 14.sp)
                }
            }
        }
    }
}

// ----------------------------------------------------------------- Settings

@Composable
private fun SettingsScreen(station: Station, s: EngineState) {
    val cfg by station.config.collectAsState()
    val help = LocalHelp.current
    var saved by remember { mutableStateOf(false) }
    var e2eAsk by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(saved) { if (saved) { kotlinx.coroutines.delay(1500); saved = false } }
    fun str(k: String) = (cfg[k] as? JsonPrimitive)?.content ?: ""
    fun set(k: String, v: String) = station.edit { it[k] = JsonPrimitive(v) }
    val cat = cfg["cat"] as? JsonObject
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).imePadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        WCard("Help") {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Help mode", color = C.text, modifier = Modifier.weight(1f))
                Switch(help.value, { help.value = it }, colors = SwitchDefaults.colors(checkedTrackColor = C.teal))
            }
            Text(HelpTopic.helpMode.body, color = C.dim, fontSize = 12.sp)
        }
        WCard("Station", HelpTopic.station) {
            Field(str("callsign"), { set("callsign", it.uppercase()) }, "Callsign", Modifier.fillMaxWidth())
            Field(str("name"), { set("name", it) }, "Name", Modifier.fillMaxWidth(), caps = false)
            Field(str("grid"), { set("grid", it.uppercase()) }, "Grid, e.g. DM79", Modifier.fillMaxWidth())
        }
        WCard("Demo", HelpTopic.demo) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Demo station (no radio)", color = C.text, modifier = Modifier.weight(1f))
                Switch(
                    (cfg["demo"] as? JsonPrimitive)?.content == "true",
                    { on -> station.edit { it["demo"] = JsonPrimitive(on) } },
                    colors = SwitchDefaults.colors(checkedTrackColor = C.teal),
                )
            }
            Text("Takes effect when you tap Save.", color = C.dim, fontSize = 12.sp)
        }
        RadioCard(station, s, cfg)
        WCard("Modem (Mercury)", HelpTopic.modem) {
            Field(str("mercury_host"), { set("mercury_host", it) }, "Host", Modifier.fillMaxWidth(), caps = false)
            NumberField(station, "mercury_port", "Port", 1.0..65534.0, decimal = false)
        }
        WCard("While away", HelpTopic.autoAccept) {
            val on = (cfg["auto_accept_away"] as? JsonPrimitive)?.content == "true"
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Accept files by myself", color = C.text, modifier = Modifier.weight(1f))
                Switch(
                    on,
                    { v -> station.edit { it["auto_accept_away"] = JsonPrimitive(v) } },
                    colors = SwitchDefaults.colors(checkedTrackColor = C.teal),
                    modifier = Modifier.semantics { contentDescription = "Accept files by myself while away" },
                )
            }
            if (on) {
                NumberField(station, "auto_accept_max_mb", "Each up to (MB)", 0.1..200.0, decimal = true)
            }
            Text("Off by default. A bigger file waits for you. Takes effect when you tap Save.", color = C.dim, fontSize = 12.sp)
        }
        WCard("Encryption", HelpTopic.encryption) {
            val e2e = str("e2e").ifEmpty { "off" }
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                listOf("off" to "Off", "prefer" to "Prefer", "require" to "Require").forEach { (k, label) ->
                    val on = e2e == k
                    Text(
                        label, color = if (on) C.ink else C.text, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center,
                        modifier = Modifier.weight(1f)
                            .background(if (on) C.brass else C.cardHi, RoundedCornerShape(8.dp))
                            .clickable { if (e2e == "off" && k != "off") e2eAsk = k else set("e2e", k) }
                            .padding(vertical = 8.dp)
                            .semantics { contentDescription = "Encryption $label"; stateDescription = if (on) "selected" else "" },
                    )
                }
            }
            Text(
                when (e2e) {
                    "prefer" -> "Encrypt when the other station does too; otherwise plain text."
                    "require" -> "Encrypt or hang up: never plain text."
                    else -> "Links are plain text (the default)."
                },
                color = C.dim, fontSize = 12.sp, minLines = 2, maxLines = 2,
            )
            Text("⚠ $E2E_LEGAL_WARNING", color = C.red, fontSize = 12.sp)
            Text("Roundtables are never encrypted. Keys are made per link and never saved. Takes effect when you tap Save, from the next link.", color = C.dim, fontSize = 12.sp)
        }
        BrassButton(if (saved) "Saved" else "Save") {
            // The engine restarts the station itself when a connection
            // setting (demo, modem, radio, callsign) changed.
            if (station.saveConfig()) {
                saved = true
            }
        }
        WCard("About") {
            Text("Modem: ${s.modemVersion.ifEmpty { "—" }}", color = C.dim, fontSize = 13.sp)
            Text("Whistlestop is free software (GPL-3.0). It includes the Mercury modem by Rhizomatica.", color = C.dim, fontSize = 12.sp)
        }
    }
    e2eAsk?.let { want ->
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { e2eAsk = null },
            title = { Text("Turn on encryption?") },
            text = { Text(E2E_LEGAL_WARNING) },
            confirmButton = { TextButton(onClick = { set("e2e", want); e2eAsk = null }) { Text("Turn on", color = C.red) } },
            dismissButton = { TextButton(onClick = { e2eAsk = null }) { Text("Cancel") } },
        )
    }
}

/** The engine's `e2e::LEGAL_WARNING`, word for word. */
private const val E2E_LEGAL_WARNING = "Encryption is NOT legal on amateur radio bands in the US (47 CFR 97.113(a)(4)) and in most other countries: amateur transmissions may not be encoded to obscure their meaning. Use it only where it is lawful (for example on non-amateur frequencies you are licensed for, or over a wired or test link)."

// ------------------------------------------------------------------- Radio

/** How the phone reaches the radio's CAT. */
private enum class CatLink(val label: String) {
    Usb("USB cable"),
    Bridge("Serial bridge (host:port)"),
    Rigctld("rigctld (host:port)"),
    None("None (VOX)"),
}

/** The radio's protocol (`cat.family`). */
private val families = listOf(
    "Auto" to "Automatic (detect)",
    "Icom" to "Icom CI-V (and Xiegu)",
    "Kenwood" to "Kenwood (QRP Labs, truSDX, Flex)",
    "Elecraft" to "Elecraft K3 / K4 / KX",
    "Yaesu" to "Yaesu (FT-991, FT-891, FT-710, FTDX…)",
    "Yaesu817" to "Yaesu FT-817 / 818 / 857 / 897",
)

@Composable
private fun RadioCard(station: Station, s: EngineState, cfg: JsonObject) {
    val cat = cfg["cat"] as? JsonObject
    fun catStr(k: String) = (cat?.get(k) as? JsonPrimitive)?.content ?: ""
    val mode = (cfg["rig_mode"] as? JsonPrimitive)?.content ?: "External"
    val rigctld = (cfg["rigctld"] as? JsonPrimitive)?.content ?: ""
    val port = catStr("port")
    val link = when {
        mode == "Direct" && (port.isBlank() || port.startsWith("usb:")) -> CatLink.Usb
        mode == "Direct" -> CatLink.Bridge
        rigctld == "none" -> CatLink.None
        // A phone runs no rigctld of its own: the desktop default means "not set up",
        // and the app uses a radio on USB when one is plugged in.
        rigctld in listOf("", "127.0.0.1:4532", "localhost:4532") -> CatLink.Usb
        else -> CatLink.Rigctld
    }
    fun setCat(k: String, v: JsonPrimitive) = station.edit { m ->
        val c = (m["cat"] as? JsonObject)?.toMutableMap() ?: mutableMapOf()
        c[k] = v
        m["cat"] = JsonObject(c)
    }
    fun choose(l: CatLink) = station.edit { m ->
        val c = (m["cat"] as? JsonObject)?.toMutableMap() ?: mutableMapOf()
        when (l) {
            CatLink.Usb -> { m["rig_mode"] = JsonPrimitive("Direct"); c["port"] = JsonPrimitive("") }
            CatLink.Bridge -> { m["rig_mode"] = JsonPrimitive("Direct"); if (port.isBlank() || port.startsWith("usb:")) c["port"] = JsonPrimitive("") }
            CatLink.Rigctld -> { m["rig_mode"] = JsonPrimitive("External"); if (rigctld == "none" || rigctld.startsWith("127.") || rigctld.startsWith("localhost")) m["rigctld"] = JsonPrimitive("") }
            CatLink.None -> { m["rig_mode"] = JsonPrimitive("External"); m["rigctld"] = JsonPrimitive("none") }
        }
        m["cat"] = JsonObject(c)
    }
    // What's on USB now, re-read when the radio's state changes.
    val onUsb = remember(s.rigOk, s.rigDesc, link) { UsbCat.attached() }
    WCard("Radio", HelpTopic.radio) {
        CatLink.entries.forEach { l ->
            Row(
                Modifier.fillMaxWidth().clickable { choose(l) }.padding(vertical = 2.dp)
                    .semantics { contentDescription = l.label; stateDescription = if (l == link) "selected" else "" },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RadioButton(l == link, { choose(l) }, colors = RadioButtonDefaults.colors(selectedColor = C.teal))
                Text(l.label, color = C.text)
            }
        }
        when (link) {
            CatLink.Usb -> Text(
                if (onUsb.isEmpty()) "Nothing on USB. Plug the radio into the phone (USB-C, or an OTG adapter)."
                else "On USB: " + onUsb.joinToString(", ") { it.second },
                color = C.dim, fontSize = 12.sp, maxLines = 2, overflow = TextOverflow.Ellipsis,
            )
            CatLink.Bridge -> Field(port, { setCat("port", JsonPrimitive(it.trim())) }, "Bridge host:port", Modifier.fillMaxWidth(), caps = false)
            CatLink.Rigctld -> Field(rigctld, { v -> station.edit { it["rigctld"] = JsonPrimitive(v.trim()) } }, "rigctld host:port", Modifier.fillMaxWidth(), caps = false)
            CatLink.None -> Text("Whistlestop can't read or tune the dial; key the radio with VOX.", color = C.dim, fontSize = 12.sp)
        }
        if (link == CatLink.Usb || link == CatLink.Bridge) {
            var open by remember { mutableStateOf(false) }
            val fam = catStr("family").ifEmpty { "Auto" }
            Box {
                Row(Modifier.fillMaxWidth().clickable { open = true }.padding(vertical = 6.dp)) {
                    Text("Protocol", color = C.dim, modifier = Modifier.width(80.dp))
                    Text(families.firstOrNull { it.first == fam }?.second ?: fam, color = C.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                DropdownMenu(open, { open = false }) {
                    families.forEach { (k, label) ->
                        DropdownMenuItem({ Text(label) }, { setCat("family", JsonPrimitive(k)); open = false })
                    }
                }
            }
        }
        // Fixed two lines, so the card doesn't jump as the radio comes and goes.
        Text(
            when {
                s.demo -> "Demo station: the radio is simulated."
                s.rigOk -> "Connected: ${s.rigDesc}"
                else -> "Not connected (see Log)."
            },
            color = if (s.rigOk || s.demo) C.teal else C.dim, fontSize = 12.sp,
            minLines = 2, maxLines = 2, overflow = TextOverflow.Ellipsis,
        )
        if (station.modemNeedsReopen()) {
            Text(
                "The modem started before this radio setting. Close Whistlestop (swipe it away) and open it again so Mercury keys the radio this way.",
                color = C.brass, fontSize = 12.sp,
            )
        }
    }
}

/**
 * A number setting typed as text. What you type stays as typed ("2." on the
 * way to "2.5", or empty while you retype); every keystroke that makes a
 * number in `range` goes into the settings at once. Out of range or not a
 * number: shown in red and not taken.
 */
@Composable
private fun NumberField(station: Station, key: String, label: String, range: ClosedFloatingPointRange<Double>, decimal: Boolean) {
    val cfg by station.config.collectAsState()
    val saved = (cfg[key] as? JsonPrimitive)?.content?.toDoubleOrNull()
    fun show(v: Double?) = when {
        v == null -> ""
        decimal && v != Math.rint(v) -> v.toString()
        else -> v.toLong().toString()
    }
    var text by remember { mutableStateOf(show(saved)) }
    // Reloaded (after Save, or a revert): show the saved value, unless it's
    // what's typed already.
    LaunchedEffect(saved) { if (kHz(text) != saved) text = show(saved) }
    val ok = kHz(text)?.let { it in range } == true
    Column {
        Field(text, { t ->
            text = t
            kHz(t)?.takeIf { it in range }?.let { v ->
                station.edit { it[key] = if (decimal) JsonPrimitive(v) else JsonPrimitive(v.toLong()) }
            }
        }, label, Modifier.fillMaxWidth(), caps = false, number = true)
        if (!ok && text.isNotEmpty()) {
            Text("Not taken: enter ${show(range.start)} to ${show(range.endInclusive)}", color = C.red, fontSize = 12.sp)
        }
    }
}
