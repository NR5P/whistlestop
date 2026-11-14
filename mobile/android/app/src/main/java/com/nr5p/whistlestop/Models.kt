package com.nr5p.whistlestop

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull

// The engine's state (Rust `engine::State`), read leniently: anything
// missing gets a default, and fields this app doesn't show are ignored.

private fun JsonObject.o(k: String) = this[k] as? JsonObject
private fun JsonObject.a(k: String) = (this[k] as? JsonArray) ?: JsonArray(emptyList())
private fun JsonObject.s(k: String) = (this[k] as? JsonPrimitive)?.takeIf { it.isString }?.content
private fun JsonObject.b(k: String) = (this[k] as? JsonPrimitive)?.booleanOrNull ?: false
private fun JsonObject.d(k: String) = (this[k] as? JsonPrimitive)?.doubleOrNull
private fun JsonObject.i(k: String) = (this[k] as? JsonPrimitive)?.intOrNull ?: 0
private fun JsonObject.l(k: String) = (this[k] as? JsonPrimitive)?.longOrNull ?: 0L
/** The exact digits of a number (64-bit unsigned ids don't fit a Long). */
private fun JsonObject.digits(k: String) = (this[k] as? JsonPrimitive)?.content ?: "0"
private fun JsonElement.obj() = this as? JsonObject ?: JsonObject(emptyMap())

data class EngineState(
    /** Parsed from the engine (false: the placeholder before the first poll). */
    val loaded: Boolean = false,
    val mycall: String = "",
    val demo: Boolean = false,
    val rigOk: Boolean = false,
    val mercuryOk: Boolean = false,
    val audioOk: Boolean = false,
    val freqHz: Double = 0.0,
    val mode: String = "",
    val tx: Boolean = false,
    val rigDesc: String = "",
    val link: Link? = null,
    val chat: List<ChatLine> = emptyList(),
    val heard: List<Heard> = emptyList(),
    val whistle: Whistle? = null,
    val search: Search = Search(),
    val cq: Cq = Cq(),
    val log: List<Pair<Long, String>> = emptyList(),
    val modemVersion: String = "",
    val depot: Depot = Depot(),
    val away: Boolean = false,
    val qsyNote: String? = null,
    val table: TableState? = null,
    /** A Roundtable we'll join as soon as Mercury is free. */
    val tablePending: String? = null,
    /** A call going out (Connect, or an automatic one), until it links. */
    val calling: String? = null,
) {
    companion object {
        fun parse(json: String): EngineState {
            val s = Json.parseToJsonElement(json).obj()
            val rig = s.o("rig")
            return EngineState(
                loaded = true,
                mycall = s.s("mycall") ?: "",
                demo = s.b("demo"),
                rigOk = s.b("rig_ok"),
                mercuryOk = s.b("mercury_ok"),
                audioOk = s.b("audio_ok"),
                freqHz = rig?.d("freq_hz") ?: 0.0,
                mode = rig?.s("mode") ?: "",
                tx = s.b("tx"),
                rigDesc = s.s("rig_desc") ?: "",
                link = s.o("link")?.let {
                    Link(
                        peer = it.s("peer") ?: "",
                        peerName = it.s("peer_name") ?: "",
                        peerGrid = it.s("peer_grid") ?: "",
                        mode = it.s("mode") ?: "",
                        bps = it.d("bps") ?: 0.0,
                        since = it.d("since") ?: 0.0,
                        e2e = E2e.parse(it["e2e"]),
                    )
                },
                chat = s.a("chat").map { it.obj() }.map {
                    ChatLine(it.l("at"), it.s("who") ?: "", it.s("from") ?: "", it.s("text") ?: "", it.b("delivered"), it.b("unconfirmed"))
                },
                heard = s.a("heard").map { it.obj() }.map {
                    Heard(it.l("at"), it.s("call") ?: "", it.d("dial_hz") ?: 0.0, it.s("what") ?: "", it.s("table"))
                },
                whistle = s.o("whistle")?.let {
                    Whistle(it.d("at") ?: 0.0, it.d("dial_hz") ?: 0.0, it.d("offset_hz") ?: 0.0, it.d("score") ?: 0.0)
                },
                search = s.o("search")?.let {
                    Search(it.b("active"), it.i("index"), it.i("total"), it.d("channel_hz") ?: 0.0, it.s("note") ?: "")
                } ?: Search(),
                cq = s.o("cq")?.let { Cq(it.b("active"), it.i("cycle"), it.i("cycles"), it.s("phase") ?: "") } ?: Cq(),
                log = s.a("log").mapNotNull { e ->
                    val a = e as? JsonArray ?: return@mapNotNull null
                    val t = (a.getOrNull(0) as? JsonPrimitive)?.longOrNull ?: 0L
                    val x = (a.getOrNull(1) as? JsonPrimitive)?.content ?: ""
                    t to x
                },
                modemVersion = s.s("modem_version") ?: "",
                depot = s.o("depot")?.let { d ->
                    fun box(k: String) = d.a(k).map { it.obj() }.map {
                        Mail(
                            id = it.digits("id"), from = it.s("from") ?: "", to = it.s("to") ?: "",
                            created = it.l("created"), text = it.s("text") ?: "", read = it.b("read"),
                            via = it.s("via"),
                        )
                    }
                    Depot(box("arrivals"), box("departures"), box("transit"), box("delivered"))
                } ?: Depot(),
                away = s.b("away"),
                qsyNote = s.s("qsy_note"),
                table = s.o("table")?.let { t ->
                    TableState(
                        name = t.s("name") ?: "",
                        host = t.b("host"),
                        mode = t.s("mode") ?: "",
                        members = (t.o("members") ?: JsonObject(emptyMap())).mapValues { (_, m) ->
                            (m as? JsonObject)?.let { (it["snr_of_host"] as? JsonPrimitive)?.intOrNull }
                        },
                        floor = t.s("floor"),
                        lines = t.a("lines").mapNotNull { e ->
                            val a = e as? JsonArray ?: return@mapNotNull null
                            TableLine(
                                (a.getOrNull(0) as? JsonPrimitive)?.longOrNull ?: 0L,
                                (a.getOrNull(1) as? JsonPrimitive)?.content ?: "",
                                (a.getOrNull(2) as? JsonPrimitive)?.content ?: "",
                            )
                        },
                        sendingEtaS = t.d("sending_eta_s") ?: 0.0,
                    )
                },
                tablePending = s.s("table_pending"),
                calling = s.s("calling"),
            )
        }
    }
}

