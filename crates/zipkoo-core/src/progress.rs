use serde::Serialize;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

/// Progress for a running pack or extract job.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub current: String,
}

/// Callback plus cancel flag shared by long jobs.
pub struct Reporter<'a> {
    pub on_progress: &'a mut dyn FnMut(&Progress),
    pub cancel: Option<&'a AtomicBool>,
    pub state: Progress,
    last_emit: u64,
}

impl<'a> Reporter<'a> {
    pub fn new(on_progress: &'a mut dyn FnMut(&Progress), cancel: Option<&'a AtomicBool>, total: u64) -> Self {
        Self {
            on_progress,
            cancel,
            state: Progress { done_bytes: 0, total_bytes: total, current: String::new() },
            last_emit: 0,
        }
    }
    pub fn set_current(&mut self, name: &str) {
        self.state.current = name.to_string();
        (self.on_progress)(&self.state);
    }
    pub fn add(&mut self, n: u64) -> io::Result<()> {
        self.state.done_bytes += n;
        if self.state.done_bytes - self.last_emit >= 1 << 20 {
            self.last_emit = self.state.done_bytes;
            (self.on_progress)(&self.state);
        }
        if self.cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
            return Err(io::Error::new(io::ErrorKind::Other, "zipkoo: cancelled"));
        }
        Ok(())
    }
    pub fn finish(&mut self) {
        (self.on_progress)(&self.state);
    }
}

/// Reader that reports bytes read.
pub struct CountingReader<'r, 'a, R> {
    pub inner: R,
    pub reporter: &'r mut Reporter<'a>,
}

impl<R: Read> Read for CountingReader<'_, '_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.reporter.add(n as u64)?;
        Ok(n)
    }
}

pub fn noop() -> impl FnMut(&Progress) {
    |_p: &Progress| {}
}
