use std::{collections::HashMap, convert::Infallible};

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Sse,
    },
    Json,
};
use chrono::Utc;
use futures::{Stream, StreamExt};
use opentelemetry::{trace::FutureExt, Context};
use tonic::Request;
use tracing::{debug, info_span, instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use super::{
    ChatCompletionChoice, ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionLogprob,
    ChatCompletionLogprobs, ChatCompletionMessage, ChatCompletionRequest, ChatCompletionResponse,
    ChatCompletionTopLogprob, StopTokens, TgisAdapter, Usage, SAMPLING_EPS,
};
use crate::{
    pb::fmaas::{
        BatchedGenerationRequest, DecodingMethod, DecodingParameters, GenerationRequest,
        Parameters, ResponseOptions, SamplingParameters, SingleGenerationRequest, StopReason,
        StoppingCriteria, TokenInfo,
    },
    server::AppState,
    tracing_utils::InjectTelemetryContext,
};

/// Handles OpenAI-compatible Chat Completions requests.
#[instrument(skip_all)]
pub async fn chat_completions(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<String>)> {
    let ctx = Span::current().context();
    if request.best_of.is_some() {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            Json("`best_of` is not yet implemented".into()),
        ));
    }
    if request.use_beam_search.is_some_and(|x| x) {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            Json("`use_beam_search` is not yet implemented".into()),
        ));
    }
    let request_id = format!("chatcmpl-{}", Uuid::new_v4().as_simple());
    let model_id = request.model.as_str();
    let stream = request.stream.unwrap_or_default();
    let created_time = Utc::now().timestamp();
    debug!(
        %request_id,
        %stream,
        "Routing chat_completions request for model id `{model_id}`"
    );
    let client = state
        .clients()
        .get(model_id)
        .ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(format!("Unrecognized model id `{model_id}`")),
            )
        })?
        .clone();
    let chat_template = state
        .model_map()
        .chat_templates()
        .get(model_id)
        .ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(format!("Chat template not found for model id `{model_id}`")),
            )
        })?;
    let prompt = chat_template.render(&request.messages);
    let tgis_adapter = TgisAdapter::new(client);
    if stream {
        let response_stream = tgis_adapter
            .chat_generate_stream(request_id, created_time, request, prompt)
            .with_context(ctx)
            .await;
        let sse = Sse::new(response_stream).keep_alive(KeepAlive::default());
        Ok(sse.into_response())
    } else {
        let response = tgis_adapter
            .chat_generate(request_id, created_time, request, prompt)
            .with_context(ctx)
            .await;
        Ok(Json(response).into_response())
    }
}

impl TgisAdapter {
    pub async fn chat_generate(
        &self,
        request_id: String,
        created_time: i64,
        request: ChatCompletionRequest,
        prompt: String,
    ) -> ChatCompletionResponse {
        let ctx = Context::current();
        let span = info_span!(
            "fmaas.GenerationService/Generate",
            rpc.system = "grpc",
            rpc.method = "Generate",
            rpc.service = "GenerationService",
            model_id = &request.model,
        );
        span.set_parent(ctx);

        let n_logprobs = request.top_logprobs.unwrap_or_default();
        let request: BatchedGenerationRequest = (request, prompt).into();
        let model_id = request.model_id.clone();

        let mut client = self.client.clone();
        let mut response = client
            .generate(Request::new(request).inject_context_span(&span))
            .await
            .unwrap()
            .into_inner();
        debug!(%request_id, ?response, "Received TGIS generate response");
        let response = response.responses.swap_remove(0);
        let finish_reason = match response.stop_reason() {
            StopReason::MaxTokens | StopReason::TokenLimit => "length",
            StopReason::StopSequence | StopReason::EosToken => "stop",
            StopReason::Cancelled | StopReason::TimeLimit | StopReason::Error => "abort",
            StopReason::NotFinished => unimplemented!(), // should not reach here in non-streaming case
        };
        let logprobs = create_logprobs(response.tokens, n_logprobs);
        let usage = Usage {
            completion_tokens: response.generated_token_count,
            prompt_tokens: response.input_token_count,
            total_tokens: response.input_token_count + response.generated_token_count,
        };
        let choice = ChatCompletionChoice {
            index: 0,
            message: ChatCompletionMessage {
                role: Some("assistant".into()),
                content: Some(response.text),
            },
            logprobs,
            finish_reason: finish_reason.into(),
        };
        ChatCompletionResponse {
            id: request_id,
            object: "chat.completion",
            created: created_time,
            model: model_id,
            system_fingerprint: None,
            choices: vec![choice],
            usage,
        }
    }

