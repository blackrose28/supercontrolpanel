package run.scp.mobile.app.terminal

import java.nio.ByteBuffer
import java.nio.CharBuffer
import java.nio.charset.CodingErrorAction

/**
 * A deliberately small line-oriented terminal renderer: it keeps a scrollback of text
 * lines and one cursor column, honours CR/LF/BS/TAB and the CSI cursor-left/right and
 * erase-in-line sequences that shells use for line editing, and drops every other
 * escape sequence (colours, titles, modes). Full-screen programs (vim, htop) are not
 * supported. Input may arrive in arbitrary chunks, including mid-sequence and mid-UTF-8.
 */
class TerminalBuffer(private val maxLines: Int = 2000) {
    private val rows = ArrayList<StringBuilder>().apply { add(StringBuilder()) }
    private var col = 0

    private enum class Mode { TEXT, ESC, CSI, OSC, OSC_ESC }
    private var mode = Mode.TEXT
    private val params = StringBuilder()

    private val decoder = Charsets.UTF_8.newDecoder()
        .onMalformedInput(CodingErrorAction.REPLACE)
        .onUnmappableCharacter(CodingErrorAction.REPLACE)
    private var pending = ByteArray(0)

    init {
        require(maxLines >= 1) { "maxLines must be >= 1" }
    }

    fun lines(): List<String> = rows.map { it.toString() }

    fun append(bytes: ByteArray) {
        val input = ByteBuffer.wrap(pending + bytes)
        val out = CharBuffer.allocate(input.remaining() + 1)
        decoder.decode(input, out, false)
        pending = ByteArray(input.remaining()).also { input.get(it) }
        out.flip()
        while (out.hasRemaining()) feed(out.get())
    }

    private fun feed(c: Char) {
        when (mode) {
            Mode.TEXT -> text(c)
            Mode.ESC -> when (c) {
                '[' -> { mode = Mode.CSI; params.setLength(0) }
                ']' -> mode = Mode.OSC
                else -> mode = Mode.TEXT // two-char sequence (e.g. ESC =, ESC 7): drop
            }
            Mode.CSI -> when (c) {
                in '@'..'~' -> { csi(c); mode = Mode.TEXT }
                else -> params.append(c) // parameter and intermediate bytes (0x20-0x3F)
            }
            Mode.OSC -> when (c) {
                '\u0007' -> mode = Mode.TEXT
                '\u001B' -> mode = Mode.OSC_ESC
            }
            Mode.OSC_ESC -> mode = if (c == '\\') Mode.TEXT else Mode.OSC // ST ends OSC
        }
    }

    private fun text(c: Char) {
        when (c) {
            '\u001B' -> mode = Mode.ESC
            '\r' -> col = 0
            '\n' -> newLine()
            '\b' -> if (col > 0) col--
            '\t' -> { val next = (col / 8 + 1) * 8; while (col < next) put(' ') }
            else -> if (c >= ' ' && c != '\u007F') put(c)
        }
    }

    private fun put(c: Char) {
        val line = rows.last()
        while (line.length < col) line.append(' ')
        if (col < line.length) line.setCharAt(col, c) else line.append(c)
        col++
    }

    private fun newLine() {
        rows.add(StringBuilder())
        col = 0
        if (rows.size > maxLines) rows.subList(0, rows.size - maxLines).clear()
    }

    private fun csi(final: Char) {
        val p = params.toString()
        if (p.isNotEmpty() && !p[0].isDigit() && p[0] != ';') return // private modes like ?2004
        val n = p.substringBefore(';').toIntOrNull()
        val line = rows.last()
        when (final) {
            'C' -> col += maxOf(n ?: 1, 1)
            'D' -> col = maxOf(col - maxOf(n ?: 1, 1), 0)
            'K' -> when (n ?: 0) {
                0 -> if (col < line.length) line.setLength(col)
                1 -> for (i in 0 until minOf(col + 1, line.length)) line.setCharAt(i, ' ') // through the cursor cell
                2 -> line.setLength(0)
            }
        }
    }
}
