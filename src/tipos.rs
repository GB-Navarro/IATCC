/*
   Este arquivo define os tipos de domínio compartilhados pelo MVP: pernas,
   grupos, features calculadas, testes de hedge e respostas de classificação.
   Parser, cálculo, banco e providers usam esses contratos para trocar dados
   sem acoplar a lógica de negócio a uma API de IA específica.
*/

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Perna {
    pub id: String,
    pub grupo_id: String,
    pub execution_timestamp: DateTime<Utc>,
    pub platform_identifier: String,
    pub par: String,
    pub familia: String,
    pub estilo: Option<String>,
    pub call_ccy: Option<String>,
    pub base_ccy: Option<String>,
    pub strike: Option<f64>,
    pub vencimento: Option<String>,
    pub notional: Option<f64>,
    pub notional_currency: Option<String>,
    pub notional_2: Option<f64>,
    pub notional_currency_2: Option<String>,
    pub premium: Option<f64>,
    pub premium_currency: Option<String>,
    pub exchange_rate: Option<f64>,
    pub exchange_rate_basis: Option<String>,
    pub capado: bool,
    pub package_indicator: bool,
    pub source_file: String,
    pub source_line: u64,
    pub raw: serde_json::Value,
}

impl Perna {
    // Qualquer opção entra no grupo (seção 5.3 do MVP), inclusive as exóticas.
    pub fn is_option(&self) -> bool {
        self.familia.starts_with("Opção")
    }

    // Só vanilla e NDO têm preço Black-76 e entram no teste do hedge.
    pub fn is_vanilla_ou_ndo(&self) -> bool {
        self.familia == "Opção vanilla" || self.familia == "Opção NDO"
    }

    // Identifica instrumentos que podem funcionar como hedge do pacote.
    pub fn is_hedge(&self) -> bool {
        self.familia == "NDF" || self.familia == "Forward"
    }

    // Devolve o notional da perna que está na moeda pedida, se houver.
    pub fn notional_em(&self, moeda: &str) -> Option<f64> {
        if self.notional_currency.as_deref() == Some(moeda) {
            self.notional
        } else if self.notional_currency_2.as_deref() == Some(moeda) {
            self.notional_2
        } else {
            None
        }
    }
}

/// Números do teste do hedge da seção 6 do MVP. D_* estão na moeda-base;
/// resíduos e separações são divididos por N_c + N_p.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HedgeTest {
    pub moeda_base: String,
    pub forward: f64,
    pub prazo_anos: f64,
    pub sigma_call: f64,
    pub sigma_put: f64,
    pub delta_call: f64,
    pub delta_put: f64,
    pub notional_call: f64,
    pub notional_put: f64,
    pub hedge_notional: f64,
    pub d_mesma: f64,
    pub d_opostas: f64,
    pub d_cheio: f64,
    pub res_mesma: f64,
    pub res_opostas: f64,
    pub res_cheio: f64,
    pub sep: f64,
    pub sep_cheio: f64,
    pub tau_res: f64,
    pub tau_sep: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Features {
    pub n_opt: usize,
    pub n_hedge: usize,
    pub n_vencimentos: usize,
    pub n_strikes: usize,
    pub n_lados: usize,
    pub teste_hedge: Option<HedgeTest>,
    pub motivo_teste_hedge: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grupo {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub platform_identifier: String,
    pub par: String,
    pub pernas: Vec<Perna>,
    pub features: Features,
    /// Estrutura que a tabela de decisão do brain, aplicada em código, devolve.
    pub regra_codigo: String,
    /// Regra do brain que decidiu `regra_codigo` (ex.: "03/R4").
    pub regra_codigo_ref: String,
}

/// Resposta da IA no formato do arquivo 04 do brain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classificacao {
    pub estrutura: String,
    pub alternativas: Vec<String>,
    pub regra: String,
    pub confianca: String,
    pub justificativa: String,
}

/// Nomes aceitos em `estrutura`: a lista do arquivo 02 do brain mais "Ambiguous".
pub const ESTRUTURAS: &[&str] = &[
    "Opção única",
    "Straddle",
    "Strangle",
    "Risk reversal",
    "Collar",
    "Spread vertical",
    "Calendar spread",
    "Spread diagonal",
    "Fracionamento de notional",
    "Lados opostos, vencimentos diferentes",
    "Seagull",
    "Strip/Strap",
    "Ratio spread",
    "Iron condor",
    "Butterfly",
    "Condor",
    "Programa de hedge",
    "Não identificado",
    "Ambiguous",
];

pub const CONFIANCAS: &[&str] = &["alta", "media", "baixa"];
