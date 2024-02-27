use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;
use anyhow::Context;

use axum::Router;
use axum::routing::get;
use futures::future::try_join_all;
use ginepro::LoadBalancedChannel;
use lazy_static::lazy_static;
use tokio::fs::read;
use tokio::signal;
use tokio::time::sleep;
use tonic::{Request, Response, Status, Streaming};
use tonic::transport::{Certificate, ClientTlsConfig, Identity, Server, ServerTlsConfig};
use tracing::instrument;
use crate::pb::fmaas::{
    BatchedGenerationRequest, BatchedGenerationResponse, GenerationResponse,
    SingleGenerationRequest, BatchedTokenizeRequest, BatchedTokenizeResponse,
    ModelInfoRequest, ModelInfoResponse,
};
use crate::pb::fmaas::generation_service_client::GenerationServiceClient;
use crate::pb::fmaas::generation_service_server::{GenerationService, GenerationServiceServer};


const MODEL_MAP_ENV_VAR_NAME: &str = "MODEL_MAP_CONFIG";

lazy_static! {
    static ref MODEL_MAP: HashMap<&'static str, &'static str> = {
        lazy_static! {
            static ref MODEL_MAP_STR: String = {
                match std::env::var(MODEL_MAP_ENV_VAR_NAME) {
                    Ok(p) => {
                        tracing::info!("Loading model mapping config from: {p}");
                        std::fs::read_to_string(p).expect("Failed to load model mapping config")
                    }
                    Err(_) => {
                        panic!("{MODEL_MAP_ENV_VAR_NAME} env var must be set, and point to a valid model_map.yml file")
                    }
                }
            };
        }
        let map: HashMap<&'static str, &'static str> = serde_yaml::from_str(&MODEL_MAP_STR)
            .expect("Failed to parse the model mapping config");
        tracing::info!("{} model mappings configured", map.len());
        map
    };
}

pub async fn run(
    grpc_addr: SocketAddr,
    http_addr: SocketAddr,
    tls_key_pair: Option<(String, String)>,
    tls_client_ca_cert: Option<String>,
    default_target_port: u16,
    upstream_tls: bool,
    upstream_tls_ca_cert: Option<String>
) {
    let mut builder = Server::builder();

    // Configure TLS if requested
    let mut client_tls = upstream_tls.then_some(ClientTlsConfig::new());
    if let Some(cert_path) = upstream_tls_ca_cert {
        tracing::info!("Configuring TLS for outgoing connections to model servers");
        let cert_pem = load_pem(cert_path, "cert").await;
        let cert = Certificate::from_pem(cert_pem);
        client_tls = client_tls.map(|c| c.ca_certificate(cert));
    }

    if let Some((cert_path, key_path)) = tls_key_pair {
        tracing::info!("Configuring Server TLS for incoming connections");
        let mut tls_config = ServerTlsConfig::new();
        let cert_pem = load_pem(cert_path, "cert").await;
        let key_pem = load_pem(key_path, "key").await;
        let identity = Identity::from_pem(cert_pem, key_pem);
        if upstream_tls {
            client_tls = client_tls.map(|c| c.identity(identity.clone()));
        }
        tls_config = tls_config.identity(identity);
        if let Some(ca_cert_path) = tls_client_ca_cert {
            tracing::info!("Configuring TLS trust certificate (mTLS) for incoming connections");
            let ca_cert_pem = load_pem(ca_cert_path, "client ca cert").await;
            tls_config = tls_config.client_ca_root(Certificate::from_pem(ca_cert_pem));
        }
        builder = builder.tls_config(tls_config).expect("tls configuration error");
    } else if upstream_tls {
        panic!("Upstream TLS enabled without any certificates");
    }

    // Set up clients
    let clients = try_join_all(
        MODEL_MAP.iter().map(|(model, service)| async {
            tracing::info!("Configuring client for model name: [{}]", *model);
            // Parse hostname and optional port from target service name
            let mut service_parts = service.split(":");
            let hostname = service_parts.next().unwrap();
            let port = service_parts.next().map_or(
                default_target_port,
                |p| p.parse::<u16>().expect(
                    &*format!("Invalid port in configured service name: {}", p)
                ),
            );
            if service_parts.next().is_some() {
                panic!("Configured service name contains more than one : character");
            }
            // Build a load-balanced channel given a service name and a port.
            let mut builder = LoadBalancedChannel::builder((hostname, port));
                //.dns_probe_interval(Duration::from_secs(10))
            if let Some(tls_config) = &client_tls {
                builder = builder.with_tls(tls_config.clone());
            }
            let channel = builder.channel().await
                .context(format!("Channel failed for service {}", *service))?;
            Ok((*model, GenerationServiceClient::new(channel))) as Result<
                (&'static str, GenerationServiceClient<LoadBalancedChannel>), anyhow::Error
            >
    })).await.expect("Error creating upstream service clients").into_iter().collect();
    tracing::info!("{} upstream gRPC clients created successfully", grpc_addr.port());

    // Build and start gRPC server in background task
    let grpc_service = GenerationServicer { clients };
    let grpc_server = builder
        .add_service(GenerationServiceServer::new(grpc_service))
        .serve_with_shutdown(grpc_addr, shutdown_signal());

    let grpc_server_handle = tokio::spawn(async move {
        tracing::info!("gRPC server started on port {}", grpc_addr.port());
        grpc_server.await
    });

    // Wait two seconds to ensure gRPC server does not immediately
    // fail before starting
    sleep(Duration::from_secs(2)).await;
    if grpc_server_handle.is_finished() {
        grpc_server_handle.await.unwrap().expect("gRPC server startup failed");
        panic!(); // should not reach here
    }

    // Build and await on the HTTP server
    let app = Router::new()
      .route("/health", get(health));

    let server = axum::Server::bind(&http_addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal());

    tracing::info!("HTTP server started on port {}", http_addr.port());
    server.await.expect("HTTP server crashed!");

    grpc_server_handle.await.unwrap().expect("gRPC server crashed");
}

async fn health() -> &'static str {
    // TODO: determine how to detect if the router should be considered unhealthy
    "Ok"
}

