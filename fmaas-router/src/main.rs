use clap::Parser;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tracing_subscriber::{EnvFilter};
use fmaas_router::server;

/// App Configuration
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(default_value = "8033", long, short, env)]
    grpc_port: u16,
    #[clap(default_value = "3000", long, short, env)]
    probe_port: u16,
    #[clap(default_value = "8033", long, short, env)]
    default_upstream_port: u16,
    #[clap(long, env)]
    json_output: bool,
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
}


fn main() -> Result<(), std::io::Error> {
    //Get args
    let args = Args::parse();

    // Configure log level; use info by default
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    if args.json_output {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter_layer)
            .with_current_span(false)
            .init();
    } else {
        tracing_subscriber::fmt()
            .compact()
            .with_env_filter(filter_layer)
            .init();
    }

    if args.tls_key_path.is_some() != args.tls_cert_path.is_some() {
        panic!("tls: must provide both cert and key")
    }

    if args.tls_client_ca_cert_path.is_some() && args.tls_cert_path.is_none() {
        panic!("tls: cannot provide client ca cert without keypair")
    }

    // Launch Tokio runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            //TODO initialize clients

            let grpc_addr = SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.grpc_port
            );
            let http_addr = SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.probe_port
            );

            server::run(
                grpc_addr,
                http_addr,
                args.tls_cert_path.map(|cp| (cp, args.tls_key_path.unwrap())),
                args.tls_client_ca_cert_path,
                args.default_upstream_port,
                args.upstream_tls,
                args.upstream_tls_ca_cert_path
            ).await;

            Ok(())
        })
}
