//! Northflank APIからBotのService情報と時系列Metricを取得する。

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::runtime::BotStatusConfig;
use crate::services::{HttpService, HttpServiceError};

use super::model::{
    ContainerStatus, MetricPoint, MetricSeries, ServiceStatus, StatusRange, StatusSnapshot,
};

const API_BASE_URL: &str = "https://api.northflank.com/v1";
const METADATA_CACHE_TTL: Duration = Duration::from_secs(60);
const METRICS_CACHE_TTL: Duration = Duration::from_secs(12);
const MAX_PAGES: usize = 10;

#[derive(Clone)]
struct CachedValue<T> {
    stored_at: Instant,
    value: T,
}

#[derive(Default)]
struct Cache {
    service: Option<CachedValue<ServiceStatus>>,
    containers: Option<CachedValue<Vec<ContainerStatus>>>,
    metrics_30m: Option<CachedValue<MetricBundle>>,
    metrics_1h: Option<CachedValue<MetricBundle>>,
    initial_deployment_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default)]
struct MetricBundle {
    cpu: MetricSeries,
    memory: MetricSeries,
    network_ingress: MetricSeries,
    network_egress: MetricSeries,
    requests: MetricSeries,
    http_5xx: MetricSeries,
}

pub(super) struct NorthflankDataSource {
    config: BotStatusConfig,
    http: Arc<HttpService>,
    cache: Mutex<Cache>,
}

impl NorthflankDataSource {
    pub(super) fn new(
        config: BotStatusConfig,
        http: Arc<HttpService>,
    ) -> Result<Self, BotStatusDataError> {
        if !valid_resource_id(&config.project_id)
            || !valid_resource_id(&config.service_id)
            || config.api_token.trim().is_empty()
        {
            return Err(BotStatusDataError::InvalidConfiguration);
        }
        Ok(Self {
            config,
            http,
            cache: Mutex::new(Cache::default()),
        })
    }

    pub(super) fn config(&self) -> &BotStatusConfig {
        &self.config
    }

    pub(super) async fn snapshot(
        &self,
        range: StatusRange,
        now: DateTime<Utc>,
    ) -> Result<StatusSnapshot, BotStatusDataError> {
        let started_at = Instant::now();
        let (service, containers, metrics) = tokio::try_join!(
            self.service_status(),
            self.containers(),
            self.metrics(range),
        )?;
        Ok(StatusSnapshot {
            fetched_at: now,
            api_elapsed: started_at.elapsed(),
            service,
            containers,
            cpu: metrics.cpu,
            memory: metrics.memory,
            network_ingress: metrics.network_ingress,
            network_egress: metrics.network_egress,
            requests: metrics.requests,
            http_5xx: metrics.http_5xx,
        })
    }

