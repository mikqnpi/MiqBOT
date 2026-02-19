use anyhow::{Context, Result};
use rustls::server::AllowAnyAuthenticatedClient;
use rustls::{Certificate, PrivateKey, RootCertStore, ServerConfig};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;

pub fn make_server_config(
    server_cert_pem: &str,
    server_key_pem: &str,
    client_ca_cert_pem: &str,
) -> Result<Arc<ServerConfig>> {
    let certs = load_certs(server_cert_pem).context("load server cert")?;
    let key = load_private_key(server_key_pem).context("load server key")?;

    let mut roots = RootCertStore::empty();
    for ca in load_certs(client_ca_cert_pem).context("load client ca")? {
        roots.add(&ca).context("add client ca")?;
    }

    let verifier = AllowAnyAuthenticatedClient::new(roots);

    let cfg = ServerConfig::builder()
        .with_safe_defaults()
        .with_client_cert_verifier(Arc::new(verifier))
        .with_single_cert(certs, key)
        .context("build rustls ServerConfig")?;

    Ok(Arc::new(cfg))
}

fn load_certs(path: &str) -> Result<Vec<Certificate>> {
    let f = File::open(path).with_context(|| format!("open cert file: {path}"))?;
    let mut r = BufReader::new(f);
    let certs = rustls_pemfile::certs(&mut r).context("read certs")?;
    Ok(certs.into_iter().map(Certificate).collect())
}

fn load_private_key(path: &str) -> Result<PrivateKey> {
    let f = File::open(path).with_context(|| format!("open key file: {path}"))?;
    let mut r = BufReader::new(f);

    let keys = rustls_pemfile::pkcs8_private_keys(&mut r).context("read pkcs8 keys")?;
    if let Some(k) = keys.into_iter().next() {
        return Ok(PrivateKey(k));
    }

    let f = File::open(path).with_context(|| format!("open key file: {path}"))?;
    let mut r = BufReader::new(f);
    let keys = rustls_pemfile::rsa_private_keys(&mut r).context("read rsa keys")?;
    if let Some(k) = keys.into_iter().next() {
        return Ok(PrivateKey(k));
    }

    anyhow::bail!("no private key found in {path}");
}
