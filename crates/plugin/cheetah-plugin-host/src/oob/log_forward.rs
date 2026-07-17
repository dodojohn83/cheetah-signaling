//! Buffered, length-limited forwarding of a child process's stdout/stderr.
//!
//! Each stream is read line-by-line up to `max_line_len`; longer lines are
//! truncated and a marker appended so a misbehaving plugin cannot exhaust host
//! memory with a newline-free flood.

use super::log_sanitize::sanitize_log_line;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout};
use tokio::sync::oneshot;
use tracing::{info, warn};

/// Forward captured stdout/stderr until both streams reach EOF or `shutdown` fires.
pub async fn forward_logs(
    plugin_name: String,
    stdout: ChildStdout,
    stderr: ChildStderr,
    mut shutdown: oneshot::Receiver<()>,
    max_line_len: usize,
) {
    let mut stdout_reader = BufReader::new(stdout);
    let mut stderr_reader = BufReader::new(stderr);
    let mut stdout_buf: Vec<u8> = Vec::new();
    let mut stderr_buf: Vec<u8> = Vec::new();
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut stdout_in_pem = false;
    let mut stderr_in_pem = false;

    loop {
        if stdout_done && stderr_done {
            break;
        }
        tokio::select! {
            _ = &mut shutdown => break,
            line = next_limited_line(&mut stdout_reader, &mut stdout_buf, max_line_len), if !stdout_done => match line {
                Ok(Some((line, truncated))) => {
                    if truncated {
                        warn!(plugin = %plugin_name, stream = "stdout", "plugin log line exceeded max length and was truncated");
                    }
                    let sanitized = redact(&line, truncated, &mut stdout_in_pem);
                    info!(plugin = %plugin_name, stream = "stdout", "{sanitized}");
                }
                Ok(None) => stdout_done = true,
                Err(e) => {
                    warn!(plugin = %plugin_name, stream = "stdout", error = %e, "log read failed");
                    stdout_done = true;
                }
            },
            line = next_limited_line(&mut stderr_reader, &mut stderr_buf, max_line_len), if !stderr_done => match line {
                Ok(Some((line, truncated))) => {
                    if truncated {
                        warn!(plugin = %plugin_name, stream = "stderr", "plugin log line exceeded max length and was truncated");
                    }
                    let sanitized = redact(&line, truncated, &mut stderr_in_pem);
                    warn!(plugin = %plugin_name, stream = "stderr", "{sanitized}");
                }
                Ok(None) => stderr_done = true,
                Err(e) => {
                    warn!(plugin = %plugin_name, stream = "stderr", error = %e, "log read failed");
                    stderr_done = true;
                }
            },
        }
    }
}

fn redact(line: &str, truncated: bool, in_pem_block: &mut bool) -> String {
    let trimmed = line.trim_start();
    if trimmed.starts_with("-----BEGIN") {
        // If the line was truncated, the remainder (and any END marker)
        // was discarded, so do not enter multi-line redaction mode.
        if truncated && !trimmed.contains("-----END") {
            return "[REDACTED PEM]".to_string();
        }
        // A PEM block may be printed on a single line; only stay in
        // multi-line redaction mode if the END marker is not on this line.
        *in_pem_block = !trimmed.contains("-----END");
        return "[REDACTED PEM]".to_string();
    }
    if *in_pem_block {
        if trimmed.starts_with("-----END") {
            *in_pem_block = false;
        }
        return "[REDACTED PEM]".to_string();
    }
    sanitize_log_line(line)
}

async fn skip_until_newline<R: AsyncBufRead + Unpin + ?Sized>(
    reader: &mut R,
) -> std::io::Result<()> {
    loop {
        let data = reader.fill_buf().await?;
        if data.is_empty() {
            return Ok(());
        }
        if let Some(pos) = data.iter().position(|&b| b == b'\n') {
            reader.consume(pos + 1);
            return Ok(());
        }
        let len = data.len();
        reader.consume(len);
    }
}

async fn next_limited_line<R: AsyncBufRead + Unpin + ?Sized>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max_len: usize,
) -> std::io::Result<Option<(String, bool)>> {
    if max_len == 0 {
        return Ok(None);
    }

    loop {
        let data = reader.fill_buf().await?;
        if data.is_empty() {
            if buf.is_empty() {
                return Ok(None);
            }
            return Ok(Some(take_line(buf, false)));
        }

        if let Some(pos) = data.iter().position(|&b| b == b'\n') {
            let mut take = pos;
            let truncated = if buf.len() + take > max_len {
                take = max_len.saturating_sub(buf.len());
                true
            } else {
                false
            };
            buf.extend_from_slice(&data[..take]);
            reader.consume(pos + 1);
            return Ok(Some(take_line(buf, truncated)));
        }

        let available = data.len();
        if buf.len() + available > max_len {
            let take = max_len.saturating_sub(buf.len());
            buf.extend_from_slice(&data[..take]);
            reader.consume(available);
            skip_until_newline(reader).await?;
            return Ok(Some(take_line(buf, true)));
        }

        buf.extend_from_slice(data);
        reader.consume(available);
    }
}

fn take_line(buf: &mut Vec<u8>, truncated: bool) -> (String, bool) {
    let bytes = std::mem::take(buf);
    let mut line = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        line.push_str(" [truncated]");
    }
    (line, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncated_begin_does_not_stick_in_pem_mode() {
        let mut in_pem = false;
        let line = "-----BEGIN CERTIFICATE-----";
        assert_eq!(redact(line, true, &mut in_pem), "[REDACTED PEM]");
        assert!(!in_pem);
        // Subsequent normal log lines are not redacted.
        assert_eq!(redact("hello world", false, &mut in_pem), "hello world");
    }

    #[test]
    fn multi_line_pem_is_redacted_until_end() {
        let mut in_pem = false;
        assert_eq!(
            redact("-----BEGIN CERTIFICATE-----", false, &mut in_pem),
            "[REDACTED PEM]"
        );
        assert!(in_pem);
        assert_eq!(
            redact("base64encodeddata", false, &mut in_pem),
            "[REDACTED PEM]"
        );
        assert!(in_pem);
        assert_eq!(
            redact("-----END CERTIFICATE-----", false, &mut in_pem),
            "[REDACTED PEM]"
        );
        assert!(!in_pem);
    }

    #[test]
    fn single_line_pem_does_not_stick() {
        let mut in_pem = false;
        let line = "-----BEGIN CERTIFICATE----- ... -----END CERTIFICATE-----";
        assert_eq!(redact(line, false, &mut in_pem), "[REDACTED PEM]");
        assert!(!in_pem);
    }
}
