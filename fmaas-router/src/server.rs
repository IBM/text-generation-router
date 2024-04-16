use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    routing::{get, post},
    Router,
};
use ginepro::LoadBalancedChannel;
use tokio::{fs::read, net::TcpListener, signal, time::sleep};
use tonic::transport::{
    server::RoutesBuilder, Certificate, ClientTlsConfig, Identity, Server, ServerTlsConfig,
};
use tracing::info;

use crate::{
    create_clients,
    openai::{chat_completions, completions},
    pb::{
        caikit::runtime::nlp::nlp_service_server::NlpServiceServer,
        fmaas::{
            generation_service_client::GenerationServiceClient,
            generation_service_server::GenerationServiceServer,
        },
    },
    rpc::{generation::GenerationServicer, nlp::NlpServicer},
    ModelMap,
};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    grpc_addr: SocketAddr,
    http_addr: SocketAddr,
    tls_key_pair: Option<(String, String)>,
    tls_client_ca_cert: Option<String>,
    default_target_port: u16,
    upstream_tls: bool,
    upstream_tls_ca_cert: Option<String>,
    model_map: Arc<ModelMap>,
) {
    let mut builder = Server::builder();

    // Configure TLS if requested
    let mut client_tls = upstream_tls.then_some(ClientTlsConfig::new());
    if let Some(cert_path) = upstream_tls_ca_cert {
        info!("Configuring TLS for outgoing connections to model servers");
        let cert_pem = load_pem(cert_path, "cert").await;
        let cert = Certificate::from_pem(cert_pem);
        client_tls = client_tls.map(|c| c.ca_certificate(cert));
    }
    if let Some((cert_path, key_path)) = tls_key_pair {
        info!("Configuring Server TLS for incoming connections");
        let mut tls_config = ServerTlsConfig::new();
        let cert_pem = load_pem(cert_path, "cert").await;
        let key_pem = load_pem(key_path, "key").await;
        let identity = Identity::from_pem(cert_pem, key_pem);
        if upstream_tls {
            client_tls = client_tls.map(|c| c.identity(identity.clone()));
        }
        tls_config = tls_config.identity(identity);
        if let Some(ca_cert_path) = tls_client_ca_cert {
            info!("Configuring TLS trust certificate (mTLS) for incoming connections");
            let ca_cert_pem = load_pem(ca_cert_path, "client ca cert").await;
            tls_config = tls_config.client_ca_root(Certificate::from_pem(ca_cert_pem));
        }
        builder = builder
            .tls_config(tls_config)
            .expect("tls configuration error");
    } else if upstream_tls {
        panic!("Upstream TLS enabled without any certificates");
    }

    let clients = if model_map.generation().is_some() {
        create_clients(
            default_target_port,
            client_tls.as_ref(),
            model_map.generation().unwrap(),
            GenerationServiceClient::new,
        )
        .await
    } else {
        HashMap::default()
    };
    let clients = Arc::new(clients);

    // Build and start gRPC server in background task
    let mut routes_builder = RoutesBuilder::default();
    if model_map.generation().is_some() {
        info!("Enabling GenerationService");
        let generation_servicer = GenerationServicer::new(clients.clone());
        routes_builder.add_service(GenerationServiceServer::new(generation_servicer));
    }
    if let Some(model_map) = model_map.embeddings() {
        info!("Enabling NlpService");
        let nlp_servicer =
            NlpServicer::new(default_target_port, client_tls.as_ref(), model_map).await;
        routes_builder.add_service(NlpServiceServer::new(nlp_servicer));
    }
    let grpc_server = builder
        .add_routes(routes_builder.routes())
        .serve_with_shutdown(grpc_addr, shutdown_signal());
    let grpc_server_handle = tokio::spawn(async move {
        info!("gRPC server started on port {}", grpc_addr.port());
        grpc_server.await
    });

    // Wait two seconds to ensure gRPC server does not immediately
    // fail before starting
    sleep(Duration::from_secs(2)).await;
    if grpc_server_handle.is_finished() {
        grpc_server_handle
            .await
            .unwrap()
            .expect("gRPC server startup failed");
        panic!(); // should not reach here
    }

    // Build and await on the HTTP server
    let listener = TcpListener::bind(&http_addr)
        .await
        .unwrap_or_else(|_| panic!("HTTP server startup failed: unable to listen on {http_addr}"));
    let server = if model_map.generation().is_some() {
        info!("Enabling OpenAI-compatible Chat and Completions service");
        let state = AppState::new(model_map, clients);
        let app = Router::new()
            .route("/health", get(health))
            .route("/v1/chat/completions", post(chat_completions))
            .route("/v1/completions", post(completions))
            //.layer(TraceLayer::new_for_http())
            .with_state(state);
        axum::serve(listener, app.into_make_service()).with_graceful_shutdown(shutdown_signal())
    } else {
        let app = Router::new().route("/health", get(health));
        axum::serve(listener, app.into_make_service()).with_graceful_shutdown(shutdown_signal())
    };
    info!("HTTP server started on port {}", http_addr.port());
    server.await.expect("HTTP server crashed!");

    grpc_server_handle
        .await
        .unwrap()
        .expect("gRPC server crashed");
}

async fn health() -> &'static str {
    // TODO: determine how to detect if the router should be considered unhealthy
    "Ok"
}

async fn load_pem(path: String, name: &str) -> Vec<u8> {
    read(&path)
        .await
        .unwrap_or_else(|_| panic!("couldn't load {name} from {path}"))
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

    info!("signal received, starting graceful shutdown");
}

#[derive(Clone)]
pub struct AppState {
    model_map: Arc<ModelMap>,
    clients: Arc<HashMap<String, GenerationServiceClient<LoadBalancedChannel>>>,
}

impl AppState {
    pub fn new(
        model_map: Arc<ModelMap>,
        clients: Arc<HashMap<String, GenerationServiceClient<LoadBalancedChannel>>>,
    ) -> Self {
        Self { model_map, clients }
    }

    pub fn model_map(&self) -> &Arc<ModelMap> {
        &self.model_map
    }

    pub fn clients(&self) -> &Arc<HashMap<String, GenerationServiceClient<LoadBalancedChannel>>> {
        &self.clients
    }
}
