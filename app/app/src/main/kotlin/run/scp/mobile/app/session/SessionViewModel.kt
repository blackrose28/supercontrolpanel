package run.scp.mobile.app.session

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import run.scp.mobile.app.terminal.TerminalBuffer
import run.scp.mobile.app.tofu.HostKeyChallenge
import run.scp.mobile.app.tofu.KnownHostsStore
import run.scp.mobile.app.tofu.TofuHostKeyDecider

sealed interface SessionState {
    data object Idle : SessionState
    data class Connecting(val target: SshTarget) : SessionState
    data class Connected(val target: SshTarget) : SessionState
    data class Ended(val target: SshTarget, val reason: String?) : SessionState
    data class Failed(val target: SshTarget, val message: String) : SessionState
}

data class TerminalSnapshot(val lines: List<String>, val revision: Long)

/**
 * Owns one SSH shell at a time: connection state, the TOFU host-key prompt, and the
 * rendered terminal. Shell writes/resizes/close go through an ordered per-connection queue
 * on [ioDispatcher] so a stalled network never blocks the UI thread.
 */
class SessionViewModel(
    private val connector: ShellConnector,
    store: KnownHostsStore,
    private val ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
    hostKeyTimeoutMs: Long = 120_000,
) : ViewModel() {

    private class Attempt(val id: Long, val target: SshTarget) {
        val ops = Channel<(ShellSession) -> Unit>(Channel.UNLIMITED)
        var session: ShellSession? = null
        var finished = false // Ended/Failed/disconnected: ignore later events
    }

    private val lock = Any()
    private var attempt: Attempt? = null
    private var nextId = 0L

    private val _state = MutableStateFlow<SessionState>(SessionState.Idle)
    val state: StateFlow<SessionState> = _state.asStateFlow()

    private val _pendingHostKey = MutableStateFlow<HostKeyChallenge?>(null)
    val pendingHostKey: StateFlow<HostKeyChallenge?> = _pendingHostKey.asStateFlow()
    private var pendingDecision: CompletableDeferred<Boolean>? = null

    private var buffer = TerminalBuffer()
    private var revision = 0L
    private val _terminal = MutableStateFlow(TerminalSnapshot(buffer.lines(), 0))
    val terminal: StateFlow<TerminalSnapshot> = _terminal.asStateFlow()
    private val dirty = Channel<Unit>(Channel.CONFLATED)

    private val decider = TofuHostKeyDecider(store, ::promptHostKey, hostKeyTimeoutMs)

    init {
        // Coalesce bursts of output into at most one snapshot per frame-ish interval.
        viewModelScope.launch(ioDispatcher) {
            for (ignored in dirty) {
                publishTerminal()
                delay(PUBLISH_INTERVAL_MS)
            }
        }
    }

    fun connect(target: SshTarget, cols: Int, rows: Int) {
        val a: Attempt
        synchronized(lock) {
            val s = _state.value
            if (s is SessionState.Connecting || s is SessionState.Connected) return
            a = Attempt(++nextId, target)
            attempt = a
            buffer = TerminalBuffer()
            _state.value = SessionState.Connecting(target)
        }
        publishTerminal()

        val events = object : ShellEvents {
            override fun onOutput(bytes: ByteArray) {
                synchronized(lock) {
                    if (attempt !== a || a.finished) return
                    buffer.append(bytes)
                }
                dirty.trySend(Unit)
            }

            override fun onClosed(reason: String) {
                synchronized(lock) {
                    if (attempt !== a || a.finished) return
                    a.finished = true
                    _state.value = SessionState.Ended(target, reason.ifBlank { null })
                    a.ops.trySend { it.close() }
                    a.ops.close()
                }
                publishTerminal()
            }
        }

        viewModelScope.launch(ioDispatcher) {
            val session = try {
                connector.open(target, cols, rows, decider, events)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                synchronized(lock) {
                    if (attempt === a && !a.finished) {
                        a.finished = true
                        a.ops.close()
                        _state.value = SessionState.Failed(target, e.message ?: e::class.java.simpleName)
                    }
                }
                return@launch
            }
            val keep = synchronized(lock) {
                if (attempt === a && !a.finished) {
                    a.session = session
                    _state.value = SessionState.Connected(target)
                    true
                } else {
                    false // disconnected or closed while the handshake was in flight
                }
            }
            if (!keep) {
                runCatching { session.close() }
                a.ops.close()
                return@launch
            }
            for (op in a.ops) runCatching { op(session) }
        }
    }

    fun send(text: String) = send(text.toByteArray(Charsets.UTF_8))

    fun send(bytes: ByteArray) = enqueue { it.write(bytes) }

    fun resize(cols: Int, rows: Int) = enqueue { it.resize(cols, rows) }

    private fun enqueue(op: (ShellSession) -> Unit) {
        synchronized(lock) {
            val a = attempt ?: return
            if (a.finished || _state.value !is SessionState.Connected) return
            a.ops.trySend(op)
        }
    }

    /** Aborts any connect in progress, closes the shell, and returns to Idle. */
    fun disconnect() {
        synchronized(lock) {
            attempt?.let { a ->
                if (!a.finished) {
                    a.finished = true
                    if (a.session != null) a.ops.trySend { it.close() }
                }
                a.ops.close()
            }
            attempt = null
            _state.value = SessionState.Idle
        }
        pendingDecision?.complete(false)
        _pendingHostKey.value = null // clear now; the prompting coroutine unwinds asynchronously
    }

    /** The user's answer to [pendingHostKey]. */
    fun resolveHostKey(trust: Boolean) {
        pendingDecision?.complete(trust)
    }

    private suspend fun promptHostKey(challenge: HostKeyChallenge): Boolean {
        val decision = CompletableDeferred<Boolean>()
        pendingDecision = decision
        _pendingHostKey.value = challenge
        try {
            return decision.await()
        } finally {
            // Only clear our own prompt; a newer attempt may already have posted another.
            if (pendingDecision === decision) {
                _pendingHostKey.value = null
                pendingDecision = null
            }
        }
    }

    private fun publishTerminal() {
        val lines = synchronized(lock) { buffer.lines() }
        _terminal.value = TerminalSnapshot(lines, ++revision)
    }

    override fun onCleared() {
        disconnect()
    }

    private companion object {
        const val PUBLISH_INTERVAL_MS = 16L
    }
}
