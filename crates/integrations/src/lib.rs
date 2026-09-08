//! Typed boundaries for external services.

use std::{collections::HashSet, hash::BuildHasher, sync::Arc, time::Duration};

use anyhow::Result;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use url::Url;
use vpn_domain::{ProxyNode, SubscriptionProfile};

pub mod provider {
    use super::{Client, ProxyNode, SubscriptionProfile, Url, Value};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum ProviderError {
        #[error("provider request failed")]
        Request(#[from] reqwest::Error),
        #[error("provider response was invalid")]
        InvalidResponse,
        #[error("provider has no subscription source")]
        MissingSubscriptionSource,
        #[error("provider source has no supported nodes")]
        NoSupportedNodes,
    }

    #[derive(Clone)]
    pub struct RemnawaveClient {
        http: Client,
        endpoint: Url,
        token: String,
    }

    impl RemnawaveClient {
        /// Creates an adapter for the Remnawave API.
        ///
        /// # Errors
        ///
        /// Returns an error when the endpoint is not an absolute URL or the HTTP client cannot be created.
        pub fn new(endpoint: Url, token: String) -> Result<Self, reqwest::Error> {
            Ok(Self {
                http: Client::builder()
                    .timeout(std::time::Duration::from_secs(20))
                    .build()?,
                endpoint,
                token,
            })
        }

        /// Fetches and normalizes the provider profile without exposing provider fields.
        ///
        /// # Errors
        ///
        /// Returns an error for transport failures, malformed provider data, or an empty source.
        pub async fn fetch_profile(
            &self,
            external_user_id: i64,
        ) -> Result<SubscriptionProfile, ProviderError> {
            let path = format!("api/users/by-telegram-id/{external_user_id}");
            let url = self
                .endpoint
                .join(&path)
                .map_err(|_| ProviderError::InvalidResponse)?;
            let response = self
                .http
                .get(url)
                .bearer_auth(&self.token)
                .send()
                .await?
                .error_for_status()?;
            let payload = response.json::<Value>().await?;
            let source = find_string(&payload, &["subscriptionUrl", "subscription_url"])
                .ok_or(ProviderError::MissingSubscriptionSource)?;
            let source_response = self.http.get(source).send().await?.error_for_status()?;
            let source_body = source_response.text().await?;
            let nodes = parse_nodes(&source_body);
            if nodes.is_empty() {
                return Err(ProviderError::NoSupportedNodes);
            }
            Ok(SubscriptionProfile {
                title: "VECTOR subscription".to_owned(),
                expires_at: find_i64(&payload, &["expireAt", "expiresAt", "expires_at"]),
                traffic_used_bytes: find_i64(
                    &payload,
                    &["usedTrafficBytes", "trafficUsedBytes", "traffic_used_bytes"],
                )
                .unwrap_or(0),
                traffic_limit_bytes: find_i64(
                    &payload,
                    &["trafficLimitBytes", "traffic_limit_bytes"],
                ),
                support_url: None,
                nodes,
            })
        }
    }

    fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
        if let Some(object) = value.as_object() {
            if let Some(found) = keys
                .iter()
                .find_map(|key| object.get(*key).and_then(Value::as_str).map(str::to_owned))
            {
                return Some(found);
            }
            return object.values().find_map(|item| find_string(item, keys));
        }
        value
            .as_array()?
            .iter()
            .find_map(|item| find_string(item, keys))
    }

    fn find_i64(value: &Value, keys: &[&str]) -> Option<i64> {
        if let Some(object) = value.as_object() {
            if let Some(found) = keys.iter().find_map(|key| {
                object
                    .get(*key)
                    .and_then(|item| item.as_i64().or_else(|| item.as_str()?.parse().ok()))
            }) {
                return Some(found);
            }
            return object.values().find_map(|item| find_i64(item, keys));
        }
        value
            .as_array()?
            .iter()
            .find_map(|item| find_i64(item, keys))
    }

