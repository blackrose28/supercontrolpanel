package run.scp.mobile.app.terminal

import kotlin.test.Test
import kotlin.test.assertEquals

class TerminalBufferTest {
    private fun TerminalBuffer.feed(s: String) = append(s.toByteArray(Charsets.UTF_8))

    @Test fun `plain text with CRLF splits lines`() {
        val b = TerminalBuffer()
        b.feed("one\r\ntwo\r\nthree")
        assertEquals(listOf("one", "two", "three"), b.lines())
    }

    @Test fun `carriage return overwrites from line start`() {
        val b = TerminalBuffer()
        b.feed("hello\rHE")
        assertEquals(listOf("HEllo"), b.lines())
    }

    @Test fun `backspace moves cursor left and erase to end clears`() {
        val b = TerminalBuffer()
        b.feed("ls -x\b\u001B[K")
        b.feed("a")
        assertEquals(listOf("ls -a"), b.lines())
    }

    @Test fun `SGR colors and OSC titles are stripped`() {
        val b = TerminalBuffer()
        b.feed("\u001B[1;32muser@host\u001B[0m:\u001B]0;title\u0007~$ ")
        assertEquals(listOf("user@host:~$ "), b.lines())
    }

    @Test fun `escape sequence split across chunks is still consumed`() {
        val b = TerminalBuffer()
        b.feed("a\u001B[3")
        b.feed("1mb")
        assertEquals(listOf("ab"), b.lines())
    }

    @Test fun `multibyte utf8 split across chunks decodes`() {
        val b = TerminalBuffer()
        val bytes = "é".toByteArray(Charsets.UTF_8)
        b.append(byteArrayOf(bytes[0]))
        b.append(byteArrayOf(bytes[1]))
        assertEquals(listOf("é"), b.lines())
    }

    @Test fun `cursor left and right and erase whole line`() {
        val b = TerminalBuffer()
        b.feed("abc\u001B[2Dx\u001B[1C!")
        assertEquals(listOf("axc!"), b.lines())
        b.feed("\u001B[2K")
        assertEquals(listOf(""), b.lines())
    }

    @Test fun `erase to start of line blanks up to cursor`() {
        val b = TerminalBuffer()
        b.feed("abcd\u001B[2D\u001B[1K")
        assertEquals(listOf("   d"), b.lines())
    }

    @Test fun `tab advances to next stop of eight`() {
        val b = TerminalBuffer()
        b.feed("ab\tc")
        assertEquals(listOf("ab      c"), b.lines())
    }

    @Test fun `scrollback is capped dropping oldest lines`() {
        val b = TerminalBuffer(maxLines = 3)
        b.feed("1\n2\n3\n4\n5")
        assertEquals(listOf("3", "4", "5"), b.lines())
    }

    @Test fun `bell and other control characters are ignored`() {
        val b = TerminalBuffer()
        b.feed("a\u0007\u0001b")
        assertEquals(listOf("ab"), b.lines())
    }

    @Test fun `unknown CSI with intermediate params is dropped without eating text`() {
        val b = TerminalBuffer()
        b.feed("x\u001B[?2004hy\u001B[?2004lz")
        assertEquals(listOf("xyz"), b.lines())
    }
}