data class Link(val peer: String, val peerName: String, val peerGrid: String, val mode: String, val bps: Double, val since: Double, val e2e: E2e = E2e.Off)

/** Encryption on the link (Rust `engine::E2eView`). */
sealed class E2e {
    data object Off : E2e()
    data object Negotiating : E2e()
    data class Encrypted(val sas: String) : E2e()
    data class Plain(val why: String) : E2e()

    companion object {
        /** `"Off"`, `"Negotiating"`, `{"Encrypted":{"sas":…}}` or `{"Plain":{"why":…}}`. */
        fun parse(e: JsonElement?): E2e {
            (e as? JsonPrimitive)?.let { return if (it.content == "Negotiating") Negotiating else Off }
            val o = e as? JsonObject ?: return Off
            o.o("Encrypted")?.let { return Encrypted(it.s("sas") ?: "") }
            o.o("Plain")?.let { return Plain(it.s("why") ?: "") }
            return Off
        }
    }
}

/** The Roundtable we're at (Rust `engine::TableState`). */
data class TableState(
    val name: String,
    val host: Boolean,
    /** Mercury's broadcast mode, e.g. "DATAC3". */
    val mode: String,
    /** Callsign → how well they hear the host (dB). */
    val members: Map<String, Int?>,
    /** Who holds the floor (the talking stick). */
    val floor: String?,
    val lines: List<TableLine>,
    /** Air time still to go for frames being sent. */
    val sendingEtaS: Double,
)
/** `from` is empty for the table's own notices. */
data class TableLine(val at: Long, val from: String, val text: String)
data class ChatLine(val at: Long, val who: String, val from: String, val text: String, val delivered: Boolean, val unconfirmed: Boolean = false) {
    val mine get() = who == "Me"
    val system get() = who == "System"
}
data class Heard(val at: Long, val call: String, val dialHz: Double, val what: String, val table: String? = null)
data class Whistle(val agoS: Double, val dialHz: Double, val offsetHz: Double, val score: Double)
data class Search(val active: Boolean = false, val index: Int = 0, val total: Int = 0, val channelHz: Double = 0.0, val note: String = "")
data class Cq(val active: Boolean = false, val cycle: Int = 0, val cycles: Int = 0, val phase: String = "")
data class Mail(val id: String, val from: String, val to: String, val created: Long, val text: String, val read: Boolean, val via: String?)
data class Depot(
    val arrivals: List<Mail> = emptyList(),
    val departures: List<Mail> = emptyList(),
    val transit: List<Mail> = emptyList(),
    val delivered: List<Mail> = emptyList(),
)

object Fmt {
    fun dial(hz: Double) = if (hz <= 0) "—" else "%.2f".format(hz / 1000)
    fun mhz(hz: Double) = if (hz <= 0) "—" else "%.3f".format(hz / 1_000_000)
    fun utc(t: Long): String {
        val s = t % 86_400
        return "%02d:%02dZ".format(s / 3600, (s % 3600) / 60)
    }
    fun ago(secs: Double): String {
        val s = secs.toInt()
        return when {
            s < 60 -> "$s s"
            s < 3600 -> "${s / 60} min"
            else -> "${s / 3600} h ${s % 3600 / 60} min"
        }
    }
}
