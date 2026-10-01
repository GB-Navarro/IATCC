/*
   Este arquivo calcula as features numéricas do MVP (seção 6) e aplica em
   código a tabela de decisão do brain (arquivos 02 e 03) como linha de base.
   Ele fica entre o parser/grupador e os classificadores: prepara os números do
   teste do hedge para que a IA decida sem refazer contas.
*/

use crate::tipos::{Features, Grupo, HedgeTest, Perna};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use libm::{erf, log};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    #[serde(default = "default_tau")]
    pub tau_res: f64,
    #[serde(default = "default_tau")]
    pub tau_sep: f64,
    #[serde(default = "default_min_t")]
    pub min_t_days: i64,
    #[serde(default = "default_min_ratio")]
    pub min_forward_strike_ratio: f64,
    #[serde(default = "default_max_ratio")]
    pub max_forward_strike_ratio: f64,
}

fn default_tau() -> f64 {
    // Limiar padrão para considerar um resíduo numericamente pequeno.
    0.05
}
fn default_min_t() -> i64 {
    // Prazo mínimo em dias para permitir o valuation.
    2
}
fn default_min_ratio() -> f64 {
    // Limite inferior da razão F/K.
    0.7
}
fn default_max_ratio() -> f64 {
    // Limite superior da razão F/K.
    1.4
}

impl Default for Thresholds {
    // Constrói thresholds completos quando a configuração não informa algum campo.
    fn default() -> Self {
        Self {
            tau_res: default_tau(),
            tau_sep: default_tau(),
            min_t_days: default_min_t(),
            min_forward_strike_ratio: default_min_ratio(),
            max_forward_strike_ratio: default_max_ratio(),
        }
    }
}

pub fn load_thresholds(path: impl AsRef<Path>) -> Result<Thresholds> {
    // Lê o TOML de thresholds e o transforma no contrato de cálculo.
    let content = std::fs::read_to_string(path.as_ref())
        .with_context(|| format!("lendo configuração {}", path.as_ref().display()))?;
    toml::from_str(&content).context("interpretando thresholds.toml")
}

pub fn phi(x: f64) -> f64 {
    // Avalia a distribuição normal acumulada usada na fórmula Black.
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

fn d1(forward: f64, strike: f64, sigma: f64, t: f64) -> f64 {
    (log(forward / strike) + sigma * sigma * t / 2.0) / (sigma * t.sqrt())
}

fn black_price(forward: f64, strike: f64, sigma: f64, t: f64, is_call: bool) -> f64 {
    // Calcula o preço Black-76 sem desconto para uma volatilidade dada.
    if sigma <= 0.0 || t <= 0.0 {
        let intrinsic = if is_call {
            forward - strike
        } else {
            strike - forward
        };
        return intrinsic.max(0.0);
    }
    let d1 = d1(forward, strike, sigma, t);
    let d2 = d1 - sigma * t.sqrt();
    if is_call {
        forward * phi(d1) - strike * phi(d2)
    } else {
        strike * phi(-d2) - forward * phi(-d1)
    }
}

pub fn implied_volatility(
    target: f64,
    forward: f64,
    strike: f64,
    t: f64,
    is_call: bool,
) -> Option<f64> {
    // Encontra σ por bisseção entre 0,0001 e 3 e rejeita preços fora do intervalo.
    if !target.is_finite() || !forward.is_finite() || !strike.is_finite() || target < 0.0 {
        return None;
    }
    let (mut low, mut high) = (0.0001, 3.0);
    if black_price(forward, strike, low, t, is_call) > target + 1e-12
        || black_price(forward, strike, high, t, is_call) < target - 1e-12
    {
        return None;
    }
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if black_price(forward, strike, mid, t, is_call) < target {
            low = mid;
        } else {
            high = mid;
        }
    }
    Some((low + high) / 2.0)
}

fn chave_strike(strike: f64) -> String {
    // Strikes vêm do mesmo texto do CSV; a chave evita comparar f64 diretamente.
    format!("{strike:.10}")
}

