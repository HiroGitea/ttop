//! Minimal tonic client for libtpu's `RuntimeMetricService`, written by hand so
//! that no `protoc` is needed at build time.

use std::time::Duration;

use anyhow::{Context, Result};
use tonic::codec::ProstCodec;
use tonic::codegen::http::uri::PathAndQuery;
use tonic::transport::{Channel, Endpoint};

use crate::proto::*;

const GET_RUNTIME_METRIC: &str =
    "/tpu.monitoring.runtime.RuntimeMetricService/GetRuntimeMetric";
const LIST_SUPPORTED_METRICS: &str =
    "/tpu.monitoring.runtime.RuntimeMetricService/ListSupportedMetrics";

#[derive(Clone)]
pub struct RuntimeMetricClient {
    inner: tonic::client::Grpc<Channel>,
}

impl RuntimeMetricClient {
    /// Connects to `localhost:<port>`.  libtpu's server uses gRPC "local"
    /// credentials, which only check that the peer is on loopback and are
    /// plaintext HTTP/2 on the wire, so an insecure channel is sufficient.
    pub async fn connect(port: u16, timeout: Duration) -> Result<Self> {
        let ep = Endpoint::from_shared(format!("http://localhost:{port}"))?
            .connect_timeout(timeout)
            .timeout(timeout)
            .tcp_nodelay(true);
        let channel = ep
            .connect()
            .await
            .with_context(|| format!("connect to libtpu metrics server on port {port}"))?;
        Ok(Self {
            inner: tonic::client::Grpc::new(channel),
        })
    }

    pub async fn get_runtime_metric(&mut self, name: &str) -> Result<Vec<Metric>> {
        self.inner
            .ready()
            .await
            .map_err(|e| anyhow::anyhow!("grpc not ready: {e}"))?;
        let codec: ProstCodec<MetricRequest, MetricResponse> = ProstCodec::default();
        let path = PathAndQuery::from_static(GET_RUNTIME_METRIC);
        let req = tonic::Request::new(MetricRequest {
            metric_name: name.to_string(),
            skip_node_aggregation: false,
        });
        let resp = self
            .inner
            .unary(req, path, codec)
            .await
            .with_context(|| format!("GetRuntimeMetric({name})"))?;
        Ok(resp.into_inner().metric.map(|m| m.metrics).unwrap_or_default())
    }

    pub async fn list_supported_metrics(&mut self) -> Result<Vec<String>> {
        self.inner
            .ready()
            .await
            .map_err(|e| anyhow::anyhow!("grpc not ready: {e}"))?;
        let codec: ProstCodec<ListSupportedMetricsRequest, ListSupportedMetricsResponse> =
            ProstCodec::default();
        let path = PathAndQuery::from_static(LIST_SUPPORTED_METRICS);
        let req = tonic::Request::new(ListSupportedMetricsRequest::default());
        let resp = self
            .inner
            .unary(req, path, codec)
            .await
            .context("ListSupportedMetrics")?;
        Ok(resp
            .into_inner()
            .supported_metric
            .into_iter()
            .map(|m| m.metric_name)
            .collect())
    }
}
