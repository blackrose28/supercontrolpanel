//! Terminal inspection, raw/mode restoration guard, TTY prompts, and OS termination signals.
use crate::error::CoreError;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;
use zeroize::{Zeroize, Zeroizing};

const SIG_NONE: u8 = 0;
const SIG_INTERRUPT: u8 = 1;
const SIG_TERMINATE: u8 = 2;

static PENDING_SIGNAL: AtomicU8 = AtomicU8::new(SIG_NONE);
static WATCHER_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Normalized process termination signal observed by foreground CLI commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationSignal {
    Interrupt,
    Terminate,
}

impl TerminationSignal {
    fn from_code(code: u8) -> Option<Self> {
        match code {
            SIG_INTERRUPT => Some(Self::Interrupt),
            SIG_TERMINATE => Some(Self::Terminate),
            _ => None,
        }
    }

    fn as_code(self) -> u8 {
        match self {
            Self::Interrupt => SIG_INTERRUPT,
            Self::Terminate => SIG_TERMINATE,
        }
    }
}

/// Returns `true` when both standard input and standard output are interactive terminals.
pub fn is_interactive_tty() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Captures the active terminal mode on creation and restores it on drop.
pub struct TerminalModeGuard {
    #[cfg(unix)]
    saved_termios: Option<libc::termios>,
    #[cfg(windows)]
    saved_stdin_mode: Option<(usize, u32)>,
    #[cfg(windows)]
    saved_stdout_mode: Option<(usize, u32)>,
}

impl TerminalModeGuard {
    /// Captures the current stdin terminal/console mode when attached to a TTY.
    pub fn capture() -> Self {
        #[cfg(unix)]
        {
            Self {
                saved_termios: capture_unix_termios(),
            }
        }
        #[cfg(windows)]
        {
            Self {
                saved_stdin_mode: capture_windows_mode(STD_INPUT_HANDLE),
                saved_stdout_mode: capture_windows_mode(STD_OUTPUT_HANDLE),
            }
        }
    }

    /// Explicitly restores any captured terminal mode; idempotent across drop.
    pub fn restore(&mut self) {
        #[cfg(unix)]
        if let Some(termios) = self.saved_termios.take() {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &termios);
            }
        }
        #[cfg(windows)]
        {
            if let Some((handle, mode)) = self.saved_stdin_mode.take() {
                unsafe {
                    SetConsoleMode(handle as *mut core::ffi::c_void, mode);
                }
            }
            if let Some((handle, mode)) = self.saved_stdout_mode.take() {
                unsafe {
                    SetConsoleMode(handle as *mut core::ffi::c_void, mode);
                }
            }
        }
    }
}

impl Drop for TerminalModeGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Installs OS interrupt/termination handlers and exposes signal polling until dropped.
pub struct SignalWatcher {
    #[cfg(unix)]
    prev_int: libc::sigaction,
    #[cfg(unix)]
    prev_term: libc::sigaction,
}

impl SignalWatcher {
    /// Installs handlers for SIGINT/SIGTERM (Unix) or Console Ctrl events (Windows).
    pub fn install() -> std::io::Result<Self> {
        if WATCHER_ACTIVE
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "SignalWatcher is already active",
            ));
        }
        PENDING_SIGNAL.store(SIG_NONE, Ordering::SeqCst);
        match install_platform_handlers() {
            Ok(watcher) => Ok(watcher),
            Err(err) => {
                WATCHER_ACTIVE.store(false, Ordering::SeqCst);
                Err(err)
            }
        }
    }

    /// Returns the received termination signal, if one has arrived.
    pub fn signal(&self) -> Option<TerminationSignal> {
        TerminationSignal::from_code(PENDING_SIGNAL.load(Ordering::SeqCst))
    }

    /// Sleeps briefly or until `signal()` becomes populated, returning the signal if set.
    pub fn wait_tick(&self, interval: Duration) -> Option<TerminationSignal> {
        if let Some(sig) = self.signal() {
            return Some(sig);
        }
        std::thread::sleep(interval);
        self.signal()
    }

    /// Records a synthetic signal value (exposed primarily for tests).
    pub fn trigger_for_test(&self, signal: TerminationSignal) {
        PENDING_SIGNAL.store(signal.as_code(), Ordering::SeqCst);
    }
}

impl Drop for SignalWatcher {
    fn drop(&mut self) {
        uninstall_platform_handlers(self);
        PENDING_SIGNAL.store(SIG_NONE, Ordering::SeqCst);
        WATCHER_ACTIVE.store(false, Ordering::SeqCst);
    }
}

