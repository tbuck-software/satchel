//! HTTP defaults shared by providers, OAuth and app-side integrations.

use std::time::Duration;

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

#[cfg(test)]
mod tests {
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
