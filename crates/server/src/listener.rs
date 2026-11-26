//! Proteção contra slowloris (`server.on('connection')` e `headersTimeout` do original): cada IP (fora proxies confiáveis)
//! pode ter no máximo `RAW_PERIP` conexões TCP abertas, e o primeiro cabeçalho HTTP precisa chegar inteiro em 10 s.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};

const HEADER_MS: Duration = Duration::from_secs(10);

/// Conexões abertas por chave de IP.
pub type Counts = Arc<Mutex<HashMap<String, usize>>>;

pub struct Guarded {
    inner: TcpListener,
    counts: Counts,
    max: usize,
    trusted: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub refused: Arc<std::sync::atomic::AtomicU64>,
}

impl std::fmt::Debug for Guarded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guarded").field("max", &self.max).finish_non_exhaustive()
    }
}

impl Guarded {
    pub fn new(inner: TcpListener, max: usize, trusted: Arc<dyn Fn(&str) -> bool + Send + Sync>) -> Self {
        Self { inner, counts: Arc::default(), max, trusted, refused: Arc::default() }
    }
}

/// A conexão aceita: conta para o IP enquanto existir e corta se o cabeçalho demorar.
#[derive(Debug)]
pub struct Conn {
    io: TcpStream,
    key: Option<String>,
    counts: Counts,
    hdr_ok: bool,
    tail: [u8; 4],
    deadline: Pin<Box<tokio::time::Sleep>>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        let Some(k) = &self.key else { return };
        let mut c = self.counts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(n) = c.get_mut(k) {
            *n -= 1;
            if *n == 0 {
                c.remove(k);
            }
        }
    }
}

impl AsyncRead for Conn {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        if !self.hdr_ok && self.deadline.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "cabeçalho lento")));
        }
        let before = buf.filled().len();
        let r = Pin::new(&mut self.io).poll_read(cx, buf);
        if !self.hdr_ok && matches!(r, Poll::Ready(Ok(()))) {
            let new: Vec<u8> = buf.filled()[before..].to_vec();
            for b in new {
                self.tail = [self.tail[1], self.tail[2], self.tail[3], b];
                if &self.tail == b"\r\n\r\n" {
                    self.hdr_ok = true;
                    break;
                }
            }
        }
        r
    }
}

impl AsyncWrite for Conn {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

use std::future::Future;

impl axum::serve::Listener for Guarded {
    type Io = Conn;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (io, addr) = match self.inner.accept().await {
                Ok(x) => x,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    continue;
                }
            };
            // como o Node (noDelay): mensagens pequenas saem na hora
            let _ = io.set_nodelay(true);
            let ip = crate::netutil::norm_ip(&addr.ip().to_string());
            let key = if ip == "?" || (self.trusted)(&ip) {
                None
            } else {
                let k = crate::netutil::rate_key(&ip);
                let mut c = self.counts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let n = c.entry(k.clone()).or_insert(0);
                if *n >= self.max {
                    drop(c);
                    self.refused.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    drop(io);
                    continue;
                }
                *n += 1;
                Some(k)
            };
            let conn = Conn {
                io,
                key,
                counts: self.counts.clone(),
                hdr_ok: false,
                tail: [0; 4],
                deadline: Box::pin(tokio::time::sleep(HEADER_MS)),
            };
            return (conn, addr);
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

/// Endereço de quem conectou, para os tratadores (`ConnectInfo<Peer>`).
#[derive(Debug, Clone, Copy)]
pub struct Peer(pub SocketAddr);

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, Guarded>> for Peer {
    fn connect_info(s: axum::serve::IncomingStream<'_, Guarded>) -> Self {
        Peer(*s.remote_addr())
    }
}
