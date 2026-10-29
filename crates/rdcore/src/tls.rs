//! TLS 1.3 / QUIC sozlamalari.
//!
//! Viewer tomonda sertifikat TLS darajasida tekshirilmaydi (CA yo'q), faqat qayd etiladi.
//! Haqiqiylik `known_hosts` fingerprint tekshiruvi va autentifikatsiya isbotidagi
//! fingerprint bog'lanishi orqali ta'minlanadi (`client.rs`, `auth.rs`).

use anyhow::{Context, Result};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{ring as ring_provider, CryptoProvider},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::cert::HostIdentity;

/// Handshake paytida ko'rilgan Host sertifikati (DER).
pub type SeenCert = Arc<Mutex<Option<Vec<u8>>>>;

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring_provider::default_provider())
}

fn transport() -> Result<Arc<quinn::TransportConfig>> {
    let mut t = quinn::TransportConfig::default();
    t.max_concurrent_bidi_streams(16u32.into());
    t.keep_alive_interval(Some(Duration::from_secs(5)));
    t.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(Duration::from_secs(30)).context("idle timeout")?,
    ));
    Ok(Arc::new(t))
}

pub fn server_config(identity: &HostIdentity) -> Result<quinn::ServerConfig> {
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("TLS versiyasi")?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(identity.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_der.clone())),
        )
        .context("sertifikat yoki kalit noto'g'ri")?;
    tls.alpn_protocols = vec![protocol::ALPN.to_vec()];
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls).context("QUIC server sozlamasi")?;
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    cfg.transport_config(transport()?);
    Ok(cfg)
}

#[derive(Debug)]
struct CaptureVerifier {
    seen: SeenCert,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for CaptureVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let mut guard = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(end_entity.as_ref().to_vec());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

pub fn client_config(seen: SeenCert) -> Result<quinn::ClientConfig> {
    let provider = provider();
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("TLS versiyasi")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(CaptureVerifier { seen, provider }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![protocol::ALPN.to_vec()];
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls).context("QUIC klient sozlamasi")?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(crypto));
    cfg.transport_config(transport()?);
    Ok(cfg)
}
