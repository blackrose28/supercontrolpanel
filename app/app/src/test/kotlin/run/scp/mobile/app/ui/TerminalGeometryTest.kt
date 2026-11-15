package run.scp.mobile.app.ui

import kotlin.test.Test
import kotlin.test.assertEquals

class TerminalGeometryTest {
    @Test fun `divides the area by the cell size`() {
        assertEquals(TerminalSize(80, 24), terminalSize(800f, 480f, 10f, 20f))
    }

    @Test fun `partial cells are dropped`() {
        assertEquals(TerminalSize(79, 23), terminalSize(799f, 479f, 10f, 20f))
    }

    @Test fun `tiny or degenerate areas clamp to a usable minimum`() {
        assertEquals(TerminalSize(20, 5), terminalSize(10f, 10f, 10f, 20f))
    }

    @Test fun `zero cell size falls back to the default`() {
        assertEquals(TerminalSize(80, 24), terminalSize(800f, 480f, 0f, 20f))
    }
}