    fn parse_nodes(body: &str) -> Vec<ProxyNode> {
        let decoded = if body.contains("://") || body.contains('\n') {
            body.to_owned()
        } else {
            STANDARD
                .decode(body.trim())
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_else(|| body.to_owned())
        };
        decoded
            .lines()
            .filter_map(|line| {
                let uri = line.trim();
                if uri.is_empty() || uri.starts_with('#') {
                    return None;
                }
                let parsed = Url::parse(uri).ok()?;
                if !matches!(parsed.scheme(), "vless" | "hysteria2" | "hy2") {
                    return None;
                }
                let name = parsed
                    .fragment()
                    .filter(|value| !value.is_empty())
                    .unwrap_or("VECTOR node")
                    .to_owned();
                Some(ProxyNode {
                    name,
                    uri: uri.to_owned(),
                })
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::parse_nodes;

        #[test]
        fn parses_only_supported_subscription_nodes() {
            let nodes = parse_nodes("vless://user@edge.example:443#AMS\nss://ignored@edge:443");
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].name, "AMS");
        }
    }
}

pub mod rich_text {
    use thiserror::Error;

    const MAX_MESSAGE_LENGTH: usize = 4096;
    const MAX_CAPTION_LENGTH: usize = 1024;
    const ALLOWED_TAGS: &[&str] = &[
        "b",
        "strong",
        "i",
        "em",
        "u",
        "s",
        "code",
        "pre",
        "blockquote",
        "a",
    ];

    #[derive(Debug, Error, PartialEq, Eq)]
    pub enum RichTextError {
        #[error("text exceeds the Telegram message limit")]
        MessageTooLong,
        #[error("caption exceeds the Telegram caption limit")]
        CaptionTooLong,
        #[error("unsupported or malformed HTML tag")]
        InvalidTag,
        #[error("HTML tags are not balanced")]
        UnbalancedTags,
        #[error("anchor href must use https:// or tg://user")]
        UnsafeUrl,
    }

    /// Escapes a dynamic value before it is inserted into Telegram HTML.
    #[must_use]
    pub fn escape_html(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    }

    /// Validates the restricted HTML subset accepted by the Telegram boundary.
    ///
    /// # Errors
    ///
    /// Returns an error for oversized content, unsupported attributes/tags,
    /// unsafe links, or unbalanced tags.
    pub fn validate_html(text: &str, caption: bool) -> Result<(), RichTextError> {
        let limit = if caption {
            MAX_CAPTION_LENGTH
        } else {
            MAX_MESSAGE_LENGTH
        };
        if text.chars().count() > limit {
            return Err(if caption {
                RichTextError::CaptionTooLong
            } else {
                RichTextError::MessageTooLong
            });
        }

        let mut stack = Vec::new();
        let mut position = 0;
        while let Some(relative_start) = text[position..].find('<') {
            let start = position + relative_start;
            let Some(relative_end) = text[start..].find('>') else {
                return Err(RichTextError::InvalidTag);
            };
            let end = start + relative_end;
            let raw = text[start + 1..end].trim();
            let closing = raw.strip_prefix('/');
            let tag_body = closing.unwrap_or(raw).trim();
            let tag_name = tag_body
                .split_ascii_whitespace()
                .next()
                .and_then(|name| name.strip_suffix('/').or(Some(name)))
                .ok_or(RichTextError::InvalidTag)?
                .to_ascii_lowercase();
            if !ALLOWED_TAGS.contains(&tag_name.as_str()) {
                return Err(RichTextError::InvalidTag);
            }
            if tag_name == "a" {
                if closing.is_some() {
                    if tag_body != "a" {
                        return Err(RichTextError::InvalidTag);
                    }
                } else if !valid_anchor(tag_body) {
                    return Err(RichTextError::UnsafeUrl);
                }
            } else if closing.is_none()
                && tag_body != tag_name
                && tag_body != format!("{tag_name}/")
            {
                return Err(RichTextError::InvalidTag);
            }

            if closing.is_some() {
                if stack.pop().as_deref() != Some(tag_name.as_str()) {
                    return Err(RichTextError::UnbalancedTags);
                }
            } else if !tag_body.ends_with('/') {
                stack.push(tag_name);
            }
            position = end + 1;
        }
        if !stack.is_empty() {
            return Err(RichTextError::UnbalancedTags);
        }
        Ok(())
    }

    fn valid_anchor(tag_body: &str) -> bool {
        let attributes = tag_body
            .strip_prefix("a ")
            .and_then(|value| value.strip_suffix('/').or(Some(value)));
        let Some(attributes) = attributes else {
            return false;
        };
        let Some(href) = attributes
            .strip_prefix("href=\"")
            .and_then(|value| value.strip_suffix('"'))
        else {
            return false;
        };
        href.starts_with("https://") || href.starts_with("tg://user")
    }

    #[cfg(test)]
    mod tests {
        use super::{RichTextError, escape_html, validate_html};

