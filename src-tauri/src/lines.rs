//! Reading process output line by line with a cap per line.
//!
//! `BufReader::lines()` / `read_until(b'\n')` grow the line buffer without limit: a process that
//! prints megabytes without a newline (a minified bundle, a binary on stdout, a progress bar that
//! only uses `\r`) would make the app allocate all of it. Here a line keeps at most `max` bytes;
//! the rest of that line is read and discarded, and the line ends with [`TRUNCATED_MARK`].

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Per-line cap for logs (dev servers, installs, setup steps).
pub const MAX_LOG_LINE_BYTES: usize = 16 * 1024;
pub const TRUNCATED_MARK: &str = "… [linha truncada]";

pub struct BoundedLines<R> {
    reader: R,
    max: usize,
    buf: Vec<u8>,
    eof: bool,
}

impl<R: AsyncBufRead + Unpin> BoundedLines<R> {
    pub fn new(reader: R, max: usize) -> Self {
        Self { reader, max, buf: Vec::new(), eof: false }
    }

    /// The next line without its `\n` / `\r\n`, lossily decoded; `None` at end of stream.
    pub async fn next_line(&mut self) -> std::io::Result<Option<String>> {
        if self.eof {
            return Ok(None);
        }
        self.buf.clear();
        let mut truncated = false;
        let mut read_any = false;
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                self.eof = true;
                break;
            }
            read_any = true;
            let newline = available.iter().position(|&b| b == b'\n');
            let chunk = &available[..newline.unwrap_or(available.len())];
            let room = self.max.saturating_sub(self.buf.len());
            if chunk.len() > room {
                truncated = true;
            }
            self.buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
            let consumed = newline.map_or(available.len(), |i| i + 1);
            self.reader.consume(consumed);
            if newline.is_some() {
                break;
            }
        }
        if !read_any {
            return Ok(None);
        }
        if !truncated && self.buf.last() == Some(&b'\r') {
            self.buf.pop();
        }
        let mut line = String::from_utf8_lossy(&self.buf).into_owned();
        if truncated {
            // The cut may split a multi-byte character: drop the replacement char it leaves.
            line.truncate(line.trim_end_matches('\u{FFFD}').len());
            line.push(' ');
            line.push_str(TRUNCATED_MARK);
        }
        Ok(Some(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    async fn collect(input: &[u8], max: usize) -> Vec<String> {
        // A small BufReader capacity exercises lines that span many fill_buf() calls.
        let mut lines = BoundedLines::new(BufReader::with_capacity(64, input), max);
        let mut out = Vec::new();
        while let Some(line) = lines.next_line().await.unwrap() {
            out.push(line);
        }
        out
    }

    #[tokio::test]
    async fn reads_lines_like_lines() {
        assert_eq!(collect(b"a\nbb\r\n\nlast", 100).await, ["a", "bb", "", "last"]);
        assert_eq!(collect(b"", 100).await, Vec::<String>::new());
        assert_eq!(collect(b"x\n", 100).await, ["x"]);
    }

    #[tokio::test]
    async fn caps_a_2mb_line_without_newline() {
        // Security audit (0.1.3): 2 MB with no `\n` must not become a 2 MB String.
        let huge = vec![b'x'; 2 * 1024 * 1024];
        let input = [huge.as_slice(), b"\ndepois\n"].concat();
        let lines = collect(&input, MAX_LOG_LINE_BYTES).await;
        assert_eq!(lines.len(), 2, "the stream goes on after the truncated line");
        assert_eq!(lines[0].len(), MAX_LOG_LINE_BYTES + 1 + TRUNCATED_MARK.len());
        assert!(lines[0].starts_with("xxx") && lines[0].ends_with(TRUNCATED_MARK));
        assert_eq!(lines[1], "depois");

        // Same at the end of the stream (no final newline).
        let lines = collect(&huge, MAX_LOG_LINE_BYTES).await;
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with(TRUNCATED_MARK));
    }

    #[tokio::test]
    async fn truncation_never_leaves_half_a_character() {
        // "é" is 2 bytes; a 5-byte cap lands in the middle of the third one.
        let lines = collect("ééééé\n".as_bytes(), 5).await;
        assert_eq!(lines, [format!("éé {TRUNCATED_MARK}")]);
    }
}
