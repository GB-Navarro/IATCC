/*
   Este adaptador integra o Jev (TypeSafe, System One) como motor de decisão. O
   perfil local transforma os critérios de estrutura em uma pergunta choice; o
   brain vai junto no estado, porque o Jev não lê arquivos. As probabilidades e
   a confiança devolvidas são convertidas para a Classificacao canônica.
*/

use crate::classifier::{
    ClassificationRequest, Classifier, ProviderMetadata, ProviderOutcome,
    confidence_from_probability, parse_classificacao,
};
use crate::provider_http::{HttpTransport, SharedTransport, UreqTransport, request_with_retries};
use crate::tipos::{Classificacao, ESTRUTURAS};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const DEFAULT_MODEL: &str = "jev-1.13.0";
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JevProfile {
    pub instructions: String,
    pub criteria: BTreeMap<String, String>,
}

impl JevProfile {
    // Lê as instruções e critérios versionáveis que serão enviados ao Jev.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("lendo perfil Jev {}", path.display()))?;
        let profile: Self = serde_json::from_str(&text)
            .with_context(|| format!("lendo JSON do perfil Jev {}", path.display()))?;
        // Cada rótulo da pergunta precisa ser uma estrutura aceita pelo resto do MVP.
        if let Some(label) = profile
            .criteria
            .keys()
            .find(|k| !ESTRUTURAS.contains(&k.as_str()))
        {
            bail!("critério Jev fora da lista de estruturas: {label}");
        }
        Ok(profile)
    }

    // Serializa o perfil para compor sua identidade no hash de configuração.
    fn material(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

pub struct JevClassifier {
    api_key: String,
    model: String,
    base_url: String,
    profile: JevProfile,
    profile_hash: String,
    transport: SharedTransport,
}

impl JevClassifier {
    // Cria um classificador Jev usando o transporte real.
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        profile: JevProfile,
    ) -> Result<Self> {
        Self::with_transport(
            api_key,
            model,
            base_url,
            profile,
            Arc::new(UreqTransport::default()),
        )
    }

    // Cria um classificador Jev com transporte injetável para testes.
    pub fn with_transport(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        profile: JevProfile,
        transport: Arc<dyn HttpTransport>,
    ) -> Result<Self> {
        let material = profile.material()?;
        let mut hash = Sha256::new();
        hash.update(material.as_bytes());
        Ok(Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            profile,
            profile_hash: format!("{:x}", hash.finalize()),
            transport,
        })
    }

    // Obtém credenciais, modelo, endpoint e perfil a partir do ambiente/arquivo.
    pub fn from_env(profile_path: &Path) -> Result<Self> {
        let key = std::env::var("JEV_API_KEY")
            .or_else(|_| std::env::var("TYPESAFE_API_KEY"))
            .context("JEV_API_KEY (ou TYPESAFE_API_KEY) não está definida")?;
        Self::new(
            key,
            std::env::var("JEV_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
            std::env::var("JEV_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into()),
            JevProfile::load(profile_path)?,
        )
    }
}

impl Classifier for JevClassifier {
    fn provider(&self) -> &str {
        "jev"
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn config_material(&self) -> &str {
        &self.profile_hash
    }

    fn classify(&self, request: &ClassificationRequest) -> Result<ProviderOutcome> {
        // Envia uma pergunta choice e mapeia a decisão probabilística do Jev.
        let started = Instant::now();
        let body = json!({
            "model": self.model,
            "state": {"brain": request.brain, "grupo": request.grupo},
            "questions": {
                "structure": {
                    "type": "choice",
                    "instructions": self.profile.instructions,
                    "criteria": self.profile.criteria
                }
            }
        });
        let headers = vec![
            ("Authorization".into(), format!("Bearer {}", self.api_key)),
            ("Content-Type".into(), "application/json".into()),
        ];
        let url = format!("{}/v1/systemone", self.base_url);
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
                .get("model_version")
                .or_else(|| response.body.get("model"))
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
            bail!("Jev HTTP {}: {}", response.status, response.body);
        }
        let answer = response
            .body
            .pointer("/answers/structure")
            .or_else(|| response.body.pointer("/results/structure"))
            .ok_or_else(|| {
                anyhow::anyhow!("resposta Jev sem answers.structure: {}", response.body)
            })?;
        let label = answer
            .get("choice")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("resposta Jev sem choice: {answer}"))?;
        let probabilities: BTreeMap<String, f64> = answer
            .get("probabilities")
            .and_then(Value::as_object)
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
                    .collect()
            })
            .unwrap_or_default();
        let probability = probabilities.get(label).copied().unwrap_or(0.0);
        let confidence = answer
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(probability);
        // Alternativas: outras estruturas com pelo menos 10% de probabilidade.
        let mut alternativas: Vec<(&String, f64)> = probabilities
            .iter()
            .filter(|(k, p)| k.as_str() != label && k.as_str() != "Ambiguous" && **p >= 0.10)
            .map(|(k, p)| (k, *p))
            .collect();
        alternativas.sort_by(|a, b| b.1.total_cmp(&a.1));
        let resumo = alternativas
            .iter()
            .map(|(k, p)| format!("{k} {p:.2}"))
            .collect::<Vec<_>>()
            .join(", ");
        let classification = Classificacao {
            estrutura: label.to_string(),
            alternativas: if label == "Ambiguous" {
                alternativas.iter().map(|(k, _)| (*k).clone()).collect()
            } else {
                Vec::new()
            },
            regra: "jev/choice".into(),
            confianca: confidence_from_probability(confidence),
            justificativa: format!(
                "Jev escolheu {label} com probabilidade {probability:.2} e confiança {confidence:.2}.{}",
                if resumo.is_empty() {
                    String::new()
                } else {
                    format!(" Outras: {resumo}.")
                }
            ),
        };
        Ok(ProviderOutcome {
            classificacao: Some(parse_classificacao(serde_json::to_value(classification)?)?),
            metadata,
        })
    }
}
