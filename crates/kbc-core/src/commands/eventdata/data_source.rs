//! 公式・KBCイベントデータのURL発行、取得、検証、暗号化を行う。

use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;

use aes::Aes128;
use cipher::{BlockEncryptMut, KeyInit, block_padding::Pkcs7};
use ecb::Encryptor;
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::services::{Clock, HttpService};

use super::{EventCountry, EventDataType, SelectedRequest};

const OFFICIAL_ORIGIN: &str = "https://nyanko-events.ponosgames.com";
const KBC_ORIGIN: &str = "https://kbc-rakv0.vercel.app";
const USER_AGENT: &str = "Dalvik/2.1.0 (Linux; Android 9; SM-G955F Build/N2G48B)";
const CREDENTIAL_TTL: Duration = Duration::from_secs(10 * 60);

struct Credential {
    token: String,
    created_at: Instant,
}

#[derive(Clone)]
pub(super) struct EventDataSource {
    http: Arc<HttpService>,
    clock: Arc<dyn Clock>,
    credential: Arc<Mutex<Option<Credential>>>,
}

impl EventDataSource {
    pub(super) fn new(http: Arc<HttpService>, clock: Arc<dyn Clock>) -> Self {
        Self {
            http,
            clock,
            credential: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) async fn official_links(
        &self,
        types: &[EventDataType],
        country: EventCountry,
    ) -> Result<HashMap<EventDataType, String>, EventDataError> {
        let token = if types.iter().any(|data_type| data_type.needs_jwt()) {
            Some(self.token().await?)
        } else {
            None
        };
        let mut links = HashMap::new();
        for data_type in types {
            let url = build_official_url(*data_type, country, token.as_deref())
                .ok_or_else(|| EventDataError::new("official event URL could not be created"))?;
            links.insert(*data_type, url);
        }
        Ok(links)
    }

    pub(super) async fn attachment(
        &self,
        request: SelectedRequest,
    ) -> Result<EventAttachment, EventDataError> {
        let file = file_info(request.data_type);
        if request.kbc {
            let data = self
                .fetch_bytes(&build_kbc_url(
                    request.data_type,
                    request.country,
                    request.encrypted,
                ))
                .await?;
            if !request.encrypted {
                validate_event_data(request.data_type, &data)?;
            }
            return Ok(EventAttachment {
                data,
                file_name: if request.encrypted {
                    file.encrypted
                } else {
                    file.plain
                }
                .to_owned(),
            });
        }

        let links = self
            .official_links(&[request.data_type], request.country)
            .await?;
        let url = links
            .get(&request.data_type)
            .ok_or_else(|| EventDataError::new("official event URL is unavailable"))?;
        let data = self.fetch_bytes(url).await?;
        validate_event_data(request.data_type, &data)?;
        Ok(EventAttachment {
            data: if request.encrypted {
                encrypt_event_data(&data, request.country)?
            } else {
                data
            },
            file_name: if request.encrypted {
                file.encrypted
            } else {
                file.plain
            }
            .to_owned(),
        })
    }

    async fn token(&self) -> Result<String, EventDataError> {
        let mut credential = self.credential.lock().await;
        let now = Instant::now();
        if let Some(current) = credential.as_ref()
            && now.duration_since(current.created_at) < CREDENTIAL_TTL
        {
            return Ok(current.token.clone());
        }
        let token = self.issue_token().await?;
        *credential = Some(Credential {
            token: token.clone(),
            created_at: now,
        });
        Ok(token)
    }

    async fn issue_token(&self) -> Result<String, EventDataError> {
        let account: AccountResponse = self
            .fetch_json(
                "https://nyanko-backups.ponosgames.com/?action=createAccount&referenceId=",
                Method::GET,
                Vec::new(),
                None,
            )
            .await?;
        let account_code = account
            .account_id
            .filter(|value| !value.is_empty())
            .ok_or_else(|| EventDataError::new("event account response is invalid"))?;
        let timestamp = self.clock.now().timestamp();

        let user_body = serde_json::to_vec(&json!({
            "accountCode": account_code,
            "accountCreatedAt": timestamp.to_string(),
            "nonce": random_hex(16)?,
        }))
        .map_err(|error| EventDataError::new(format!("user request encoding failed: {error}")))?;
        let user: UserResponse = self
            .fetch_json(
                "https://nyanko-auth.ponosgames.com/v1/users",
                Method::POST,
                signed_headers(&account_code, timestamp, &user_body)?,
                Some(user_body),
            )
            .await?;
        let password = user
            .payload
            .and_then(|payload| payload.password)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| EventDataError::new("event user response is invalid"))?;

        let token_body = serde_json::to_vec(&json!({
            "clientInfo": {
                "client": { "countryCode": "ja", "version": "999999" },
                "device": { "model": "ONEPLUS A3010" },
                "os": { "type": "android", "version": "7.1.1" }
            },
            "password": password,
            "accountCode": account_code,
            "nonce": random_hex(16)?,
        }))
        .map_err(|error| EventDataError::new(format!("token request encoding failed: {error}")))?;
        let token: TokenResponse = self
            .fetch_json(
                "https://nyanko-auth.ponosgames.com/v1/tokens",
                Method::POST,
                signed_headers(&account_code, timestamp, &token_body)?,
                Some(token_body),
            )
            .await?;
        token
            .payload
            .and_then(|payload| payload.token)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| EventDataError::new("event token response is invalid"))
    }

    async fn fetch_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        method: Method,
        mut headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    ) -> Result<T, EventDataError> {
        headers.push(("user-agent".to_owned(), USER_AGENT.to_owned()));
        let data = self
            .http
            .request_bytes(method, url, &headers, body)
            .await
            .map_err(|error| {
                EventDataError::new(format!("event authentication failed: {error}"))
            })?;
        serde_json::from_slice(&data).map_err(|error| {
            EventDataError::new(format!("event JSON response is invalid: {error}"))
        })
    }

    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>, EventDataError> {
        self.http
            .request_bytes(
                Method::GET,
                url,
                &[("user-agent".to_owned(), USER_AGENT.to_owned())],
                None,
            )
            .await
            .map_err(|error| EventDataError::new(format!("event data request failed: {error}")))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountResponse {
    account_id: Option<String>,
}

#[derive(Deserialize)]
struct UserResponse {
    payload: Option<UserPayload>,
}

#[derive(Deserialize)]
struct UserPayload {
    password: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    payload: Option<TokenPayload>,
}

#[derive(Deserialize)]
struct TokenPayload {
    token: Option<String>,
}

fn signed_headers(
    account_code: &str,
    timestamp: i64,
    body: &[u8],
) -> Result<Vec<(String, String)>, EventDataError> {
    let random_data = random_hex(32)?;
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(format!("{account_code}{random_data}").as_bytes())
            .map_err(|_| EventDataError::new("event signature key is invalid"))?;
    mac.update(body);
    let signature = format!("{random_data}{}", hex_encode(&mac.finalize().into_bytes()));
    Ok(vec![
        ("content-type".to_owned(), "application/json".to_owned()),
        ("nyanko-signature".to_owned(), signature),
        ("nyanko-timestamp".to_owned(), timestamp.to_string()),
        ("nyanko-signature-version".to_owned(), "1".to_owned()),
        (
            "nyanko-signature-algorithm".to_owned(),
            "HMACSHA256".to_owned(),
        ),
    ])
}

fn random_hex(length: usize) -> Result<String, EventDataError> {
    let mut bytes = vec![0; length];
    getrandom::fill(&mut bytes).map_err(|error| {
        EventDataError::new(format!("secure random generation failed: {error}"))
    })?;
    Ok(hex_encode(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

struct RegionInfo {
    product: &'static str,
    event_base: &'static str,
    salt: &'static str,
}

fn region_info(country: EventCountry) -> RegionInfo {
    match country {
        EventCountry::Jp => RegionInfo {
            product: "battlecats",
            event_base: "battlecats_production",
            salt: "battlecats",
        },
        EventCountry::En => RegionInfo {
            product: "battlecatsen",
            event_base: "battlecatsen_production",
            salt: "battlecatsen",
        },
        EventCountry::Kr => RegionInfo {
            product: "battlecatskr",
            event_base: "battlecatskr_production",
            salt: "battlecatskr",
        },
        EventCountry::Tw => RegionInfo {
            product: "battlecatstw",
            event_base: "battlecatstw_production",
            salt: "battlecatstw",
        },
    }
}

pub(super) fn build_official_url(
    data_type: EventDataType,
    country: EventCountry,
    token: Option<&str>,
) -> Option<String> {
    let region = region_info(country);
    match data_type {
        EventDataType::Ad => Some(format!(
            "{OFFICIAL_ORIGIN}/control/ad/battlecats/adcontrol.json"
        )),
        EventDataType::Notice => Some(format!(
            "{OFFICIAL_ORIGIN}/control/placement/{}/event.json",
            region.product
        )),
        _ => Some(format!(
            "{OFFICIAL_ORIGIN}/{}/{}.tsv?jwt={}",
            region.event_base,
            data_type.label(),
            token?
        )),
    }
}

pub(super) fn build_kbc_url(
    data_type: EventDataType,
    country: EventCountry,
    encrypted: bool,
) -> String {
    let region = region_info(country);
    let path = match data_type {
        EventDataType::Ad => "control/ad/battlecats/adcontrol.json".to_owned(),
        EventDataType::Notice => {
            format!("control/placement/{}/event.json", region.product)
        }
        _ => format!("{}/{}.tsv", region.event_base, data_type.label()),
    };
    format!(
        "{KBC_ORIGIN}/nyanko-events/{path}{}",
        if encrypted { "?enc=1" } else { "" }
    )
}

struct FileInfo {
    plain: &'static str,
    encrypted: &'static str,
}

fn file_info(data_type: EventDataType) -> FileInfo {
    match data_type {
        EventDataType::Sale => FileInfo {
            plain: "sale.tsv",
            encrypted: "002a4b18244f32d7833fd81bc833b97f.dat",
        },
        EventDataType::Gatya => FileInfo {
            plain: "gatya.tsv",
            encrypted: "09b1058188348630d98a08e0f731f6bd.dat",
        },
        EventDataType::Item => FileInfo {
            plain: "item.tsv",
            encrypted: "408f66def075926baea9466e70504a3b.dat",
        },
        EventDataType::Ad => FileInfo {
            plain: "ad.json",
            encrypted: "523af537946b79c4f8369ed39ba78605.dat",
        },
        EventDataType::Notice => FileInfo {
            plain: "popup_notice.json",
            encrypted: "e4698396f16e151d6634fee4dfa32741.dat",
        },
    }
}

fn validate_event_data(data_type: EventDataType, value: &[u8]) -> Result<(), EventDataError> {
    if value.is_empty() {
        return Err(EventDataError::new("event data response is empty"));
    }
    let text = std::str::from_utf8(value)
        .map_err(|_| EventDataError::new("event data response is not UTF-8"))?
        .trim_start_matches('\u{feff}');
    match data_type {
        EventDataType::Ad | EventDataType::Notice => {
            let parsed: Value = serde_json::from_str(text)
                .map_err(|error| EventDataError::new(format!("event JSON is invalid: {error}")))?;
            if !parsed.is_object() {
                return Err(EventDataError::new("event JSON root is not an object"));
            }
        }
        _ if !text.contains('\t') => {
            return Err(EventDataError::new("event TSV has no tab-separated fields"));
        }
        _ => {}
    }
    Ok(())
}

fn encrypt_event_data(value: &[u8], country: EventCountry) -> Result<Vec<u8>, EventDataError> {
    let digest = Md5::digest(b"battlecats");
    let key_hex = hex_encode(&digest);
    let encryptor = Encryptor::<Aes128>::new_from_slice(&key_hex.as_bytes()[..16])
        .map_err(|_| EventDataError::new("event encryption key is invalid"))?;
    let ciphertext = encryptor.encrypt_padded_vec_mut::<Pkcs7>(value);
    let mut signature_input =
        Vec::with_capacity(region_info(country).salt.len() + ciphertext.len());
    signature_input.extend_from_slice(region_info(country).salt.as_bytes());
    signature_input.extend_from_slice(&ciphertext);
    let signature = hex_encode(&Md5::digest(&signature_input));
    let mut output = Vec::with_capacity(ciphertext.len() + signature.len());
    output.extend_from_slice(&ciphertext);
    output.extend_from_slice(signature.as_bytes());
    Ok(output)
}

pub(super) struct EventAttachment {
    pub(super) data: Vec<u8>,
    pub(super) file_name: String,
}

#[derive(Debug)]
pub(super) struct EventDataError {
    reason: String,
}

impl EventDataError {
    pub(super) fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl Display for EventDataError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.reason)
    }
}
