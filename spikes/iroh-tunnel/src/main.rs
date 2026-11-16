// SPDX-License-Identifier: AGPL-3.0-or-later
//! Spike (not product code): an iroh tunnel that forwards TCP.
//!   serve <key-file> <allow-file> <target host:port>   the home server side
//!   proxy <key-file> <server-id> <listen addr> [<server ip:port>]   the phone side
//! Relay: env RELAY=<url> (own relay) or none (direct only).
//! Env RELAY_ONLY=1 on the proxy: no direct IP path at all.
use std::{net::SocketAddr, str::FromStr, time::Instant};

use anyhow::{Context, Result};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey,
    endpoint::{Builder, presets},
};
use tokio::net::{TcpListener, TcpStream};

const ALPN: &[u8] = b"alparchive/tunnel/0";

fn key(path: &str) -> Result<SecretKey> {
    if let Ok(b) = std::fs::read(path) {
        let b: [u8; 32] = b.try_into().ok().context("key file")?;
        return Ok(SecretKey::from_bytes(&b));
    }
    let k = SecretKey::generate();
    std::fs::write(path, k.to_bytes())?;
    Ok(k)
}

fn builder(path: &str) -> Result<Builder> {
    let relay = match std::env::var("RELAY") {
        Ok(u) => RelayMode::Custom(RelayUrl::from_str(&u)?.into()),
        Err(_) => RelayMode::Disabled,
    };
    Ok(Endpoint::builder(presets::Minimal)
        .secret_key(key(path)?)
        .relay_mode(relay))
}

#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    match a[1].as_str() {
        "id" => println!("{}", key(&a[2])?.public()),
        "serve" => serve(&a[2], &a[3], &a[4]).await?,
        "proxy" => proxy(&a[2], &a[3], &a[4], a.get(5)).await?,
        _ => anyhow::bail!("usage"),
    }
    Ok(())
}

async fn serve(keyf: &str, allowf: &str, target: &str) -> Result<()> {
    let ep = builder(keyf)?.alpns(vec![ALPN.to_vec()]).bind().await?;
    println!("server id {} addr {:?}", ep.id(), ep.addr());
    while let Some(incoming) = ep.accept().await {
        let (allowf, target) = (allowf.to_string(), target.to_string());
        tokio::spawn(async move {
            let conn = match incoming.await {
                Ok(c) => c,
                Err(e) => return eprintln!("handshake: {e}"),
            };
            let who = conn.remote_id();
            // The approved list is read for every connection: a removed phone is out at once.
            let allowed = std::fs::read_to_string(&allowf)
                .unwrap_or_default()
                .lines()
                .any(|l| EndpointId::from_str(l.trim()).is_ok_and(|id| id == who));
            if !allowed {
                eprintln!("refused {who}");
                conn.close(1u32.into(), b"not approved");
                return;
            }
            eprintln!("accepted {who}");
            while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                let target = target.clone();
                tokio::spawn(async move {
                    let Ok(tcp) = TcpStream::connect(&target).await else { return };
                    let (mut r, mut w) = tcp.into_split();
                    let up = tokio::io::copy(&mut recv, &mut w);
                    let down = async {
                        let n = tokio::io::copy(&mut r, &mut send).await;
                        let _ = send.finish();
                        n
                    };
                    let _ = tokio::join!(up, down);
                });
            }
        });
    }
    Ok(())
}

async fn proxy(keyf: &str, server: &str, listen: &str, direct: Option<&String>) -> Result<()> {
    let mut b = builder(keyf)?;
    if std::env::var("RELAY_ONLY").is_ok() {
        b = b.clear_ip_transports();
    }
    let ep = b.bind().await?;
    let mut addr = EndpointAddr::new(EndpointId::from_str(server)?);
    if let Ok(u) = std::env::var("RELAY") {
        addr = addr.with_relay_url(RelayUrl::from_str(&u)?);
    }
    if let Some(d) = direct {
        addr = addr.with_ip_addr(SocketAddr::from_str(d)?);
    }
    let t = Instant::now();
    let conn = ep.connect(addr, ALPN).await?;
    eprintln!("connected in {:?}; paths: {:?}", t.elapsed(), conn.paths());
    let l = TcpListener::bind(listen).await?;
    loop {
        let (tcp, _) = l.accept().await?;
        let conn = conn.clone();
        tokio::spawn(async move {
            let Ok((mut send, mut recv)) = conn.open_bi().await else { return };
            let (mut r, mut w) = tcp.into_split();
            let up = async {
                let n = tokio::io::copy(&mut r, &mut send).await;
                let _ = send.finish();
                n
            };
            let down = tokio::io::copy(&mut recv, &mut w);
            let _ = tokio::join!(up, down);
        });
    }
}
