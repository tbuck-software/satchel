/// Ceiling for one message assembled out of an mbox stream.
///
/// The stream holds one message at a time, so the limit belongs on the message
/// rather than on the file — a legitimate mbox export is arbitrarily large.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

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
    /// A leading RFC header identifies a bare EML, whose body may contain
    /// unescaped From lines without becoming another mbox entry.
    bare: bool,
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
            bare: false,
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
        Ok(self.next_raw_entry().await?.map(|raw| {
            let text = String::from_utf8_lossy(&raw);
            if raw.starts_with(b"From ") {
                text.trim_matches(['\r', '\n']).to_string()
            } else {
                text.into_owned()
            }
        }))
    }

    /// Read MIME bytes without changing encodings, line endings or signatures.
    /// Bare EML files are not subject to mbox framing or From-line unescaping.
    pub async fn next_message_bytes(&mut self) -> Result<Option<Vec<u8>>, String> {
        let Some(raw) = self.next_raw_entry().await? else {
            return Ok(None);
        };
        if !raw.starts_with(b"From ") {
            return Ok(Some(raw));
        }
        let start = raw
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(raw.len(), |at| at + 1);
        let message = &raw[start..];
        // Our writer adds two LF bytes independently of the original ending.
        let message = message.strip_suffix(b"\n\n").unwrap_or(message);
        let mut decoded = Vec::with_capacity(message.len());
        for line in message.split_inclusive(|byte| *byte == b'\n') {
            let unquoted = line
                .iter()
                .position(|byte| *byte != b'>')
                .unwrap_or(line.len());
            if unquoted > 0 && line[unquoted..].starts_with(b"From ") {
                decoded.extend_from_slice(&line[1..]);
            } else {
                decoded.extend_from_slice(line);
            }
        }
        Ok(Some(decoded))
    }

    async fn next_raw_entry(&mut self) -> Result<Option<Vec<u8>>, String> {
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
            if self.bytes_read == read as u64 {
                self.bare = line
                    .iter()
                    .position(|byte| *byte == b':')
                    .is_some_and(|end| {
                        end > 0 && line[..end].iter().all(|byte| byte.is_ascii_graphic())
                    });
            }
            if !self.bare && line.starts_with(b"From ") {
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
    fn take_entry(&mut self, at_end: bool) -> Option<Vec<u8>> {
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
            if !at_end
                || !(raw.windows(2).any(|part| part == b"\n\n")
                    || raw.windows(4).any(|part| part == b"\r\n\r\n"))
            {
                return None;
            }
            return Some(raw);
        }
        if raw.iter().all(|byte| matches!(byte, b'\r' | b'\n')) {
            return None;
        }
        Some(raw)
    }
}

pub fn strip_mbox_from_line(raw: &str) -> &str {
    if !raw.starts_with("From ") {
        return raw;
    }
    raw.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
}

/// Write reversible mboxrd quoting while retaining the original MIME bytes.
/// The envelope separator is supplied by the caller.
pub async fn write_message<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    raw: &[u8],
) -> std::io::Result<u64> {
    use tokio::io::AsyncWriteExt;
    let mut written = 0;
    for line in raw.split_inclusive(|byte| *byte == b'\n') {
        let unquoted = line
            .iter()
            .position(|byte| *byte != b'>')
            .unwrap_or(line.len());
        if line[unquoted..].starts_with(b"From ") {
            writer.write_all(b">").await?;
            written += 1;
        }
        writer.write_all(line).await?;
        written += line.len() as u64;
    }
    writer.write_all(b"\n\n").await?;
    Ok(written + 2)
}
