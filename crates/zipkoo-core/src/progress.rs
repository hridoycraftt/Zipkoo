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
    last_emit: std::time::Instant,
}

impl<'a> Reporter<'a> {
    pub fn new(
        on_progress: &'a mut dyn FnMut(&Progress),
        cancel: Option<&'a AtomicBool>,
        total: u64,
    ) -> Self {
        Self {
            on_progress,
            cancel,
            state: Progress {
                done_bytes: 0,
                total_bytes: total,
                current: String::new(),
            },
            last_emit: std::time::Instant::now(),
        }
    }
    /// Updates are throttled by time (about 12 per second), so a 100 GB job and a folder of
    /// 100,000 tiny files both stay smooth without flooding the UI.
    fn maybe_emit(&mut self) {
        if self.last_emit.elapsed() >= std::time::Duration::from_millis(80) {
            self.last_emit = std::time::Instant::now();
            (self.on_progress)(&self.state);
        }
    }
    pub fn set_current(&mut self, name: &str) {
        self.state.current.clear();
        self.state.current.push_str(name);
        self.maybe_emit();
    }
    pub fn add(&mut self, n: u64) -> io::Result<()> {
        self.state.done_bytes += n;
        self.maybe_emit();
        if self
            .cancel
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(false)
        {
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
