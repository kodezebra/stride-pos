//! A peer reached over the shop LAN.
//!
//! The transport (`lan-http`) knows how to put a JSON body on a socket and read
//! one back. This file only names the two routes a till needs and turns bytes
//! into the protocol types. Keeping it separate from the worker is what lets
//! the cursor loop be tested without sockets.

use anyhow::{Context as _, Result};
use lan_http::client::{Client, Endpoint, Request};

use crate::sync::protocol::{CatalogSnapshot, PROTOCOL, PullRequest, PullResponse};
use crate::sync::worker::PeerLink;

const PULL_PATH: &str = "/v1/sync/pull";
const CATALOG_PATH: &str = "/v1/sync/catalog";

/// One other till, addressed by its LAN host and port.
pub struct HttpPeer {
    client: Client,
    endpoint: Endpoint,
    token: String,
}

impl HttpPeer {
    pub fn new(host: &str, port: u16, token: &str) -> Self {
        Self {
            client: Client::new(),
            endpoint: Endpoint::new(host, port),
            token: token.to_owned(),
        }
    }

    fn post(&self, path: &str, body: Vec<u8>) -> Result<Vec<u8>> {
        let request = Request::post(path, body).with_token(self.token.clone());
        let response = self
            .client
            .request(&self.endpoint, &request)
            .with_context(|| {
                format!(
                    "asking {}:{} for {path}",
                    self.endpoint.host, self.endpoint.port
                )
            })?;
        Ok(response.body)
    }
}

impl PeerLink for HttpPeer {
    fn pull(&self, since_seq: i64, limit: usize, catalog_version: i64) -> Result<PullResponse> {
        let body = serde_json::to_vec(&PullRequest {
            protocol: PROTOCOL,
            since_seq,
            limit,
            catalog_version,
        })?;
        let bytes = self.post(PULL_PATH, body)?;
        serde_json::from_slice(&bytes).context("the peer's pull answer is not valid JSON")
    }

    fn catalog(&self) -> Result<CatalogSnapshot> {
        let bytes = self.post(CATALOG_PATH, Vec::new())?;
        serde_json::from_slice(&bytes).context("the peer's catalog is not valid JSON")
    }
}
