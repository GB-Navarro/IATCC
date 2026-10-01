/*
   Este parser transforma os CSVs contidos nos slices DTCC em pernas tipadas.
   Ele normaliza cabeçalhos, datas, moedas, notional e flags, preservando o
   registro bruto para auditoria antes de o módulo grupos formar candidatos.
*/

use crate::tipos::Perna;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Timelike, Utc};
use csv::StringRecord;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;
use zip::ZipArchive;

fn normalizar_campo(valor: &str) -> String {
    // Uniformiza o nome de uma coluna para facilitar aliases do DTCC.
    valor
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn mapa_linha(headers: &StringRecord, row: &StringRecord) -> HashMap<String, String> {
    // Cria o mapa cabeçalho->valor da linha atual.
    headers
        .iter()
        .zip(row.iter())
        .map(|(header, value)| (normalizar_campo(header), value.trim().to_string()))
        .collect()
}

fn campo<'a>(linha: &'a HashMap<String, String>, nomes: &[&str]) -> Option<&'a str> {
    // Devolve o primeiro alias com valor; uma coluna presente mas vazia passa ao próximo.
    nomes.iter().find_map(|nome| {
        linha
            .get(&normalizar_campo(nome))
            .map(String::as_str)
            .filter(|valor| !valor.trim().is_empty())
    })
}

fn parse_numero(valor: &str) -> Option<f64> {
    // Converte números textuais removendo moeda, espaços e separadores.
    let limpo = valor.trim().replace([',', '$', ' '], "");
    if limpo.is_empty() || matches!(limpo.to_ascii_uppercase().as_str(), "NA" | "N/A" | "NULL") {
        return None;
    }
    limpo.trim_end_matches('+').parse().ok()
}

fn parse_numero_com_cap(valor: Option<&str>) -> (Option<f64>, bool) {
    // Extrai número e flag de cap representada por um sinal de mais.
    let Some(valor) = valor else {
        return (None, false);
    };
    (parse_numero(valor), valor.trim().ends_with('+'))
}

fn parse_bool(valor: Option<&str>) -> bool {
    // Interpreta a flag booleana case-insensitively.
    valor.is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

fn parse_timestamp(valor: &str) -> Option<DateTime<Utc>> {
    // Aceita timestamp RFC3339 ou o formato legado do CSV.
    DateTime::parse_from_rfc3339(valor)
        .ok()
        .map(|v| v.with_timezone(&Utc))
        .or_else(|| {
            NaiveDateTime::parse_from_str(valor, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|v| DateTime::from_naive_utc_and_offset(v, Utc))
        })
}

fn parse_vencimento(valor: Option<&str>) -> Option<String> {
    // Normaliza vencimento para data ISO, aceitando formatos comuns da amostra.
    let valor = valor?.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(valor) {
        return Some(dt.date_naive().to_string());
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(valor, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.date().to_string());
    }
    NaiveDate::parse_from_str(valor, "%Y-%m-%d")
        .ok()
        .map(|v| v.to_string())
        .or_else(|| {
            NaiveDate::parse_from_str(valor, "%m/%d/%Y")
                .ok()
                .map(|v| v.to_string())
        })
}

fn moedas(valor: &str) -> Vec<String> {
    // Extrai códigos alfabéticos de três letras de um texto cambial.
    valor
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|item| item.len() == 3)
        .map(|item| item.to_ascii_uppercase())
        .collect()
}

fn par_moedas(leg1: Option<&str>, leg2: Option<&str>) -> Option<String> {
    // Infere um par estável a partir das moedas das pernas.
    let mut lista = Vec::new();
    for valor in [leg1, leg2].into_iter().flatten() {
        if valor.len() == 3 && valor.chars().all(|c| c.is_ascii_alphabetic()) {
            lista.push(valor.to_ascii_uppercase());
        } else {
            lista.extend(moedas(valor));
        }
    }
    lista.dedup();
    if lista.len() < 2 {
        return None;
    }
    lista.truncate(2);
    lista.sort();
    Some(format!("{}-{}", lista[0], lista[1]))
}

fn primeiro_cambio(valor: Option<&str>) -> Option<String> {
    // Obtém a primeira moeda de uma expressão de cotação.
    moedas(valor?).into_iter().next()
}

fn quase_igual(a: f64, b: f64) -> bool {
    // Tolerância de 2% cobre o arredondamento de notional feito pelo DTCC.
    a > 0.0 && b > 0.0 && (a / b - 1.0).abs() < 0.02
}

