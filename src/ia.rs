/*
   Este módulo orquestra o caso de uso de classificação: carrega o brain,
   seleciona o provider por configuração, monta o grupo no formato que a IA
   recebe, calcula hashes de reprodutibilidade e persiste o resultado com
   auditoria. Os binários chamam esta camada em vez de conhecer detalhes de
   qualquer API de IA.
*/

use crate::classifier::{ClassificationRequest, Classifier, ProviderOutcome, SharedClassifier};
use crate::db;
use crate::providers::{
    anthropic::AnthropicClassifier, devin::DevinClassifier, jev::JevClassifier,
    openai::OpenAiClassifier,
};
use crate::tipos::{Classificacao, Grupo};
use anyhow::{Context, Result, bail};
use chrono::SecondsFormat;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const PROVIDERS: &[&str] = &["anthropic", "openai", "jev", "devin"];

pub fn carregar_brain(dir: impl AsRef<Path>) -> Result<String> {
    // Lê os Markdown em ordem alfabética (00 a 05) para formar o contexto versionado do brain.
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.as_ref())
        .with_context(|| format!("lendo pasta do brain {}", dir.as_ref().display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("nenhum arquivo .md em {}", dir.as_ref().display());
    }
    files
        .iter()
        .map(|path| {
            std::fs::read_to_string(path).with_context(|| format!("lendo brain {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("\n\n"))
}

pub fn sha256(text: &str) -> String {
    // Calcula a impressão digital usada para rastrear brain, configuração e entrada.
    let mut hash = Sha256::new();
    hash.update(text.as_bytes());
    format!("{:x}", hash.finalize())
}

pub fn provider_name(provider: Option<&str>) -> String {
    // --provider tem prioridade sobre CLASSIFIER_PROVIDER; o padrão é anthropic.
    provider
        .map(str::to_ascii_lowercase)
        .or_else(|| {
            std::env::var("CLASSIFIER_PROVIDER")
                .ok()
                .map(|value| value.to_ascii_lowercase())
        })
        .map(|name| {
            if name == "claude" {
                "anthropic".into()
            } else {
                name
            }
        })
        .unwrap_or_else(|| "anthropic".into())
}

pub fn classifier_from_env(
    provider: Option<&str>,
    jev_profile_path: &Path,
) -> Result<SharedClassifier> {
    // Resolve o nome do provider e constrói somente o adaptador escolhido.
    match provider_name(provider).as_str() {
        "anthropic" => Ok(std::sync::Arc::new(AnthropicClassifier::from_env()?)),
        "openai" => Ok(std::sync::Arc::new(OpenAiClassifier::from_env()?)),
        "jev" => Ok(std::sync::Arc::new(JevClassifier::from_env(
            jev_profile_path,
        )?)),
        "devin" => Ok(std::sync::Arc::new(DevinClassifier::from_env()?)),
        other => bail!(
            "provider desconhecido: {other}; use {}",
            PROVIDERS.join(", ")
        ),
    }
}

/// O grupo como a IA o recebe (seções 5.4 e 8 do MVP): pernas com família, estilo,
/// lado, strike, vencimento, notionais, prêmio e `capado`, mais os números do grupo.
/// Ficam de fora `regra_codigo` (a IA não vê a resposta do código) e a linha CSV bruta.
pub fn payload(grupo: &Grupo) -> Value {
    let pernas: Vec<Value> = grupo
        .pernas
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "familia": p.familia,
                "estilo": p.estilo,
                "call_ccy": p.call_ccy,
                "moeda_base": p.base_ccy,
                "strike": p.strike,
                "vencimento": p.vencimento,
                "notional": p.notional,
                "notional_currency": p.notional_currency,
                "notional_2": p.notional_2,
                "notional_currency_2": p.notional_currency_2,
                "premium": p.premium,
                "premium_currency": p.premium_currency,
                "exchange_rate": p.exchange_rate,
                "exchange_rate_basis": p.exchange_rate_basis,
                "capado": p.capado,
                "package_indicator": p.package_indicator,
            })
        })
        .collect();
    json!({
        "grupo": grupo.id,
        "timestamp": grupo.timestamp.to_rfc3339_opts(SecondsFormat::Secs, true),
        "plataforma": grupo.platform_identifier,
        "par": grupo.par,
        "pernas": pernas,
        "numeros": grupo.features,
    })
}

/// Hashes de reprodutibilidade (seção 8 do MVP): brain, configuração e entrada completa.
pub struct Hashes {
    pub brain: String,
    pub config: String,
    pub input: String,
}

pub fn hashes(brain: &str, config: &str, grupo: &Value, classifier: &dyn Classifier) -> Hashes {
    let brain_hash = sha256(brain);
    let config_hash = sha256(&format!("{config}\n{}", classifier.config_material()));
    let input = json!({
        "grupo": grupo,
        "brain_hash": brain_hash,
        "config_hash": config_hash,
        "provider": classifier.provider(),
        "modelo": classifier.model(),
    });
    Hashes {
        input: sha256(&input.to_string()),
        brain: brain_hash,
        config: config_hash,
    }
}

/// Classifica um grupo do banco e grava a resposta. Devolve `None` se a IA recusar.
pub fn classificar_e_gravar(
    conn: &Connection,
    group_id: &str,
    brain_dir: &Path,
    config_path: &Path,
    classifier: &dyn Classifier,
) -> Result<ProviderOutcome> {
    let grupo = db::require_group(conn, group_id)?;
    let brain = carregar_brain(brain_dir)?;
    let config = std::fs::read_to_string(config_path)
        .with_context(|| format!("lendo {}", config_path.display()))?;
    let grupo_ia = payload(&grupo);
    let hashes = hashes(&brain, &config, &grupo_ia, classifier);
    let outcome = classifier.classify(&ClassificationRequest {
        brain,
        grupo: grupo_ia,
    })?;
    if let Some(classification) = &outcome.classificacao {
        db::store_classification(
            conn,
            group_id,
            &hashes.brain,
            &hashes.config,
            &hashes.input,
            classification,
            &outcome.metadata,
        )?;
    }
    Ok(outcome)
}

/// Resumo de uso de tokens da resposta (Anthropic e OpenAI), para conferir o cache.
pub fn uso_tokens(outcome: &ProviderOutcome) -> Option<String> {
    let usage = outcome.metadata.raw_response.get("usage")?;
    let campo = |nome: &str| usage.get(nome).and_then(Value::as_u64).unwrap_or(0);
    Some(format!(
        "entrada {} (cache lido {}, cache gravado {}), saída {}",
        campo("input_tokens"),
        campo("cache_read_input_tokens"),
        campo("cache_creation_input_tokens"),
        campo("output_tokens")
    ))
}

pub fn resumo_classificacao(c: &Classificacao) -> String {
    let alternativas = if c.alternativas.is_empty() {
        String::new()
    } else {
        format!(" [{}]", c.alternativas.join(", "))
    };
    format!(
        "{} ({}, {}){alternativas}",
        c.estrutura, c.regra, c.confianca
    )
}