    pub(super) async fn initial_v2_deployment_at(
        &self,
        commit_sha: &str,
    ) -> Result<Option<DateTime<Utc>>, BotStatusDataError> {
        if let Some(value) = self.cache.lock().await.initial_deployment_at {
            return Ok(Some(value));
        }
        let mut cursor = None;
        for _ in 0..MAX_PAGES {
            let mut url = self.resource_url("deployments")?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("per_page", "100");
                if let Some(cursor) = cursor.as_deref() {
                    query.append_pair("cursor", cursor);
                }
            }
            let page: DeploymentsResponse = self.get_json(url).await?;
            if let Some(created_at) = page
                .data
                .deployments
                .iter()
                .filter(|deployment| {
                    deployment
                        .commit
                        .as_ref()
                        .and_then(|commit| commit.sha.as_deref())
                        == Some(commit_sha)
                })
                .filter_map(|deployment| parse_time(&deployment.created_at))
                .min()
            {
                self.cache.lock().await.initial_deployment_at = Some(created_at);
                return Ok(Some(created_at));
            }
            if !page.pagination.has_next_page {
                return Ok(None);
            }
            cursor = page.pagination.cursor;
            if cursor.is_none() {
                return Ok(None);
            }
        }
        Ok(None)
    }

    async fn service_status(&self) -> Result<ServiceStatus, BotStatusDataError> {
        let now = Instant::now();
        if let Some(cached) = self.cache.lock().await.service.clone()
            && now.duration_since(cached.stored_at) < METADATA_CACHE_TTL
        {
            return Ok(cached.value);
        }
        let mut deployments_url = self.resource_url("deployments")?;
        deployments_url
            .query_pairs_mut()
            .append_pair("per_page", "100");
        let (response, deployments): (Value, DeploymentsResponse) = tokio::try_join!(
            self.get_json(self.service_url()?),
            self.get_json(deployments_url),
        )?;
        let data = response
            .get("data")
            .ok_or(BotStatusDataError::InvalidResponse("missing service data"))?;
        let deployment = deployments
            .data
            .deployments
            .iter()
            .find(|deployment| deployment.active)
            .or_else(|| deployments.data.deployments.first());
        let status = ServiceStatus {
            name: text_at(data, "/name"),
            service_type: text_at(data, "/serviceType"),
            deployment_status: text_at(data, "/status/deployment/status"),
            build_status: text_at(data, "/status/build/status"),
            region: text_at(data, "/deployment/region")
                .or_else(|| text_at(data, "/region"))
                .or_else(|| text_at(data, "/cluster/name"))
                .or_else(|| text_at(data, "/cluster/id")),
            deployment_plan: text_at(data, "/billing/deploymentPlan"),
            instances: number_at(data, "/deployment/instances")
                .or_else(|| deployment.and_then(|deployment| deployment.instances)),
            branch: text_at(data, "/deployment/internal/branch")
                .or_else(|| text_at(data, "/vcsData/projectBranch"))
                .or_else(|| text_at(data, "/buildConfiguration/branch")),
            commit_sha: deployment
                .and_then(|deployment| deployment.commit.as_ref())
                .and_then(|commit| commit.sha.clone())
                .or_else(|| text_at(data, "/deployment/internal/deployedSHA"))
                .or_else(|| text_at(data, "/deployment/internal/buildSHA"))
                .or_else(|| text_at(data, "/status/deployment/sha")),
        };
        self.cache.lock().await.service = Some(CachedValue {
            stored_at: now,
            value: status.clone(),
        });
        Ok(status)
    }

    async fn containers(&self) -> Result<Vec<ContainerStatus>, BotStatusDataError> {
        let now = Instant::now();
        if let Some(cached) = self.cache.lock().await.containers.clone()
            && now.duration_since(cached.stored_at) < METADATA_CACHE_TTL
        {
            return Ok(cached.value);
        }
        let mut result = Vec::new();
        let mut cursor = None;
        for _ in 0..MAX_PAGES {
            let mut url = self.resource_url("containers")?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("per_page", "100");
                if let Some(cursor) = cursor.as_deref() {
                    query.append_pair("cursor", cursor);
                }
            }
            let page: ContainersResponse = self.get_json(url).await?;
            result.extend(page.data.containers.into_iter().filter_map(|container| {
                Some(ContainerStatus {
                    name: container.name,
                    created_at: unix_time(container.created_at)?,
                    status: container.status,
                })
            }));
            if !page.pagination.has_next_page {
                break;
            }
            cursor = page.pagination.cursor;
            if cursor.is_none() {
                break;
            }
        }
        self.cache.lock().await.containers = Some(CachedValue {
            stored_at: now,
            value: result.clone(),
        });
        Ok(result)
    }

    async fn metrics(&self, range: StatusRange) -> Result<MetricBundle, BotStatusDataError> {
        let now = Instant::now();
        let cached = {
            let cache = self.cache.lock().await;
            match range {
                StatusRange::ThirtyMinutes => cache.metrics_30m.clone(),
                StatusRange::OneHour => cache.metrics_1h.clone(),
            }
        };
        if let Some(cached) = cached
            && now.duration_since(cached.stored_at) < METRICS_CACHE_TTL
        {
            return Ok(cached.value);
        }

        let mut url = self.resource_url("metrics")?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("queryType", "range")
                .append_pair("duration", &range.duration().as_secs().to_string());
            for metric in [
                "cpu",
                "memory",
                "networkIngress",
                "networkEgress",
                "requests",
                "http5xxResponses",
            ] {
                query.append_pair("metricTypes", metric);
            }
        }
        let response: MetricsResponse = self.get_json(url).await?;
        let metrics = MetricBundle {
            cpu: series(&response.data, "cpu", Aggregation::Average),
            memory: series(&response.data, "memory", Aggregation::Average),
            network_ingress: series(&response.data, "networkIngress", Aggregation::Sum),
            network_egress: series(&response.data, "networkEgress", Aggregation::Sum),
            requests: series(&response.data, "requests", Aggregation::Sum),
            http_5xx: series(&response.data, "http5xxResponses", Aggregation::Sum),
        };
        let cached = CachedValue {
            stored_at: now,
            value: metrics.clone(),
        };
        let mut cache = self.cache.lock().await;
        match range {
            StatusRange::ThirtyMinutes => cache.metrics_30m = Some(cached),
            StatusRange::OneHour => cache.metrics_1h = Some(cached),
        }
        Ok(metrics)
    }

    async fn get_json<T: DeserializeOwned>(&self, url: Url) -> Result<T, BotStatusDataError> {
        let headers = [(
            "authorization".to_owned(),
            format!("Bearer {}", self.config.api_token),
        )];
        let response = self
            .http
            .request(Method::GET, url.as_str(), &headers, None)
            .await?;
        if response.status == StatusCode::UNAUTHORIZED || response.status == StatusCode::FORBIDDEN {
            return Err(BotStatusDataError::Unauthorized);
        }
        if !response.status.is_success() {
            return Err(BotStatusDataError::Status(response.status));
        }
        serde_json::from_slice(&response.body).map_err(BotStatusDataError::Json)
    }

    fn service_url(&self) -> Result<Url, BotStatusDataError> {
        Url::parse(&format!(
            "{API_BASE_URL}/projects/{}/services/{}",
            self.config.project_id, self.config.service_id
        ))
        .map_err(|_| BotStatusDataError::InvalidConfiguration)
    }

    fn resource_url(&self, resource: &str) -> Result<Url, BotStatusDataError> {
        Url::parse(&format!("{}/{resource}", self.service_url()?))
            .map_err(|_| BotStatusDataError::InvalidConfiguration)
    }
}