/// Moeda-base: a moeda em que `taxa` (strike ou câmbio) é cotada por unidade.
///
/// A notação "X/Y" do DTCC não é consistente entre reportantes (há "BRL/USD" com
/// strike 5,05, que é USD-base), então primeiro se testa qual perna de notional,
/// multiplicada pela taxa, reproduz a outra. Só sem essa evidência vale a regra da
/// seção 5.2 do MVP: primeira moeda do par do strike, senão do Exchange rate basis.
fn moeda_base(
    par: &str,
    pernas: [(Option<f64>, Option<&str>); 2],
    taxa: Option<f64>,
    cotacoes: [Option<&str>; 2],
) -> Option<String> {
    let [(n1, c1), (n2, c2)] = pernas;
    if let (Some(n1), Some(c1), Some(n2), Some(c2), Some(k)) = (n1, c1, n2, c2, taxa) {
        let erro = |a: f64, b: f64| (a / b - 1.0).abs();
        match (quase_igual(n2, n1 * k), quase_igual(n1, n2 * k)) {
            (true, false) => return Some(c1.to_ascii_uppercase()),
            (false, true) => return Some(c2.to_ascii_uppercase()),
            (true, true) if erro(n2, n1 * k) <= erro(n1, n2 * k) => {
                return Some(c1.to_ascii_uppercase());
            }
            (true, true) => return Some(c2.to_ascii_uppercase()),
            _ => {}
        }
    }
    cotacoes
        .into_iter()
        .filter_map(primeiro_cambio)
        .find(|moeda| par.split('-').any(|ccy| ccy == moeda))
}

fn familia(fisn: &str) -> Option<&'static str> {
    // Mapeia prefixos FISN para as famílias relevantes ao MVP.
    let fisn = fisn.trim();
    let upper = fisn.to_ascii_uppercase();
    if upper.starts_with("NA/FWD NDF") {
        return Some("NDF");
    }
    if upper.starts_with("NA/FWD ") {
        let tokens: Vec<_> = upper.split_whitespace().collect();
        if tokens.len() == 3 && tokens[1].len() == 3 && tokens[2].len() == 3 {
            return Some("Forward");
        }
    }
    if upper.starts_with("NA/FX FWD NSTD") {
        return Some("Forward");
    }
    if upper.starts_with("NA/O NDO") {
        return Some("Opção NDO");
    }
    if upper.starts_with("NA/O VAN") {
        return Some("Opção vanilla");
    }
    if [
        "NA/O DIG",
        "NA/O BAR",
        "NA/O TARG",
        "NA/FX O NSTD",
        "NA/FX OP",
    ]
    .iter()
    .any(|prefix| upper.starts_with(prefix))
    {
        return Some("Opção exótica");
    }
    None
}

