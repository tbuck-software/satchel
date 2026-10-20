//! HTTP defaults shared by providers, OAuth and app-side integrations.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .tcp_keepalive(Duration::from_secs(60))
}

pub fn blocking_client_builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .tcp_keepalive(Duration::from_secs(60))
}

pub fn client() -> reqwest::Client {
    client_builder()
        .build()
        .expect("HTTP client initialization failed")
}

/// Added to the base deadline of an upload, per byte at this rate: 256 kbit/s,
/// a weak mobile uplink. A send has to survive that, not just Wi-Fi.
const SLOWEST_UPLINK_BYTES_PER_SEC: u64 = 32 * 1024;

/// Base deadline of an HTTP send: TLS, server-side processing and the reply.
pub const UPLOAD_BASE_TIMEOUT: Duration = Duration::from_secs(60);

/// Longest any single upload may take, however large.
pub const UPLOAD_TIMEOUT_CAP: Duration = Duration::from_secs(15 * 60);

/// Deadline for a request that uploads `body_bytes`: `base` plus the time the
/// body needs over the slowest uplink we support, capped at
/// [`UPLOAD_TIMEOUT_CAP`] but never below `base`.
///
/// The flat [`REQUEST_TIMEOUT`] covers the whole exchange, upload included.
/// A few megabytes over a mobile connection never fit into it, and an upload
/// cut off that way failed on every retry for the same reason. IMAP `APPEND`
/// uses this too, with its operation timeout as the base.
pub fn upload_timeout(base: Duration, body_bytes: usize) -> Duration {
    let transfer = Duration::from_secs(body_bytes as u64 / SLOWEST_UPLINK_BYTES_PER_SEC);
    base.saturating_add(transfer)
        .min(UPLOAD_TIMEOUT_CAP.max(base))
}

/// Size of the slices an [`UploadProgress`] body hands to the transport.
const UPLOAD_CHUNK_BYTES: usize = 64 * 1024;

/// Whether the last byte of a request body has been handed to the transport.
///
/// An HTTP server cannot act on a message it has not received in full, so a
/// send that fails before that point delivered nothing and is safe to retry.
/// Only afterwards can a lost reply hide an accepted message. The body is fed
/// to the transport in slices, so the flag flips when the final slice is taken
/// — at most one write buffer before it has actually left, which errs on the
/// side of "may have been delivered".
#[derive(Clone, Debug, Default)]
pub struct UploadProgress(Arc<AtomicBool>);

impl UploadProgress {
    /// A request body over `data` that reports to this tracker. Each call
    /// starts over, so a retried request builds a fresh body.
    pub fn body(&self, data: impl Into<Bytes>) -> reqwest::Body {
        let remaining = data.into();
        self.0.store(remaining.is_empty(), Ordering::Release);
        reqwest::Body::wrap(ProgressBody {
            remaining,
            handed_over: self.0.clone(),
        })
    }

    /// The whole body has been taken by the transport.
    pub fn fully_handed_over(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

struct ProgressBody {
    remaining: Bytes,
    handed_over: Arc<AtomicBool>,
}

impl http_body::Body for ProgressBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        if this.remaining.is_empty() {
            return Poll::Ready(None);
        }
        let chunk = this
            .remaining
            .split_to(this.remaining.len().min(UPLOAD_CHUNK_BYTES));
        if this.remaining.is_empty() {
            this.handed_over.store(true, Ordering::Release);
        }
        Poll::Ready(Some(Ok(http_body::Frame::data(chunk))))
    }

    fn is_end_stream(&self) -> bool {
        self.remaining.is_empty()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        http_body::SizeHint::with_exact(self.remaining.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_timeout_grows_with_the_body_between_base_and_cap() {
        let base = UPLOAD_BASE_TIMEOUT;
        assert_eq!(upload_timeout(base, 0), base);
        assert_eq!(upload_timeout(base, 16 * 1024), base);
        // 5 MiB at 32 KiB/s is 160 s on top of the base.
        assert_eq!(
            upload_timeout(base, 5 * 1024 * 1024),
            base + Duration::from_secs(160)
        );
        assert!(upload_timeout(base, 5 * 1024 * 1024) > REQUEST_TIMEOUT);
        assert_eq!(upload_timeout(base, 1024 * 1024 * 1024), UPLOAD_TIMEOUT_CAP);
        assert_eq!(upload_timeout(base, usize::MAX), UPLOAD_TIMEOUT_CAP);
        // A base above the cap (an IMAP timeout raised by env) is kept.
        let long = UPLOAD_TIMEOUT_CAP * 2;
        assert_eq!(upload_timeout(long, 1024 * 1024 * 1024), long);
        assert_eq!(
            upload_timeout(Duration::from_secs(20), 0),
            Duration::from_secs(20)
        );
    }

    /// Accepts one request, reads its head and — if `read_body` — the whole
    /// body, then never answers.
    async fn stalling_server(read_body: bool) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let mut chunk = vec![0; 64 * 1024];
            let end = loop {
                let n = socket.read(&mut chunk).await.unwrap();
                data.extend_from_slice(&chunk[..n]);
                if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            if read_body {
                let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
                let length: usize = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map(|value| value.trim().parse().unwrap())
                    .expect("sized body");
                let mut read = data.len() - end;
                while read < length {
                    read += socket.read(&mut chunk).await.unwrap();
                }
            }
            std::future::pending::<()>().await;
        });
        (format!("http://{address}"), task)
    }

    async fn send_into(read_body: bool) -> (reqwest::Error, UploadProgress) {
        let (url, server) = stalling_server(read_body).await;
        let upload = UploadProgress::default();
        // Far more than loopback socket buffers hold, so a server that stops
        // reading really does leave part of the body on our side.
        let body = vec![b'x'; 64 * 1024 * 1024];
        let error = client()
            .post(url)
            .timeout(Duration::from_millis(500))
            .body(upload.body(body))
            .send()
            .await
            .unwrap_err();
        server.abort();
        (error, upload)
    }

    #[tokio::test]
    async fn an_upload_cut_off_midway_is_retryable() {
        let (error, upload) = send_into(false).await;
        assert!(error.is_timeout());
        assert!(!upload.fully_handed_over());
        let error = crate::sync::errors::http_submission_transport_failure(
            "test",
            error,
            upload.fully_handed_over(),
        );
        assert!(!error.is_delivery_unknown(), "{error:?}");
        assert!(!error.is_permanent(), "{error:?}");
    }

    #[tokio::test]
    async fn silence_after_a_complete_upload_is_delivery_unknown() {
        let (error, upload) = send_into(true).await;
        assert!(error.is_timeout());
        assert!(upload.fully_handed_over());
        let error = crate::sync::errors::http_submission_transport_failure(
            "test",
            error,
            upload.fully_handed_over(),
        );
        assert!(error.is_delivery_unknown(), "{error:?}");
    }

    #[test]
    fn a_new_body_resets_the_tracker() {
        let upload = UploadProgress::default();
        upload.0.store(true, Ordering::Release);
        let _body = upload.body(vec![1, 2, 3]);
        assert!(!upload.fully_handed_over());
    }

    #[tokio::test]
    async fn request_deadline_covers_a_stalled_response_body() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio::io::BufReader::new(socket);
            let mut request = String::new();
            while !request.ends_with("\r\n\r\n") {
                assert!(socket.read_line(&mut request).await.unwrap() > 0);
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let client = super::client_builder()
            .timeout(std::time::Duration::from_millis(50))
            .build()
            .unwrap();
        let response = client
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap();
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(1), response.bytes()).await;
        server.abort();
        assert!(result.unwrap().unwrap_err().is_timeout());
    }
}
