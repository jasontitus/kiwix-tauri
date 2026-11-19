#![forbid(unsafe_code)]

#[cfg(all(feature = "reference", feature = "candidate"))]
compile_error!("the two binaries use separate feature sets");

#[cfg(feature = "candidate")]
mod tokens;

use std::{
    net::{Ipv4Addr, UdpSocket},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use quinn::{ClientConfig, Endpoint, ServerConfig};
use rustls::{
    CipherSuite, DigitallySignedStruct, Error, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{self, CryptoProvider},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime, pem::PemObject},
    quic::Suite,
    version::TLS13,
};
use std::io::Write;

const ALPN: &[u8] = b"localswarm-v1";
const SUITES: [CipherSuite; 3] = [
    CipherSuite::TLS13_AES_128_GCM_SHA256,
    CipherSuite::TLS13_AES_256_GCM_SHA384,
    CipherSuite::TLS13_CHACHA20_POLY1305_SHA256,
];

fn provider(suite: CipherSuite) -> Result<(Arc<CryptoProvider>, Suite)> {
    #[cfg(feature = "reference")]
    let mut provider = crypto::ring::default_provider();
    #[cfg(feature = "candidate")]
    let mut provider = oxiquic_crypto::quic_crypto_provider();

    let initial = provider
        .cipher_suites
        .iter()
        .find(|cs| cs.suite() == CipherSuite::TLS13_AES_128_GCM_SHA256)
        .and_then(|cs| cs.tls13().and_then(|tls| tls.quic_suite()))
        .context("AES-128 QUIC Initial suite")?;
    provider.cipher_suites.retain(|s| s.suite() == suite);
    ensure!(
        provider.cipher_suites.len() == 1,
        "requested unique suite {suite:?}"
    );
    provider
        .kx_groups
        .retain(|group| group.name() == rustls::NamedGroup::X25519);
    ensure!(
        provider.kx_groups.len() == 1,
        "one common X25519 group required"
    );
    Ok((Arc::new(provider), initial))
}

