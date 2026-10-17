/// Ceiling for one message assembled out of an mbox stream.
///
/// The stream holds one message at a time, so the limit belongs on the message
/// rather than on the file — a legitimate mbox export is arbitrarily large.
pub const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

/// One `From `-separated mbox entry at a time.
///
/// Entry boundaries and trimming match what the previous whole-file split
/// produced, including that content before the first `From ` line is dropped —
/// unless the file has no `From ` line at all, in which case the whole file is
/// one message (a bare `.eml`).
pub struct MboxEntries<R> {
    reader: R,
    ceiling: usize,
    current: Vec<u8>,
    /// Set once the entry passed the ceiling; the rest of it is discarded.
    oversize: bool,
    /// A `From ` separator has been seen, so what came before it is preamble.
    separated: bool,
    done: bool,
    /// Entries dropped for exceeding [`MAX_MESSAGE_BYTES`].
    pub dropped: usize,
    /// Bytes taken off the stream, for the progress estimate.
    pub bytes_read: u64,
}

impl<R: tokio::io::AsyncBufRead + Unpin> MboxEntries<R> {
    pub fn new(reader: R) -> Self {
        Self::with_ceiling(reader, MAX_MESSAGE_BYTES)
    }

    /// A ceiling other than [`MAX_MESSAGE_BYTES`], so a test can reach it
    /// without allocating 32 MiB.
    pub fn with_ceiling(reader: R, ceiling: usize) -> Self {
        Self {
            reader,
            ceiling,
            current: Vec::new(),
            oversize: false,
            separated: false,
            done: false,
            dropped: 0,
            bytes_read: 0,
        }
    }

    /// The next entry, or `None` at the end of the file.
    ///
    /// Bytes are decoded per message, so a file that is not valid UTF-8 costs
    /// one lossy copy of one message rather than of the whole file.
    pub async fn next_entry(&mut self) -> Result<Option<String>, String> {
        use tokio::io::AsyncBufReadExt;

        loop {
            if self.done {
                return Ok(None);
            }
            let mut line = Vec::new();
            let read = self
                .reader
                .read_until(b'\n', &mut line)
                .await
                .map_err(|err| err.to_string())?;
            self.bytes_read += read as u64;
            if read == 0 {
                self.done = true;
                if let Some(entry) = self.take_entry(true) {
                    return Ok(Some(entry));
                }
                return Ok(None);
            }
            if line.starts_with(b"From ") {
                let previous = self.take_entry(false);
                self.separated = true;
                self.push(&line);
                if let Some(entry) = previous {
                    return Ok(Some(entry));
                }
                continue;
            }
            self.push(&line);
        }
    }

    fn push(&mut self, line: &[u8]) {
        if self.oversize {
            return;
        }
        if self.current.len() + line.len() > self.ceiling {
            self.oversize = true;
            self.current = Vec::new();
            return;
        }
        self.current.extend_from_slice(line);
    }

    /// Close off the entry being assembled. `at_end` allows the whole-file
    /// fallback, which only applies when no separator was ever seen.
    fn take_entry(&mut self, at_end: bool) -> Option<String> {
        let raw = std::mem::take(&mut self.current);
        let separated = self.separated;
        let oversize = std::mem::replace(&mut self.oversize, false);
        if oversize {
            // A preamble that never becomes an entry is not a dropped message.
            if separated || at_end {
                self.dropped += 1;
            }
            return None;
        }
        if !separated {
            // Preamble: a message only if the file holds nothing else.
            let text = String::from_utf8_lossy(&raw);
            if !at_end || !text.contains("\n\n") {
                return None;
            }
            return Some(text.into_owned());
        }
        let text = String::from_utf8_lossy(&raw);
        let trimmed = text.trim_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return None;
        }
        Some(trimmed.to_string())
    }
}

pub fn strip_mbox_from_line(raw: &str) -> &str {
    if !raw.starts_with("From ") {
        return raw;
    }
    raw.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
}
