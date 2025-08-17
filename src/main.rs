use std::sync::Arc;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::server::conn::{http1, http2};
use hyper_util::rt::{TokioIo, TokioExecutor};
use tokio::net::TcpStream;
use tokio::net::TcpListener;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tracing::{info, debug, Level};

use conex::{AppState, Bind, ProxyService};

const HTTP2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

struct BufferedStream {
    stream: TcpStream,
    buffer: Option<Vec<u8>>,
    buffer_pos: usize,
}

impl BufferedStream {
    fn new(stream: TcpStream, buffer: Vec<u8>) -> Self {
        Self {
            stream,
            buffer: Some(buffer),
            buffer_pos: 0,
        }
    }
}

impl AsyncRead for BufferedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // First, serve any buffered data
        if let Some(buffer) = &self.buffer {
            if self.buffer_pos < buffer.len() {
                let remaining = &buffer[self.buffer_pos..];
                let to_copy = std::cmp::min(remaining.len(), buf.remaining());
                buf.put_slice(&remaining[..to_copy]);
                
                let new_pos = self.buffer_pos + to_copy;
                let buffer_len = buffer.len();
                
                // Update position after using the buffer reference
                self.buffer_pos = new_pos;
                
                // If we've consumed all buffer data, remove it
                if self.buffer_pos >= buffer_len {
                    self.buffer = None;
                }
                
                return Poll::Ready(Ok(()));
            }
        }
        
        // Buffer exhausted, delegate to inner stream
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for BufferedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

async fn detect_http2_preface(mut stream: TcpStream) -> Result<(bool, BufferedStream), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::io::AsyncReadExt;
    
    let mut buf = [0; 24];
    
    // Read the first 24 bytes to check for HTTP/2 preface
    match tokio::time::timeout(std::time::Duration::from_millis(500), stream.read_exact(&mut buf)).await {
        Ok(Ok(_)) => {
            let is_http2 = buf == HTTP2_PREFACE;
            debug!("Protocol detection: {}", if is_http2 { "HTTP/2" } else { "HTTP/1.1" });
            
            // Create buffered stream with the read data
            let buffered_stream = BufferedStream::new(stream, buf.to_vec());
            Ok((is_http2, buffered_stream))
        }
        Ok(Err(_)) | Err(_) => {
            // If we can't read 24 bytes or timeout, assume HTTP/1.1
            debug!("Protocol detection: HTTP/1.1 (fallback)");
            let buffered_stream = BufferedStream::new(stream, Vec::new());
            Ok((false, buffered_stream))
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    let state = Arc::new(AppState::new().await);

    let bind = Bind::default();
    let addr = format!("{}:{}", bind.host.unwrap(), bind.port.unwrap());
    
    let listener = TcpListener::bind(&addr).await?;
    info!("listening on {}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();

        tokio::task::spawn(async move {
            let service = ProxyService::new(state);
            
            // Detect HTTP version
            match detect_http2_preface(stream).await {
                Ok((is_http2, detected_stream)) => {
                    let io = TokioIo::new(detected_stream);
                    
                    if is_http2 {
                        // Serve HTTP/2 connection
                        info!("Serving HTTP/2 connection");
                        if let Err(err) = http2::Builder::new(TokioExecutor::new())
                            .serve_connection(io, service)
                            .await
                        {
                            tracing::error!("Error serving HTTP/2 connection: {:?}", err);
                        }
                    } else {
                        // Serve HTTP/1 connection
                        debug!("Serving HTTP/1.1 connection");
                        if let Err(err) = http1::Builder::new()
                            .preserve_header_case(true)
                            .title_case_headers(true)
                            .serve_connection(io, service)
                            .await
                        {
                            tracing::error!("Error serving HTTP/1 connection: {:?}", err);
                        }
                    }
                }
                Err(err) => {
                    tracing::error!("Error detecting HTTP version: {:?}", err);
                }
            }
        });
    }
}