pub fn features_basicas(pernas: &[Perna]) -> Features {
    // Calcula contagens e cardinalidades simples utilizadas pelas regras (seção 5.4).
    let options: Vec<_> = pernas.iter().filter(|p| p.is_option()).collect();
    let vencimentos = options
        .iter()
        .filter_map(|p| p.vencimento.clone())
        .collect::<BTreeSet<_>>();
    let strikes = options
        .iter()
        .filter_map(|p| p.strike.map(chave_strike))
        .collect::<BTreeSet<_>>();
    let lados = options
        .iter()
        .filter_map(|p| p.call_ccy.clone())
        .collect::<BTreeSet<_>>();
    Features {
        n_opt: options.len(),
        n_hedge: pernas.iter().filter(|p| p.is_hedge()).count(),
        n_vencimentos: vencimentos.len(),
        n_strikes: strikes.len(),
        n_lados: lados.len(),
        ..Default::default()
    }
}

fn prazo_dias(grupo: &Grupo, vencimento: &str) -> Option<i64> {
    // Dias inteiros entre a execução e o vencimento (meia-noite UTC).
    let expiry = NaiveDate::parse_from_str(vencimento, "%Y-%m-%d").ok()?;
    Some((expiry - grupo.timestamp.date_naive()).num_days())
}

fn forward_na_base(hedge: &Perna, base: &str, quote: &str, strikes: (f64, f64)) -> Option<f64> {
    // F do hedge cotado como moeda de cotação por unidade da base; 1/F se invertido.
    let f = hedge.exchange_rate.filter(|f| *f > 0.0 && f.is_finite())?;
    match hedge.base_ccy.as_deref() {
        Some(b) if b == base => Some(f),
        Some(b) if b == quote => Some(1.0 / f),
        // Sem base conhecida, vale a orientação mais próxima dos strikes.
        _ => {
            let k = (strikes.0 * strikes.1).sqrt();
            Some(if log(f / k).abs() <= log(1.0 / (f * k)).abs() {
                f
            } else {
                1.0 / f
            })
        }
    }
}

