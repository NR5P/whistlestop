package com.nr5p.whistlestop

import android.content.Context
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.floatOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.put
import java.io.File

/** The Rust engine's native calls (mobile/core/src/lib.rs, `android` module). */
object Core {
    init {
        System.loadLibrary("whistlestop_mobile")
    }

    @JvmStatic external fun start(dir: String): Int
    @JvmStatic external fun changes(): Long
    @JvmStatic external fun state(): String
    @JvmStatic external fun rows(): String
    @JvmStatic external fun send(json: String): Int
    @JvmStatic external fun config(): String
    @JvmStatic external fun setConfig(json: String): Int
    @JvmStatic external fun stop()
    /** 1 when Mercury runs inside the app. */
    @JvmStatic external fun mercuryEmbedded(): Int
    /** The phone's sound devices for Mercury: "id\tin|out\tname" per line. */
    @JvmStatic external fun setAudioDevices(list: String)
    /** The radio's sound card for Mercury ("dev:<id>", "" = the default route). */
    @JvmStatic external fun setRadioAudio(input: String, output: String)
    /** UsbCat is ready: the engine finds and opens USB radios through it. */
    @JvmStatic external fun usbInit()
    /** 1 when Mercury keys the radio differently from the settings (reopen the app). */
    @JvmStatic external fun mercuryRestartNeeded(): Int
}

/**
 * The engine as Compose sees it: polls the change counter ten times a second
 * and re-reads the state only when it moved.
 */
class Station(private val scope: CoroutineScope) {
    private val _state = MutableStateFlow(EngineState())
    val state: StateFlow<EngineState> = _state

    private val _rows = MutableStateFlow<List<FloatArray>>(emptyList())
    val rows: StateFlow<List<FloatArray>> = _rows

    /** Settings, every key kept, so saving never drops one this app doesn't show. */
    private val _config = MutableStateFlow(JsonObject(emptyMap()))
    val config: StateFlow<JsonObject> = _config

    var startError: String? = null
        private set

    private var seen = Long.MIN_VALUE
    private var job: Job? = null

    fun start(context: Context, uiTest: Boolean = false) {
        if (job != null) return
        var dir = context.filesDir
        if (uiTest) {
            // A fresh demo station in a folder of its own.
            dir = File(context.cacheDir, "uitest").apply {
                deleteRecursively()
                mkdirs()
                File(this, "whistlestop.toml").writeText(
                    "callsign = \"N0PHN\"\ngrid = \"DM79\"\ndemo = true\ndemo_real_mercury = false\n"
                )
            }
        }
        Core.setAudioDevices(audioDevices(context))
        val (inDev, outDev) = radioSoundCard(context)
        Core.setRadioAudio(inDev, outDev)
        if (!uiTest) {
            UsbCat.init(context)
            // Once Android allows the radio's USB, connect to it.
            UsbCat.onPermission = { granted -> if (granted) scope.launch { saveAndRestart() } }
            // A radio plugged in while the app runs: connect to it.
            UsbCat.onAttached = { scope.launch { delay(1000); radioArrived() } }
        }
        if (Core.start(dir.absolutePath) != 0) {
            startError = "The engine couldn't start."
            return
        }
        loadConfig()
        job = scope.launch(Dispatchers.Default) {
            while (isActive) {
                poll()
                delay(100)
            }
        }
    }

    private fun poll() {
        val newRows = runCatching {
            Json.parseToJsonElement(Core.rows()).jsonArray.map { r ->
                val a = r.jsonArray
                FloatArray(a.size) { (a[it] as? JsonPrimitive)?.floatOrNull ?: -120f }
            }
        }.getOrDefault(emptyList())
        if (newRows.isNotEmpty()) {
            _rows.value = (_rows.value + newRows).takeLast(MAX_ROWS)
        }
        val n = Core.changes()
        if (n == seen) return
        seen = n
        runCatching { EngineState.parse(Core.state()) }
            .onSuccess { _state.value = it }
            .onFailure { Log.w("whistlestop", "state didn't decode", it) }
    }

    // ---------------------------------------------------------------- commands

    /** One engine command: a bare name (`"Stop"`) or `{name: value}`. */
    fun send(name: String, value: JsonElement? = null) {
        val json = if (value == null) JsonPrimitive(name).toString()
        else JsonObject(mapOf(name to value)).toString()
        if (Core.send(json) != 0) Log.w("whistlestop", "the engine refused $json")
        seen = Long.MIN_VALUE
    }

