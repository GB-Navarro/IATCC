/*
   Este módulo lê os golden cases (os exemplos do arquivo 05 do brain, com a
   resposta esperada). Cada caso traz o grupo exatamente como a IA o recebe;
   o mesmo arquivo alimenta o teste da regra do código e `classificar --golden`,
   que mede a consistência da IA (seção 12 do MVP).
*/

use crate::tipos::{Features, Grupo, Perna};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Esperado {
    pub estrutura: String,
    pub regra: String,
    #[serde(default)]
    pub alternativas: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CasoGolden {
    pub nome: String,
    /// Grupo no formato de `ia::payload`.
    pub entrada: Value,
    pub esperado: Esperado,
}

pub fn carregar(path: impl AsRef<Path>) -> Result<Vec<CasoGolden>> {
    let path = path.as_ref();
    let text =
        std::fs::read_to_string(path).with_context(|| format!("lendo {}", path.display()))?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(i, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("{}: linha {} inválida", path.display(), i + 1))
        })
        .collect()
}

#[derive(Deserialize)]
struct PernaEntrada {
    id: String,
    familia: String,
    #[serde(default)]
    estilo: Option<String>,
    #[serde(default)]
    call_ccy: Option<String>,
    #[serde(default)]
    moeda_base: Option<String>,
    #[serde(default)]
    strike: Option<f64>,
    #[serde(default)]
    vencimento: Option<String>,
    #[serde(default)]
    notional: Option<f64>,
    #[serde(default)]
    notional_currency: Option<String>,
    #[serde(default)]
    notional_2: Option<f64>,
    #[serde(default)]
    notional_currency_2: Option<String>,
    #[serde(default)]
    premium: Option<f64>,
    #[serde(default)]
    premium_currency: Option<String>,
    #[serde(default)]
    exchange_rate: Option<f64>,
    #[serde(default)]
    exchange_rate_basis: Option<String>,
    #[serde(default)]
    capado: bool,
    #[serde(default)]
    package_indicator: bool,
}

#[derive(Deserialize)]
struct GrupoEntrada {
    grupo: String,
    timestamp: DateTime<Utc>,
    plataforma: String,
    par: String,
    pernas: Vec<PernaEntrada>,
    numeros: Features,
}

impl CasoGolden {
    /// Reconstrói o Grupo a partir da entrada, para aplicar a regra do código.
    pub fn grupo(&self) -> Result<Grupo> {
        let g: GrupoEntrada = serde_json::from_value(self.entrada.clone())
            .with_context(|| format!("entrada do caso {}", self.nome))?;
        let pernas = g
            .pernas
            .into_iter()
            .map(|p| Perna {
                id: p.id,
                grupo_id: g.grupo.clone(),
                execution_timestamp: g.timestamp,
                platform_identifier: g.plataforma.clone(),
                par: g.par.clone(),
                familia: p.familia,
                estilo: p.estilo,
                call_ccy: p.call_ccy,
                base_ccy: p.moeda_base,
                strike: p.strike,
                vencimento: p.vencimento,
                notional: p.notional,
                notional_currency: p.notional_currency,
                notional_2: p.notional_2,
                notional_currency_2: p.notional_currency_2,
                premium: p.premium,
                premium_currency: p.premium_currency,
                exchange_rate: p.exchange_rate,
                exchange_rate_basis: p.exchange_rate_basis,
                capado: p.capado,
                package_indicator: p.package_indicator,
                source_file: "golden".into(),
                source_line: 0,
                raw: Value::Null,
            })
            .collect();
        Ok(Grupo {
            id: g.grupo,
            timestamp: g.timestamp,
            platform_identifier: g.plataforma,
            par: g.par,
            pernas,
            features: g.numeros,
            regra_codigo: String::new(),
            regra_codigo_ref: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calculos::{features_basicas, regra_codigo};

    #[test]
    fn regra_do_codigo_reproduz_os_exemplos_do_brain() {
        let casos = carregar(concat!(env!("CARGO_MANIFEST_DIR"), "/golden/casos.jsonl")).unwrap();
        assert_eq!(casos.len(), 7);
        for caso in casos {
            let grupo = caso.grupo().unwrap();
            let (estrutura, regra) = regra_codigo(&grupo);
            assert_eq!(
                (estrutura.as_str(), regra.as_str()),
                (
                    caso.esperado.estrutura.as_str(),
                    caso.esperado.regra.as_str()
                ),
                "caso {}",
                caso.nome
            );
            // As contagens da entrada precisam bater com o que o código calcularia.
            let base = features_basicas(&grupo.pernas);
            let n = &grupo.features;
            assert_eq!(
                (n.n_opt, n.n_hedge, n.n_vencimentos, n.n_strikes, n.n_lados),
                (
                    base.n_opt,
                    base.n_hedge,
                    base.n_vencimentos,
                    base.n_strikes,
                    base.n_lados
                ),
                "contagens do caso {}",
                caso.nome
            );
        }
    }
}