#[derive(Clone, Copy)]
enum Aggregation {
    Average,
    Sum,
}

fn series(
    metrics: &HashMap<String, MetricResponse>,
    name: &str,
    aggregation: Aggregation,
) -> MetricSeries {
    let Some(metric) = metrics.get(name) else {
        return MetricSeries::default();
    };
    let mut buckets = BTreeMap::<i64, (DateTime<Utc>, f64, usize)>::new();
    for value_set in &metric.values {
        for point in &value_set.data {
            let Some(at) = parse_time(&point.ts) else {
                continue;
            };
            if !point.value.is_finite() {
                continue;
            }
            let entry = buckets.entry(at.timestamp_millis()).or_insert((at, 0.0, 0));
            entry.1 += point.value;
            entry.2 += 1;
        }
    }
    let points = buckets
        .into_values()
        .map(|(at, total, count)| MetricPoint {
            at,
            value: match aggregation {
                Aggregation::Average => total / count.max(1) as f64,
                Aggregation::Sum => total,
            },
        })
        .collect();
    MetricSeries {
        unit: metric.metric_info.metric_unit.clone(),
        points,
    }
}

fn text_at(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer)?.as_str().map(str::to_owned)
}

fn number_at(value: &Value, pointer: &str) -> Option<u32> {
    let value = value.pointer(pointer)?;
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|number| number.round() as u64))
        .and_then(|number| u32::try_from(number).ok())
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn unix_time(value: i64) -> Option<DateTime<Utc>> {
    if value >= 1_000_000_000_000 {
        Utc.timestamp_millis_opt(value).single()
    } else {
        Utc.timestamp_opt(value, 0).single()
    }
}

fn valid_resource_id(value: &str) -> bool {
    let mut characters = value.chars();
    value.len() >= 3
        && value.len() <= 54
        && characters
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '-')
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContainersResponse {
    data: ContainerData,
    pagination: Pagination,
}

#[derive(Deserialize)]
struct ContainerData {
    containers: Vec<ApiContainer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiContainer {
    name: String,
    created_at: i64,
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeploymentsResponse {
    data: DeploymentData,
    pagination: Pagination,
}

#[derive(Deserialize)]
struct DeploymentData {
    deployments: Vec<ApiDeployment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiDeployment {
    created_at: String,
    #[serde(default)]
    active: bool,
    instances: Option<u32>,
    commit: Option<ApiCommit>,
}

#[derive(Deserialize)]
struct ApiCommit {
    sha: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    has_next_page: bool,
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct MetricsResponse {
    data: HashMap<String, MetricResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetricResponse {
    metric_info: MetricInfo,
    values: Vec<MetricValues>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetricInfo {
    metric_unit: String,
}

#[derive(Deserialize)]
struct MetricValues {
    data: Vec<ApiMetricPoint>,
}

#[derive(Deserialize)]
struct ApiMetricPoint {
    value: f64,
    ts: String,
}

#[derive(Debug)]
pub(super) enum BotStatusDataError {
    InvalidConfiguration,
    Unauthorized,
    Status(StatusCode),
    Http(HttpServiceError),
    Json(serde_json::Error),
    InvalidResponse(&'static str),
}

impl Display for BotStatusDataError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfiguration => formatter.write_str("invalid Northflank configuration"),
            Self::Unauthorized => formatter.write_str("Northflank API access was denied"),
            Self::Status(status) => write!(formatter, "Northflank API returned {status}"),
            Self::Http(error) => Display::fmt(error, formatter),
            Self::Json(error) => write!(formatter, "invalid Northflank response: {error}"),
            Self::InvalidResponse(reason) => {
                write!(formatter, "invalid Northflank response: {reason}")
            }
        }
    }
}

impl Error for BotStatusDataError {}

impl From<HttpServiceError> for BotStatusDataError {
    fn from(error: HttpServiceError) -> Self {
        Self::Http(error)
    }
}
