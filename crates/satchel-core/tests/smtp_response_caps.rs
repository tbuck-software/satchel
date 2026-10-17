//! Exercise both vendored SMTP readers against a hostile peer over real sockets.
use lettre::transport::smtp::{
    client::{AsyncSmtpConnection, SmtpConnection},
    extension::ClientId,
};
use std::{io::Write, net::TcpListener, sync::mpsc, thread, time::Duration};

fn hostile_peer(
    reply: Vec<u8>,
    hold_open: bool,
) -> (
    std::net::SocketAddr,
    thread::JoinHandle<()>,
    mpsc::Sender<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release, wait) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let _ = socket.write_all(&reply);
        if hold_open {
            let _ = wait.recv_timeout(Duration::from_secs(15));
        }
    });
    (address, server, release)
}

fn oversized_line() -> Vec<u8> {
    format!("220 {}\r\n", "x".repeat(1000)).into_bytes()
}

fn oversized_response() -> Vec<u8> {
    let mut reply = format!("220-{}\r\n", "x".repeat(990)).repeat(101);
    reply.push_str("220 done\r\n");
    reply.into_bytes()
}

#[test]
fn synchronous_reader_rejects_oversized_replies() {
    for (reply, hold_open, message) in [
        (oversized_line(), false, "SMTP response line too long"),
        (oversized_response(), false, "SMTP response too large"),
        (
            format!("220 {}", "x".repeat(1000)).into_bytes(),
            true,
            "SMTP response line too long",
        ),
    ] {
        let (address, server, release) = hostile_peer(reply, hold_open);
        let result = SmtpConnection::connect(
            address,
            Some(Duration::from_secs(5)),
            &ClientId::default(),
            None,
            None,
        );
        drop(release);
        server.join().unwrap();
        let error = result.err().expect("oversized reply must fail");
        assert!(error.is_response(), "{error:?}");
        assert!(error.to_string().contains(message), "{error}");
    }
}

#[tokio::test]
async fn asynchronous_reader_rejects_oversized_replies() {
    for (reply, hold_open, message) in [
        (oversized_line(), false, "SMTP response line too long"),
        (oversized_response(), false, "SMTP response too large"),
        (
            format!("220 {}", "x".repeat(1000)).into_bytes(),
            true,
            "SMTP response line too long",
        ),
    ] {
        let (address, server, release) = hostile_peer(reply, hold_open);
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            AsyncSmtpConnection::connect_tokio1(
                address,
                Some(Duration::from_secs(5)),
                &ClientId::default(),
                None,
                None,
            ),
        )
        .await
        .expect("reply must be bounded");
        drop(release);
        server.join().unwrap();
        let error = result.err().expect("oversized reply must fail");
        assert!(error.is_response(), "{error:?}");
        assert!(error.to_string().contains(message), "{error}");
    }
}
