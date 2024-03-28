//! Inspired by: https://github.com/open-telemetry/opentelemetry-rust gRPC examples
use opentelemetry::{
    global,
    propagation::{Extractor, Injector},
    KeyValue,
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace, trace::Sampler, Resource};
use tonic::Request;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

struct MetadataExtractor<'a>(&'a tonic::metadata::MetadataMap);

impl<'a> Extractor for MetadataExtractor<'a> {
    /// Get a value for a key from the MetadataMap.  If the value can't be converted to &str, returns None
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|metadata| metadata.to_str().ok())
    }

    /// Collect all the keys from the MetadataMap.
    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .map(|key| match key {
                tonic::metadata::KeyRef::Ascii(v) => v.as_str(),
                tonic::metadata::KeyRef::Binary(v) => v.as_str(),
            })
            .collect::<Vec<_>>()
    }
}

/// Inject context in the metadata of a gRPC request.
struct MetadataInjector<'a>(pub &'a mut tonic::metadata::MetadataMap);

impl<'a> Injector for MetadataInjector<'a> {
    /// Set a key and value in the MetadataMap.  Does nothing if the key or value are not valid inputs
    fn set(&mut self, key: &str, value: String) {
        if let Ok(key) = tonic::metadata::MetadataKey::from_bytes(key.as_bytes()) {
            if let Ok(val) = value.parse() {
                self.0.insert(key, val);
            }
        }
    }
}

/// Get context from a span and inject into GRPC requests's metadata
fn inject_span(metadata: &mut tonic::metadata::MetadataMap, span: &Span) {
    let ctx = span.context();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&ctx, &mut MetadataInjector(metadata))
    })
}

pub trait InjectTelemetryContext {
    fn inject_context_span(self, span: &Span) -> Self;
}

impl<T> InjectTelemetryContext for Request<T> {
    fn inject_context_span(mut self, span: &Span) -> Self {
        inject_span(self.metadata_mut(), span);
        self
    }
}

/// Extract context from metadata and set as passed span's context
fn extract_span(metadata: &tonic::metadata::MetadataMap, span: &mut Span) {
    let parent_cx =
        global::get_text_map_propagator(|prop| prop.extract(&MetadataExtractor(metadata)));
    span.set_parent(parent_cx);
}

pub trait ExtractTelemetryContext {
    fn extract_context_span(self, span: &mut Span) -> Self;
}

impl<T> ExtractTelemetryContext for Request<T> {
    fn extract_context_span(self, span: &mut Span) -> Self {
        extract_span(self.metadata(), span);
        self
    }
}

pub fn init_logging(service_name: String, json_output: bool, otlp_endpoint: Option<String>) {
    let mut layers = Vec::new();
    // Configure log level; use info by default
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    let fmt_layer = match json_output {
        true => tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .boxed(),
        false => tracing_subscriber::fmt::layer().boxed(),
    };
    layers.push(fmt_layer);

    if let Some(tracing_otlp_endpoint) = otlp_endpoint {
        global::set_text_map_propagator(TraceContextPropagator::new());
        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(
                opentelemetry_otlp::new_exporter()
                    .tonic()
                    .with_endpoint(tracing_otlp_endpoint),
            )
            .with_trace_config(
                trace::config()
                    .with_resource(Resource::new(vec![KeyValue::new(
                        "service.name",
                        service_name,
                    )]))
                    .with_sampler(Sampler::AlwaysOn),
            )
            .install_batch(opentelemetry_sdk::runtime::Tokio);

        if let Ok(tracer) = tracer {
            layers.push(tracing_opentelemetry::layer().with_tracer(tracer).boxed());
        };
    }

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(layers)
        .init();
}