async fn load_pem(path: String, name: &str) -> Vec<u8> {
    read(&path).await.expect(&*format!("couldn't load {name} from {path}"))
}

/*
  TODO:
  - Log errors/timings
*/


#[derive(Debug, Default)]
pub struct GenerationServicer {
    clients: HashMap<&'static str, GenerationServiceClient<LoadBalancedChannel>>
}

impl GenerationServicer {
    async fn client(&self, model_id: &String)
        -> Result<GenerationServiceClient<LoadBalancedChannel>, Status> {
        Ok(
            self.clients.get(&**model_id)
                .ok_or_else(|| Status::not_found(
                    format!("Unrecognized model_id: {model_id}"))
                )?
                .clone()
        )
    }
}

#[tonic::async_trait]
impl GenerationService for GenerationServicer {
    #[instrument(skip_all)]
    async fn generate(&self, request: Request<BatchedGenerationRequest>)
        -> Result<Response<BatchedGenerationResponse>, Status> {
        //let start_time = Instant::now();
        let br = request.get_ref();
        if br.requests.is_empty() {
            return Ok(Response::new(BatchedGenerationResponse{ responses: vec![] }));
        }
        tracing::debug!("Routing generation request for Model ID {}", &br.model_id);
        self.client(&br.model_id).await?.generate(request).await
    }

    type GenerateStreamStream = Streaming<GenerationResponse>;

    #[instrument(skip_all)]
    async fn generate_stream(&self, request: Request<SingleGenerationRequest>)
        -> Result<Response<Self::GenerateStreamStream>, Status> {
        let sr = request.get_ref();
        if sr.request.is_none() {
            return Err(Status::invalid_argument("missing request"));
        }
        tracing::debug!("Routing streaming generation request for Model ID {}", &sr.model_id);
        self.client(&sr.model_id).await?.generate_stream(request).await
    }

    #[instrument(skip_all)]
    async fn tokenize(&self, request: Request<BatchedTokenizeRequest>)
        -> Result<Response<BatchedTokenizeResponse>, Status> {
        let br = request.get_ref();
        if br.requests.is_empty() {
            return Ok(Response::new(BatchedTokenizeResponse{ responses: vec![] }));
        }
        tracing::debug!("Routing tokenization request for Model ID {}", &br.model_id);
        self.client(&br.model_id).await?.tokenize(request).await
    }

    #[instrument(skip_all)]
    async fn model_info(&self, request: Request<ModelInfoRequest>)
        -> Result<Response<ModelInfoResponse>, Status> {
        tracing::debug!("Routing model info request for Model ID {}", &request.get_ref().model_id);
        self.client(&request.get_ref().model_id).await?.model_info(request).await
    }
}

/// Shutdown signal handler
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
        let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("signal received, starting graceful shutdown");
}