#[cfg(unix)]
fn capture_unix_termios() -> Option<libc::termios> {
    if !std::io::stdin().is_terminal() {
        return None;
    }
    let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
    let rc = unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut termios) };
    (rc == 0).then_some(termios)
}

#[cfg(unix)]
extern "C" fn handle_posix_signal(sig: libc::c_int) {
    let code = match sig {
        libc::SIGINT => SIG_INTERRUPT,
        libc::SIGTERM => SIG_TERMINATE,
        _ => SIG_NONE,
    };
    if code != SIG_NONE {
        let _ = PENDING_SIGNAL.compare_exchange(SIG_NONE, code, Ordering::SeqCst, Ordering::SeqCst);
    }
}

#[cfg(unix)]
fn install_platform_handlers() -> std::io::Result<SignalWatcher> {
    let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
    action.sa_sigaction = handle_posix_signal as *const () as libc::sighandler_t;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
    }
    action.sa_flags = libc::SA_RESTART;

    let mut prev_int = unsafe { std::mem::zeroed::<libc::sigaction>() };
    if unsafe { libc::sigaction(libc::SIGINT, &action, &mut prev_int) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut prev_term = unsafe { std::mem::zeroed::<libc::sigaction>() };
    if unsafe { libc::sigaction(libc::SIGTERM, &action, &mut prev_term) } != 0 {
        let err = std::io::Error::last_os_error();
        unsafe {
            libc::sigaction(libc::SIGINT, &prev_int, std::ptr::null_mut());
        }
        return Err(err);
    }
    Ok(SignalWatcher {
        prev_int,
        prev_term,
    })
}

#[cfg(unix)]
fn uninstall_platform_handlers(watcher: &SignalWatcher) {
    unsafe {
        libc::sigaction(libc::SIGINT, &watcher.prev_int, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &watcher.prev_term, std::ptr::null_mut());
    }
}

#[cfg(windows)]
const STD_INPUT_HANDLE: u32 = (-10i32) as u32;
#[cfg(windows)]
const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
#[cfg(windows)]
const CTRL_C_EVENT: u32 = 0;
#[cfg(windows)]
const CTRL_BREAK_EVENT: u32 = 1;
#[cfg(windows)]
const CTRL_CLOSE_EVENT: u32 = 2;
#[cfg(windows)]
const CTRL_LOGOFF_EVENT: u32 = 5;
#[cfg(windows)]
const CTRL_SHUTDOWN_EVENT: u32 = 6;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetStdHandle(n_std_handle: u32) -> *mut core::ffi::c_void;
    fn GetConsoleMode(h_console_handle: *mut core::ffi::c_void, lp_mode: *mut u32) -> i32;
    fn SetConsoleMode(h_console_handle: *mut core::ffi::c_void, dw_mode: u32) -> i32;
    fn SetConsoleCtrlHandler(
        handler_routine: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

#[cfg(windows)]
fn capture_windows_mode(std_handle_id: u32) -> Option<(usize, u32)> {
    let handle = unsafe { GetStdHandle(std_handle_id) };
    if handle.is_null() || handle == (-1isize as *mut core::ffi::c_void) {
        return None;
    }
    let mut mode = 0u32;
    let ok = unsafe { GetConsoleMode(handle, &mut mode) };
    (ok != 0).then_some((handle as usize, mode))
}

#[cfg(windows)]
unsafe extern "system" fn handle_windows_ctrl(ctrl_type: u32) -> i32 {
    let code = match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => SIG_INTERRUPT,
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => SIG_TERMINATE,
        _ => return 0,
    };
    let _ = PENDING_SIGNAL.compare_exchange(SIG_NONE, code, Ordering::SeqCst, Ordering::SeqCst);
    1
}

