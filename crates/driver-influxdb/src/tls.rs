//! Builds the rustls configuration for a connection's [`TlsConfig`].

use std::sync::Arc;

use dbclient_core::{Error, Result, TlsConfig, TlsMode};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

/// `host` is the name the certificate has to be valid for.
pub fn client_config(tls: &TlsConfig, host: &str) -> Result<ClientConfig> {
    // Named explicitly: other crates in the build may enable a second provider,
    // and rustls refuses to guess between them.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Internal(e.to_string()))?;

    let verifier: Arc<dyn ServerCertVerifier> = match tls.mode {
        TlsMode::Insecure => Arc::new(AcceptAnyCertificate(provider)),
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
            let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
                .build()
                .map_err(|e| Error::Internal(e.to_string()))?;
            let name = ServerName::try_from(host.to_owned())
                .map_err(|_| Error::InvalidConfig(format!("not a valid host name: {host}")))?;
            Arc::new(CheckAgainstHost { inner, name })
        }
    };
    Ok(builder
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

/// Checks the certificate against the configured host whatever address the
/// socket was opened to. Through a tunnel the request goes to a local address,
/// and the certificate must still be the real server's.
#[derive(Debug)]
struct CheckAgainstHost {
    inner: Arc<WebPkiServerVerifier>,
    name: ServerName<'static>,
}

impl ServerCertVerifier for CheckAgainstHost {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        self.inner
            .verify_server_cert(end_entity, intermediates, &self.name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
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
