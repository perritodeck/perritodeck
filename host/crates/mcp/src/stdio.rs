//! The stdio transport: one JSON-RPC message per line on stdin, replies on stdout.

use std::{io, sync::Arc};

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::{
    engine::{Server, Session},
    jsonrpc::RpcError,
    Backend, DEFAULT_MAX_BODY,
};

/// Serve MCP on this process's stdin and stdout until stdin closes.
pub async fn serve_stdio(backend: Arc<dyn Backend>) -> io::Result<()> {
    serve_stream(backend, tokio::io::stdin(), tokio::io::stdout()).await
}

/// Serve MCP over any byte stream with stdio framing (newline-delimited JSON).
/// Returns when `input` ends.
pub async fn serve_stream<R, W>(
    backend: Arc<dyn Backend>,
    input: R,
    mut output: W,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let server = Server::new(backend);
    let mut session = Session::new();
    let mut input = BufReader::new(input);
    while let Some(line) = read_line(&mut input, DEFAULT_MAX_BODY).await? {
        let reply = match line {
            Line::Text(text) => {
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                server.handle_text(text, &mut session).await
            }
            Line::TooLong => Some(
                RpcError::invalid_request(format!("message longer than {DEFAULT_MAX_BODY} bytes"))
                    .response(serde_json::Value::Null)
                    .to_string(),
            ),
        };
        if let Some(mut reply) = reply {
            reply.push('\n');
            output.write_all(reply.as_bytes()).await?;
            output.flush().await?;
        }
    }
    Ok(())
}

pub(crate) enum Line {
    Text(String),
    /// Longer than the limit; the rest of it was skipped.
    TooLong,
}

/// Read one line of at most `max` bytes (without the newline). `None` at the end.
pub(crate) async fn read_line<R: AsyncBufRead + Unpin>(
    r: &mut R,
    max: usize,
) -> io::Result<Option<Line>> {
    let mut buf = Vec::new();
    let mut too_long = false;
    loop {
        let chunk = r.fill_buf().await?;
        if chunk.is_empty() {
            if buf.is_empty() && !too_long {
                return Ok(None);
            }
            break;
        }
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        if !too_long {
            if buf.len() + take > max + 1 {
                too_long = true;
                buf = Vec::new();
            } else {
                buf.extend_from_slice(&chunk[..take]);
            }
        }
        r.consume(take);
        if done {
            break;
        }
    }
    if too_long {
        return Ok(Some(Line::TooLong));
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    Ok(Some(Line::Text(String::from_utf8_lossy(&buf).into_owned())))
}