    pub async fn chat_generate_stream(
        &self,
        request_id: String,
        created_time: i64,
        request: ChatCompletionRequest,
        prompt: String,
    ) -> impl Stream<Item = Result<Event, Infallible>> {
        let ctx = Context::current();
        let span = info_span!(
            "fmaas.GenerationService/GenerateStream",
            rpc.system = "grpc",
            rpc.method = "GenerateStream",
            rpc.service = "GenerationService",
            model_id = &request.model,
        );
        span.set_parent(ctx);

        let n_logprobs = request.top_logprobs.unwrap_or_default();
        let request: SingleGenerationRequest = (request, prompt).into();
        let model_id = request.model_id.clone();

        let mut client = self.client.clone();
        async_stream::stream! {
            let mut prompt_tokens: u32 = 0;
            let mut response_stream = client
                .generate_stream(Request::new(request).inject_context_span(&span))
                .await
                .unwrap()
                .into_inner();

            let chunk = ChatCompletionChunk {
                id: request_id.clone(),
                choices: vec![ChatCompletionChunkChoice {
                    delta: ChatCompletionMessage {
                        role: Some("assistant".into()),
                        content: None,
                    },
                    index: 0,
                    logprobs: None,
                    finish_reason: None,
                }],
                created: created_time,
                model: model_id.clone(),
                object: "chat.completion.chunk",
                usage: None,
            };
            yield Ok(chunk.into());

            // The first message includes input_token_count
            let response = response_stream.next().await.unwrap().unwrap();
            debug!(%request_id, ?response, "Received TGIS generate_stream response [1]");
            if response.input_token_count > 0 {
                prompt_tokens = response.input_token_count;
            }

            while let Some(Ok(response)) = response_stream.next().await {
                debug!(%request_id, ?response, "Received TGIS generate_stream response");
                let finish_reason: Option<String> = match response.stop_reason() {
                    StopReason::MaxTokens | StopReason::TokenLimit => Some("length".into()),
                    StopReason::StopSequence | StopReason::EosToken => Some("stop".into()),
                    StopReason::Cancelled | StopReason::TimeLimit | StopReason::Error => Some("abort".into()),
                    StopReason::NotFinished => None
                };
                let usage = if finish_reason.is_some() {
                    let completion_tokens = response.generated_token_count;
                    let total_tokens = prompt_tokens + completion_tokens;
                    Some(Usage {
                        completion_tokens,
                        prompt_tokens,
                        total_tokens,
                    })
                } else {
                    None
                };
                let logprobs = create_logprobs(response.tokens, n_logprobs);
                let chunk = ChatCompletionChunk {
                    id: request_id.clone(),
                    choices: vec![ChatCompletionChunkChoice {
                        delta: ChatCompletionMessage {
                            role: None,
                            content: Some(response.text),
                        },
                        index: 0,
                        logprobs,
                        finish_reason,
                    }],
                    created: created_time,
                    model: model_id.clone(),
                    object: "chat.completion.chunk",
                    usage,
                };
                yield Ok(chunk.into());
            }
            yield Ok(Event::default().data("[DONE]"));
        }
    }
}

