package com.nr5p.whistlestop

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.util.Log
import androidx.core.content.ContextCompat
import com.hoho.android.usbserial.driver.CdcAcmSerialDriver
import com.hoho.android.usbserial.driver.Ch34xSerialDriver
import com.hoho.android.usbserial.driver.Cp21xxSerialDriver
import com.hoho.android.usbserial.driver.FtdiSerialDriver
import com.hoho.android.usbserial.driver.ProlificSerialDriver
import com.hoho.android.usbserial.driver.UsbSerialDriver
import com.hoho.android.usbserial.driver.UsbSerialPort
import com.hoho.android.usbserial.driver.UsbSerialProber
import com.hoho.android.usbserial.util.SerialInputOutputManager
import java.io.ByteArrayOutputStream
import java.util.concurrent.atomic.AtomicInteger

/**
 * Radios on USB for the engine's direct CAT. Android apps can't open
 * /dev/ttyUSB0, so this opens the radio's USB serial chip (Silicon Labs
 * CP210x as in Icom and Kenwood radios, the CP2105 in Yaesus, FTDI, CH340,
 * Prolific, and USB CDC radios like the IC-705) through the USB host API,
 * and the Rust CAT drivers read and write through these calls (JNI,
 * mobile/core/src/lib.rs `usb`). One radio is open at a time.
 *
 * Ports are named "usb:VVVV:PPPP" (vendor and product id, hex).
 */
object UsbCat {
    private const val TAG = "whistlestop"
    private const val ACTION_PERMISSION = "com.nr5p.whistlestop.USB_PERMISSION"

    private var app: Context? = null
    private val usb get() = app?.getSystemService(Context.USB_SERVICE) as? UsbManager

    /** Called when the operator allows (or refuses) USB access to a radio. */
    var onPermission: ((granted: Boolean) -> Unit)? = null

    /** Called when a USB device is plugged in. */
    var onAttached: (() -> Unit)? = null

    private var lastError = ""
    private val handles = AtomicInteger(0)

    private class Open(val handle: Int, val port: UsbSerialPort, val io: SerialInputOutputManager) {
        val lock = Object()
        val buf = ByteArrayOutputStream()
        @Volatile var dead = false
    }

    @Volatile private var open: Open? = null

