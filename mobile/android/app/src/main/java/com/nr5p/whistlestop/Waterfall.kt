package com.nr5p.whistlestop

import android.graphics.Bitmap
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private val LUT: IntArray = run {
    val stops = listOf(
        0.00f to floatArrayOf(6f, 10f, 22f), 0.25f to floatArrayOf(22f, 38f, 92f),
        0.50f to floatArrayOf(110f, 44f, 140f), 0.72f to floatArrayOf(226f, 96f, 60f),
        0.88f to floatArrayOf(246f, 186f, 70f), 1.00f to floatArrayOf(255f, 250f, 225f),
    )
    IntArray(256) { i ->
        val t = i / 255f
        val k = (stops.indexOfLast { it.first <= t }.coerceAtLeast(0)).coerceAtMost(stops.size - 2)
        val (a, b) = stops[k] to stops[k + 1]
        val u = (t - a.first) / (b.first - a.first)
        fun c(j: Int) = (a.second[j] + (b.second[j] - a.second[j]) * u).toInt().coerceIn(0, 255)
        (0xFF shl 24) or (c(0) shl 16) or (c(1) shl 8) or c(2)
    }
}

private const val COLS = 256
private const val RANGE_DB = 45f

/**
 * The radio's audio passband (0–3 kHz), newest at the top: the desktop's
 * colours and noise tracking (floor = 30th percentile of what's shown).
 */
@Composable
fun Waterfall(rows: List<FloatArray>, modifier: Modifier = Modifier) {
    val bitmap = remember(rows) { render(rows) }
    Box(
        modifier
            .clip(RoundedCornerShape(8.dp))
            .background(Color(0xFF060A16))
            .semantics { contentDescription = "Waterfall" },
    ) {
        if (bitmap != null) {
            val img = remember(bitmap) { bitmap.asImageBitmap() }
            Canvas(Modifier.fillMaxSize()) {
                drawImage(
                    img, dstSize = IntSize(size.width.toInt(), size.height.toInt()), dstOffset = IntOffset.Zero,
                    filterQuality = FilterQuality.Low,
                )
                // The whistle band, 1000–2000 Hz.
                drawRect(C.brass.copy(alpha = 0.08f), topLeft = androidx.compose.ui.geometry.Offset(size.width / 3, 0f),
                    size = androidx.compose.ui.geometry.Size(size.width / 3, size.height))
            }
        }
        Row(Modifier.fillMaxWidth().align(Alignment.BottomCenter).padding(horizontal = 4.dp, vertical = 2.dp)) {
            listOf("0", "1k", "2k", "3k").forEachIndexed { i, s ->
                Text(s, color = C.dim, fontSize = 9.sp)
                if (i < 3) androidx.compose.foundation.layout.Spacer(Modifier.weight(1f))
            }
        }
    }
}

private fun render(rows: List<FloatArray>): Bitmap? {
    if (rows.isEmpty()) return null
    val h = rows.size
    val px = IntArray(COLS * h)
    val line = FloatArray(COLS)
    var noise = -100f
    rows.forEachIndexed { n, row ->
        if (row.size < 3) return@forEachIndexed
        for (c in 0 until COLS) {
            val f = (c + 0.5f) / COLS * 3000f
            val bin = f / 4000f * row.size
            val i = bin.toInt().coerceAtMost(row.size - 2)
            val u = bin - i
            line[c] = row[i] * (1 - u) + row[i + 1] * u
        }
        val sorted = line.filter { it.isFinite() }.sorted()
        if (sorted.isEmpty()) return@forEachIndexed
        val p = sorted[sorted.size * 3 / 10]
        noise = if (n == 0) p else 0.93f * noise + 0.07f * p
        val floor = noise - 4f
        val y = h - 1 - n
        for (c in 0 until COLS) {
            val t = ((line[c] - floor) / RANGE_DB).coerceIn(0f, 1f)
            px[y * COLS + c] = LUT[(t * 255).toInt()]
        }
    }
    return Bitmap.createBitmap(px, COLS, h, Bitmap.Config.ARGB_8888)
}
