//! Single-use, password-authenticated LAN channel. Each purpose gets its own
//! AEAD key and exactly one message. Discovery records carry no credentials.
use super::{failed, CommandResult};
use ring::{aead, hkdf};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const LIMIT: usize = 128 * 1024;
const SENDER: &[u8] = b"satchel/setup/v1/sender";
const RECEIVER: &[u8] = b"satchel/setup/v1/receiver";

pub(super) struct Channel {
    socket: TcpStream,
    secret: hkdf::Prk,
}

impl Channel {
    pub(super) async fn open(
        mut socket: TcpStream,
        code: &str,
        sender: bool,
    ) -> CommandResult<Self> {
        let password = Password::new(code.as_bytes());
        let a = Identity::new(SENDER);
        let b = Identity::new(RECEIVER);
        let (pake, outgoing) = if sender {
            Spake2::<Ed25519Group>::start_a(&password, &a, &b)
        } else {
            Spake2::<Ed25519Group>::start_b(&password, &a, &b)
        };
        socket.write_all(&outgoing).await.map_err(|_| failed())?;
        let mut incoming = vec![0; outgoing.len()];
        socket
            .read_exact(&mut incoming)
            .await
            .map_err(|_| failed())?;
        let key = pake.finish(&incoming).map_err(|_| failed())?;
        let secret =
            hkdf::Salt::new(hkdf::HKDF_SHA256, b"satchel/account-transfer/v1").extract(&key);
        let mut channel = Self { socket, secret };
        if sender {
            if !channel.read(b"receiver confirmation").await?.is_empty() {
                return Err(failed());
            }
        } else {
            channel.write(b"receiver confirmation", Vec::new()).await?;
        }
        Ok(channel)
    }

    fn key(&self, purpose: &[u8]) -> CommandResult<aead::LessSafeKey> {
        let labels = [purpose];
        let okm = self
            .secret
            .expand(&labels, &aead::AES_256_GCM)
            .map_err(|_| failed())?;
        Ok(aead::LessSafeKey::new(aead::UnboundKey::from(okm)))
    }

    pub(super) async fn write(&mut self, purpose: &[u8], mut body: Vec<u8>) -> CommandResult<()> {
        if body.len() + aead::AES_256_GCM.tag_len() > LIMIT {
            return Err(failed());
        }
        self.key(purpose)?
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key([0; 12]),
                aead::Aad::from(purpose),
                &mut body,
            )
            .map_err(|_| failed())?;
        self.socket
            .write_u32(body.len() as u32)
            .await
            .map_err(|_| failed())?;
        self.socket.write_all(&body).await.map_err(|_| failed())
    }

    pub(super) async fn read(&mut self, purpose: &[u8]) -> CommandResult<Vec<u8>> {
        let len = self.socket.read_u32().await.map_err(|_| failed())? as usize;
        if !(aead::AES_256_GCM.tag_len()..=LIMIT).contains(&len) {
            return Err(failed());
        }
        let mut body = vec![0; len];
        self.socket
            .read_exact(&mut body)
            .await
            .map_err(|_| failed())?;
        let plain = self
            .key(purpose)?
            .open_in_place(
                aead::Nonce::assume_unique_for_key([0; 12]),
                aead::Aad::from(purpose),
                &mut body,
            )
            .map_err(|_| failed())?;
        let len = plain.len();
        body.truncate(len);
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn credentials_require_the_receivers_code() {
        for matches in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let receiver = tokio::spawn(async move {
                let socket = listener.accept().await.unwrap().0;
                let mut channel = Channel::open(socket, "012345", false).await.unwrap();
                channel.read(b"accounts").await
            });
            let socket = TcpStream::connect(address).await.unwrap();
            let result =
                Channel::open(socket, if matches { "012345" } else { "987654" }, true).await;
            if matches {
                result
                    .unwrap()
                    .write(b"accounts", b"fixture".to_vec())
                    .await
                    .unwrap();
                assert_eq!(receiver.await.unwrap().unwrap(), b"fixture");
            } else {
                assert!(result.is_err());
                assert!(receiver.await.unwrap().is_err());
            }
        }
    }
}
