package run.scp.mobile.app.tofu

import java.io.File

/** A host key the user has explicitly trusted, keyed by the server's real host:port. */
data class TrustedHost(val alias: String, val keyType: String, val fingerprint: String)

interface KnownHostsStore {
    fun find(alias: String): TrustedHost?
    fun save(host: TrustedHost)
}

/**
 * Tab-separated `alias<TAB>keyType<TAB>fingerprint` lines, rewritten atomically
 * (temp file + rename) on every save. Unparseable lines are skipped on read.
 */
class FileKnownHostsStore(private val file: File) : KnownHostsStore {
    private val lock = Any()

    override fun find(alias: String): TrustedHost? = synchronized(lock) { readAll()[alias] }

    override fun save(host: TrustedHost) {
        require(listOf(host.alias, host.keyType, host.fingerprint).none { it.isEmpty() || '\t' in it || '\n' in it || '\r' in it }) {
            "known-hosts fields must be non-empty and free of tabs/newlines"
        }
        synchronized(lock) {
            val all = readAll().toMutableMap()
            all[host.alias] = host
            file.absoluteFile.parentFile?.mkdirs()
            val tmp = File(file.absolutePath + ".tmp")
            tmp.writeText(all.values.joinToString("") { "${it.alias}\t${it.keyType}\t${it.fingerprint}\n" })
            if (!tmp.renameTo(file)) {
                // renameTo can fail over an existing file on some platforms.
                file.delete()
                check(tmp.renameTo(file)) { "could not update ${file.name}" }
            }
        }
    }

    private fun readAll(): Map<String, TrustedHost> {
        if (!file.exists()) return emptyMap()
        val out = LinkedHashMap<String, TrustedHost>()
        for (line in file.readLines()) {
            val p = line.split('\t')
            if (p.size == 3 && p.none { it.isEmpty() }) out[p[0]] = TrustedHost(p[0], p[1], p[2])
        }
        return out
    }
}
