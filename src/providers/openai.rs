/*
   Este adaptador integra a Responses API da OpenAI. Ele usa text.format com
   JSON Schema estrito, extrai output_text (ou o conteúdo equivalente) e
   devolve a mesma Classificacao usada por todos os providers do MVP.
*/

use crate::classifier::{
    ClassificationRequest, Classifier, ProviderMetadata, ProviderOutcome, classification_schema,
    parse_classificacao,
};
use crate::provider_http::{HttpTransport, SharedTransport, UreqTransport, request_with_retries};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const DEFAULT_MODEL: &str = "gpt-6-astra";
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

pub struct OpenAiClassifier {
    api_key: String,
    model: String,
    base_url: String,
    transport: SharedTransport,
}

impl OpenAiClassifier {
    // Constrói uma instância que usa o transporte HTTP padrão.
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self::with_transport(api_key, model, base_url, Arc::new(UreqTransport::default()))
    }

    // Permite injetar um transporte fake e também configurar endpoints compatíveis.
    pub fn with_transport(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            transport,
        }
    }

    // Carrega credenciais, modelo e endpoint da configuração do ambiente.
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("OPENAI_API_KEY").context("OPENAI_API_KEY não está definida")?;
        Ok(Self::new(
            key,
            std::env::var("OPENAI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into()),
        ))
    }

    fn extract_output(response: &Value) -> Result<Option<String>> {
        // Normaliza as variantes de resposta da Responses API em texto JSON ou recusa.
        if response.get("error").is_some() {
            bail!("OpenAI retornou erro: {response}");
        }
        if response.pointer("/status").and_then(Value::as_str) == Some("incomplete") {
            bail!("resposta OpenAI incompleta: {response}");
        }
        if let Some(text) = response.get("output_text").and_then(Value::as_str) {
            return Ok(Some(text.to_string()));
        }
        let Some(items) = response.get("output").and_then(Value::as_array) else {
            bail!("resposta OpenAI sem output_text/output: {response}");
        };
        for item in items {
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for block in content {
                    if block.get("type").and_then(Value::as_str) == Some("refusal") {
                        return Ok(None);
                    }
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        return Ok(Some(text.to_string()));
                    }
                }
            }
        }
        bail!("resposta OpenAI sem conteúdo de classificação: {response}");
    }
}

impl Classifier for OpenAiClassifier {
    fn provider(&self) -> &str {
        "openai"
    }
    fn model(&self) -> &str {
        &self.model
    }

    fn classify(&self, request: &ClassificationRequest) -> Result<ProviderOutcome> {
        // Envia o brain e o grupo, aplica o schema estrito e valida o JSON retornado.
        let started = Instant::now();
        let body = json!({
            "model": self.model,
            "store": false,
            "instructions": request.brain,
            "input": format!("Classifique este grupo usando as features prontas.\n{}", serde_json::to_string_pretty(&request.grupo)?),
            "text": {"format": {
                "type": "json_schema",
                "name": "classification",
                "strict": true,
                "schema": classification_schema()
            }}
        });
        let headers = vec![
            ("Authorization".into(), format!("Bearer {}", self.api_key)),
            ("Content-Type".into(), "application/json".into()),
        ];
        let url = format!("{}/responses", self.base_url);
        let response = request_with_retries(
            self.transport.as_ref(),
            "POST",
            &url,
            &headers,
            Some(&body),
            3,
            Duration::from_secs(2),
        )?;
        let metadata = ProviderMetadata {
            provider: self.provider().into(),
            model: self.model.clone(),
            model_version: response
                .body
                .get("model")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            request_id: response
                .body
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            latency_ms: started.elapsed().as_millis(),
            raw_response: response.body.clone(),
            status: response.status.to_string(),
        };
        if response.status >= 400 {
            bail!("OpenAI HTTP {}: {}", response.status, response.body);
        }
        let Some(text) = Self::extract_output(&response.body)? else {
            return Ok(ProviderOutcome {
                classificacao: None,
                metadata,
            });
        };
        let value: Value =
            serde_json::from_str(&text).context("JSON estruturado da OpenAI inválido")?;
        Ok(ProviderOutcome {
            classificacao: Some(parse_classificacao(value)?),
            metadata,
        })
    }
}