        #[test]
        fn escapes_dynamic_values() {
            assert_eq!(escape_html("<&\">'"), "&lt;&amp;&quot;&gt;&#39;");
        }

        #[test]
        fn rejects_markup_injection_and_unsafe_links() {
            assert_eq!(
                validate_html("<script>alert(1)</script>", false),
                Err(RichTextError::InvalidTag)
            );
            assert_eq!(
                validate_html("<a href=\"javascript:alert(1)\">x</a>", false),
                Err(RichTextError::UnsafeUrl)
            );
        }

        #[test]
        fn accepts_allowed_markup() {
            assert!(
                validate_html(
                    "<b>Ready</b> <a href=\"https://example.com\">open</a>",
                    false
                )
                .is_ok()
            );
        }

        #[test]
        fn rejects_unbalanced_and_oversized_content() {
            assert_eq!(
                validate_html("<b>broken", false),
                Err(RichTextError::UnbalancedTags)
            );
            assert_eq!(
                validate_html(&"x".repeat(4097), false),
                Err(RichTextError::MessageTooLong)
            );
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
    pub edited_message: Option<TelegramMessage>,
    pub callback_query: Option<TelegramCallbackQuery>,
    pub pre_checkout_query: Option<TelegramPreCheckoutQuery>,
    pub channel_post: Option<TelegramMessage>,
    pub edited_channel_post: Option<TelegramMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramMessage {
    pub message_id: i64,
    pub from: Option<TelegramUser>,
    pub chat: TelegramChat,
    pub text: Option<String>,
    pub caption: Option<String>,
    pub successful_payment: Option<TelegramSuccessfulPayment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramUser {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
    pub language_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramCallbackQuery {
    pub id: String,
    pub from: TelegramUser,
    pub message: Option<TelegramMessage>,
    pub data: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramPreCheckoutQuery {
    pub id: String,
    pub from: TelegramUser,
    pub currency: String,
    pub total_amount: i64,
    pub invoice_payload: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramSuccessfulPayment {
    pub currency: String,
    pub total_amount: i64,
    pub invoice_payload: String,
    pub telegram_payment_charge_id: String,
    pub provider_payment_charge_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SendMessageRequest<'a> {
    pub chat_id: i64,
    pub text: &'a str,
    pub parse_mode: &'static str,
}

#[derive(Debug, Serialize)]
pub struct EditMessageTextRequest<'a> {
    pub chat_id: i64,
    pub message_id: i64,
    pub text: &'a str,
    pub parse_mode: &'static str,
}

#[derive(Debug, Serialize)]
pub struct CallbackAnswerRequest<'a> {
    pub callback_query_id: &'a str,
    pub text: Option<&'a str>,
    pub show_alert: bool,
}

#[derive(Debug, Serialize)]
pub struct PreCheckoutAnswerRequest<'a> {
    pub pre_checkout_query_id: &'a str,
    pub ok: bool,
    pub error_message: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct GetUpdatesRequest {
    timeout: u16,
    offset: Option<i64>,
    allowed_updates: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct TelegramEnvelope<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub description: Option<String>,
    pub parameters: Option<TelegramResponseParameters>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramResponseParameters {
    pub retry_after: Option<u64>,
}

#[derive(Debug, Error)]
pub enum TelegramError {
    #[error("Telegram request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Telegram returned a permanent error: {0}")]
    Permanent(String),
    #[error("Telegram requested a retry after {0} seconds")]
    RetryAfter(u64),
    #[error("Telegram returned a successful response without a result")]
    MissingResult,
    #[error("message text is not valid Telegram HTML: {0}")]
    InvalidRichText(String),
}

impl TelegramError {
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Request(_) | Self::RetryAfter(_))
    }
}

#[derive(Debug, Clone)]
pub struct TelegramClient {
    client: Client,
    token: Arc<str>,
    base_url: Url,
}

impl TelegramClient {
    /// Builds a typed client around a Bot API base URL.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be initialized.
    pub fn new(token: impl Into<Arc<str>>, base_url: Url) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: Client::builder().timeout(Duration::from_secs(20)).build()?,
            token: token.into(),
            base_url,
        })
    }

    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Sends an HTML-formatted message through the typed Bot API boundary.
    ///
    /// # Errors
    ///
    /// Returns a classified transport, rate-limit, or Telegram API error.
    /// Long-polls updates with a bounded Telegram timeout.
    ///
    /// # Errors
    ///
    /// Returns a classified transport, rate-limit, or Telegram API error.
    pub async fn get_updates(
        &self,
        offset: Option<i64>,
    ) -> Result<Vec<TelegramUpdate>, TelegramError> {
        let endpoint = self
            .base_url
            .join(&format!("bot{}/getUpdates", self.token))
            .map_err(|error| TelegramError::Permanent(error.to_string()))?;
        let response = self
            .client
            .get(endpoint)
            .query(&GetUpdatesRequest {
                timeout: 30,
                offset,
                allowed_updates: r#"["message","callback_query","pre_checkout_query","edited_message"]"#,
            })
            .send()
            .await?;
        let status = response.status();
        let payload = response
            .json::<TelegramEnvelope<Vec<TelegramUpdate>>>()
            .await?;
        if payload.ok {
            payload.result.ok_or(TelegramError::MissingResult)
        } else if payload
            .parameters
            .as_ref()
            .and_then(|item| item.retry_after)
            .is_some()
            || status == StatusCode::TOO_MANY_REQUESTS
        {
            Err(TelegramError::RetryAfter(
                payload
                    .parameters
                    .and_then(|item| item.retry_after)
                    .unwrap_or(1),
            ))
        } else {
            Err(TelegramError::Permanent(
                payload
                    .description
                    .unwrap_or_else(|| "unknown Telegram error".to_owned()),
            ))
        }
    }

