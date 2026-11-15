package run.scp.mobile.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import run.scp.mobile.app.session.SessionState
import run.scp.mobile.app.session.SessionViewModel
import run.scp.mobile.app.ssh.KeystoreDeviceIdentity
import run.scp.mobile.app.ssh.OpenShellConnector
import run.scp.mobile.app.tofu.FileKnownHostsStore
import run.scp.mobile.app.ui.ConnectScreen
import run.scp.mobile.app.ui.ConnectingScreen
import run.scp.mobile.app.ui.TerminalScreen
import run.scp.mobile.app.ui.TofuScreen

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val identity = KeystoreDeviceIdentity(applicationContext)
        val knownHosts = FileKnownHostsStore(File(filesDir, "known_hosts"))
        val factory = viewModelFactory {
            initializer { SessionViewModel(OpenShellConnector(identity), knownHosts) }
        }
        setContent {
            MaterialTheme(colorScheme = darkColorScheme()) {
                val vm: SessionViewModel = viewModel(factory = factory)
                var publicKey by remember { mutableStateOf("") }
                // Key generation/decryption touches the Keystore and native code: keep it off the UI thread.
                LaunchedEffect(Unit) { publicKey = withContext(Dispatchers.IO) { identity.publicAuthorizedKey() } }
                Box(Modifier.fillMaxSize()) { AppRoot(vm, publicKey) }
            }
        }
    }
}

@Composable
private fun AppRoot(vm: SessionViewModel, publicKey: String) {
    val state by vm.state.collectAsStateWithLifecycle()
    val challenge by vm.pendingHostKey.collectAsStateWithLifecycle()
    val terminal by vm.terminal.collectAsStateWithLifecycle()

    val pending = challenge
    when {
        pending != null -> TofuScreen(pending, onDecision = vm::resolveHostKey)
        else -> when (val s = state) {
            SessionState.Idle -> ConnectScreen(publicKey, error = null, onConnect = { vm.connect(it, 80, 24) })
            is SessionState.Failed -> ConnectScreen(publicKey, error = s.message, onConnect = {
                vm.disconnect()
                vm.connect(it, 80, 24)
            })
            is SessionState.Connecting -> ConnectingScreen(s.target, onCancel = vm::disconnect)
            is SessionState.Connected, is SessionState.Ended -> TerminalScreen(
                state = s,
                terminal = terminal,
                onSend = vm::send,
                onResize = vm::resize,
                onClose = vm::disconnect,
            )
        }
    }
}
