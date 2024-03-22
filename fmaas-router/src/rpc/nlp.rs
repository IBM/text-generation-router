use std::collections::HashMap;

use ginepro::LoadBalancedChannel;
use tonic::{transport::ClientTlsConfig, Code, Request, Response, Status, Streaming};
use tracing::{debug, instrument};

use crate::{pb::{
    caikit::runtime::nlp::{
        nlp_service_client::NlpServiceClient, nlp_service_server::NlpService,
        BidiStreamingTokenClassificationTaskRequest, EmbeddingTaskRequest,
        EmbeddingTasksRequest, RerankTaskRequest, RerankTasksRequest,
        SentenceSimilarityTaskRequest, SentenceSimilarityTasksRequest,
        ServerStreamingTextGenerationTaskRequest, TextClassificationTaskRequest,
        TextGenerationTaskRequest, TokenClassificationTaskRequest,
    },
    caikit_data_model::{
        caikit_nlp::{
            EmbeddingResult, EmbeddingResults, RerankResult, RerankResults,
            SentenceSimilarityResult, SentenceSimilarityResults,
        },
        nlp::{
            ClassificationResults, GeneratedTextResult, GeneratedTextStreamResult,
            TokenClassificationResults, TokenClassificationStreamResult,
        },
    },
}, create_clients, ServiceAddr};

const METADATA_NAME_MODEL_ID: &str = "mm-model-id";

#[derive(Debug, Default)]
pub struct NlpServicer {
    clients: HashMap<String, NlpServiceClient<LoadBalancedChannel>>,
}

impl NlpServicer {
    pub async fn new(
        default_target_port: u16,
        client_tls: Option<&ClientTlsConfig>,
        model_map: &HashMap<String, ServiceAddr>,
    ) -> Self {
        let clients = create_clients(
            default_target_port, client_tls, model_map, NlpServiceClient::new
        ).await;
        Self { clients }
    }

    async fn client(
        &self,
        model_id: &str,
    ) -> Result<NlpServiceClient<LoadBalancedChannel>, Status> {
        Ok(self
            .clients
            .get(model_id)
            .ok_or_else(|| Status::not_found(format!("Unrecognized model_id: {model_id}")))?
            .clone())
    }
}

#[tonic::async_trait]
impl NlpService for NlpServicer {
    #[instrument(skip_all)]
    async fn embedding_tasks_predict(
        &self,
        request: Request<EmbeddingTasksRequest>,
    ) -> Result<Response<EmbeddingResults>, Status> {
        let model_id = extract_model_id(&request)?;
        let br: &EmbeddingTasksRequest = request.get_ref();
        if br.texts.is_empty() {
            return Ok(Response::new(EmbeddingResults::default()));
        }
        debug!(
            "Routing embeddings tasks predict request for Model ID {}",
            model_id
        );
        self.client(model_id)
            .await?
            .embedding_tasks_predict(request)
            .await
    }

    #[instrument(skip_all)]
    async fn embedding_task_predict(
        &self,
        request: Request<EmbeddingTaskRequest>,
    ) -> Result<Response<EmbeddingResult>, Status> {
        let model_id = extract_model_id(&request)?;
        let br = request.get_ref();
        if br.text.is_empty() {
            return Ok(Response::new(EmbeddingResult::default()));
        }
        debug!(
            "Routing embeddings task predict request for Model ID {}",
            model_id
        );
        self.client(model_id)
            .await?
            .embedding_task_predict(request)
            .await
    }

    #[instrument(skip_all)]
    async fn rerank_tasks_predict(
        &self,
        request: Request<RerankTasksRequest>,
    ) -> Result<Response<RerankResults>, Status> {
        let model_id = extract_model_id(&request)?;
        let rtr: &RerankTasksRequest = request.get_ref();
        if rtr.documents.is_empty() || rtr.queries.is_empty() {
            return Ok(Response::new(RerankResults::default()));
        }
        debug!(
            "Routing rerank tasks predict request for Model ID {}",
            model_id
        );
        self.client(model_id)
            .await?
            .rerank_tasks_predict(request)
            .await
    }