fn parse_linha(
    headers: &StringRecord,
    row: &StringRecord,
    source_file: &str,
    source_line: u64,
) -> Result<Option<Perna>> {
    // Converte uma linha NEWT em Perna e descarta registros incompletos ou fora do escopo.
    let linha = mapa_linha(headers, row);
    let action = campo(&linha, &["Action type", "ActionType"]);
    if action.is_some_and(|v| !v.eq_ignore_ascii_case("NEWT")) {
        return Ok(None);
    }
    let fisn = campo(&linha, &["UPI FISN", "UPIFISN"]).unwrap_or_default();
    let Some(familia) = familia(fisn) else {
        return Ok(None);
    };
    let Some(id) = campo(
        &linha,
        &["Dissemination Identifier", "DisseminationIdentifier"],
    )
    .map(ToOwned::to_owned) else {
        return Ok(None);
    };
    let Some(timestamp_raw) = campo(&linha, &["Execution Timestamp", "ExecutionTimestamp"]) else {
        return Ok(None);
    };
    let Some(mut timestamp) = parse_timestamp(timestamp_raw) else {
        return Ok(None);
    };
    timestamp = timestamp.with_nanosecond(0).unwrap_or(timestamp);
    let platform = campo(&linha, &["Platform identifier", "PlatformIdentifier"])
        .unwrap_or("UNKNOWN")
        .to_ascii_uppercase();
    let ccy1 = campo(
        &linha,
        &[
            "Notional currency-Leg 1",
            "NotionalCurrencyLeg1",
            "Notional currency",
        ],
    );
    let ccy2 = campo(&linha, &["Notional currency-Leg 2", "NotionalCurrencyLeg2"]);
    let Some(par) = par_moedas(ccy1, ccy2) else {
        return Ok(None);
    };
    let put_ccy = campo(&linha, &["Put currency", "PutCurrency"]).map(|v| v.to_ascii_uppercase());
    let call_ccy = campo(&linha, &["Call currency", "CallCurrency"])
        .map(|v| v.to_ascii_uppercase())
        .or_else(|| {
            let put = put_ccy.as_deref()?;
            par.split('-')
                .find(|ccy| *ccy != put)
                .map(ToOwned::to_owned)
        });
    let (notional, cap_notional) = parse_numero_com_cap(campo(
        &linha,
        &[
            "Notional amount-Leg 1",
            "NotionalAmountLeg1",
            "Notional amount",
            "NotionalAmount",
        ],
    ));
    let (notional_2, cap_notional_2) = parse_numero_com_cap(campo(
        &linha,
        &["Notional amount-Leg 2", "NotionalAmountLeg2"],
    ));
    // O DTCC publica o prêmio em "Option Premium Amount/Currency"; os demais são aliases.
    let (premium, cap_premium) = parse_numero_com_cap(campo(
        &linha,
        &["Option Premium Amount", "Premium amount", "PremiumAmount"],
    ));
    let strike = campo(&linha, &["Strike price", "StrikePrice", "Strike"]).and_then(parse_numero);
    let exchange_rate = campo(&linha, &["Exchange rate", "ExchangeRate"]).and_then(parse_numero);
    let exchange_rate_basis = campo(&linha, &["Exchange rate basis", "ExchangeRateBasis"]);
    let base_ccy = moeda_base(
        &par,
        [(notional, ccy1), (notional_2, ccy2)],
        if familia.starts_with("Opção") {
            strike
        } else {
            exchange_rate
        },
        [
            campo(
                &linha,
                &[
                    "Strike price currency/currency pair",
                    "StrikePriceCurrencyCurrencyPair",
                ],
            ),
            exchange_rate_basis,
        ],
    );
    let mut raw = Map::new();
    for (header, value) in headers.iter().zip(row.iter()) {
        raw.insert(header.to_string(), Value::String(value.to_string()));
    }
    Ok(Some(Perna {
        id,
        grupo_id: String::new(),
        execution_timestamp: timestamp,
        platform_identifier: platform,
        par,
        familia: familia.to_string(),
        estilo: campo(&linha, &["Option style", "OptionStyle", "Style"]).map(ToOwned::to_owned),
        call_ccy,
        base_ccy,
        strike,
        vencimento: parse_vencimento(campo(
            &linha,
            &[
                "Expiration date",
                "ExpirationDate",
                "Contract expiration date",
                "Maturity date",
                "MaturityDate",
            ],
        )),
        notional,
        notional_currency: ccy1.map(|v| v.to_ascii_uppercase()),
        notional_2,
        notional_currency_2: ccy2.map(|v| v.to_ascii_uppercase()),
        premium,
        premium_currency: campo(
            &linha,
            &[
                "Option Premium Currency",
                "Premium currency",
                "PremiumCurrency",
            ],
        )
        .map(|v| v.to_ascii_uppercase()),
        exchange_rate,
        exchange_rate_basis: exchange_rate_basis.map(ToOwned::to_owned),
        capado: cap_notional || cap_notional_2 || cap_premium,
        package_indicator: parse_bool(campo(&linha, &["Package indicator", "PackageIndicator"])),
        source_file: source_file.to_string(),
        source_line,
        raw: Value::Object(raw),
    }))
}

pub fn parse_csv<R: Read>(reader: R, source_file: &str) -> Result<Vec<Perna>> {
    // Itera pelo CSV e associa falhas ao arquivo e ao número da linha.
    let mut csv = csv::ReaderBuilder::new().flexible(true).from_reader(reader);
    let headers = csv.headers().context("lendo cabeçalho CSV")?.clone();
    let mut pernas = Vec::new();
    for (index, row) in csv.records().enumerate() {
        let row = row.with_context(|| format!("lendo linha {} de {source_file}", index + 2))?;
        if let Some(perna) = parse_linha(&headers, &row, source_file, index as u64 + 2)? {
            pernas.push(perna);
        }
    }
    Ok(pernas)
}

