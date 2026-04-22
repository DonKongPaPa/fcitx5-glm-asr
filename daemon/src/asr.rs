use reqwest::multipart;
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, error, info};

#[derive(Debug, Deserialize)]
pub struct AsrResponse {
    pub text: Option<String>,
    pub error: Option<AsrError>,
}

#[derive(Debug, Deserialize)]
pub struct AsrError {
    pub code: String,
    pub message: String,
}

#[derive(Clone)]
pub struct AsrClient {
    client: reqwest::Client,
    api_key: String,
    model: String,
    api_url: String,
    hotwords: Vec<String>,
}

impl AsrClient {
    pub fn new(api_key: &str, model: &str, api_url: &str, hotwords: &[String]) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_key: api_key.to_string(),
            model: model.to_string(),
            api_url: api_url.to_string(),
            hotwords: hotwords.to_vec(),
        }
    }

    pub async fn recognize(&self, wav_data: Vec<u8>) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        info!("Sending {} bytes to ASR API", wav_data.len());

        let file_part = multipart::Part::bytes(wav_data)
            .file_name("audio.wav")
            .mime_str("audio/wav")?;

        let mut form = multipart::Form::new()
            .text("model", self.model.clone())
            .text("stream", "false")
            .part("file", file_part);

        if !self.hotwords.is_empty() {
            let hotwords_json = serde_json::to_string(&self.hotwords)?;
            form = form.text("hotwords", hotwords_json);
        }

        let response = self
            .client
            .post(&self.api_url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key),
            )
            .multipart(form)
            .send()
            .await?;

        let status = response.status();
        debug!("API response status: {}", status);

        if !status.is_success() {
            let body = response.text().await?;
            error!("API error ({}): {}", status, body);
            return Err(format!("API error ({}): {}", status, body).into());
        }

        let body = response.text().await?;
        debug!("API response body: {}", body);

        let asr_resp: AsrResponse = serde_json::from_str(&body)?;

        if let Some(err) = asr_resp.error {
            return Err(format!("ASR error: {} - {}", err.code, err.message).into());
        }

        let text = asr_resp.text.unwrap_or_default();
        info!("Recognition result: {}", text);
        Ok(text)
    }
}
