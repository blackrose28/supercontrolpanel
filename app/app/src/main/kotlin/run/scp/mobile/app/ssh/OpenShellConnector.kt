package run.scp.mobile.app.ssh

import run.scp.mobile.app.session.ShellConnector
import run.scp.mobile.app.session.ShellEvents
import run.scp.mobile.app.session.ShellSession
import run.scp.mobile.app.session.SshTarget
import run.scp.mobile.app.tofu.HostKeyDecider
import run.scp.mobile.scpmobile.HostKeyVerifier
import run.scp.mobile.scpmobile.OutputSink
import run.scp.mobile.scpmobile.Scpmobile
import run.scp.mobile.scpmobile.Shell

/** Backed by scpmobile.aar: DialDirect for transport, OpenShell for SSH + PTY. */
class OpenShellConnector(private val identity: DeviceIdentity) : ShellConnector {

    override suspend fun open(
        target: SshTarget,
        cols: Int,
        rows: Int,
        hostKeys: HostKeyDecider,
        events: ShellEvents,
    ): ShellSession {
        // The AAR closes the connection itself if openShell fails.
        val conn = Scpmobile.dialDirect(target.host, target.port.toLong(), 0)
        val verifier = object : HostKeyVerifier {
            override fun verify(alias: String, keyType: String, fingerprint: String, authorizedKey: ByteArray): Boolean =
                hostKeys.decide(alias, keyType, fingerprint)
        }
        val sink = object : OutputSink {
            override fun write(p0: ByteArray) = events.onOutput(p0)
            override fun closed(reason: String) = events.onClosed(reason)
        }
        val shell: Shell = Scpmobile.openShell(
            conn, target.user, identity.privateKeyPem(), target.alias, verifier, sink, cols.toLong(), rows.toLong(),
        )
        return object : ShellSession {
            override fun write(bytes: ByteArray) = shell.write(bytes)
            override fun resize(cols: Int, rows: Int) = shell.resize(cols.toLong(), rows.toLong())
            override fun close() = shell.close()
        }
    }
}