#[cfg(windows)]
fn install_platform_handlers() -> std::io::Result<SignalWatcher> {
    if unsafe { SetConsoleCtrlHandler(Some(handle_windows_ctrl), 1) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(SignalWatcher {})
}

#[cfg(windows)]
fn uninstall_platform_handlers(_watcher: &SignalWatcher) {
    unsafe {
        SetConsoleCtrlHandler(Some(handle_windows_ctrl), 0);
    }
}

/// Gracefully terminates a foreground child process (`SIGTERM` then `SIGKILL` on Unix).
pub fn terminate_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(250);
        while std::time::Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

const MAX_TTY_INPUT_BYTES: usize = 65_536;

/// Reads a single visible line from the controlling terminal, interruptible by `SignalWatcher`.
pub fn read_tty_line(prompt: &str, watcher: Option<&SignalWatcher>) -> Result<String, CoreError> {
    let raw = read_tty_impl(prompt, true, watcher)?;
    Ok(raw.to_string())
}

/// Reads a single line with terminal echo disabled, always restoring echo on return or interrupt.
pub fn read_tty_secret(
    prompt: &str,
    watcher: Option<&SignalWatcher>,
) -> Result<Zeroizing<String>, CoreError> {
    read_tty_impl(prompt, false, watcher)
}

#[cfg(unix)]
struct UnixTtyEchoGuard {
    fd: std::os::fd::RawFd,
    saved: Option<libc::termios>,
}

#[cfg(unix)]
impl UnixTtyEchoGuard {
    fn disable_echo(fd: std::os::fd::RawFd) -> Result<Self, CoreError> {
        let mut current = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut current) } != 0 {
            return Err(CoreError::capability_unavailable(
                "tty_required",
                "Interactive terminal (/dev/tty) does not support echo control",
                "run_in_tty_or_use_file",
            ));
        }
        let saved = current;
        current.c_lflag &= !(libc::ECHO | libc::ECHOE | libc::ECHOK);
        current.c_lflag |= libc::ECHONL;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &current) } != 0 {
            return Err(CoreError::operation_failed(
                "tty_mode_failed",
                "Failed to disable terminal echo for secret prompt",
                false,
                "run_in_tty_or_use_file",
            ));
        }
        Ok(Self {
            fd,
            saved: Some(saved),
        })
    }
}

#[cfg(unix)]
impl Drop for UnixTtyEchoGuard {
    fn drop(&mut self) {
        if let Some(saved) = self.saved.take() {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, &saved);
            }
        }
    }
}

#[cfg(unix)]
fn read_tty_impl(
    prompt: &str,
    echo: bool,
    watcher: Option<&SignalWatcher>,
) -> Result<Zeroizing<String>, CoreError> {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::fd::AsRawFd;

    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| {
            CoreError::capability_unavailable(
                "tty_required",
                "Interactive terminal (/dev/tty) is required",
                "run_in_tty_or_use_file",
            )
        })?;
    if !tty.is_terminal() {
        return Err(CoreError::capability_unavailable(
            "tty_required",
            "Controlling terminal (/dev/tty) is not interactive",
            "run_in_tty_or_use_file",
        ));
    }
    let fd = tty.as_raw_fd();
    let _echo_guard = if echo {
        None
    } else {
        Some(UnixTtyEchoGuard::disable_echo(fd)?)
    };

    tty.write_all(prompt.as_bytes()).map_err(|e| {
        CoreError::operation_failed(
            "tty_write_failed",
            format!("Failed to write terminal prompt: {e}"),
            false,
            "run_in_tty_or_use_file",
        )
    })?;
    let _ = tty.flush();

    let mut buf = Zeroizing::new(Vec::<u8>::with_capacity(128));
    loop {
        if watcher.and_then(SignalWatcher::signal).is_some() {
            if !echo {
                let _ = tty.write_all(b"\n");
                let _ = tty.flush();
            }
            return Err(CoreError::interrupted(
                "Interrupted while waiting for terminal input",
            ));
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd, 1, 50) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(CoreError::operation_failed(
                "tty_read_failed",
                format!("Terminal poll failed: {err}"),
                false,
                "run_in_tty_or_use_file",
            ));
        }
        if rc == 0 {
            continue;
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0
            && pfd.revents & libc::POLLIN == 0
        {
            return Err(CoreError::invalid_arguments(
                "tty_eof",
                "Terminal input closed before completion",
            ));
        }
        let mut byte = [0u8; 1];
        let n = unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(CoreError::operation_failed(
                "tty_read_failed",
                format!("Terminal read failed: {err}"),
                false,
                "run_in_tty_or_use_file",
            ));
        }
        if n == 0 {
            if buf.is_empty() {
                return Err(CoreError::invalid_arguments(
                    "tty_eof",
                    "Terminal input closed unexpectedly",
                ));
            }
            break;
        }
        if byte[0] == b'\n' {
            break;
        }
        if buf.len() >= MAX_TTY_INPUT_BYTES {
            byte.zeroize();
            return Err(CoreError::invalid_arguments(
                "input_too_large",
                "Terminal input exceeds maximum allowed size",
            ));
        }
        buf.push(byte[0]);
        byte.zeroize();
    }
    while buf.last() == Some(&b'\r') {
        buf.pop();
    }
    if !echo {
        // Ensure cursor advances cleanly on virtual PTYs that ignore ECHONL.
        let _ = tty.write_all(b"\r\n");
        let _ = tty.flush();
    }
    let text = String::from_utf8(std::mem::take(&mut *buf)).map_err(|_| {
        CoreError::invalid_arguments("invalid_utf8", "Terminal input must be valid UTF-8")
    })?;
    Ok(Zeroizing::new(text))
}

