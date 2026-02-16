use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A wrapper around a `Write` implementation that tracks the number of bytes written.
/// When the byte count exceeds a threshold, further writes are no-ops.
#[allow(dead_code)]
pub struct SizeLimitedWriter<W: Write> {
    inner: W,
    bytes_written: Arc<AtomicU64>,
    max_bytes: u64,
    limit_exceeded: bool,
}


#[allow(dead_code)]
impl<W: Write> SizeLimitedWriter<W> {
    pub fn new(inner: W, max_bytes: u64) -> Self {
        Self {
            inner,
            bytes_written: Arc::new(AtomicU64::new(0)),
            max_bytes,
            limit_exceeded: false,
        }
    }

    /// Returns the number of bytes written so far
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    /// Returns true if the size limit has been exceeded
    pub fn limit_exceeded(&self) -> bool {
        self.limit_exceeded
    }

    /// Get a handle to check bytes written from another thread
    pub fn bytes_written_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.bytes_written)
    }
}

impl<W: Write> Write for SizeLimitedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.limit_exceeded {
            // Silently drop writes once limit is exceeded
            return Ok(buf.len());
        }

        let current = self.bytes_written.load(Ordering::Relaxed);
        if current >= self.max_bytes {
            self.limit_exceeded = true;
            return Ok(buf.len());
        }

        let remaining = self.max_bytes - current;
        let to_write = buf.len().min(remaining as usize);

        let written = self.inner.write(&buf[..to_write])?;
        self.bytes_written
            .fetch_add(written as u64, Ordering::Relaxed);

        if self.bytes_written.load(Ordering::Relaxed) >= self.max_bytes {
            self.limit_exceeded = true;
        }

        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_limited_writer_under_limit() {
        let mut buf = Vec::new();
        let mut writer = SizeLimitedWriter::new(&mut buf, 100);

        writer.write_all(b"Hello").unwrap();
        assert_eq!(writer.bytes_written(), 5);
        assert!(!writer.limit_exceeded());
        assert_eq!(buf, b"Hello");
    }

    #[test]
    fn test_size_limited_writer_at_limit() {
        let mut buf = Vec::new();
        let mut writer = SizeLimitedWriter::new(&mut buf, 5);

        writer.write_all(b"Hello").unwrap();
        assert_eq!(writer.bytes_written(), 5);
        assert!(writer.limit_exceeded());

        // Further writes should be silently dropped
        writer.write_all(b" World").unwrap();
        assert_eq!(writer.bytes_written(), 5);
        assert_eq!(buf, b"Hello");
    }

    #[test]
    fn test_size_limited_writer_exceeds_limit() {
        let mut buf = Vec::new();
        let mut writer = SizeLimitedWriter::new(&mut buf, 10);

        writer.write_all(b"Hello World!").unwrap();
        assert_eq!(writer.bytes_written(), 10);
        assert!(writer.limit_exceeded());
        assert_eq!(buf, b"Hello Worl");
    }
}
