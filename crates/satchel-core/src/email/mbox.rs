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
    /// Still checking the first line for an RFC header name followed by a colon.
    header_pending: bool,
    /// The first five bytes of this line have already been classified.
    in_line: bool,
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
            header_pending: true,
            in_line: false,
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

        // Only the line prefix needs lookahead. The remainder goes straight
        // from the reader into the bounded message buffer, or is discarded.
        // Parser storage is O(ceiling) plus these five bytes, even for a line
        // of arbitrary length. The caller owns the reader's buffer capacity.
        let mut prefix = [0; 5];
        let mut prefix_len = 0;
        loop {
            if self.done {
                return Ok(None);
            }
            let available = self
                .reader
                .fill_buf()
                .await
                .map_err(|err| err.to_string())?;
            if available.is_empty() {
                // A final line shorter than "From " has no newline to flush it.
                push_bounded(
                    &mut self.current,
                    &mut self.oversize,
                    self.ceiling,
                    &prefix[..prefix_len],
                );
                self.done = true;
                return Ok(self.take_entry(true));
            }
            let mut read = memchr::memchr(b'\n', available).map_or(available.len(), |at| at + 1);
            if !self.in_line {
                read = read.min(prefix.len() - prefix_len);
            }
            let chunk = &available[..read];
            let ends_line = chunk.ends_with(b"\n");
            if self.header_pending {
                // This also works for a header name longer than the ceiling:
                // classification needs no copy and continues after discarding.
                if let Some(at) = chunk
                    .iter()
                    .position(|byte| *byte == b':' || !byte.is_ascii_graphic())
                {
                    self.bare = chunk[at] == b':' && (self.bytes_read > 0 || at > 0);
                    self.header_pending = false;
                }
            }
            if self.in_line {
                push_bounded(&mut self.current, &mut self.oversize, self.ceiling, chunk);
            } else {
                prefix[prefix_len..prefix_len + read].copy_from_slice(chunk);
                prefix_len += read;
            }
            self.reader.consume(read);
            self.bytes_read += read as u64;
            if self.in_line {
                self.in_line = !ends_line;
                continue;
            }
            if prefix_len < prefix.len() && !ends_line {
                continue;
            }
            let previous = if !self.bare && prefix[..prefix_len] == *b"From " {
                let previous = self.take_entry(false);
                self.separated = true;
                previous
            } else {
                None
            };
            push_bounded(
                &mut self.current,
                &mut self.oversize,
                self.ceiling,
                &prefix[..prefix_len],
            );
            prefix_len = 0;
            self.in_line = !ends_line;
            if let Some(entry) = previous {
                return Ok(Some(entry));
            }
        }
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

/// Append only while the entry fits. Vec growth is amortized, so capacity can
/// exceed the ceiling by a constant factor, but never depends on line length.
fn push_bounded(current: &mut Vec<u8>, oversize: &mut bool, ceiling: usize, chunk: &[u8]) {
    if *oversize {
        return;
    }
    if chunk.len() > ceiling - current.len() {
        *oversize = true;
        *current = Vec::new();
        return;
    }
    current.extend_from_slice(chunk);
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
