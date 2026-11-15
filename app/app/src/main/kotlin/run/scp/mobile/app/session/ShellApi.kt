package run.scp.mobile.app.session

import run.scp.mobile.app.tofu.HostKeyDecider

data class SshTarget(val host: String, val port: Int, val user: String) {
    /** Key under which the host key is trusted: the server's real host:port. */
    val alias: String get() = "$host:$port"
}

/** Callbacks from the shell. Invoked on a native thread; [onClosed] fires at most once. */
interface ShellEvents {
    fun onOutput(bytes: ByteArray)
    fun onClosed(reason: String)
}

/** An open PTY shell. Methods may block on the network; call them off the main thread. */
interface ShellSession {
    fun write(bytes: ByteArray)
    fun resize(cols: Int, rows: Int)
    fun close()
}

/** Opens a shell, consulting [hostKeys] during the handshake. Blocking; runs off the main thread. */
interface ShellConnector {
    suspend fun open(
        target: SshTarget,
        cols: Int,
        rows: Int,
        hostKeys: HostKeyDecider,
        events: ShellEvents,
    ): ShellSession
}
