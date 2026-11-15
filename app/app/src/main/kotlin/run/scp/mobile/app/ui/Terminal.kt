package run.scp.mobile.app.ui

enum class SpecialKey(val bytes: ByteArray) {
    Esc(byteArrayOf(0x1B)),
    Tab(byteArrayOf(0x09)),
    CtrlC(byteArrayOf(0x03)),
    CtrlD(byteArrayOf(0x04)),
    Backspace(byteArrayOf(0x7F)),
    Enter(byteArrayOf(0x0D)),
    Up("\u001B[A".toByteArray()),
    Down("\u001B[B".toByteArray()),
    Right("\u001B[C".toByteArray()),
    Left("\u001B[D".toByteArray()),
}

data class TerminalSize(val cols: Int, val rows: Int)

private const val DEFAULT_COLS = 80
private const val DEFAULT_ROWS = 24
private const val MIN_COLS = 20
private const val MIN_ROWS = 5

/** Whole character cells that fit in the given pixel area. */
fun terminalSize(widthPx: Float, heightPx: Float, cellWidthPx: Float, cellHeightPx: Float): TerminalSize {
    if (cellWidthPx <= 0f || cellHeightPx <= 0f) return TerminalSize(DEFAULT_COLS, DEFAULT_ROWS)
    return TerminalSize(
        (widthPx / cellWidthPx).toInt().coerceAtLeast(MIN_COLS),
        (heightPx / cellHeightPx).toInt().coerceAtLeast(MIN_ROWS),
    )
}
