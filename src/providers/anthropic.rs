/*
   Este adaptador chama a Messages API da Anthropic, envia o brain como contexto
   e exige uma resposta no schema canônico. Ele preserva a integração original
   do MVP, mas agora entrega metadados por meio do mesmo contrato dos demais providers.
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

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

// Modelos que aceitam `fallbacks: "default"`: numa recusa por classificador de segurança
// a própria API refaz o pedido no modelo recomendado, em vez de devolver stop_reason=refusal.
const MODELOS_COM_FALLBACK: &[&str] = &[
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-fable-5-1",
    "claude-sonnet-5-5",
];

pub struct AnthropicClassifier {
    api_key: String,
    model: String,
    transport: SharedTransport,
}

impl AnthropicClassifier {
    // Constrói o adaptador com transporte HTTP real.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::with_transport(api_key, model, Arc::new(UreqTransport::default()))
    }

    // Permite substituir a rede por um transporte controlado nos testes.
    pub fn with_transport(
        api_key: impl Into<String>,
        model: impl Into<String>,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            transport,
        }
    }

    // Lê chave e modelo a partir das variáveis usadas na execução do CLI.
    pub fn from_env() -> Result<Self> {
        let key =
            std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY não está definida")?;
        Ok(Self::new(
            key,
            std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
        ))
    }
}

impl Classifier for AnthropicClassifier {
    fn provider(&self) -> &str {
        "anthropic"
    }
    fn model(&self) -> &str {
        &self.model
    }

    fn classify(&self, request: &ClassificationRequest) -> Result<ProviderOutcome> {
        // Monta a requisição, executa retries, valida a resposta e a converte ao domínio.
        let started = Instant::now();
        // O modelo pensa antes de responder e esse raciocínio conta em max_tokens:
        // 8000 (seção 8 do MVP) deixa folga para o JSON final.
        let mut body = json!({
            "model": self.model,
            "max_tokens": 8000,
            "system": [{"type":"text","text":request.brain,"cache_control":{"type":"ephemeral"}}],
            "output_config": {"format": {"type":"json_schema","schema":classification_schema()}},
            "messages": [{"role":"user","content":format!("Classifique este grupo:\n{}", request.grupo)}]
        });
        let mut headers = vec![
            ("x-api-key".into(), self.api_key.clone()),
            ("anthropic-version".into(), "2023-06-01".into()),
            ("content-type".into(), "application/json".into()),
        ];
        if MODELOS_COM_FALLBACK.contains(&self.model.as_str()) {
            body["fallbacks"] = json!("default");
            headers.push((
                "anthropic-beta".into(),
                "server-side-fallback-2026-07-01".into(),
            ));
        }
        let response = request_with_retries(
            self.transport.as_ref(),
            "POST",
            "https://api.anthropic.com/v1/messages",
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
            bail!("Anthropic HTTP {}: {}", response.status, response.body);
        }
        match response.body.get("stop_reason").and_then(Value::as_str) {
            Some("refusal") => {
                return Ok(ProviderOutcome {
                    classificacao: None,
                    metadata,
                });
            }
            Some("max_tokens") => bail!(
                "Anthropic atingiu max_tokens antes de terminar o JSON: {}",
                response.body["usage"]
            ),
            _ => {}
        }
        let Some(text) = response
            .body
            .get("content")
            .and_then(Value::as_array)
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                    .and_then(|block| block.get("text"))
            })
            .and_then(Value::as_str)
        else {
            bail!("Anthropic sem bloco de texto: {}", response.body);
        };
        let value: Value = serde_json::from_str(text).context("JSON da Anthropic inválido")?;
        Ok(ProviderOutcome {
            classificacao: Some(parse_classificacao(value)?),
            metadata,
        })
    }
}