struct Identity {
    leaf: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

impl Identity {
    fn load(cert: &[u8], key: &[u8]) -> Result<Self> {
        Ok(Self {
            leaf: CertificateDer::from_pem_slice(cert)?,
            key: PrivateKeyDer::from_pem_slice(key)?,
        })
    }
}

#[derive(Debug)]
struct ExactPin {
    leaf: CertificateDer<'static>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for ExactPin {
    fn verify_server_cert(
        &self,
        leaf: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, Error> {
        if leaf.as_ref() != self.leaf.as_ref() || !intermediates.is_empty() {
            return Err(Error::General("probe certificate pin mismatch".into()));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        crypto::verify_tls12_signature(
            msg,
            cert,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        crypto::verify_tls13_signature(
            msg,
            cert,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn client_config(suite: CipherSuite, id: &Identity) -> Result<ClientConfig> {
    let (provider, initial) = provider(suite)?;
    let pin = Arc::new(ExactPin {
        leaf: id.leaf.clone(),
        provider: provider.clone(),
    });
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(pin)
        .with_client_auth_cert(vec![id.leaf.clone()], id.key.clone_key())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let config = quinn::crypto::rustls::QuicClientConfig::with_initial(Arc::new(tls), initial)?;
    Ok(ClientConfig::new(Arc::new(config)))
}

fn server_config(suite: CipherSuite, id: &Identity) -> Result<ServerConfig> {
    let (provider, initial) = provider(suite)?;
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&TLS13])?
        .with_no_client_auth()
        .with_single_cert(vec![id.leaf.clone()], id.key.clone_key())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let config = quinn::crypto::rustls::QuicServerConfig::with_initial(Arc::new(tls), initial)?;
    #[cfg(feature = "reference")]
    return Ok(ServerConfig::with_crypto(Arc::new(config)));
    #[cfg(feature = "candidate")]
    return tokens::server(Arc::new(config));
}

fn check_alpn(connection: &quinn::Connection) -> Result<()> {
    let data = connection.handshake_data().context("handshake result")?;
    let details = data
        .downcast::<quinn::crypto::rustls::HandshakeData>()
        .map_err(|_| anyhow::anyhow!("unexpected handshake data type"))?;
    ensure!(details.protocol.as_deref() == Some(ALPN), "unexpected ALPN");
    Ok(())
}

fn endpoint(server: Option<ServerConfig>) -> Result<Endpoint> {
    #[cfg(feature = "reference")]
    let config = quinn::EndpointConfig::default();
    #[cfg(feature = "candidate")]
    let config = tokens::endpoint()?;
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(Endpoint::new(
        config,
        server,
        socket,
        Arc::new(quinn::TokioRuntime),
    )?)
}

fn payload(size: usize) -> Vec<u8> {
    (0..size as u64)
        .map(|index| ((index ^ (index >> 11)) % 251) as u8)
        .collect()
}

async fn server(
    suite: CipherSuite,
    id: &Identity,
    retry: bool,
    wrong_pin: bool,
    bytes: &[u8],
) -> Result<()> {
    let server = endpoint(Some(server_config(suite, id)?))?;
    println!("READY {}", server.local_addr()?.port());
    std::io::stdout().flush()?;
    let mut incoming = server.accept().await.context("first incoming")?;
    if retry {
        ensure!(
            !incoming.remote_address_validated(),
            "first address already validated"
        );
        incoming.retry()?;
        incoming = server.accept().await.context("retry incoming")?;
        ensure!(
            incoming.remote_address_validated(),
            "retry token did not validate address"
        );
        println!("RETRY verified");
    }
    let conn = incoming.await;
    if wrong_pin {
        ensure!(conn.is_err(), "wrong-pin peer authenticated");
        println!("REJECTED server");
        server.wait_idle().await;
        return Ok(());
    }
    let conn = conn?;
    check_alpn(&conn)?;
    let (mut send, mut recv) = conn.accept_bi().await?;
    ensure!(recv.read_to_end(4).await? == b"read", "bad control bytes");
    send.write_all(bytes).await?;
    send.finish()?;
    ensure!(send.stopped().await?.is_none(), "peer stopped stream");
    println!("SENT {}", bytes.len());
    server.close(0u32.into(), b"probe complete");
    server.wait_idle().await;
    Ok(())
}

async fn client(
    suite: CipherSuite,
    id: &Identity,
    port: u16,
    wrong_pin: bool,
    expected: &[u8],
) -> Result<()> {
    let mut client = endpoint(None)?;
    client.set_default_client_config(client_config(suite, id)?);
    let start = Instant::now();
    let result = client
        .connect(
            (Ipv4Addr::LOCALHOST, port).into(),
            "paired.kiwix-quic.invalid",
        )?
        .await;
    if wrong_pin {
        let Err(error) = result else {
            anyhow::bail!("wrong pin connected");
        };
        ensure!(
            error.to_string().contains("probe certificate pin mismatch"),
            "unexpected error: {error}"
        );
        println!("REJECTED client");
        client.wait_idle().await;
        return Ok(());
    }
    let conn = result?;
    check_alpn(&conn)?;
    let hs = start.elapsed().as_secs_f64() * 1000.0;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(b"read").await?;
    send.finish()?;
    let received = recv.read_to_end(expected.len()).await?;
    let complete = start.elapsed().as_secs_f64() * 1000.0;
    ensure!(received == expected, "payload mismatch");
    println!("READ {} {hs:.6} {complete:.6}", received.len());
    // Keep the connection alive while the server observes the stream ACK.
    let _ = conn.closed().await;
    client.wait_idle().await;
    Ok(())
}

// Independently compare the fixed-size mask path with the reference using
// public deterministic key and nonce material. No network is involved.
fn masks() -> Result<()> {
    #[cfg(feature = "candidate")]
    use rustls::quic::Algorithm;
    #[cfg(feature = "candidate")]
    let key = oxiquic_crypto::quic::CHACHA20_POLY1305.header_protection_key([0x42; 32].into());
    #[cfg(feature = "reference")]
    let key = ring::aead::quic::HeaderProtectionKey::new(&ring::aead::quic::CHACHA20, &[0x42; 32])
        .map_err(|_| anyhow::anyhow!("reference mask key"))?;
    for counter in [0u32, 1, 7, 0x1234_5678, u32::MAX - 1, u32::MAX] {
        let mut sample = [0x71u8; 16];
        sample[..4].copy_from_slice(&counter.to_le_bytes());
        #[cfg(feature = "candidate")]
        {
            let mut first = 0x03;
            let mut pn = [0u8; 4];
            key.encrypt_in_place(&sample, &mut first, &mut pn)?;
            println!("{:02x} {:02x?}", first ^ 0x03, pn);
        }
        #[cfg(feature = "reference")]
        {
            let mask = key
                .new_mask(&sample)
                .map_err(|_| anyhow::anyhow!("reference mask"))?;
            println!("{:02x} {:02x?}", mask[0] & 0x1f, &mask[1..]);
        }
    }
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["masks"] {
        return masks();
    }
    ensure!(
        args.len() == 6,
        "server/client fixture-dir suite size retry/port normal/wrong"
    );
    let suite = *SUITES
        .get(args[2].parse::<usize>()?)
        .context("cipher suite index")?;
    let size = args[3].parse::<usize>()?;
    ensure!((1..=8 * 1024 * 1024).contains(&size), "fixture length");
    let wrong = args[5] == "wrong";
    ensure!(args[5] == "normal" || wrong, "fixture selection");
    let stem = if args[0] == "server" && wrong {
        "wrong"
    } else {
        "paired"
    };
    let cert = std::fs::read(Path::new(&args[1]).join(format!("{stem}.cert.pem")))?;
    let key = std::fs::read(Path::new(&args[1]).join(format!("{stem}.key.pem")))?;
    let id = Identity::load(&cert, &key)?;
    let bytes = payload(size);
    let role = async {
        match args[0].as_str() {
            "server" => server(suite, &id, args[4].parse::<bool>()?, wrong, &bytes).await,
            "client" => client(suite, &id, args[4].parse::<u16>()?, wrong, &bytes).await,
            _ => anyhow::bail!("invalid role"),
        }
    };
    tokio::time::timeout(Duration::from_secs(10), role).await?
}
