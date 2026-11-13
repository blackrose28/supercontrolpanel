//! Linux closure observation without a blocking stdin read. This bounds only the
//! observer's idle polling/join, not the operation's native cleanup or file I/O.

use std::{
    fs::File,
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
};

pub(super) struct EofWatch {
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<(), String>>>,
}

impl EofWatch {
    pub(super) fn start(cancelled: Arc<AtomicBool>) -> Result<Self, String> {
        let stdin = std::io::stdin();
        let fd = stdin
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| "duplicate stdin for EOF monitoring")?;
        Self::from_file(File::from(fd), cancelled)
    }

    fn from_file(file: File, cancelled: Arc<AtomicBool>) -> Result<Self, String> {
        let metadata = file
            .metadata()
            .map_err(|_| "inspect stdin for EOF monitoring")?;
        let mut watch = Self {
            stopped: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        let kind = metadata.file_type();
        if kind.is_char_device()
            && std::fs::metadata("/dev/null").is_ok_and(|null| null.rdev() == metadata.rdev())
        {
            cancelled.store(true, Ordering::Release);
            return Ok(watch);
        }
        if !kind.is_fifo() && !kind.is_socket() {
            return Err(
                "stdin EOF monitoring requires a pipe, socket, or /dev/null; no child was launched"
                    .into(),
            );
        }
        if closed(&file, 0)? {
            cancelled.store(true, Ordering::Release);
            return Ok(watch);
        }
        let stopped = Arc::clone(&watch.stopped);
        watch.worker = Some(
            std::thread::Builder::new()
                .name("scp-stdin-eof".into())
                .spawn(move || {
                    let result = observe(file, &stopped, &cancelled);
                    if result.is_err() {
                        cancelled.store(true, Ordering::Release);
                    }
                    result
                })
                .map_err(|_| "start stdin EOF observer")?,
        );
        Ok(watch)
    }

    pub(super) fn finish(&mut self) -> Result<(), String> {
        self.stopped.store(true, Ordering::Release);
        match self.worker.take() {
            Some(worker) => worker.join().map_err(|_| "stdin EOF observer panicked")?,
            None => Ok(()),
        }
    }
}

impl Drop for EofWatch {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn observe(file: File, stopped: &AtomicBool, cancelled: &AtomicBool) -> Result<(), String> {
    while !stopped.load(Ordering::Acquire) && !cancelled.load(Ordering::Acquire) {
        if closed(&file, 20)? {
            cancelled.store(true, Ordering::Release);
            break;
        }
    }
    Ok(())
}

fn closed(file: &File, timeout_ms: i32) -> Result<bool, String> {
    // HUP is unconditional for pipes; RDHUP also observes a socket's read-half
    // closure. Deliberately omit POLLIN: ignored input cannot cause a busy loop.
    let mut fd = libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLRDHUP,
        revents: 0,
    };
    // SAFETY: one initialized descriptor, owned by file for this entire call.
    let result = unsafe { libc::poll(&mut fd, 1, timeout_ms) };
    if result < 0 {
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err("poll stdin EOF failed".into());
    }
    if fd.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
        return Err("stdin EOF descriptor failed".into());
    }
    Ok(fd.revents & (libc::POLLHUP | libc::POLLRDHUP) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::Shutdown,
        os::{fd::OwnedFd, unix::net::UnixStream},
        time::{Duration, Instant},
    };

    #[test]
    fn socket_half_close_cancels_without_reading_pending_bytes() {
        let (socket, mut peer) = UnixStream::pair().unwrap();
        let mut retained = socket.try_clone().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut watch =
            EofWatch::from_file(File::from(OwnedFd::from(socket)), Arc::clone(&cancelled)).unwrap();
        peer.write_all(b"unused bytes").unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !cancelled.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "socket EOF not observed");
            std::thread::sleep(Duration::from_millis(5));
        }
        watch.finish().unwrap();
        assert!(watch.worker.is_none());
        let mut bytes = Vec::new();
        retained.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"unused bytes");
    }

    #[test]
    fn idle_observer_joins_without_waiting_for_input_or_setting_cancellation() {
        let (socket, _peer) = UnixStream::pair().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut watch =
            EofWatch::from_file(File::from(OwnedFd::from(socket)), Arc::clone(&cancelled)).unwrap();
        let start = Instant::now();
        watch.finish().unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(watch.worker.is_none());
        assert!(!cancelled.load(Ordering::Acquire));
        watch.finish().unwrap();
    }
}
