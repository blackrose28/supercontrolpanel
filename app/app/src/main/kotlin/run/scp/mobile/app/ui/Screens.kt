package run.scp.mobile.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import run.scp.mobile.app.session.SessionState
import run.scp.mobile.app.session.SshTarget
import run.scp.mobile.app.session.TerminalSnapshot
import run.scp.mobile.app.tofu.HostKeyChallenge

@Composable
fun ConnectScreen(
    publicKey: String,
    error: String?,
    onConnect: (SshTarget) -> Unit,
) {
    var host by remember { mutableStateOf("") }
    var port by remember { mutableStateOf("22") }
    var user by remember { mutableStateOf("") }
    val portNum = port.toIntOrNull()?.takeIf { it in 1..65535 }
    val canConnect = host.isNotBlank() && user.isNotBlank() && portNum != null

    Column(
        Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp).verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Connect", style = MaterialTheme.typography.headlineMedium)
        OutlinedTextField(host, { host = it.trim() }, label = { Text("Host") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(
            port, { port = it.filter(Char::isDigit) }, label = { Text("Port") }, singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(user, { user = it.trim() }, label = { Text("User") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        if (error != null) Text(error, color = MaterialTheme.colorScheme.error)
        Button(
            onClick = { onConnect(SshTarget(host, portNum!!, user)) },
            enabled = canConnect,
            modifier = Modifier.fillMaxWidth(),
        ) { Text("Connect") }

        Text("Device public key", style = MaterialTheme.typography.titleSmall)
        Text("Add this line to the server's authorized_keys:", style = MaterialTheme.typography.bodySmall)
        Text(publicKey.ifBlank { "…" }, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
    }
}

@Composable
fun ConnectingScreen(target: SshTarget, onCancel: () -> Unit) {
    Column(
        Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        CircularProgressIndicator()
        Text("Connecting to ${target.user}@${target.alias}…", Modifier.padding(top = 16.dp))
        TextButton(onClick = onCancel) { Text("Cancel") }
    }
}

/** Trust-on-first-use prompt. A changed key is presented as a warning and defaults to refusing. */
@Composable
fun TofuScreen(challenge: HostKeyChallenge, onDecision: (trust: Boolean) -> Unit) {
    val changed = challenge.isChanged
    Column(
        Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp).verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            if (changed) "Host key has CHANGED" else "Trust this host?",
            style = MaterialTheme.typography.headlineMedium,
            color = if (changed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
        )
        Text(
            if (changed) {
                "The key for ${challenge.alias} is different from the one you trusted before. " +
                    "This can mean the server was reinstalled — or that someone is intercepting the connection."
            } else {
                "You have not connected to ${challenge.alias} before. Verify the fingerprint with the server's administrator."
            },
        )
        Text("Key type", style = MaterialTheme.typography.labelMedium)
        Text(challenge.keyType, fontFamily = FontFamily.Monospace)
        if (changed) {
            Text("Previously trusted", style = MaterialTheme.typography.labelMedium)
            Text(challenge.previousFingerprint.orEmpty(), fontFamily = FontFamily.Monospace)
        }
        Text(if (changed) "New fingerprint" else "Fingerprint", style = MaterialTheme.typography.labelMedium)
        Text(challenge.fingerprint, fontFamily = FontFamily.Monospace)

        Button(onClick = { onDecision(false) }, modifier = Modifier.fillMaxWidth()) { Text("Cancel") }
        OutlinedButton(
            onClick = { onDecision(true) },
            modifier = Modifier.fillMaxWidth(),
            colors = if (changed) ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.error)
            else ButtonDefaults.outlinedButtonColors(),
        ) { Text(if (changed) "Replace trusted key and connect" else "Trust and connect") }
    }
}

@Composable
fun TerminalScreen(
    state: SessionState,
    terminal: TerminalSnapshot,
    onSend: (ByteArray) -> Unit,
    onResize: (cols: Int, rows: Int) -> Unit,
    onClose: () -> Unit,
) {
    val connected = state is SessionState.Connected
    val style = remember { TextStyle(fontFamily = FontFamily.Monospace, fontSize = 13.sp, color = Color(0xFFE0E0E0)) }
    val measurer: TextMeasurer = rememberTextMeasurer()
    val cell = remember(style) { measurer.measure("W", style).size }
    val listState = rememberLazyListState()
    val focus = remember { FocusRequester() }

    LaunchedEffect(terminal.revision) {
        if (terminal.lines.isNotEmpty()) listState.scrollToItem(terminal.lines.lastIndex)
    }
    LaunchedEffect(connected) { if (connected) focus.requestFocus() }

    Column(Modifier.fillMaxSize().background(Color(0xFF101010)).safeDrawingPadding().imePadding()) {
        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 4.dp)) {
            val density = LocalDensity.current
            val size = with(density) {
                terminalSize(maxWidth.toPx(), maxHeight.toPx(), cell.width.toFloat(), cell.height.toFloat())
            }
            LaunchedEffect(size, connected) { if (connected) onResize(size.cols, size.rows) }
            LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
                items(terminal.lines) { line -> Text(line.ifEmpty { " " }, style = style, softWrap = false, maxLines = 1) }
            }
        }

        when (state) {
            is SessionState.Ended ->
                Text(
                    "Session ended" + (state.reason?.let { ": $it" } ?: ""),
                    color = Color(0xFFFFB74D), modifier = Modifier.padding(8.dp),
                )
            is SessionState.Failed -> Text(state.message, color = Color(0xFFFF8A80), modifier = Modifier.padding(8.dp))
            else -> Unit
        }

        // Typed characters are sent immediately; the field itself stays empty.
        BasicTextField(
            value = "",
            onValueChange = { if (it.isNotEmpty() && connected) onSend(it.toByteArray(Charsets.UTF_8)) },
            enabled = connected,
            singleLine = true,
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Password, autoCorrectEnabled = false, imeAction = ImeAction.Send,
            ),
            keyboardActions = KeyboardActions(onSend = { onSend(SpecialKey.Enter.bytes) }),
            modifier = Modifier
                .alpha(0f)
                .focusRequester(focus)
                .onPreviewKeyEvent { e ->
                    if (e.type == KeyEventType.KeyDown && e.key == Key.Backspace) {
                        onSend(SpecialKey.Backspace.bytes); true
                    } else false
                },
        )
        KeyRow(connected, onSend)
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.End) {
            TextButton(onClick = onClose) { Text(if (connected) "Disconnect" else "Close") }
        }
    }
}

@Composable
private fun KeyRow(enabled: Boolean, onSend: (ByteArray) -> Unit) {
    val keys = listOf(
        "Esc" to SpecialKey.Esc, "Tab" to SpecialKey.Tab, "^C" to SpecialKey.CtrlC, "^D" to SpecialKey.CtrlD,
        "⌫" to SpecialKey.Backspace, "↑" to SpecialKey.Up, "↓" to SpecialKey.Down, "←" to SpecialKey.Left,
        "→" to SpecialKey.Right, "⏎" to SpecialKey.Enter,
    )
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        for ((label, key) in keys) {
            OutlinedButton(
                onClick = { onSend(key.bytes) }, enabled = enabled, contentPadding = PaddingValues(0.dp),
                modifier = Modifier.weight(1f),
            ) { Text(label, fontSize = 12.sp, maxLines = 1) }
        }
    }
}