/// Teste do hedge da seção 6 do MVP. `Err` traz o motivo enviado à IA.
pub fn calcula_teste_hedge(grupo: &Grupo, thresholds: &Thresholds) -> Result<HedgeTest, String> {
    let opcoes: Vec<&Perna> = grupo.pernas.iter().filter(|p| p.is_option()).collect();
    if opcoes.len() != 2 || !opcoes.iter().all(|p| p.is_vanilla_ou_ndo()) {
        return Err("não se aplica: o teste exige exatamente 2 opções vanilla ou NDO".into());
    }
    let (Some(lado_a), Some(lado_b)) =
        (opcoes[0].call_ccy.as_deref(), opcoes[1].call_ccy.as_deref())
    else {
        return Err("lado da opção ausente".into());
    };
    if lado_a == lado_b {
        return Err("não se aplica: as 2 opções estão do mesmo lado".into());
    }
    let (Some(venc_a), Some(venc_b)) = (
        opcoes[0].vencimento.as_deref(),
        opcoes[1].vencimento.as_deref(),
    ) else {
        return Err("vencimento da opção ausente".into());
    };
    if venc_a != venc_b {
        return Err("não se aplica: vencimentos diferentes".into());
    }
    let hedges: Vec<&Perna> = grupo.pernas.iter().filter(|p| p.is_hedge()).collect();
    let hedge = match hedges.as_slice() {
        [] => return Err("nenhum hedge no grupo".into()),
        [hedge] => *hedge,
        _ => return Err("mais de um hedge no grupo".into()),
    };
    if grupo.pernas.iter().any(|p| p.capado) {
        return Err("notional capado".into());
    }
    let bases: BTreeSet<&str> = opcoes
        .iter()
        .filter_map(|p| p.base_ccy.as_deref())
        .collect();
    let base = match bases.len() {
        0 => return Err("moeda-base ausente".into()),
        1 => bases.into_iter().next().unwrap_or_default().to_string(),
        _ => return Err("moeda-base diferente entre as opções".into()),
    };
    let Some(quote) = grupo
        .par
        .split('-')
        .find(|ccy| *ccy != base)
        .filter(|_| grupo.par.split('-').any(|ccy| ccy == base))
        .map(ToOwned::to_owned)
    else {
        return Err("moeda-base fora do par".into());
    };
    let (call, put) = if lado_a == base {
        (opcoes[0], opcoes[1])
    } else {
        (opcoes[1], opcoes[0])
    };
    if call.call_ccy.as_deref() != Some(base.as_str())
        || put.call_ccy.as_deref() != Some(quote.as_str())
    {
        return Err("não há uma call e uma put da moeda-base".into());
    }
    let (Some(k_call), Some(k_put)) = (
        call.strike.filter(|k| *k > 0.0),
        put.strike.filter(|k| *k > 0.0),
    ) else {
        return Err("strike ausente".into());
    };
    let Some(forward) = forward_na_base(hedge, &base, &quote, (k_call, k_put)) else {
        return Err("câmbio do hedge ausente".into());
    };
    let faixa = thresholds.min_forward_strike_ratio..=thresholds.max_forward_strike_ratio;
    if !faixa.contains(&(forward / k_call)) || !faixa.contains(&(forward / k_put)) {
        return Err(format!(
            "F/K fora de {} a {}",
            thresholds.min_forward_strike_ratio, thresholds.max_forward_strike_ratio
        ));
    }
    let dias = prazo_dias(grupo, venc_a).ok_or("vencimento da opção ilegível")?;
    if dias < thresholds.min_t_days {
        return Err(format!("T menor que {} dias", thresholds.min_t_days));
    }
    let t = dias as f64 / 365.0;
    let (Some(n_call), Some(n_put)) = (call.notional_em(&base), put.notional_em(&base)) else {
        return Err("notional da opção sem perna na moeda-base".into());
    };
    let Some(h) = hedge.notional_em(&base) else {
        return Err("notional do hedge sem perna na moeda-base".into());
    };
    if n_call <= 0.0 || n_put <= 0.0 {
        return Err("notional de opção inválido".into());
    }
    // V: preço por unidade da base, na moeda de cotação.
    let preco = |p: &Perna, n: f64| -> Option<f64> {
        let premium = p.premium?;
        match p.premium_currency.as_deref()? {
            c if c == quote => Some(premium / n),
            c if c == base => Some(premium * forward / n),
            _ => None,
        }
    };
    let (Some(v_call), Some(v_put)) = (preco(call, n_call), preco(put, n_put)) else {
        return Err("prêmio vazio ou numa moeda fora do par".into());
    };
    let (Some(sigma_call), Some(sigma_put)) = (
        implied_volatility(v_call, forward, k_call, t, true),
        implied_volatility(v_put, forward, k_put, t, false),
    ) else {
        return Err("σ sem solução".into());
    };
    let delta_call = phi(d1(forward, k_call, sigma_call, t));
    let delta_put = phi(d1(forward, k_put, sigma_put, t)) - 1.0;
    let total = n_call + n_put;
    let d_mesma = (delta_call * n_call + delta_put * n_put).abs();
    let d_opostas = delta_call * n_call + delta_put.abs() * n_put;
    let d_cheio = total / 2.0;
    Ok(HedgeTest {
        moeda_base: base,
        forward,
        prazo_anos: t,
        sigma_call,
        sigma_put,
        delta_call,
        delta_put,
        notional_call: n_call,
        notional_put: n_put,
        hedge_notional: h,
        d_mesma,
        d_opostas,
        d_cheio,
        res_mesma: (d_mesma - h).abs() / total,
        res_opostas: (d_opostas - h).abs() / total,
        res_cheio: (d_cheio - h).abs() / total,
        sep: (d_opostas - d_mesma) / total,
        sep_cheio: (d_cheio - d_opostas).abs() / total,
        tau_res: thresholds.tau_res,
        tau_sep: thresholds.tau_sep,
    })
}

