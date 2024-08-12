use std::collections::HashMap;

use ginepro::LoadBalancedChannel;
use tonic::{transport::ClientTlsConfig, Request, Response, Status};
use tracing::{debug, instrument};

use crate::rpc::extract_model_id;

use crate::{create_clients, pb::{
    caikit::runtime::info::{
        info_service_client::InfoServiceClient, info_service_server::InfoService
    },
    caikit_data_model::common::runtime::{
        ModelInfoRequest, ModelInfoResponse, RuntimeInfoRequest, RuntimeInfoResponse
    }
}, ServiceAddr};


#[derive(Debug, Default)]
pub struct InfoServicer {
    clients: HashMap<String, InfoServiceClient<LoadBalancedChannel>>,
}

impl InfoServicer {
    pub async fn new(
        default_target_port: u16,
        client_tls: Option<&ClientTlsConfig>,
        model_map: &HashMap<String, ServiceAddr>,
    ) -> Self {
        let clients = create_clients(
            default_target_port, client_tls, model_map, InfoServiceClient::new
        ).await;
        Self { clients }
    }

    async fn client(
        &self,
        model_id: &str,
    ) -> Result<InfoServiceClient<LoadBalancedChannel>, Status> {
        Ok(self
            .clients
            .get(model_id)
            .ok_or_else(|| Status::not_found(format!("Unrecognized model_id: {model_id}")))?
            .clone())
    }
}

#[tonic::async_trait]
impl InfoService for InfoServicer {
    #[instrument(skip_all)]
    async fn get_models_info(
        &self,
        request: Request<ModelInfoRequest>,
    ) -> Result<Response<ModelInfoResponse>, Status> {
        let model_id = extract_model_id(&request)?;
        let mir: &ModelInfoRequest = request.get_ref();
        if mir.model_ids.is_empty() {
            return Ok(Response::new(ModelInfoResponse::default()));
        }
        debug!(
            "Routing get models info request for Model ID {}",
            model_id
        );
        self.client(model_id)
            .await?
            .get_models_info(request)
            .await
    }
    #[instrument(skip_all)]
    async fn get_runtime_info(
        &self,
        _request: Request<RuntimeInfoRequest>,
    ) -> Result<Response<RuntimeInfoResponse>, Status> {
        Err(Status::unimplemented("not implemented"))
    }
}