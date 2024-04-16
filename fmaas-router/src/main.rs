use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

use clap::Parser;
use fmaas_router::{server, tracing_utils::init_logging, ModelMap};

/// App Configuration
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(default_value = "8033", long, short, env)]
    grpc_port: u16,
    #[clap(default_value = "3000", long, short, env)]
    port: u16,
    #[clap(default_value = "8033", long, short, env)]
    default_upstream_port: u16,
    #[clap(long, env)]
    json_output: bool,
    #[clap(long, env)]
    model_map_config: String,
    #[clap(long, env)]
    tls_cert_path: Option<String>,
    #[clap(long, env)]
    tls_key_path: Option<String>,
    #[clap(long, env)]
    tls_client_ca_cert_path: Option<String>,
    #[clap(long, env)]
    upstream_tls: bool,
    #[clap(long, env)]
    upstream_tls_ca_cert_path: Option<String>,
    #[clap(long, env = "OTEL_EXPORTER_OTLP_ENDPOINT")]
    otlp_endpoint: Option<String>,
    #[clap(long, env = "OTEL_SERVICE_NAME", default_value = "fmaas-router")]
    otlp_service_name: String,
}

fn main() -> Result<(), std::io::Error> {
    //Get args
    let args = Args::parse();

    if args.tls_key_path.is_some() != args.tls_cert_path.is_some() {
        panic!("tls: must provide both cert and key")
    }
    if args.tls_client_ca_cert_path.is_some() && args.tls_cert_path.is_none() {
        panic!("tls: cannot provide client ca cert without keypair")
    }

    // Load model map config
    let model_map = Arc::new(ModelMap::load(args.model_map_config));

    // Launch Tokio runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let grpc_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.grpc_port);
            let http_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.port);

            init_logging(args.otlp_service_name, args.json_output, args.otlp_endpoint);

            server::run(
                grpc_addr,
                http_addr,
                args.tls_cert_path
                    .map(|cp| (cp, args.tls_key_path.unwrap())),
                args.tls_client_ca_cert_path,
                args.default_upstream_port,
                args.upstream_tls,
                args.upstream_tls_ca_cert_path,
                model_map,
            )
            .await;

            Ok(())
        })
}
