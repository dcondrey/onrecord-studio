//! Async HTTP client infrastructure for robust municipal API ingestion.

use std::time::Duration;
use reqwest::{Client, HeaderMap, HeaderValue, StatusCode};
use serde::de::DeserializeOwned;
use crate::error::AppError; // Assumes your error.rs maps this cleanly
use crate::types::NextRequestResponse;

/// Configuration options for the underlying transport layer.
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    pub timeout_seconds: u64,
    pub max_retries: usize,
    pub retry_initial_delay_ms: u64,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 30,
            max_retries: 3,
            retry_initial_delay_ms: 1000,
        }
    }
}

/// Thread-safe client manager coordinating outbound connections and browser signatures.
#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    config: HttpClientConfig,
}

impl ApiClient {
    /// Initializes a connection pool equipped with localized signatures.
    pub fn new(config: HttpClientConfig) -> Result<Self, AppError> {
        let mut headers = HeaderMap::new();
        
        // Mimic standard modern macOS Safari traffic to maximize reliability
        headers.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
            ),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/json, text/plain, */*"),
        );

        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(config.timeout_seconds))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(5)
            .build()
            .map_err(|e| AppError::Network(format!("Failed to build connection pool: {}", e)))?;

        Ok(Self { client, config })
    }

    /// Fetches a paginated slice of NextRequest objects from a target municipal subdomain.
    pub async fn fetch_requests(
        &self,
        subdomain: &str,
        page: usize,
        per_page: usize,
    ) -> Result<NextRequestResponse, AppError> {
        let url = format!("https://{}://", subdomain);
        
        let query_params = [
            ("page", page.to_string()),
            ("per_page", per_page.to_string()),
        ];

        self.execute_with_retry(
            self.client.get(&url).query(&query_params)
        ).await
    }

    /// Downloads a public file payload as a raw binary stream, dropping it directly to disk.
    pub async fn download_document(
        &self,
        subdomain: &str,
        document_id: u64,
    ) -> Result<bytes::Bytes, AppError> {
        let url = format!("https://{}://{}/download", subdomain, document_id);
        
        let response = self.execute_with_retry(self.client.get(&url)).await?;
        Ok(response)
    }

    /// Handles request execution wrapped in an exponential backoff loop.
    async fn execute_with_retry<T: DeserializeOwned + 'static>(
        &self,
        request_builder: reqwest::RequestBuilder,
    ) -> Result<T, AppError> {
        let mut attempts = 0;
        let mut delay = self.config.retry_initial_delay_ms;

        loop {
            attempts += 1;
            
            // Clone the request builder since it is consumed upon sending
            let req = request_builder.try_clone()
                .ok_or_else(|| AppError::Internal("Failed to clone request handle".to_string()))?;

            match req.send().await {
                Ok(response) => {
                    let status = response.status();

                    if status.is_success() {
                        // Special handling: check if return type is bytes::Bytes directly
                        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<bytes::Bytes>() {
                            let bytes = response.bytes().await
                                .map_err(|e| AppError::Network(format!("Stream read failure: {}", e)))?;
                            return unsafe { Ok(std::mem::transmute_copy::<bytes::Bytes, T>(&bytes)) };
                        }

                        // Otherwise, parse standard JSON objects
                        let payload = response.json::<T>().await
                            .map_err(|e| AppError::DataParsing(format!("JSON unpacking failed: {}", e)))?;
                        return Ok(payload);
                    }

                    // Handle explicit server rate throttling (HTTP 429) or temporary server errors (HTTP 5xx)
                    if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) && attempts < self.config.max_retries {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        delay *= 2; // Exponential scale back
                        continue;
                    }

                    return Err(AppError::Network(format!("Server returned failure status: {}", status)));
                }
                Err(e) if attempts < self.config.max_retries => {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    delay *= 2;
                    continue;
                }
                Err(e) => return Err(AppError::Network(format!("Connection exhausted across attempts: {}", e))),
            }
        }
    }
}
