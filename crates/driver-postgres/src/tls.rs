//! Builds the rustls configuration for a connection's [`TlsConfig`].

use std::sync::Arc;

use dbclient_core::{Error, Result, TlsConfig, TlsMode};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

pub fn client_config(tls: &TlsConfig) -> Result<ClientConfig> {
    // Named explicitly: other crates in the build may enable a second provider,
    // and rustls refuses to guess between them.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Internal(e.to_string()))?;

    Ok(match tls.mode {
        TlsMode::Insecure => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate(provider)))
            .with_no_client_auth(),
        TlsMode::Verify | TlsMode::Off => {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            if let Some(path) = tls.ca_file.as_deref().filter(|p| !p.is_empty()) {
                let invalid = |e: &dyn std::fmt::Display| {
                    Error::InvalidConfig(format!("CA certificate file {path}: {e}"))
                };
                let mut added = 0;
                for cert in CertificateDer::pem_file_iter(path).map_err(|e| invalid(&e))? {
                    roots
                        .add(cert.map_err(|e| invalid(&e))?)
                        .map_err(|e| invalid(&e))?;
                    added += 1;
                }
                if added == 0 {
                    return Err(invalid(&"no certificates found"));
                }
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        }
    })
}

/// Skips the identity check but still verifies handshake signatures.
#[derive(Debug)]
struct AcceptAnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
