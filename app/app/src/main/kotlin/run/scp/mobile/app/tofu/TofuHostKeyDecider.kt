package run.scp.mobile.app.tofu

import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull

/** What the user is asked to approve. [previousFingerprint] is non-null when the stored key differs. */
data class HostKeyChallenge(
    val alias: String,
    val keyType: String,
    val fingerprint: String,
    val previousFingerprint: String?,
) {
    val isChanged: Boolean get() = previousFingerprint != null
}

fun interface HostKeyPrompter {
    suspend fun confirm(challenge: HostKeyChallenge): Boolean
}

/** Blocking host-key decision, called from the SSH handshake thread. */
fun interface HostKeyDecider {
    fun decide(alias: String, keyType: String, fingerprint: String): Boolean
}

/**
 * Trust on first use: a known, identical key passes silently; an unknown or changed key
 * needs an explicit user decision and is persisted only when accepted. Timeouts and
 * prompter failures reject (fail closed).
 */
class TofuHostKeyDecider(
    private val store: KnownHostsStore,
    private val prompter: HostKeyPrompter,
    private val timeoutMs: Long = 120_000,
) : HostKeyDecider {
    override fun decide(alias: String, keyType: String, fingerprint: String): Boolean {
        val known = store.find(alias)
        if (known != null && known.keyType == keyType && known.fingerprint == fingerprint) return true

        val challenge = HostKeyChallenge(alias, keyType, fingerprint, known?.fingerprint)
        val accepted = try {
            runBlocking { withTimeoutOrNull(timeoutMs) { prompter.confirm(challenge) } } ?: false
        } catch (e: Exception) {
            false
        }
        if (accepted) store.save(TrustedHost(alias, keyType, fingerprint))
        return accepted
    }
}
