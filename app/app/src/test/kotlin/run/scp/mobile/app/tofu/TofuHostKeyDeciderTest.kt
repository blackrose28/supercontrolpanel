package run.scp.mobile.app.tofu

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.coroutines.delay

private class MemStore : KnownHostsStore {
    val map = mutableMapOf<String, TrustedHost>()
    override fun find(alias: String) = map[alias]
    override fun save(host: TrustedHost) { map[host.alias] = host }
}

class TofuHostKeyDeciderTest {
    private val store = MemStore()
    private val asked = mutableListOf<HostKeyChallenge>()

    private fun decider(timeoutMs: Long = 2_000, answer: suspend (HostKeyChallenge) -> Boolean) =
        TofuHostKeyDecider(store, { c -> asked += c; answer(c) }, timeoutMs)

    @Test fun `known matching key is trusted without prompting`() {
        store.save(TrustedHost("h:22", "ssh-ed25519", "SHA256:a"))
        assertTrue(decider { error("must not prompt") }.decide("h:22", "ssh-ed25519", "SHA256:a"))
        assertTrue(asked.isEmpty())
    }

    @Test fun `first contact prompts and accepting persists the key`() {
        assertTrue(decider { true }.decide("h:22", "ssh-ed25519", "SHA256:a"))
        assertEquals(1, asked.size)
        assertNull(asked[0].previousFingerprint)
        assertFalse(asked[0].isChanged)
        assertEquals("SHA256:a", store.find("h:22")?.fingerprint)
    }

    @Test fun `first contact rejection is not persisted`() {
        assertFalse(decider { false }.decide("h:22", "ssh-ed25519", "SHA256:a"))
        assertNull(store.find("h:22"))
    }

    @Test fun `changed key prompts as changed and keeps old key when rejected`() {
        store.save(TrustedHost("h:22", "ssh-ed25519", "SHA256:old"))
        assertFalse(decider { false }.decide("h:22", "ssh-ed25519", "SHA256:new"))
        assertTrue(asked.single().isChanged)
        assertEquals("SHA256:old", asked.single().previousFingerprint)
        assertEquals("SHA256:old", store.find("h:22")?.fingerprint)
    }

    @Test fun `changed key accepted explicitly replaces the stored key`() {
        store.save(TrustedHost("h:22", "ssh-ed25519", "SHA256:old"))
        assertTrue(decider { true }.decide("h:22", "ssh-ed25519", "SHA256:new"))
        assertEquals("SHA256:new", store.find("h:22")?.fingerprint)
    }

    @Test fun `same fingerprint but different key type is treated as changed`() {
        store.save(TrustedHost("h:22", "ssh-rsa", "SHA256:a"))
        assertFalse(decider { false }.decide("h:22", "ssh-ed25519", "SHA256:a"))
        assertTrue(asked.single().isChanged)
    }

    @Test fun `timeout while waiting for the user fails closed`() {
        assertFalse(decider(timeoutMs = 50) { delay(5_000); true }.decide("h:22", "t", "SHA256:a"))
        assertNull(store.find("h:22"))
    }

    @Test fun `prompter failure fails closed`() {
        assertFalse(decider { throw IllegalStateException("ui gone") }.decide("h:22", "t", "SHA256:a"))
        assertNull(store.find("h:22"))
    }
}