    private val permissionReceiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            when (i.action) {
                ACTION_PERMISSION -> onPermission?.invoke(i.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false))
                UsbManager.ACTION_USB_DEVICE_ATTACHED -> onAttached?.invoke()
            }
        }
    }

    /** Once, before the engine starts. */
    fun init(context: Context) {
        if (app != null) return
        app = context.applicationContext
        ContextCompat.registerReceiver(
            context.applicationContext, permissionReceiver, IntentFilter(ACTION_PERMISSION).apply { addAction(UsbManager.ACTION_USB_DEVICE_ATTACHED) },
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        Core.usbInit()
    }

    /**
     * Every USB serial chip attached: the library's known chips first, then
     * newer chips from the same makers by vendor id (a CP2102N or CH9102 in
     * a new radio or interface), then any USB CDC device (IC-705, IC-905,
     * QMX, truSDX, Xiegu X6100, many SDR transceivers).
     */
    private fun drivers(): List<UsbSerialDriver> {
        val m = usb ?: return emptyList()
        val known = UsbSerialProber.getDefaultProber().findAllDrivers(m)
        val more = m.deviceList.values
            .filter { d -> known.none { it.device.deviceId == d.deviceId } }
            .mapNotNull { d ->
                when (d.vendorId) {
                    0x10C4 -> Cp21xxSerialDriver(d)      // Silicon Labs: Icom, Kenwood, Yaesu
                    0x0403 -> FtdiSerialDriver(d)        // FTDI: Elecraft, cables, Flex
                    0x1A86 -> Ch34xSerialDriver(d)       // WCH CH340/CH9102: Xiegu, QRP rigs
                    0x067B -> ProlificSerialDriver(d)    // Prolific: older cables
                    else -> if (isCdc(d)) CdcAcmSerialDriver(d) else null
                }
            }
        return known + more
    }

    private fun isCdc(d: UsbDevice) =
        (0 until d.interfaceCount).any { d.getInterface(it).interfaceClass == UsbConstants.USB_CLASS_COMM }

    private fun id(d: UsbDevice) = "usb:%04x:%04x".format(d.vendorId, d.productId)

    /** The radio for a port name; "usb:VVVV:PPPP" picks by ids. */
    private fun find(path: String): UsbSerialDriver? = drivers().firstOrNull { id(it.device) == path }

    /** What a person would call it: "Icom Inc. IC-705". */
    fun describe(d: UsbDevice): String = listOfNotNull(
        runCatching { d.manufacturerName }.getOrNull(),
        runCatching { d.productName }.getOrNull(),
    ).joinToString(" ").ifBlank { id(d) }

    /** For Settings: the radios attached now, "usb:VVVV:PPPP" to their names. */
    fun attached(): List<Pair<String, String>> =
        runCatching { drivers().map { id(it.device) to describe(it.device) } }.getOrDefault(emptyList())

    // ------------------------------------------------------- engine (JNI)

    /** One line per device: "usb:VVVV:PPPP\tvid\tpid\tmanufacturer\tproduct\tserial" (ids hex). */
    @JvmStatic
    fun list(): String = runCatching {
        drivers().joinToString("\n") { drv ->
            val d = drv.device
            // The serial number needs USB permission; Icom puts the model there.
            val serial = if (usb?.hasPermission(d) == true) runCatching { d.serialNumber }.getOrNull() else null
            listOf(
                id(d), "%04x".format(d.vendorId), "%04x".format(d.productId),
                runCatching { d.manufacturerName }.getOrNull() ?: "",
                runCatching { d.productName }.getOrNull() ?: "",
                serial ?: "",
            ).joinToString("\t") { it.replace('\t', ' ').replace('\n', ' ') }
        }
    }.getOrDefault("")

    @JvmStatic
    fun lastError(): String = lastError

    /** Open the radio at `baud`, RTS and DTR low. A handle (> 0), or 0 (see lastError). */
    @JvmStatic
    fun open(path: String, baud: Int): Int {
        open?.let { close(it.handle) }
        val m = usb ?: return fail("USB isn't available on this phone")
        val drv = find(path) ?: return fail("the radio isn't plugged in (no $path on USB)")
        if (!m.hasPermission(drv.device)) {
            askPermission(drv.device)
            return fail("allow Whistlestop to use the radio's USB connection (Android asks once), then it connects")
        }
        val conn = m.openDevice(drv.device) ?: return fail("Android wouldn't open the radio's USB connection")
        val port = drv.ports.first()  // CP2105 (Yaesu): CAT is the first port
        return try {
            port.open(conn)
            port.setParameters(baud, 8, UsbSerialPort.STOPBITS_1, UsbSerialPort.PARITY_NONE)
            // Many radios key the transmitter (or reset CAT) on RTS or DTR.
            runCatching { port.dtr = false }
            runCatching { port.rts = false }
            val h = handles.incrementAndGet()
            lateinit var o: Open
            val io = SerialInputOutputManager(port, object : SerialInputOutputManager.Listener {
                override fun onNewData(data: ByteArray) {
                    synchronized(o.lock) { o.buf.write(data); o.lock.notifyAll() }
                }
                override fun onRunError(e: Exception) {
                    Log.w(TAG, "radio USB: ${e.message}")
                    synchronized(o.lock) { o.dead = true; o.lock.notifyAll() }
                }
            })
            o = Open(h, port, io)
            open = o
            io.start()
            h
        } catch (e: Exception) {
            runCatching { port.close() }
            fail("couldn't open the radio's USB serial port: ${e.message}")
        }
    }

    @JvmStatic
    fun write(h: Int, data: ByteArray): Boolean {
        val o = open?.takeIf { it.handle == h && !it.dead } ?: return false
        return try {
            o.port.write(data, 500)
            true
        } catch (e: Exception) {
            Log.w(TAG, "radio USB write: ${e.message}")
            o.dead = true
            false
        }
    }

    /** Up to `max` bytes, waiting at most `timeoutMs`; empty if none came, null if the radio is gone. */
    @JvmStatic
    fun read(h: Int, max: Int, timeoutMs: Int): ByteArray? {
        val o = open?.takeIf { it.handle == h } ?: return null
        synchronized(o.lock) {
            val until = System.currentTimeMillis() + timeoutMs
            while (o.buf.size() == 0 && !o.dead) {
                val left = until - System.currentTimeMillis()
                if (left <= 0) break
                o.lock.wait(left)
            }
            if (o.buf.size() == 0) return if (o.dead) null else ByteArray(0)
            val all = o.buf.toByteArray()
            val n = minOf(max, all.size)
            o.buf.reset()
            o.buf.write(all, n, all.size - n)
            return all.copyOf(n)
        }
    }

    /** Throw away what the radio has sent and nobody has read. */
    @JvmStatic
    fun purge(h: Int) {
        val o = open?.takeIf { it.handle == h } ?: return
        synchronized(o.lock) { o.buf.reset() }
    }

    @JvmStatic
    fun close(h: Int) {
        val o = open?.takeIf { it.handle == h } ?: return
        open = null
        runCatching { o.io.stop() }
        runCatching { o.port.close() }
        synchronized(o.lock) { o.dead = true; o.lock.notifyAll() }
    }

    private fun fail(why: String): Int {
        lastError = why
        return 0
    }

    private fun askPermission(d: UsbDevice) {
        val c = app ?: return
        val i = Intent(ACTION_PERMISSION).setPackage(c.packageName)
        // Mutable: Android adds the "granted" extra to it.
        val flags = if (android.os.Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0
        usb?.requestPermission(d, PendingIntent.getBroadcast(c, 0, i, flags))
    }
}
