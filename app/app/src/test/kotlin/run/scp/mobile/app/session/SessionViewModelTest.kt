package run.scp.mobile.app.session

import java.util.concurrent.CopyOnWriteArrayList
import kotlin.test.AfterTest
import kotlin.test.BeforeTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withTimeout
import run.scp.mobile.app.tofu.HostKeyDecider
import run.scp.mobile.app.tofu.HostKeyChallenge
import run.scp.mobile.app.tofu.KnownHostsStore
import run.scp.mobile.app.tofu.TrustedHost

private class FakeSession : ShellSession {
    val written = CopyOnWriteArrayList<String>()
    val resizes = CopyOnWriteArrayList<Pair<Int, Int>>()
    @Volatile var closed = 0
    override fun write(bytes: ByteArray) { written += String(bytes) }
    override fun resize(cols: Int, rows: Int) { resizes += cols to rows }
    override fun close() { closed++ }
}

private class FakeConnector(
    private val keyType: String = "ssh-ed25519",
    private val fingerprint: String = "SHA256:fp",
    private val failWith: Throwable? = null,
) : ShellConnector {
    val session = FakeSession()
    lateinit var events: ShellEvents
    var openedWith: Triple<SshTarget, Int, Int>? = null

    override suspend fun open(
        target: SshTarget, cols: Int, rows: Int, hostKeys: HostKeyDecider, events: ShellEvents,
    ): ShellSession {
        openedWith = Triple(target, cols, rows)
        this.events = events
        failWith?.let { throw it }
        if (!hostKeys.decide(target.alias, keyType, fingerprint)) error("host key rejected")
        return session
    }
}

private class MemStore : KnownHostsStore {
    val map = mutableMapOf<String, TrustedHost>()
    override fun find(alias: String) = map[alias]
    override fun save(host: TrustedHost) { map[host.alias] = host }
}

@OptIn(ExperimentalCoroutinesApi::class)
class SessionViewModelTest {
    private val target = SshTarget("10.0.0.5", 22, "ops")
    private val store = MemStore()

    @BeforeTest fun setUp() { Dispatchers.setMain(UnconfinedTestDispatcher()) }
    @AfterTest fun tearDown() { Dispatchers.resetMain() }

    private fun vm(c: ShellConnector) =
        SessionViewModel(c, store, ioDispatcher = Dispatchers.IO, hostKeyTimeoutMs = 5_000)

    private suspend fun <T> Flow<T>.await(p: (T) -> Boolean): T = withTimeout(5_000) { first(p) }

    /** Shell I/O runs on a worker queue, so effects on the session are asynchronous. */
    private fun eventually(check: () -> Boolean) {
        val deadline = System.nanoTime() + 5_000_000_000L
        while (!check()) {
            check(System.nanoTime() < deadline) { "condition not met within 5s" }
            Thread.sleep(5)
        }
    }

    @Test fun `alias is host colon port`() = assertEquals("10.0.0.5:22", target.alias)

    @Test fun `first contact surfaces a host key challenge then connects after trust`() = runBlocking {
        val c = FakeConnector()
        val vm = vm(c)
        vm.connect(target, 80, 24)
        val ch = vm.pendingHostKey.await { it != null }!!
        assertEquals(HostKeyChallenge("10.0.0.5:22", "ssh-ed25519", "SHA256:fp", null), ch)
        assertIs<SessionState.Connecting>(vm.state.value)

        vm.resolveHostKey(trust = true)
        vm.state.await { it is SessionState.Connected }
        assertNull(vm.pendingHostKey.value)
        assertEquals("SHA256:fp", store.find("10.0.0.5:22")?.fingerprint)
        assertEquals(Triple(target, 80, 24), c.openedWith)
    }

    @Test fun `rejecting the host key fails the session and saves nothing`() = runBlocking {
        val vm = vm(FakeConnector())
        vm.connect(target, 80, 24)
        vm.pendingHostKey.await { it != null }
        vm.resolveHostKey(trust = false)
        val failed = vm.state.await { it is SessionState.Failed } as SessionState.Failed
        assertTrue(failed.message.contains("host key rejected"))
        assertNull(store.find("10.0.0.5:22"))
        assertNull(vm.pendingHostKey.value)
    }