#[cfg(windows)]
fn read_tty_impl(
    prompt: &str,
    echo: bool,
    watcher: Option<&SignalWatcher>,
) -> Result<Zeroizing<String>, CoreError> {
    use std::fs::OpenOptions;
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::io::AsRawHandle;

    const ENABLE_ECHO_INPUT: u32 = 0x0004;

    if !is_interactive_tty() {
        return Err(CoreError::capability_unavailable(
            "tty_required",
            "Interactive console is required",
            "run_in_tty_or_use_file",
        ));
    }
    let mut conout = OpenOptions::new()
        .write(true)
        .open("CONOUT$")
        .map_err(|_| {
            CoreError::capability_unavailable(
                "tty_required",
                "Interactive console output is unavailable",
                "run_in_tty_or_use_file",
            )
        })?;
    let conin = OpenOptions::new().read(true).open("CONIN$").map_err(|_| {
        CoreError::capability_unavailable(
            "tty_required",
            "Interactive console input is unavailable",
            "run_in_tty_or_use_file",
        )
    })?;

    struct WinEchoGuard {
        handle: *mut core::ffi::c_void,
        saved_mode: Option<u32>,
    }
    impl Drop for WinEchoGuard {
        fn drop(&mut self) {
            if let Some(mode) = self.saved_mode.take() {
                unsafe {
                    SetConsoleMode(self.handle, mode);
                }
            }
        }
    }

    let handle = conin.as_raw_handle();
    let _guard = if echo {
        None
    } else {
        let mut mode = 0u32;
        if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
            return Err(CoreError::capability_unavailable(
                "tty_required",
                "Interactive console does not support echo control",
                "run_in_tty_or_use_file",
            ));
        }
        let saved = mode;
        if unsafe { SetConsoleMode(handle, mode & !ENABLE_ECHO_INPUT) } == 0 {
            return Err(CoreError::operation_failed(
                "tty_mode_failed",
                "Failed to disable console echo for secret prompt",
                false,
                "run_in_tty_or_use_file",
            ));
        }
        Some(WinEchoGuard {
            handle,
            saved_mode: Some(saved),
        })
    };

    conout.write_all(prompt.as_bytes()).map_err(|e| {
        CoreError::operation_failed("tty_write_failed", e.to_string(), false, "run_in_tty")
    })?;
    let _ = conout.flush();
    if watcher.and_then(SignalWatcher::signal).is_some() {
        return Err(CoreError::interrupted(
            "Interrupted while waiting for console input",
        ));
    }
    let mut line = Zeroizing::new(String::new());
    let mut reader = BufReader::new(conin);
    let n = reader.read_line(&mut line).map_err(|e| {
        CoreError::operation_failed("tty_read_failed", e.to_string(), false, "run_in_tty")
    })?;
    if watcher.and_then(SignalWatcher::signal).is_some() {
        return Err(CoreError::interrupted(
            "Interrupted while waiting for console input",
        ));
    }
    if n == 0 {
        return Err(CoreError::invalid_arguments(
            "tty_eof",
            "Console input closed unexpectedly",
        ));
    }
    if line.len() > MAX_TTY_INPUT_BYTES {
        return Err(CoreError::invalid_arguments(
            "input_too_large",
            "Console input exceeds maximum allowed size",
        ));
    }
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    if !echo {
        let _ = conout.write_all(b"\r\n");
        let _ = conout.flush();
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_watcher_captures_and_resets_state() {
        let watcher = SignalWatcher::install().expect("install signal watcher");
        assert_eq!(watcher.signal(), None);
        watcher.trigger_for_test(TerminationSignal::Interrupt);
        assert_eq!(watcher.signal(), Some(TerminationSignal::Interrupt));
        drop(watcher);

        let watcher2 = SignalWatcher::install().expect("reinstall signal watcher");
        assert_eq!(watcher2.signal(), None);
        let mut guard = TerminalModeGuard::capture();
        guard.restore();
    }
}
