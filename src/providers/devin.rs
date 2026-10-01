/*
   Este adaptador integra o Devin como uma sessão assíncrona. Ele cria a sessão
   com structured_output_schema, consulta o status até obter structured_output
   e então valida a resposta no contrato comum do MVP.
*/

use crate::classifier::{
    ClassificationRequest, Classifier, ProviderMetadata, ProviderOutcome, parse_classificacao,
};
use crate::provider_http::{HttpTransport, SharedTransport, UreqTransport, request_with_retries};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_BASE_URL: &str = "https://api.devin.ai";

pub struct DevinClassifier {
    api_key: String,
    org_id: String,
    base_url: String,
    create_path: String,
    status_path: String,
    transport: SharedTransport,
    poll_interval: Duration,
    max_polls: u32,
}

struct DevinOptions {
    create_path: String,
    status_path: String,
    transport: SharedTransport,
    poll_interval: Duration,
    max_polls: u32,
}

impl DevinClassifier {
    // Cria um adaptador de produção com os endpoints v3 documentados.
    pub fn new(
        api_key: impl Into<String>,
        org_id: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self::with_options(
            api_key,
            org_id,
            base_url,
            DevinOptions {
                create_path: "/v3/organizations/{org_id}/sessions".into(),
                status_path: "/v3/organizations/{org_id}/sessions/{session_id}".into(),
                transport: Arc::new(UreqTransport::default()),
                poll_interval: Duration::from_secs(10),
                max_polls: 30,
            },
        )
    }

    // Permite testar criação e polling sem esperar nem acessar a rede.
    pub fn with_transport(
        api_key: impl Into<String>,
        org_id: impl Into<String>,
        base_url: impl Into<String>,
        transport: Arc<dyn HttpTransport>,
        poll_interval_seconds: u64,
        max_polls: u32,
    ) -> Self {
        Self::with_options(
            api_key,
            org_id,
            base_url,
            DevinOptions {
                create_path: "/v3/organizations/{org_id}/sessions".into(),
                status_path: "/v3/organizations/{org_id}/sessions/{session_id}".into(),
                transport,
                poll_interval: Duration::from_secs(poll_interval_seconds),
                max_polls,
            },
        )
    }

    // Agrupa opções internas para manter os construtores públicos legíveis.
    fn with_options(
        api_key: impl Into<String>,
        org_id: impl Into<String>,
        base_url: impl Into<String>,
        options: DevinOptions,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            org_id: org_id.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            create_path: options.create_path,
            status_path: options.status_path,
            transport: options.transport,
            poll_interval: options.poll_interval,
            max_polls: options.max_polls.max(1),
        }
    }

    // Lê credenciais, organização e parâmetros de polling do ambiente.
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("DEVIN_API_KEY").context("DEVIN_API_KEY não está definida")?;
        let org = std::env::var("DEVIN_ORG_ID").context("DEVIN_ORG_ID não está definida")?;
        let mut result = Self::new(
            key,
            org,
            std::env::var("DEVIN_API_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into()),
        );
        result.create_path =
            std::env::var("DEVIN_CREATE_SESSION_PATH").unwrap_or(result.create_path);
        result.status_path = std::env::var("DEVIN_SESSION_PATH").unwrap_or(result.status_path);
        result.poll_interval = Duration::from_secs(
            std::env::var("DEVIN_POLL_INTERVAL_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
        );
        result.max_polls = std::env::var("DEVIN_MAX_POLLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30)
            .max(1);
        Ok(result)
    }

    // Resolve placeholders de organização/sessão e compõe a URL final.
    fn endpoint(&self, path: &str, session_id: Option<&str>) -> String {
        let path = path
            .replace("{org_id}", &self.org_id)
            .replace("{session_id}", session_id.unwrap_or_default());
        if path.starts_with("http://") || path.starts_with("https://") {
            path
        } else {
            format!(
                "{}{}",
                self.base_url,
                if path.starts_with('/') {
                    path
                } else {
                    format!("/{path}")
                }
            )
        }
    }

    // Aceita os nomes de campo usados por diferentes versões da API do Devin.
    fn structured_output(value: &Value) -> Option<Value> {
        value
            .get("structured_output")
            .or_else(|| value.pointer("/output/structured_output"))
            .or_else(|| value.get("structuredOutput"))
            .cloned()
            .and_then(|value| match value {
                Value::String(text) => serde_json::from_str(&text).ok(),
                other => Some(other),
            })
    }
}

impl Classifier for DevinClassifier {
    fn provider(&self) -> &str {
        "devin"
    }
    fn model(&self) -> &str {
        "devin"
    }
    fn config_material(&self) -> &str {
        &self.status_path
    }

    fn classify(&self, request: &ClassificationRequest) -> Result<ProviderOutcome> {
        // Cria uma sessão, faz polling e encerra somente quando houver JSON estruturado válido.
        let started = Instant::now();
        let headers = vec![
            ("Authorization".into(), format!("Bearer {}", self.api_key)),
            ("Content-Type".into(), "application/json".into()),
        ];
        let create_body = json!({
            "prompt": format!("Você é um classificador de estruturas OTC. Siga exatamente o Brain abaixo e produza somente o JSON estruturado pedido.\n\nBrain:\n{}\n\nGrupo:\n{}", request.brain, serde_json::to_string_pretty(&request.grupo)?),
            "structured_output_schema": crate::classifier::classification_schema(),
            "title": "IATCC OTC structure classification",
            "unlisted": true
        });
        let create_url = self.endpoint(&self.create_path, None);
        let created = request_with_retries(
            self.transport.as_ref(),
            "POST",
            &create_url,
            &headers,
            Some(&create_body),
            3,
            Duration::from_secs(2),
        )?;
        if created.status >= 400 {
            bail!(
                "Devin HTTP {} ao criar sessão: {}",
                created.status,
                created.body
            );
        }
        let session_id = created
            .body
            .get("session_id")
            .or_else(|| created.body.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("resposta Devin sem session_id: {}", created.body))?
            .to_string();
        let status_url = self.endpoint(&self.status_path, Some(&session_id));
        let mut last = created.body;
        for attempt in 0..self.max_polls {
            let current = if attempt == 0 && Self::structured_output(&last).is_some() {
                last.clone()
            } else {
                if self.poll_interval > Duration::ZERO {
                    thread::sleep(self.poll_interval);
                }
                request_with_retries(
                    self.transport.as_ref(),
                    "GET",
                    &status_url,
                    &headers,
                    None,
                    3,
                    Duration::from_secs(2),
                )?
                .body
            };
            if let Some(output) = Self::structured_output(&current) {
                let classification = parse_classificacao(output)?;
                let metadata = ProviderMetadata {
                    provider: self.provider().into(),
                    model: self.model().into(),
                    model_version: current
                        .get("model_version")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    request_id: Some(session_id.clone()),
                    latency_ms: started.elapsed().as_millis(),
                    raw_response: current,
                    status: "completed".into(),
                };
                return Ok(ProviderOutcome {
                    classificacao: Some(classification),
                    metadata,
                });
            }
            last = current.clone();
            let state = current
                .get("status")
                .or_else(|| current.get("state"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if ["failed", "error", "cancelled", "canceled"].contains(&state.as_str()) {
                bail!("sessão Devin terminou em {state}: {current}");
            }
        }
        bail!(
            "sessão Devin não produziu structured_output após {} consultas: {}",
            self.max_polls,
            last
        )
    }
}