    @Test fun `trusted host connects with no prompt`() = runBlocking {
        store.save(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:fp"))
        val vm = vm(FakeConnector())
        vm.connect(target, 80, 24)
        vm.state.await { it is SessionState.Connected }
        assertNull(vm.pendingHostKey.value)
    }

    @Test fun `connector failure becomes Failed with its message`() = runBlocking {
        val vm = vm(FakeConnector(failWith = java.io.IOException("connection refused")))
        vm.connect(target, 80, 24)
        val f = vm.state.await { it is SessionState.Failed } as SessionState.Failed
        assertEquals("connection refused", f.message)
    }

    @Test fun `output is rendered into the terminal snapshot`() = runBlocking {
        store.save(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:fp"))
        val c = FakeConnector()
        val vm = vm(c)
        vm.connect(target, 80, 24)
        vm.state.await { it is SessionState.Connected }
        c.events.onOutput("hello\r\nworld".toByteArray())
        val snap = vm.terminal.await { it.lines == listOf("hello", "world") }
        assertEquals(listOf("hello", "world"), snap.lines)
    }

    @Test fun `input and resize are forwarded to the shell`() = runBlocking {
        store.save(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:fp"))
        val c = FakeConnector()
        val vm = vm(c)
        vm.connect(target, 80, 24)
        vm.state.await { it is SessionState.Connected }
        vm.send("ls\r")
        vm.resize(100, 40)
        eventually { c.session.written.size == 1 && c.session.resizes.size == 1 }
        assertEquals(listOf("ls\r"), c.session.written)
        assertEquals(listOf(100 to 40), c.session.resizes)
    }

    @Test fun `remote close moves to Ended with the reason`() = runBlocking {
        store.save(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:fp"))
        val c = FakeConnector()
        val vm = vm(c)
        vm.connect(target, 80, 24)
        vm.state.await { it is SessionState.Connected }
        c.events.onClosed("exit status 0")
        val e = vm.state.await { it is SessionState.Ended } as SessionState.Ended
        assertEquals("exit status 0", e.reason)
    }

    @Test fun `blank close reason maps to null`() = runBlocking {
        store.save(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:fp"))
        val c = FakeConnector()
        val vm = vm(c)
        vm.connect(target, 80, 24)
        vm.state.await { it is SessionState.Connected }
        c.events.onClosed("")
        assertNull((vm.state.await { it is SessionState.Ended } as SessionState.Ended).reason)
    }

    @Test fun `disconnect closes the shell and returns to Idle`() = runBlocking {
        store.save(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:fp"))
        val c = FakeConnector()
        val vm = vm(c)
        vm.connect(target, 80, 24)
        vm.state.await { it is SessionState.Connected }
        vm.disconnect()
        assertEquals(SessionState.Idle, vm.state.value)
        eventually { c.session.closed == 1 }
        Thread.sleep(50)
        assertEquals(1, c.session.closed)
    }

    @Test fun `disconnect while waiting for host key aborts the connect`() = runBlocking {
        val c = FakeConnector()
        val vm = vm(c)
        vm.connect(target, 80, 24)
        vm.pendingHostKey.await { it != null }
        vm.disconnect()
        assertEquals(SessionState.Idle, vm.state.value)
        assertNull(vm.pendingHostKey.value)
        assertNull(store.find("10.0.0.5:22"))
    }

    @Test fun `writes are delivered in order`() = runBlocking {
        store.save(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:fp"))
        val c = FakeConnector()
        val vm = vm(c)
        vm.connect(target, 80, 24)
        vm.state.await { it is SessionState.Connected }
        ('a'..'z').forEach { vm.send(it.toString()) }
        eventually { c.session.written.size == 26 }
        assertEquals(('a'..'z').map { it.toString() }, c.session.written.toList())
    }

    @Test fun `input while not connected is dropped`() = runBlocking {
        val c = FakeConnector()
        val vm = vm(c)
        vm.send("x")
        vm.connect(target, 80, 24)
        vm.pendingHostKey.await { it != null }
        vm.send("y")
        vm.resolveHostKey(true)
        vm.state.await { it is SessionState.Connected }
        Thread.sleep(50)
        assertTrue(c.session.written.isEmpty())
    }

    @Test fun `closed notification during handshake wins over late open return`() = runBlocking<Unit> {
        store.save(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:fp"))
        val c = object : ShellConnector {
            val s = FakeSession()
            override suspend fun open(t: SshTarget, cols: Int, rows: Int, hostKeys: HostKeyDecider, events: ShellEvents): ShellSession {
                events.onClosed("dropped")
                return s
            }
        }
        val vm = vm(c)
        vm.connect(target, 80, 24)
        val e = vm.state.await { it is SessionState.Ended } as SessionState.Ended
        assertEquals("dropped", e.reason)
        eventually { c.s.closed >= 1 }
        assertIs<SessionState.Ended>(vm.state.value)
    }

    @Test fun `a second connect is ignored while one is active`() = runBlocking {
        val c = FakeConnector()
        val vm = vm(c)
        vm.connect(target, 80, 24)
        vm.pendingHostKey.await { it != null }
        vm.connect(SshTarget("other", 22, "x"), 80, 24)
        assertEquals(target, c.openedWith?.first)
    }

    @Test fun `reconnect after end starts with a clean terminal`() = runBlocking {
        store.save(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:fp"))
        val c = FakeConnector()
        val vm = vm(c)
        vm.connect(target, 80, 24)
        vm.state.await { it is SessionState.Connected }
        c.events.onOutput("old".toByteArray())
        vm.terminal.await { it.lines == listOf("old") }
        c.events.onClosed("bye")
        vm.state.await { it is SessionState.Ended }
        vm.disconnect()
        vm.connect(target, 80, 24)
        vm.state.await { it is SessionState.Connected }
        assertEquals(listOf(""), vm.terminal.value.lines)
    }
}
