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

@Composable
private fun App(station: Station) {
    val s by station.state.collectAsState()
    var tab by rememberSaveable { mutableStateOf(Tab.Operate) }
    var lastPeer by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(s.link?.peer) {
        if (s.link?.peer != null && s.link?.peer != lastPeer) tab = Tab.Chat
        lastPeer = s.link?.peer
    }
    Column(Modifier.fillMaxSize().background(C.bg).systemBarsPadding()) {
        StationHeader(station, s)
        Box(Modifier.weight(1f)) {
            when (tab) {
                Tab.Operate -> OperateScreen(station, s)
                Tab.Chat -> ChatScreen(station, s)
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

private val BANDS = listOf("160m", "80m", "60m", "40m", "30m", "20m", "17m", "15m", "12m", "10m")

@Composable
private fun OperateScreen(station: Station, s: EngineState) {
    val rows by station.rows.collectAsState()
    val cfg by station.config.collectAsState()
    var khz by rememberSaveable { mutableStateOf("") }
    var call by rememberSaveable { mutableStateOf("") }
    var band by rememberSaveable { mutableStateOf("20m") }
    var bandMenu by remember { mutableStateOf(false) }
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
                QuietButton("Tune", Modifier.width(96.dp), enabled = khz.toDoubleOrNull() != null) {
                    khz.toDoubleOrNull()?.let { station.setFreq(it); khz = "" }
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
                    color = C.text, minLines = 2, maxLines = 2,
                )
                QuietButton("Stop search") { station.stopSearch() }
            } else {
                BrassButton("Search current band") { station.searchCurrentBand() }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.weight(1f)) {
                        QuietButton("$band ▾") { bandMenu = true }
                        DropdownMenu(expanded = bandMenu, onDismissRequest = { bandMenu = false }) {
                            BANDS.forEach { b -> DropdownMenuItem(text = { Text(b) }, onClick = { band = b; bandMenu = false }) }
                        }
                    }
                    QuietButton("Search $band", Modifier.weight(1f)) { station.searchBand(band) }
                }
                QuietButton("Search all bands") { station.searchAllBands() }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Answer automatically", color = C.text, modifier = Modifier.weight(1f))
                HelpButton(HelpTopic.autoAnswer)
                Switch(
                    checked = (cfg["auto_answer"] as? JsonPrimitive)?.content == "true",
                    onCheckedChange = { on -> station.edit { it["auto_answer"] = JsonPrimitive(on) }; station.saveConfig() },
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
                Text("score ${"%.1f".format(w.score)} · ${Fmt.ago(w.agoS)} ago", color = C.dim, fontSize = 12.sp)
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
                    BrassButton("Answer", Modifier.width(110.dp), enabled = s.link == null) {
                        station.setFreq(h.dialHz / 1000)
                        station.connect(h.call)
                    }
                }
            }
        }
        WCard("Call CQ", HelpTopic.cq) {
            if (s.cq.active) {
                Text("Cycle ${s.cq.cycle} of ${s.cq.cycles} · ${s.cq.phase}", color = C.text, maxLines = 1, minLines = 1)
                BrassButton("Stop CQ", color = C.red) { station.stopCq() }
            } else {
                BrassButton("Whistle and call CQ", enabled = s.link == null && s.mercuryOk) { station.callCq() }
            }
        }
        WCard("Connect", HelpTopic.connect) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Field(call, { call = it }, "Callsign", Modifier.weight(1f))
                QuietButton("Connect", Modifier.width(120.dp), enabled = call.isNotBlank() && s.link == null) {
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
            QuietButton("Ask", Modifier.width(80.dp), enabled = qsy.toDoubleOrNull() != null) {
                qsy.toDoubleOrNull()?.let { station.qsy(it); qsy = "" }
            }
            HelpButton(HelpTopic.qsy)
        }
        if (!qsyNote.isNullOrEmpty()) Text(qsyNote, color = C.brass, fontSize = 12.sp)
    }
}

// --------------------------------------------------------------------- Chat

@Composable
private fun ChatScreen(station: Station, s: EngineState) {
    var text by rememberSaveable { mutableStateOf("") }
    val list = rememberLazyListState()
    LaunchedEffect(s.chat.size) { if (s.chat.isNotEmpty()) list.animateScrollToItem(s.chat.size - 1) }
    Column(Modifier.fillMaxSize().imePadding()) {
        Row(Modifier.fillMaxWidth().background(C.panel).padding(horizontal = 16.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(8.dp).background(if (s.link != null) C.teal else C.dim, CircleShape))
            Spacer(Modifier.width(8.dp))
            Text(s.link?.let { "Linked with ${it.peer}" } ?: "Not linked", color = if (s.link != null) C.text else C.dim, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
            HelpButton(HelpTopic.chat)
            if (s.link != null) TextButton(onClick = { station.disconnect() }) { Text("Disconnect") }
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
        WCard("Radio", HelpTopic.radio) {
            Field((cat?.get("port") as? JsonPrimitive)?.content ?: "", { v ->
                station.edit { m ->
                    val c = (m["cat"] as? JsonObject)?.toMutableMap() ?: mutableMapOf()
                    c["port"] = JsonPrimitive(v)
                    m["cat"] = JsonObject(c)
                }
            }, "CAT bridge host:port", Modifier.fillMaxWidth(), caps = false)
        }
        WCard("Modem (Mercury)", HelpTopic.modem) {
            Field(str("mercury_host"), { set("mercury_host", it) }, "Host", Modifier.fillMaxWidth(), caps = false)
            Field(str("mercury_port"), { v -> v.toIntOrNull()?.let { n -> station.edit { it["mercury_port"] = JsonPrimitive(n) } } },
                "Port", Modifier.fillMaxWidth(), caps = false, number = true)
        }
        BrassButton(if (saved) "Saved" else "Save") {
            val demoChanged = ((cfg["demo"] as? JsonPrimitive)?.content == "true") != s.demo
            if (station.saveConfig()) {
                if (demoChanged) station.restart()
                saved = true
            }
        }
        WCard("About") {
            Text("Modem: ${s.modemVersion.ifEmpty { "—" }}", color = C.dim, fontSize = 13.sp)
            Text("Whistlestop is free software (GPL-3.0). It includes the Mercury modem by Rhizomatica.", color = C.dim, fontSize = 12.sp)
        }
    }
}
