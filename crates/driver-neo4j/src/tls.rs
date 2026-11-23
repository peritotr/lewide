//! Builds the rustls configuration for a connection's [`TlsConfig`].

use std::sync::Arc;

use dbclient_core::{Error, Result, TlsConfig, TlsMode};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

/// `check_as` replaces the name the certificate is checked against. It is
/// needed when the socket goes to a tunnel's address: the certificate belongs
/// to the real server, not to the tunnel's local end.
pub fn client_config(
    tls: &TlsConfig,
    check_as: Option<ServerName<'static>>,
) -> Result<ClientConfig> {
    // Named explicitly: other crates in the build may enable a second provider,
    // and rustls refuses to guess between them.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Internal(e.to_string()))?;

    let check = match tls.mode {
        TlsMode::Insecure => Check::Nothing,
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
            let Some(name) = check_as else {
                return Ok(builder.with_root_certificates(roots).with_no_client_auth());
            };
            let verifier =
                WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                    .build()
                    .map_err(|e| Error::Internal(e.to_string()))?;
            Check::Against(verifier, name)
        }
    };
    Ok(builder
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Verifier { provider, check }))
        .with_no_client_auth())
}

#[derive(Debug)]
enum Check {
    /// Accept any certificate. Handshake signatures are still verified.
    Nothing,
    /// The normal checks, but for this name instead of the one connected to.
    Against(Arc<WebPkiServerVerifier>, ServerName<'static>),
}

#[derive(Debug)]
struct Verifier {
    provider: Arc<CryptoProvider>,
    check: Check,
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        match &self.check {
            Check::Nothing => Ok(ServerCertVerified::assertion()),
            Check::Against(verifier, name) => {
                verifier.verify_server_cert(end_entity, intermediates, name, ocsp_response, now)
            }
        }
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
            &self.provider.signature_verification_algorithms,
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
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
