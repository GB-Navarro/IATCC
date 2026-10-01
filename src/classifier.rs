/*
   Este módulo define o contrato agnóstico de provider. Ele normaliza a entrada
   do brain e do grupo, valida a saída contra o mesmo esquema e oferece funções
   auxiliares para adaptar respostas probabilísticas, permitindo trocar
   Anthropic, OpenAI, Jev ou Devin sem alterar o restante do MVP.
*/

use crate::tipos::{CONFIANCAS, Classificacao, ESTRUTURAS};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ClassificationRequest {
    pub brain: String,
    /// Grupo no formato enviado à IA (`ia::payload`): sem `regra_codigo` nem linha CSV bruta.
    pub grupo: Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderMetadata {
    pub provider: String,
    pub model: String,
    pub model_version: Option<String>,
    pub request_id: Option<String>,
    pub latency_ms: u128,
    pub raw_response: Value,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct ProviderOutcome {
    pub classificacao: Option<Classificacao>,
    pub metadata: ProviderMetadata,
}

pub trait Classifier: Send + Sync {
    // Expõe identidade e configuração do provider para auditoria e hashing.
    fn provider(&self) -> &str;
    fn model(&self) -> &str;
    fn config_material(&self) -> &str {
        ""
    }
    fn classify(&self, request: &ClassificationRequest) -> Result<ProviderOutcome>;
}

pub type SharedClassifier = Arc<dyn Classifier>;

/// JSON Schema estrito da resposta (arquivo 04 do brain), igual para todos os providers.
pub fn classification_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["estrutura", "alternativas", "regra", "confianca", "justificativa"],
        "properties": {
            "estrutura": {"type": "string", "enum": ESTRUTURAS},
            "alternativas": {"type": "array", "items": {"type": "string", "enum": ESTRUTURAS}},
            "regra": {"type": "string"},
            "confianca": {"type": "string", "enum": CONFIANCAS},
            "justificativa": {"type": "string"}
        }
    })
}

pub fn parse_classificacao(value: Value) -> Result<Classificacao> {
    // Converte e valida a resposta externa antes que ela entre no banco de dados.
    let mut classification: Classificacao = serde_json::from_value(value)
        .map_err(|error| anyhow::anyhow!("JSON de classificação inválido: {error}"))?;
    if !ESTRUTURAS.contains(&classification.estrutura.as_str()) {
        bail!("estrutura não permitida: {}", classification.estrutura);
    }
    if !CONFIANCAS.contains(&classification.confianca.as_str()) {
        bail!("confiança não permitida: {}", classification.confianca);
    }
    let estrutura = classification.estrutura.clone();
    classification
        .alternativas
        .retain(|item| ESTRUTURAS.contains(&item.as_str()) && *item != estrutura);
    Ok(classification)
}

pub fn confidence_from_probability(probability: f64) -> String {
    // Traduz a maior probabilidade do provider nas faixas de confiança do MVP.
    if probability >= 0.85 {
        "alta".into()
    } else if probability >= 0.60 {
        "media".into()
    } else {
        "baixa".into()
    }
}
