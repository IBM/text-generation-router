use std::{collections::HashMap, path::Path};
use anyhow::Context;
use futures::future::try_join_all;
use ginepro::LoadBalancedChannel;

use serde::{Deserialize, Deserializer};
use tonic::transport::ClientTlsConfig;
use tracing::info;

#[allow(clippy::enum_variant_names)]
mod pb;
pub mod rpc;
pub mod server;
pub mod tracing_utils;

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceAddr {
    pub hostname: String,
    pub port: Option<u16>,
}

/// Old format without top-level keys, generation models only.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelMapV1(#[serde(deserialize_with = "de_service_addr")] HashMap<String, ServiceAddr>);

/// New format with top-level keys for generation and embeddings models.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelMapV2 {
    #[serde(deserialize_with = "de_service_addr", default = "HashMap::default")]
    generation: HashMap<String, ServiceAddr>,
    #[serde(deserialize_with = "de_service_addr", default = "HashMap::default")]
    embeddings: HashMap<String, ServiceAddr>,
}

/// Maps model names to service address.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ModelMap {
    V1(ModelMapV1),
    V2(ModelMapV2),
}

impl ModelMap {
    pub fn load(path: impl AsRef<Path>) -> Self {
        let s = std::fs::read_to_string(path).expect("Failed to load model map config");
        serde_yaml::from_str(&s).expect("Invalid model map config")
    }

    pub fn generation(&self) -> Option<&HashMap<String, ServiceAddr>> {
        match self {
            ModelMap::V1(v1) => Some(&v1.0),
            ModelMap::V2(v2) => (!v2.generation.is_empty()).then_some(&v2.generation),
        }
    }

    pub fn embeddings(&self) -> Option<&HashMap<String, ServiceAddr>> {
        match self {
            ModelMap::V1(_) => None,
            ModelMap::V2(v2) => (!v2.embeddings.is_empty()).then_some(&v2.embeddings),
        }
    }
}

fn service_addr_from_str<'de, D>(deserializer: D) -> Result<ServiceAddr, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer).map_err(serde::de::Error::custom)?;
    let mut parts = s.split(':');
    let hostname = parts.next().unwrap().to_string();
    let port = parts.next().map(|p| {
        p.parse::<u16>()
            .unwrap_or_else(|_| panic!("Invalid port in configured service name: {p}"))
    });
    if parts.next().is_some() {
        panic!("Configured service name contains more than one : character");
    }
    Ok(ServiceAddr { hostname, port })
}

fn de_service_addr<'de, D>(deserializer: D) -> Result<HashMap<String, ServiceAddr>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct Wrapper(#[serde(deserialize_with = "service_addr_from_str")] ServiceAddr);

    let v = HashMap::<String, Wrapper>::deserialize(deserializer)?;
    Ok(v.into_iter().map(|(k, Wrapper(v))| (k, v)).collect())
}

async fn create_clients<C>(
    default_target_port: u16,
    client_tls: Option<&ClientTlsConfig>,
    model_map: &HashMap<String, ServiceAddr>,
    new: fn(LoadBalancedChannel) -> C,
) -> HashMap<String, C> {
    let clients = model_map
        .iter()
        .map(|(name, service_addr)| async move {
            info!("Configuring client for model name: [{name}]");
            // Build a load-balanced channel given a service name and a port.
            let mut builder = LoadBalancedChannel::builder((
                service_addr.hostname.clone(),
                service_addr.port.unwrap_or(default_target_port),
            ));
            if let Some(tls_config) = client_tls {
                builder = builder.with_tls(tls_config.clone());
            }
            let channel = builder
                .channel()
                .await
                .context(format!("Channel failed for service {name}"))?;
            Ok((name.clone(), new(channel))) as Result<(String, C), anyhow::Error>
        })
        .collect::<Vec<_>>();
    try_join_all(clients)
        .await
        .expect("Error creating upstream service clients")
        .into_iter()
        .collect()
}
