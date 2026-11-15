package run.scp.mobile.app.tofu

import java.io.File
import kotlin.test.Test
import org.junit.Rule
import org.junit.rules.TemporaryFolder
import kotlin.test.assertEquals
import kotlin.test.assertNull

class FileKnownHostsStoreTest {
    @get:Rule val tmp = TemporaryFolder()

    private fun file() = File(tmp.root, "known_hosts")

    @Test fun `unknown alias returns null and missing file is fine`() {
        assertNull(FileKnownHostsStore(file()).find("a:22"))
    }

    @Test fun `saved host survives reopening the store`() {
        FileKnownHostsStore(file()).save(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:abc"))
        val found = FileKnownHostsStore(file()).find("10.0.0.5:22")
        assertEquals(TrustedHost("10.0.0.5:22", "ssh-ed25519", "SHA256:abc"), found)
    }

    @Test fun `saving an existing alias replaces it`() {
        val s = FileKnownHostsStore(file())
        s.save(TrustedHost("h:22", "ssh-ed25519", "SHA256:old"))
        s.save(TrustedHost("h:22", "ssh-ed25519", "SHA256:new"))
        assertEquals("SHA256:new", FileKnownHostsStore(file()).find("h:22")?.fingerprint)
    }

    @Test fun `aliases are exact so another port is a different host`() {
        val s = FileKnownHostsStore(file())
        s.save(TrustedHost("h:22", "ssh-ed25519", "SHA256:a"))
        assertNull(s.find("h:2222"))
    }

    @Test fun `corrupt lines are ignored instead of failing`() {
        file().writeText("garbage\nh:22\tssh-ed25519\tSHA256:ok\n\tbroken\n")
        assertEquals("SHA256:ok", FileKnownHostsStore(file()).find("h:22")?.fingerprint)
    }

    @Test fun `fields containing separators are rejected on save`() {
        val s = FileKnownHostsStore(file())
        val err = runCatching { s.save(TrustedHost("h\t:22", "t", "f")) }.exceptionOrNull()
        assertEquals(IllegalArgumentException::class, err?.let { it::class })
    }
}