pub fn parse_zip(path: &Path) -> Result<Vec<Perna>> {
    // Localiza um CSV no ZIP capturado e reutiliza o parser de CSV.
    let file = std::fs::File::open(path).with_context(|| format!("abrindo {}", path.display()))?;
    let mut archive = ZipArchive::new(file).context("abrindo zip da amostra")?;
    let total = archive.len();
    for index in 0..total {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if !name.to_ascii_lowercase().ends_with(".csv") && index + 1 < total {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        return parse_csv(Cursor::new(bytes), &format!("{}:{name}", path.display()));
    }
    bail!("zip {} não contém CSV", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booleano_e_capado_sao_case_insensitive() {
        assert!(parse_bool(Some("TrUe")));
        assert_eq!(
            parse_numero_com_cap(Some("10,270,500+")),
            (Some(10_270_500.0), true)
        );
    }

    // Cabeçalhos e valores copiados de linhas reais do slice CFTC_FX (30/09/2026).
    const CSV_REAL: &str = "\"Dissemination Identifier\",\"Action type\",\"Execution Timestamp\",\"Expiration Date\",\"Platform identifier\",\"Notional amount-Leg 1\",\"Notional amount-Leg 2\",\"Notional currency-Leg 1\",\"Notional currency-Leg 2\",\"Call currency\",\"Put currency\",\"Exchange rate\",\"Exchange rate basis\",\"Option Premium Amount\",\"Option Premium Currency\",\"Strike Price\",\"Strike price currency/currency pair\",\"Package indicator\",\"UPI FISN\"
\"1\",\"NEWT\",\"2026-09-30T09:38:09Z\",\"2026-12-30\",\"BILT\",\"500,000\",\"680,000\",\"GBP\",\"USD\",\"GBP\",\"USD\",\"1.36\",\"GBP/USD\",\"1,935\",\"GBP\",\"1.36\",\"GBP/USD\",\"TRUE\",\"NA/O Van Call GBP USD\"
\"2\",\"NEWT\",\"2026-09-30T10:00:00Z\",\"2026-10-30\",\"BILT\",\"4,000,000\",\"20,200,000\",\"USD\",\"BRL\",\"\",\"BRL\",\"5.05\",\"BRL/USD\",\"10,000\",\"USD\",\"5.05\",\"BRL/USD\",\"false\",\"NA/O NDO Call BRL USD\"
\"3\",\"NEWT\",\"2026-09-30T10:00:01Z\",\"2026-10-30\",\"XXXX\",\"4,000,000\",\"612,522,799\",\"USD\",\"JPY\",\"USD\",\"JPY\",\"153.13\",\"USD/JPY\",\"5,000\",\"USD\",\"153.13\",\"\",\"FALSE\",\"NA/O Van Call USD JPY\"
\"4\",\"MODI\",\"2026-09-30T10:00:02Z\",\"2026-10-30\",\"XXXX\",\"1\",\"1\",\"USD\",\"JPY\",\"USD\",\"JPY\",\"1\",\"USD/JPY\",\"1\",\"USD\",\"1\",\"USD/JPY\",\"FALSE\",\"NA/O Van Call USD JPY\"
\"5\",\"NEWT\",\"2026-09-30T10:00:03Z\",\"2026-11-02\",\"TSEF\",\"137,318,109+\",\"25,000,000\",\"BRL\",\"USD\",\"\",\"\",\"5.4927\",\"USD/BRL\",\"\",\"\",\"\",\"\",\"TRUE\",\"NA/Fwd NDF BRL USD\"
";

    #[test]
    fn linhas_reais_viram_pernas_com_premio_base_e_lado() {
        let pernas = parse_csv(CSV_REAL.as_bytes(), "real.csv").unwrap();
        assert_eq!(pernas.len(), 4, "a linha MODI deve ser descartada");
        let gbp = &pernas[0];
        assert_eq!(gbp.premium, Some(1935.0));
        assert_eq!(gbp.premium_currency.as_deref(), Some("GBP"));
        assert_eq!(gbp.base_ccy.as_deref(), Some("GBP"));
        assert_eq!(gbp.notional_em("USD"), Some(680_000.0));
        assert!(gbp.package_indicator);
        // "BRL/USD" com strike 5,05 é USD-base; o lado vem da Put currency.
        let brl = &pernas[1];
        assert_eq!(brl.base_ccy.as_deref(), Some("USD"));
        assert_eq!(brl.call_ccy.as_deref(), Some("USD"));
        assert_eq!(brl.familia, "Opção NDO");
        // Par do strike vazio: a base vem da consistência dos notionals, não da ordem alfabética.
        assert_eq!(pernas[2].base_ccy.as_deref(), Some("USD"));
        // NDF capado: a base usa o câmbio, e o "+" marca a perna.
        let ndf = &pernas[3];
        assert_eq!(ndf.familia, "NDF");
        assert!(ndf.capado);
        assert_eq!(ndf.par, "BRL-USD");
    }

    #[test]
    fn campo_passa_para_o_proximo_alias_quando_a_coluna_esta_vazia() {
        let linha: HashMap<String, String> = [
            (
                normalizar_campo("Strike price currency/currency pair"),
                String::new(),
            ),
            (
                normalizar_campo("Exchange rate basis"),
                "USD/JPY".to_string(),
            ),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            campo(
                &linha,
                &["Strike price currency/currency pair", "Exchange rate basis"]
            ),
            Some("USD/JPY")
        );
    }

    #[test]
    fn familias_respeitam_a_ordem_do_mvp() {
        assert_eq!(familia("NA/Fwd NDF USD BRL"), Some("NDF"));
        assert_eq!(familia("NA/Fwd EUR USD"), Some("Forward"));
        assert_eq!(familia("NA/O Van EUR USD"), Some("Opção vanilla"));
        assert_eq!(familia("NA/O Dig EUR USD"), Some("Opção exótica"));
        assert_eq!(familia("NA/Equity"), None);
    }
}