impl From<ChatCompletionRequest> for Parameters {
    fn from(req: ChatCompletionRequest) -> Self {
        let temperature = req.temperature.unwrap_or(1.0);
        let method = if temperature >= SAMPLING_EPS || req.seed.is_some() {
            DecodingMethod::Sample
        } else {
            DecodingMethod::Greedy
        };
        let sampling = SamplingParameters {
            temperature,
            top_k: req.top_k.unwrap_or_default() as u32,
            top_p: req.top_p.unwrap_or(1.0),
            typical_p: f32::default(),
            seed: req.seed,
        };
        let stopping = StoppingCriteria {
            max_new_tokens: req.max_tokens.unwrap_or(16),
            min_new_tokens: req.min_tokens.unwrap_or_default(),
            time_limit_millis: u32::default(),
            stop_sequences: match &req.stop {
                Some(StopTokens::Array(tokens)) => tokens.clone(),
                Some(StopTokens::String(token)) => vec![token.clone()],
                None => Vec::default(),
            },
            include_stop_sequence: None,
        };
        let generated_tokens = req.logprobs.unwrap_or_default();
        let token_logprobs = generated_tokens;
        let top_n_tokens = if generated_tokens {
            req.top_logprobs.unwrap_or(1)
        } else {
            u32::default()
        };
        let response = ResponseOptions {
            input_text: false,
            generated_tokens,
            input_tokens: false,
            token_logprobs,
            token_ranks: false,
            top_n_tokens,
        };
        let decoding = DecodingParameters {
            repetition_penalty: req.repetition_penalty.unwrap_or_default(),
            length_penalty: None, // TODO
        };
        Parameters {
            method: method as i32,
            sampling: Some(sampling),
            stopping: Some(stopping),
            response: Some(response),
            decoding: Some(decoding),
            truncate_input_tokens: u32::default(),
            beam: None, // TODO
        }
    }
}

impl From<(ChatCompletionRequest, String)> for BatchedGenerationRequest {
    fn from((req, prompt): (ChatCompletionRequest, String)) -> Self {
        let model_id = req.model.clone();
        let params: Parameters = req.into();
        BatchedGenerationRequest {
            model_id,
            prefix_id: None,
            requests: vec![GenerationRequest { text: prompt }],
            params: Some(params),
        }
    }
}

impl From<(ChatCompletionRequest, String)> for SingleGenerationRequest {
    fn from((req, prompt): (ChatCompletionRequest, String)) -> Self {
        let model_id = req.model.clone();
        let params: Parameters = req.into();
        SingleGenerationRequest {
            model_id,
            prefix_id: None,
            request: Some(GenerationRequest { text: prompt }),
            params: Some(params),
        }
    }
}

impl From<ChatCompletionChunk> for Event {
    fn from(value: ChatCompletionChunk) -> Self {
        Self::default().json_data(value).unwrap()
    }
}

fn create_logprobs(tokens: Vec<TokenInfo>, n_logprobs: u32) -> Option<ChatCompletionLogprobs> {
    if tokens.is_empty() {
        None
    } else {
        let content = tokens
            .into_iter()
            .map(|token_info| {
                let top_logprobs = if n_logprobs > 0 {
                    let mut tokens = token_info
                        .top_tokens
                        .into_iter()
                        .map(|t| (t.text, t.logprob))
                        .chain([(token_info.text.clone(), token_info.logprob)])
                        .collect::<HashMap<String, f32>>() // collected to hashmap to drop duplicates
                        .into_iter()
                        .map(|(token, logprob)| ChatCompletionTopLogprob { token, logprob })
                        .collect::<Vec<_>>();
                    tokens.sort_by(|a, b| a.logprob.partial_cmp(&b.logprob).unwrap());
                    Some(tokens)
                } else {
                    None
                };
                ChatCompletionLogprob {
                    token: token_info.text,
                    logprob: token_info.logprob,
                    top_logprobs,
                }
            })
            .collect::<Vec<_>>();
        Some(ChatCompletionLogprobs { content })
    }
}
