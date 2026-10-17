//! Socket defaults for persistent mail connections.

use std::time::Duration;

pub fn enable_mail_keepalive(stream: &tokio::net::TcpStream) -> std::io::Result<()> {
    socket2::SockRef::from(stream).set_tcp_keepalive(
        &socket2::TcpKeepalive::new()
            .with_time(Duration::from_secs(60))
            .with_interval(Duration::from_secs(15)),
    )
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn mail_sockets_enable_tcp_keepalive() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        super::enable_mail_keepalive(&stream).unwrap();
        assert!(socket2::SockRef::from(&stream).keepalive().unwrap());
    }
}