/// Forma das opções segundo o arquivo 02 do brain.
enum Forma {
    Nome(&'static str),
    /// 2 opções em lados opostos, strikes diferentes, mesmo vencimento: decide o arquivo 03.
    StrangleRrCollar,
}

fn forma(opcoes: &[&Perna]) -> Forma {
    let mut lados: BTreeMap<&str, usize> = BTreeMap::new();
    let mut vencimentos: BTreeMap<&str, usize> = BTreeMap::new();
    let mut strikes: Vec<f64> = Vec::new();
    for p in opcoes {
        let (Some(lado), Some(strike), Some(venc)) =
            (p.call_ccy.as_deref(), p.strike, p.vencimento.as_deref())
        else {
            return Forma::Nome("Não identificado");
        };
        *lados.entry(lado).or_default() += 1;
        *vencimentos.entry(venc).or_default() += 1;
        strikes.push(strike);
    }
    strikes.sort_by(f64::total_cmp);
    // Quantidade de opções por strike, em ordem crescente de strike.
    let mut por_strike: Vec<usize> = Vec::new();
    let mut anterior: Option<String> = None;
    for strike in &strikes {
        let chave = chave_strike(*strike);
        if let (Some(quantidade), true) = (por_strike.last_mut(), anterior.as_ref() == Some(&chave))
        {
            *quantidade += 1;
        } else {
            por_strike.push(1);
            anterior = Some(chave);
        }
    }
    let (n, nl, ns, nv) = (
        opcoes.len(),
        lados.len(),
        por_strike.len(),
        vencimentos.len(),
    );
    let nome = match n {
        1 => "Opção única",
        _ if nl == 1 && ns == 1 && nv == 1 => "Fracionamento de notional",
        2 if nl == 2 && nv == 1 && ns == 1 => "Straddle",
        2 if nl == 2 && nv == 1 => return Forma::StrangleRrCollar,
        2 if nl == 2 => "Lados opostos, vencimentos diferentes",
        2 if nv == 1 => "Spread vertical",
        2 if ns == 1 => "Calendar spread",
        2 => "Spread diagonal",
        3 if nl == 2 && nv == 1 && ns == 3 => "Seagull",
        3 if nl == 2 && nv == 1 && ns == 1 => "Strip/Strap",
        3 | 4 if nl == 1 && ns == 2 && por_strike[0] != por_strike[1] => "Ratio spread",
        4 if nl == 2 && lados.values().all(|q| *q == 2) => "Iron condor",
        4 if nl == 1 && por_strike == [1, 2, 1] => "Butterfly",
        4 if nl == 1 && ns == 4 => "Condor",
        n if n >= 6 && ns <= 3 && vencimentos.values().all(|q| *q == 2) => "Programa de hedge",
        _ => "Não identificado",
    };
    Forma::Nome(nome)
}

/// Tabela do arquivo 03 do brain; vale a primeira regra verdadeira.
fn tabela_hedge(teste: Option<&HedgeTest>) -> (&'static str, &'static str) {
    let Some(t) = teste else {
        return ("Ambiguous", "03/R1");
    };
    let (tau, tau_sep) = (t.tau_res, t.tau_sep);
    if t.sep < tau_sep {
        ("Ambiguous", "03/R2")
    } else if t.res_opostas <= tau && t.res_cheio <= tau && t.sep_cheio < tau_sep {
        ("Ambiguous", "03/R3")
    } else if t.res_opostas <= tau && t.res_mesma > tau {
        ("Risk reversal", "03/R4")
    } else if t.res_mesma <= tau && t.res_opostas > tau {
        ("Strangle", "03/R5")
    } else if t.res_mesma <= tau && t.res_opostas <= tau {
        ("Ambiguous", "03/R6")
    } else if t.res_cheio <= tau && t.sep_cheio >= tau_sep {
        ("Collar", "03/R7")
    } else {
        ("Ambiguous", "03/R8")
    }
}

/// Linha de base: a tabela de decisão do brain aplicada em código.
/// Devolve (estrutura, regra). Esse valor nunca é enviado à IA.
pub fn regra_codigo(grupo: &Grupo) -> (String, String) {
    let opcoes: Vec<&Perna> = grupo.pernas.iter().filter(|p| p.is_option()).collect();
    match forma(&opcoes) {
        Forma::Nome(nome) => (nome.to_string(), format!("02/{}", nome.replace('/', "-"))),
        Forma::StrangleRrCollar => {
            let (nome, regra) = tabela_hedge(grupo.features.teste_hedge.as_ref());
            (nome.to_string(), regra.to_string())
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use serde_json::json;

    pub(crate) fn opcao(id: &str, call: &str, strike: f64, notional: f64, premium: f64) -> Perna {
        Perna {
            id: id.into(),
            grupo_id: String::new(),
            execution_timestamp: DateTime::parse_from_rfc3339("2025-10-02T17:47:10Z")
                .unwrap()
                .with_timezone(&Utc),
            platform_identifier: "TSEF".into(),
            par: "GBP-USD".into(),
            familia: "Opção vanilla".into(),
            estilo: None,
            call_ccy: Some(call.into()),
            base_ccy: Some("GBP".into()),
            strike: Some(strike),
            vencimento: Some("2025-11-03".into()),
            notional: Some(notional),
            notional_currency: Some("GBP".into()),
            notional_2: Some(notional * strike),
            notional_currency_2: Some("USD".into()),
            premium: Some(premium),
            premium_currency: Some("USD".into()),
            exchange_rate: Some(strike),
            exchange_rate_basis: Some("GBP/USD".into()),
            capado: false,
            package_indicator: true,
            source_file: "test".into(),
            source_line: 1,
            raw: json!({}),
        }
    }

    pub(crate) fn hedge(notional_gbp: f64) -> Perna {
        let mut hedge = opcao("h", "GBP", 0.0, notional_gbp, 0.0);
        hedge.familia = "Forward".into();
        hedge.call_ccy = None;
        hedge.strike = None;
        hedge.premium = None;
        hedge.premium_currency = None;
        hedge.vencimento = None;
        hedge.exchange_rate = Some(1.344);
        hedge.notional_2 = Some(notional_gbp * 1.344);
        hedge
    }

    pub(crate) fn grupo(pernas: Vec<Perna>) -> Grupo {
        Grupo {
            id: "g".into(),
            timestamp: pernas[0].execution_timestamp,
            platform_identifier: "TSEF".into(),
            par: "GBP-USD".into(),
            pernas,
            features: Features::default(),
            regra_codigo: String::new(),
            regra_codigo_ref: String::new(),
        }
    }

    fn caso_gbp(hedge_gbp: f64) -> Grupo {
        // Caso real da seção 6 do MVP: GBP/USD, TSEF, 2025-10-02T17:47:10Z.
        let call = opcao("c", "GBP", 1.3615, 30_113_700.0, 113_679.217_5);
        let put = opcao("p", "USD", 1.3260, 30_000_000.0, 121_500.0);
        grupo(vec![call, put, hedge(hedge_gbp)])
    }

    #[test]
    fn caso_gbp_reproduz_o_teste_do_documento() {
        let t = calcula_teste_hedge(&caso_gbp(15_000_000.0), &Thresholds::default()).unwrap();
        assert!((t.prazo_anos - 32.0 / 365.0).abs() < 1e-12);
        assert!((t.sigma_call - 0.064031).abs() < 5e-6, "{}", t.sigma_call);
        assert!((t.delta_call - 0.250514).abs() < 5e-6, "{}", t.delta_call);
        assert!((t.sigma_put - 0.068068).abs() < 5e-6, "{}", t.sigma_put);
        assert!((t.delta_put + 0.248544).abs() < 5e-6, "{}", t.delta_put);
        assert!((t.res_mesma - 0.248070).abs() < 5e-6, "{}", t.res_mesma);
        assert!((t.res_opostas - 0.000004).abs() < 5e-6, "{}", t.res_opostas);
        assert!((t.res_cheio - 0.250473).abs() < 5e-6, "{}", t.res_cheio);
    }

    #[test]
    fn exemplos_1_e_4_do_brain_decidem_risk_reversal_e_collar() {
        for (hedge_gbp, esperado, regra) in [
            (15_000_000.0, "Risk reversal", "03/R4"),
            (30_000_000.0, "Collar", "03/R7"),
        ] {
            let mut g = caso_gbp(hedge_gbp);
            g.features.teste_hedge = calcula_teste_hedge(&g, &Thresholds::default()).ok();
            assert_eq!(regra_codigo(&g), (esperado.to_string(), regra.to_string()));
        }
    }

    #[test]
    fn motivos_da_secao_6() {
        let mut g = caso_gbp(15_000_000.0);
        g.pernas.retain(|p| !p.is_hedge());
        assert_eq!(
            calcula_teste_hedge(&g, &Thresholds::default()).unwrap_err(),
            "nenhum hedge no grupo"
        );
        let mut g = caso_gbp(15_000_000.0);
        g.pernas[0].capado = true;
        assert_eq!(
            calcula_teste_hedge(&g, &Thresholds::default()).unwrap_err(),
            "notional capado"
        );
        let mut g = caso_gbp(15_000_000.0);
        g.pernas[1].premium_currency = Some("EUR".into());
        assert_eq!(
            calcula_teste_hedge(&g, &Thresholds::default()).unwrap_err(),
            "prêmio vazio ou numa moeda fora do par"
        );
    }

    #[test]
    fn hedge_cotado_ao_contrario_usa_1_sobre_f() {
        let mut g = caso_gbp(15_000_000.0);
        let h = g.pernas.iter_mut().find(|p| p.is_hedge()).unwrap();
        h.exchange_rate = Some(1.0 / 1.344);
        h.base_ccy = Some("USD".into());
        let t = calcula_teste_hedge(&g, &Thresholds::default()).unwrap();
        assert!((t.forward - 1.344).abs() < 1e-12);
    }

    #[test]
    fn formas_do_arquivo_02() {
        let nome = |pernas: Vec<Perna>| regra_codigo(&grupo(pernas)).0;
        let c = |strike: f64| opcao("x", "GBP", strike, 1.0, 0.0);
        let p = |strike: f64| opcao("y", "USD", strike, 1.0, 0.0);
        assert_eq!(nome(vec![c(1.3)]), "Opção única");
        assert_eq!(nome(vec![c(1.3), p(1.3)]), "Straddle");
        assert_eq!(nome(vec![c(1.3), c(1.3)]), "Fracionamento de notional");
        assert_eq!(nome(vec![c(1.3), c(1.4)]), "Spread vertical");
        assert_eq!(nome(vec![c(1.2), p(1.3), p(1.4)]), "Seagull");
        assert_eq!(nome(vec![c(1.3), p(1.3), p(1.3)]), "Strip/Strap");
        assert_eq!(nome(vec![c(1.2), c(1.3), c(1.3), c(1.4)]), "Butterfly");
        assert_eq!(nome(vec![c(1.2), c(1.3), c(1.4), c(1.5)]), "Condor");
        assert_eq!(nome(vec![c(1.2), c(1.3), p(1.4), p(1.5)]), "Iron condor");
        assert_eq!(nome(vec![c(1.2), c(1.3), c(1.3)]), "Ratio spread");
        let mut longa = p(1.3);
        longa.vencimento = Some("2025-12-03".into());
        assert_eq!(
            nome(vec![c(1.3), longa.clone()]),
            "Lados opostos, vencimentos diferentes"
        );
        longa.call_ccy = Some("GBP".into());
        assert_eq!(nome(vec![c(1.3), longa]), "Calendar spread");
        // Strikes diferentes em lados opostos sem hedge: o arquivo 03 decide (R1).
        assert_eq!(
            regra_codigo(&grupo(vec![c(1.3), p(1.2)])),
            ("Ambiguous".to_string(), "03/R1".to_string())
        );
    }
}
