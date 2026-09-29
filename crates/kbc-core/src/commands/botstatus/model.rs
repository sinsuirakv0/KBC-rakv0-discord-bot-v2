//! Bot statusの取得結果と表示範囲を表す。

use std::time::Duration;

use chrono::{DateTime, Utc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatusRange {
    ThirtyMinutes,
    OneHour,
}

impl StatusRange {
    pub(super) fn duration(self) -> Duration {
        match self {
            Self::ThirtyMinutes => Duration::from_secs(30 * 60),
            Self::OneHour => Duration::from_secs(60 * 60),
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::ThirtyMinutes => "直近30分",
            Self::OneHour => "直近1時間",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct MetricPoint {
    pub(super) at: DateTime<Utc>,
    pub(super) value: f64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct MetricSeries {
    pub(super) unit: String,
    pub(super) points: Vec<MetricPoint>,
}

impl MetricSeries {
    pub(super) fn latest(&self) -> Option<f64> {
        self.points.last().map(|point| point.value)
    }

    pub(super) fn average(&self) -> Option<f64> {
        (!self.points.is_empty()).then(|| {
            self.points.iter().map(|point| point.value).sum::<f64>() / self.points.len() as f64
        })
    }

    pub(super) fn maximum(&self) -> Option<f64> {
        self.points.iter().map(|point| point.value).reduce(f64::max)
    }
}

#[derive(Clone, Debug)]
pub(super) struct ContainerStatus {
    pub(super) name: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) status: String,
}

impl ContainerStatus {
    pub(super) fn is_running(&self) -> bool {
        self.status == "TASK_RUNNING"
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct ServiceStatus {
    pub(super) name: Option<String>,
    pub(super) service_type: Option<String>,
    pub(super) deployment_status: Option<String>,
    pub(super) build_status: Option<String>,
    pub(super) region: Option<String>,
    pub(super) deployment_plan: Option<String>,
    pub(super) instances: Option<u32>,
    pub(super) branch: Option<String>,
    pub(super) commit_sha: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct StatusSnapshot {
    pub(super) fetched_at: DateTime<Utc>,
    pub(super) api_elapsed: Duration,
    pub(super) service: ServiceStatus,
    pub(super) containers: Vec<ContainerStatus>,
    pub(super) cpu: MetricSeries,
    pub(super) memory: MetricSeries,
    pub(super) network_ingress: MetricSeries,
    pub(super) network_egress: MetricSeries,
    pub(super) requests: MetricSeries,
    pub(super) http_5xx: MetricSeries,
}
