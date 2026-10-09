//! Hand-written prost definitions for the subset of
//! `tpu.monitoring.runtime.RuntimeMetricService` that we need.
//!
//! Field numbers follow `tpu_metric_service.proto` from
//! google/cloud-accelerator-diagnostics (tpu_info).  Unknown fields are
//! skipped by prost, so omitting the rest of the schema is safe.

use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct MetricRequest {
    #[prost(string, tag = "1")]
    pub metric_name: String,
    #[prost(bool, tag = "2")]
    pub skip_node_aggregation: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct MetricResponse {
    #[prost(message, optional, tag = "1")]
    pub metric: Option<TpuMetric>,
}

#[derive(Clone, PartialEq, Message)]
pub struct TpuMetric {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub description: String,
    #[prost(message, repeated, tag = "3")]
    pub metrics: Vec<Metric>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Metric {
    #[prost(message, optional, tag = "1")]
    pub attribute: Option<Attribute>,
    #[prost(oneof = "metric::Measure", tags = "3, 4, 5")]
    pub measure: Option<metric::Measure>,
}

pub mod metric {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Measure {
        #[prost(message, tag = "3")]
        Gauge(super::Gauge),
        #[prost(message, tag = "4")]
        Counter(super::Counter),
        #[prost(message, tag = "5")]
        Distribution(super::Distribution),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct Attribute {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(message, optional, tag = "2")]
    pub value: Option<AttrValue>,
}

#[derive(Clone, PartialEq, Message)]
pub struct AttrValue {
    #[prost(oneof = "attr_value::Attr", tags = "1, 2, 3, 4, 6")]
    pub attr: Option<attr_value::Attr>,
}

pub mod attr_value {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Attr {
        #[prost(string, tag = "1")]
        StringAttr(String),
        #[prost(bool, tag = "2")]
        BoolAttr(bool),
        #[prost(int64, tag = "3")]
        IntAttr(i64),
        #[prost(double, tag = "4")]
        DoubleAttr(f64),
        #[prost(message, tag = "6")]
        KvlistAttr(super::KeyValueList),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct KeyValueList {
    #[prost(message, repeated, tag = "1")]
    pub attributes: Vec<Attribute>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Gauge {
    #[prost(oneof = "gauge::Value", tags = "1, 2, 3, 4")]
    pub value: Option<gauge::Value>,
}

pub mod gauge {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Value {
        #[prost(double, tag = "1")]
        AsDouble(f64),
        #[prost(int64, tag = "2")]
        AsInt(i64),
        #[prost(string, tag = "3")]
        AsString(String),
        #[prost(bool, tag = "4")]
        AsBool(bool),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct Counter {
    #[prost(oneof = "counter::Value", tags = "1, 2")]
    pub value: Option<counter::Value>,
}

pub mod counter {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Value {
        #[prost(double, tag = "1")]
        AsDouble(f64),
        #[prost(uint64, tag = "2")]
        AsInt(u64),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct Distribution {
    #[prost(int64, tag = "1")]
    pub count: i64,
    #[prost(double, tag = "2")]
    pub mean: f64,
    #[prost(double, tag = "3")]
    pub min: f64,
    #[prost(double, tag = "4")]
    pub max: f64,
    #[prost(double, tag = "5")]
    pub sum_of_squared_deviation: f64,
    #[prost(message, optional, tag = "6")]
    pub bucket_options: Option<BucketOptions>,
    #[prost(int64, repeated, tag = "7")]
    pub bucket_counts: Vec<i64>,
}

#[derive(Clone, PartialEq, Message)]
pub struct BucketOptions {
    #[prost(oneof = "bucket_options::Options", tags = "2, 3, 4")]
    pub options: Option<bucket_options::Options>,
}

pub mod bucket_options {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Options {
        #[prost(message, tag = "2")]
        ExponentialBuckets(super::Exponential),
        #[prost(message, tag = "3")]
        ExplicitBuckets(super::Explicit),
        #[prost(message, tag = "4")]
        LinearBuckets(super::Linear),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct Exponential {
    #[prost(int32, tag = "1")]
    pub num_finite_buckets: i32,
    #[prost(double, tag = "2")]
    pub growth_factor: f64,
    #[prost(double, tag = "3")]
    pub scale: f64,
}

#[derive(Clone, PartialEq, Message)]
pub struct Explicit {
    #[prost(double, repeated, tag = "1")]
    pub bounds: Vec<f64>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Linear {
    #[prost(int32, tag = "1")]
    pub num_finite_buckets: i32,
    #[prost(double, tag = "2")]
    pub width: f64,
    #[prost(double, tag = "3")]
    pub offset: f64,
}

#[derive(Clone, PartialEq, Message)]
pub struct ListSupportedMetricsRequest {
    #[prost(string, tag = "1")]
    pub filter: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct SupportedMetric {
    #[prost(string, tag = "1")]
    pub metric_name: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ListSupportedMetricsResponse {
    #[prost(message, repeated, tag = "1")]
    pub supported_metric: Vec<SupportedMetric>,
}

// ---- convenience accessors -------------------------------------------------

impl Metric {
    /// Returns the gauge as f64 (int or double), if this metric is a gauge.
    pub fn gauge_f64(&self) -> Option<f64> {
        match &self.measure {
            Some(metric::Measure::Gauge(g)) => match &g.value {
                Some(gauge::Value::AsDouble(v)) => Some(*v),
                Some(gauge::Value::AsInt(v)) => Some(*v as f64),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn gauge_i64(&self) -> Option<i64> {
        match &self.measure {
            Some(metric::Measure::Gauge(g)) => match &g.value {
                Some(gauge::Value::AsInt(v)) => Some(*v),
                Some(gauge::Value::AsDouble(v)) => Some(*v as i64),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn distribution(&self) -> Option<&Distribution> {
        match &self.measure {
            Some(metric::Measure::Distribution(d)) => Some(d),
            _ => None,
        }
    }

    /// Device id for the "simple" metrics (hbm usage/total, duty cycle...),
    /// where `attribute.value` is a bare int.
    pub fn device_id_simple(&self) -> Option<i64> {
        match self.attribute.as_ref()?.value.as_ref()?.attr.as_ref()? {
            attr_value::Attr::IntAttr(v) => Some(*v),
            attr_value::Attr::StringAttr(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// Device id for the "kvlist" metrics (hlo queue size, exec timing...),
    /// where `attribute.value` is a key/value list containing `device_ordinal`.
    pub fn device_id_kvlist(&self) -> Option<i64> {
        let kv = match self.attribute.as_ref()?.value.as_ref()?.attr.as_ref()? {
            attr_value::Attr::KvlistAttr(kv) => kv,
            _ => return None,
        };
        for a in &kv.attributes {
            if a.key == "device_ordinal" {
                return match a.value.as_ref()?.attr.as_ref()? {
                    attr_value::Attr::IntAttr(v) => Some(*v),
                    attr_value::Attr::StringAttr(s) => s.trim().parse().ok(),
                    _ => None,
                };
            }
        }
        None
    }

    /// Either flavour of device id.
    pub fn device_id(&self) -> Option<i64> {
        self.device_id_simple().or_else(|| self.device_id_kvlist())
    }
}
