//! OTLP trace exporter setup (feature-gated behind `otlp`).

use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

/// Build OTLP span exporter + provider, return the provider.
///
/// Uses HTTP/protobuf transport via reqwest. Reads endpoint from
/// `OTEL_EXPORTER_OTLP_ENDPOINT` env var (default: `http://localhost:4318`).
///
/// Caller must hold the returned `SdkTracerProvider` alive and call
/// `provider.shutdown()` on graceful exit to flush pending spans.
pub fn build_provider() -> Result<SdkTracerProvider, Box<dyn std::error::Error + Send + Sync>> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();

    Ok(provider)
}

/// Create a tracing layer from a provider.
pub fn layer<S>(
    provider: &SdkTracerProvider,
) -> tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::Tracer>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    let tracer = provider.tracer("asterlane");
    tracing_opentelemetry::layer().with_tracer(tracer)
}
