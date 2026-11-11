package com.nr5p.whistlestop

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Help
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** The desktop app's palette: night-blue panels, brass, teal for "on". */
object C {
    val bg = Color(0xFF0D1117)
    val panel = Color(0xFF131921)
    val card = Color(0xFF1A212B)
    val cardHi = Color(0xFF222B38)
    val border = Color(0xFF2C3746)
    val text = Color(0xFFDEE4EC)
    val dim = Color(0xFF8694A5)
    val brass = Color(0xFFF2A93B)
    val teal = Color(0xFF3CC8B4)
    val red = Color(0xFFE85C54)
    val ink = Color(0xFF14100A)
}

@Composable
fun WhistlestopTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(
            primary = C.brass, onPrimary = C.ink, secondary = C.teal,
            background = C.bg, surface = C.panel, surfaceVariant = C.card,
            onBackground = C.text, onSurface = C.text, onSurfaceVariant = C.dim,
            outline = C.border, error = C.red,
        ),
        content = content,
    )
}

/** Help mode: on a phone there's no hover, so with Help on every card shows a ?. */
val LocalHelp = compositionLocalOf<MutableState<Boolean>> { mutableStateOf(false) }

@Composable
fun HelpButton(topic: HelpTopic) {
    if (!LocalHelp.current.value) return
    var open by remember { mutableStateOf(false) }
    IconButton(onClick = { open = true }, modifier = Modifier.size(32.dp)) {
        Icon(Icons.Filled.Help, contentDescription = "Help: ${topic.title}", tint = C.brass)
    }
    if (open) {
        AlertDialog(
            onDismissRequest = { open = false },
            confirmButton = { TextButton(onClick = { open = false }) { Text("OK") } },
            title = { Text(topic.title) },
            text = { Text(topic.body) },
            containerColor = C.cardHi,
        )
    }
}

@Composable
fun WCard(title: String, help: HelpTopic? = null, content: @Composable () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .background(C.card, RoundedCornerShape(12.dp))
            .border(1.dp, C.border, RoundedCornerShape(12.dp))
            .padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Text(title.uppercase(), color = C.dim, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 0.8.sp)
            Spacer(Modifier.weight(1f))
            if (help != null) HelpButton(help)
        }
        content()
    }
}

@Composable
fun Pill(label: String, on: Boolean) {
    Row(
        Modifier
            .background(C.cardHi, CircleShape)
            .padding(horizontal = 8.dp, vertical = 4.dp)
            .semantics(mergeDescendants = true) { contentDescription = "$label ${if (on) "ready" else "not ready"}" },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(7.dp).background(if (on) C.teal else C.red, CircleShape))
        Spacer(Modifier.width(5.dp))
        Text(label, color = C.text, fontSize = 12.sp)
    }
}

@Composable
fun BrassButton(text: String, modifier: Modifier = Modifier.fillMaxWidth(), color: Color = C.brass, enabled: Boolean = true, onClick: () -> Unit) {
    Button(
        onClick = onClick, enabled = enabled, modifier = modifier,
        shape = RoundedCornerShape(10.dp),
        colors = ButtonDefaults.buttonColors(containerColor = color, contentColor = C.ink, disabledContainerColor = C.cardHi, disabledContentColor = C.dim),
        contentPadding = PaddingValues(vertical = 12.dp, horizontal = 14.dp),
    ) { Text(text, fontWeight = FontWeight.SemiBold, maxLines = 1) }
}

@Composable
fun QuietButton(text: String, modifier: Modifier = Modifier.fillMaxWidth(), enabled: Boolean = true, onClick: () -> Unit) {
    OutlinedButton(
        onClick = onClick, enabled = enabled, modifier = modifier,
        shape = RoundedCornerShape(10.dp),
        border = BorderStroke(1.dp, C.border),
        colors = ButtonDefaults.outlinedButtonColors(containerColor = C.cardHi, contentColor = C.text, disabledContentColor = C.dim),
        contentPadding = PaddingValues(vertical = 12.dp, horizontal = 14.dp),
    ) { Text(text, maxLines = 1) }
}

@Composable
fun Field(value: String, onChange: (String) -> Unit, label: String, modifier: Modifier = Modifier, caps: Boolean = true, number: Boolean = false) {
    OutlinedTextField(
        value = value, onValueChange = onChange, modifier = modifier,
        placeholder = { Text(label, color = C.dim) },
        singleLine = true,
        shape = RoundedCornerShape(8.dp),
        keyboardOptions = KeyboardOptions(
            capitalization = if (caps) KeyboardCapitalization.Characters else KeyboardCapitalization.Sentences,
            keyboardType = if (number) KeyboardType.Decimal else KeyboardType.Text,
            autoCorrectEnabled = false,
        ),
        colors = OutlinedTextFieldDefaults.colors(
            focusedContainerColor = C.panel, unfocusedContainerColor = C.panel,
            focusedBorderColor = C.brass, unfocusedBorderColor = C.border,
        ),
    )
}