    fun setFreq(khz: Double) = send("SetFreq", JsonPrimitive(khz * 1000))
    fun connect(call: String) = send("Connect", JsonPrimitive(call.trim().uppercase()))
    fun disconnect() = send("Disconnect")
    fun chat(text: String) = send("Chat", JsonPrimitive(text))
    fun searchCurrentBand() = send("StartSearch", JsonPrimitive("CurrentBand"))
    fun searchAllBands() = send("StartSearch", JsonPrimitive("AllBands"))
    fun searchBand(band: String) = send("StartSearch", buildJsonObject { put("Band", band) })
    fun stopSearch() = send("StopSearch")
    fun callCq() = send("CallCq", buildJsonObject { put("table", JsonNull) })
    fun stopCq() = send("StopCq")
    fun setAway(on: Boolean) = send("SetAway", JsonPrimitive(on))
    fun qsy(khz: Double) = send("Qsy", JsonPrimitive(khz * 1000))
    fun depotPost(to: String, via: String?, text: String) = send("DepotPost", buildJsonObject {
        put("to", to.trim().uppercase())
        put("via", via?.trim()?.uppercase()?.takeIf { it.isNotEmpty() }?.let { JsonPrimitive(it) } ?: JsonNull)
        put("text", text)
    })
    // Mail ids are 64-bit and unsigned: they travel as the exact digits.
    fun depotRead(id: String) = sendRaw("""{"DepotRead":$id}""")
    fun depotDelete(id: String) = sendRaw("""{"DepotDelete":$id}""")
    fun depotConnect(call: String) = send("DepotConnect", JsonPrimitive(call.trim().uppercase()))

    private fun sendRaw(json: String) {
        if (Core.send(json) != 0) Log.w("whistlestop", "the engine refused $json")
        seen = Long.MIN_VALUE
    }

    // ---------------------------------------------------------------- settings

    fun loadConfig() {
        runCatching { Json.parseToJsonElement(Core.config()) as JsonObject }
            .onSuccess { _config.value = it }
    }

    fun string(key: String) = (_config.value[key] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: ""
    fun bool(key: String) = (_config.value[key] as? JsonPrimitive)?.content == "true"

    /** Change settings in memory (Save applies them). */
    fun edit(f: (MutableMap<String, JsonElement>) -> Unit) {
        val m = _config.value.toMutableMap()
        f(m)
        _config.value = JsonObject(m)
    }

    /** Do the edited settings reach the radio differently from the saved ones? */
    fun radioChanged(): Boolean {
        val saved = runCatching { Json.parseToJsonElement(Core.config()) as JsonObject }.getOrNull() ?: return true
        return listOf("rig_mode", "rigctld", "cat").any { saved[it] != _config.value[it] }
    }

    /** Save and apply; false if the engine refused them. */
    fun saveConfig(): Boolean {
        val ok = Core.setConfig(_config.value.toString()) == 0
        loadConfig()
        return ok
    }

    /** Restart the station with the saved settings (e.g. demo on/off). */
    fun restart() {
        send("Stop")
        send("Start", _config.value)
    }

    /** A radio was plugged in, or USB access allowed: reconnect if the radio isn't up. */
    fun radioArrived() {
        if (!_state.value.rigOk && !_state.value.demo) saveAndRestart()
    }

    private fun saveAndRestart() {
        // Saving lets the engine adopt a USB radio when none was set up.
        if (saveConfig()) restart()
    }

    /** Mercury keys the radio differently from the settings: reopen the app. */
    fun modemNeedsReopen() = Core.mercuryRestartNeeded() != 0

    companion object {
        const val MAX_ROWS = 160
    }
}

/** Android's sound devices, as Mercury wants them ("id\tin|out\tname"). */
private fun audioDevices(context: Context): String {
    val am = context.getSystemService(Context.AUDIO_SERVICE) as android.media.AudioManager
    val ins = am.getDevices(android.media.AudioManager.GET_DEVICES_INPUTS).map { "${it.id}\tin\t${it.productName} (${it.type})" }
    val outs = am.getDevices(android.media.AudioManager.GET_DEVICES_OUTPUTS).map { "${it.id}\tout\t${it.productName} (${it.type})" }
    return (ins + outs).joinToString("\n")
}

/**
 * The radio's sound card: a USB audio device (the radio's own USB audio, a
 * SignaLink, a DigiRig) when one is plugged in, as Mercury's "dev:<id>"; ""
 * = the phone's default route. A device that both records and plays is
 * preferred, so input and output are the same card.
 */
private fun radioSoundCard(context: Context): Pair<String, String> {
    val am = context.getSystemService(Context.AUDIO_SERVICE) as android.media.AudioManager
    val usb = setOf(android.media.AudioDeviceInfo.TYPE_USB_DEVICE, android.media.AudioDeviceInfo.TYPE_USB_HEADSET)
    val ins = am.getDevices(android.media.AudioManager.GET_DEVICES_INPUTS).filter { it.type in usb }
    val outs = am.getDevices(android.media.AudioManager.GET_DEVICES_OUTPUTS).filter { it.type in usb }
    val inDev = ins.firstOrNull { i -> outs.any { it.productName == i.productName } } ?: ins.firstOrNull()
    val outDev = outs.firstOrNull { it.productName == inDev?.productName } ?: outs.firstOrNull()
    return (inDev?.let { "dev:${it.id}" } ?: "") to (outDev?.let { "dev:${it.id}" } ?: "")
}

/** A JSON array of floats, or null. */
internal fun JsonElement?.floats(): List<Float> =
    (this as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.floatOrNull } ?: emptyList()