    /// Sends an HTML-formatted message through the typed Bot API boundary.
    ///
    /// # Errors
    ///
    /// Returns a classified transport, rate-limit, or Telegram API error.
    pub async fn send_message(
        &self,
        request: &SendMessageRequest<'_>,
    ) -> Result<Value, TelegramError> {
        rich_text::validate_html(request.text, false)
            .map_err(|error| TelegramError::InvalidRichText(error.to_string()))?;
        self.call("sendMessage", request).await
    }

    /// Edits a previously sent HTML-formatted message.
    ///
    /// # Errors
    ///
    /// Returns a classified transport, rate-limit, or Telegram API error.
    pub async fn edit_message_text(
        &self,
        request: &EditMessageTextRequest<'_>,
    ) -> Result<Value, TelegramError> {
        rich_text::validate_html(request.text, false)
            .map_err(|error| TelegramError::InvalidRichText(error.to_string()))?;
        self.call("editMessageText", request).await
    }

    /// Answers a callback query without exposing transport details to handlers.
    ///
    /// # Errors
    ///
    /// Returns a classified transport, rate-limit, or Telegram API error.
    pub async fn answer_callback_query(
        &self,
        request: &CallbackAnswerRequest<'_>,
    ) -> Result<Value, TelegramError> {
        self.call("answerCallbackQuery", request).await
    }

    /// Answers a Telegram Stars pre-checkout query.
    ///
    /// # Errors
    ///
    /// Returns a classified transport, rate-limit, or Telegram API error.
    pub async fn answer_pre_checkout_query(
        &self,
        request: &PreCheckoutAnswerRequest<'_>,
    ) -> Result<Value, TelegramError> {
        self.call("answerPreCheckoutQuery", request).await
    }

    async fn call<T: Serialize>(&self, method: &str, body: &T) -> Result<Value, TelegramError> {
        let endpoint = self
            .base_url
            .join(&format!("bot{}/{}", self.token, method))
            .map_err(|error| TelegramError::Permanent(error.to_string()))?;
        let response = self.client.post(endpoint).json(body).send().await?;
        let status = response.status();
        let payload = response.json::<TelegramEnvelope<Value>>().await?;
        if payload.ok {
            payload.result.ok_or(TelegramError::MissingResult)
        } else if payload
            .parameters
            .as_ref()
            .and_then(|item| item.retry_after)
            .is_some()
            || status == StatusCode::TOO_MANY_REQUESTS
        {
            Err(TelegramError::RetryAfter(
                payload
                    .parameters
                    .and_then(|item| item.retry_after)
                    .unwrap_or(1),
            ))
        } else {
            Err(TelegramError::Permanent(
                payload
                    .description
                    .unwrap_or_else(|| "unknown Telegram error".to_owned()),
            ))
        }
    }
}

/// Claims an update id in a caller-provided durable store.
///
/// This helper keeps duplicate detection deterministic while leaving storage
/// policy to the API/worker process.
#[must_use]
pub fn is_duplicate_update<S: BuildHasher>(seen: &mut HashSet<i64, S>, update_id: i64) -> bool {
    !seen.insert(update_id)
}