    #[instrument(skip_all)]
    async fn rerank_task_predict(
        &self,
        request: Request<RerankTaskRequest>,
    ) -> Result<Response<RerankResult>, Status> {
        let model_id = extract_model_id(&request)?;
        let rtr: &RerankTaskRequest = request.get_ref();
        if rtr.documents.is_empty() || rtr.query.is_empty() {
            return Ok(Response::new(RerankResult::default()));
        }
        debug!(
            "Routing rerank task predict request for Model ID {}",
            model_id
        );
        self.client(model_id)
            .await?
            .rerank_task_predict(request)
            .await
    }

    #[instrument(skip_all)]
    async fn sentence_similarity_tasks_predict(
        &self,
        request: Request<SentenceSimilarityTasksRequest>,
    ) -> Result<Response<SentenceSimilarityResults>, Status> {
        let model_id = extract_model_id(&request)?;
        let sstr: &SentenceSimilarityTasksRequest = request.get_ref();
        if sstr.source_sentences.is_empty() || sstr.sentences.is_empty() {
            return Ok(Response::new(SentenceSimilarityResults::default()));
        }
        debug!(
            "Routing sentence similarity tasks predict request for Model ID {}",
            model_id
        );
        self.client(model_id)
            .await?
            .sentence_similarity_tasks_predict(request)
            .await
    }

    #[instrument(skip_all)]
    async fn sentence_similarity_task_predict(
        &self,
        request: Request<SentenceSimilarityTaskRequest>,
    ) -> Result<Response<SentenceSimilarityResult>, Status> {
        let model_id = extract_model_id(&request)?;
        let sstr: &SentenceSimilarityTaskRequest = request.get_ref();
        if sstr.source_sentence.is_empty() || sstr.sentences.is_empty() {
            return Ok(Response::new(SentenceSimilarityResult::default()));
        }
        debug!(
            "Routing sentence similarity task predict request for Model ID {}",
            model_id
        );
        self.client(model_id)
            .await?
            .sentence_similarity_task_predict(request)
            .await
    }

    type BidiStreamingTokenClassificationTaskPredictStream =
        Streaming<TokenClassificationStreamResult>;
    #[instrument(skip_all)]
    async fn bidi_streaming_token_classification_task_predict(
        &self,
        _request: Request<Streaming<BidiStreamingTokenClassificationTaskRequest>>,
    ) -> Result<Response<Self::BidiStreamingTokenClassificationTaskPredictStream>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    type ServerStreamingTextGenerationTaskPredictStream = Streaming<GeneratedTextStreamResult>;
    #[instrument(skip_all)]
    async fn server_streaming_text_generation_task_predict(
        &self,
        _request: Request<ServerStreamingTextGenerationTaskRequest>,
    ) -> Result<Response<Self::ServerStreamingTextGenerationTaskPredictStream>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    #[instrument(skip_all)]
    async fn text_classification_task_predict(
        &self,
        _request: Request<TextClassificationTaskRequest>,
    ) -> Result<Response<ClassificationResults>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    #[instrument(skip_all)]
    async fn text_generation_task_predict(
        &self,
        _request: Request<TextGenerationTaskRequest>,
    ) -> Result<Response<GeneratedTextResult>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    #[instrument(skip_all)]
    async fn token_classification_task_predict(
        &self,
        _request: Request<TokenClassificationTaskRequest>,
    ) -> Result<Response<TokenClassificationResults>, Status> {
        Err(Status::unimplemented("not implemented"))
    }
}

/// Extracts model_id from [`Request`] metadata.
fn extract_model_id<T>(request: &Request<T>) -> Result<&str, Status> {
    let metadata = request.metadata();
    if !metadata.contains_key(METADATA_NAME_MODEL_ID) {
        return Err(Status::new(
            Code::InvalidArgument,
            "Missing required model ID",
        ));
    }
    let model_id = metadata
        .get(METADATA_NAME_MODEL_ID)
        .unwrap()
        .to_str()
        .unwrap();
    Ok(model_id)
}
