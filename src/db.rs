/*
   Este módulo é a camada de persistência SQLite do MVP. Ele cria e migra o
   schema, armazena grupos/pernas/classificações e oferece consultas usadas por
   CLI, MCP e web, incluindo metadados dos providers para auditoria.
*/

use crate::classifier::ProviderMetadata;
use crate::tipos::{Classificacao, Features, Grupo, Perna};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

pub const DB_PATH: &str = "mvp.db";

pub fn open(path: impl AsRef<Path>) -> Result<Connection> {
    // Abre (ou cria) o SQLite, ativa chaves estrangeiras e garante a versão atual do schema.
    let conn = Connection::open(path.as_ref())
        .with_context(|| format!("abrindo SQLite {}", path.as_ref().display()))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    schema(&conn)?;
    Ok(conn)
}

pub fn open_existente(path: impl AsRef<Path>) -> Result<Connection> {
    // Leitores não criam um banco vazio: sem mvp.db, o passo 2 ainda não rodou.
    if !path.as_ref().exists() {
        bail!(
            "{} não existe; rode antes `cargo run --release --bin preparar`",
            path.as_ref().display()
        );
    }
    open(path)
}

fn schema(conn: &Connection) -> Result<()> {
    // Cria tabelas/indexes e adiciona colunas novas em bancos antigos.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS grupos (
            id TEXT PRIMARY KEY,
            timestamp TEXT NOT NULL,
            platform_identifier TEXT NOT NULL,
            par TEXT NOT NULL,
            numeros TEXT NOT NULL,
            regra_codigo TEXT NOT NULL,
            regra_codigo_ref TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS pernas (
            id TEXT PRIMARY KEY,
            grupo_id TEXT NOT NULL REFERENCES grupos(id) ON DELETE CASCADE,
            execution_timestamp TEXT NOT NULL,
            platform_identifier TEXT NOT NULL,
            par TEXT NOT NULL,
            familia TEXT NOT NULL,
            estilo TEXT,
            call_ccy TEXT,
            base_ccy TEXT,
            strike REAL,
            vencimento TEXT,
            notional REAL,
            notional_currency TEXT,
            premium REAL,
            premium_currency TEXT,
            exchange_rate REAL,
            exchange_rate_basis TEXT,
            capado INTEGER NOT NULL,
            package_indicator INTEGER NOT NULL,
            source_file TEXT NOT NULL,
            source_line INTEGER NOT NULL,
            raw TEXT NOT NULL,
            notional_2 REAL,
            notional_currency_2 TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_grupos_par ON grupos(par);
        CREATE INDEX IF NOT EXISTS idx_pernas_grupo ON pernas(grupo_id);
        CREATE TABLE IF NOT EXISTS classificacoes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            grupo_id TEXT NOT NULL REFERENCES grupos(id) ON DELETE CASCADE,
            modelo TEXT NOT NULL,
            brain_hash TEXT NOT NULL,
            config_hash TEXT NOT NULL,
            input_hash TEXT NOT NULL,
            resposta TEXT NOT NULL,
            criado_em TEXT NOT NULL,
            provider TEXT NOT NULL DEFAULT 'anthropic',
            model_version TEXT,
            request_id TEXT,
            latency_ms INTEGER NOT NULL DEFAULT 0,
            raw_response TEXT NOT NULL DEFAULT 'null',
            status TEXT NOT NULL DEFAULT 'completed'
        );
        CREATE INDEX IF NOT EXISTS idx_classificacoes_grupo ON classificacoes(grupo_id, id DESC);
        CREATE TABLE IF NOT EXISTS journal (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tipo TEXT NOT NULL,
            objeto_id TEXT NOT NULL,
            payload TEXT NOT NULL,
            criado_em TEXT NOT NULL
        );
        "#,
    )?;
    for (table, name, definition) in [
        ("grupos", "regra_codigo_ref", "TEXT NOT NULL DEFAULT ''"),
        ("pernas", "notional_2", "REAL"),
        ("pernas", "notional_currency_2", "TEXT"),
        (
            "classificacoes",
            "provider",
            "TEXT NOT NULL DEFAULT 'anthropic'",
        ),
        ("classificacoes", "model_version", "TEXT"),
        ("classificacoes", "request_id", "TEXT"),
        ("classificacoes", "latency_ms", "INTEGER NOT NULL DEFAULT 0"),
        (
            "classificacoes",
            "raw_response",
            "TEXT NOT NULL DEFAULT 'null'",
        ),
        (
            "classificacoes",
            "status",
            "TEXT NOT NULL DEFAULT 'completed'",
        ),
    ] {
        ensure_column(conn, table, name, definition)?;
    }
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, name: &str, definition: &str) -> Result<()> {
    // Faz uma migração aditiva idempotente para manter compatibilidade do MVP.
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|column| column == name);
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {name} {definition}"),
            [],
        )?;
    }
    Ok(())
}

pub fn count_classifications(conn: &Connection) -> Result<usize> {
    Ok(
        conn.query_row("SELECT COUNT(*) FROM classificacoes", [], |r| {
            r.get::<_, i64>(0)
        })? as usize,
    )
}

/// Reconstrói grupos e pernas numa transação. Apaga as classificações anteriores,
/// porque os números dos grupos podem ter mudado; recusa sem `forcar` se houver alguma.
pub fn rebuild(path: impl AsRef<Path>, grupos: &[Grupo], forcar: bool) -> Result<usize> {
    let mut conn = open(path)?;
    let anteriores = count_classifications(&conn)?;
    if anteriores > 0 && !forcar {
        bail!(
            "o banco já tem {anteriores} classificação(ões) da IA; use --forcar para apagá-las e reconstruir"
        );
    }
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM classificacoes", [])?;
    tx.execute("DELETE FROM pernas", [])?;
    tx.execute("DELETE FROM grupos", [])?;
    for grupo in grupos {
        let numeros = serde_json::to_string(&grupo.features)?;
        tx.execute(
            "INSERT INTO grupos(id,timestamp,platform_identifier,par,numeros,regra_codigo,regra_codigo_ref) VALUES(?,?,?,?,?,?,?)",
            params![grupo.id, grupo.timestamp.to_rfc3339(), grupo.platform_identifier, grupo.par, numeros, grupo.regra_codigo, grupo.regra_codigo_ref],
        )?;
        for perna in &grupo.pernas {
            insert_leg(&tx, perna)?;
        }
    }
    tx.execute(
        "INSERT INTO journal(tipo,objeto_id,payload,criado_em) VALUES(?,?,?,?)",
        params![
            "rebuild",
            "*",
            json!({"grupos": grupos.len(), "classificacoes_apagadas": anteriores}).to_string(),
            Utc::now().to_rfc3339()
        ],
    )?;
    tx.commit()?;
    Ok(anteriores)
}

const COLUNAS_PERNA: &str = "id,grupo_id,execution_timestamp,platform_identifier,par,familia,estilo,call_ccy,base_ccy,strike,vencimento,notional,notional_currency,premium,premium_currency,exchange_rate,exchange_rate_basis,capado,package_indicator,source_file,source_line,raw,notional_2,notional_currency_2";

fn insert_leg(conn: &Connection, p: &Perna) -> Result<()> {
    // Persiste uma perna e seu JSON bruto associado ao grupo.
    conn.execute(
        &format!("INSERT INTO pernas({COLUNAS_PERNA}) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"),
        params![
            p.id, p.grupo_id, p.execution_timestamp.to_rfc3339(), p.platform_identifier, p.par, p.familia,
            p.estilo, p.call_ccy, p.base_ccy, p.strike, p.vencimento, p.notional, p.notional_currency,
            p.premium, p.premium_currency, p.exchange_rate, p.exchange_rate_basis, p.capado as i32,
            p.package_indicator as i32, p.source_file, p.source_line, serde_json::to_string(&p.raw)?,
            p.notional_2, p.notional_currency_2
        ],
    )?;
    Ok(())
}

fn parse_timestamp(raw: &str) -> Result<DateTime<Utc>> {
    // Converte timestamps do banco para o tipo UTC do domínio.
    Ok(DateTime::parse_from_rfc3339(raw)?.with_timezone(&Utc))
}

fn leg_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Perna> {
    // Reconstrói uma Perna a partir da linha normalizada do SQLite.
    let raw: String = row.get(21)?;
    let timestamp_raw: String = row.get(2)?;
    let execution_timestamp = parse_timestamp(&timestamp_raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.to_string(),
            )),
        )
    })?;
    let raw = serde_json::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(21, Type::Text, Box::new(error))
    })?;
    Ok(Perna {
        id: row.get(0)?,
        grupo_id: row.get(1)?,
        execution_timestamp,
        platform_identifier: row.get(3)?,
        par: row.get(4)?,
        familia: row.get(5)?,
        estilo: row.get(6)?,
        call_ccy: row.get(7)?,
        base_ccy: row.get(8)?,
        strike: row.get(9)?,
        vencimento: row.get(10)?,
        notional: row.get(11)?,
        notional_currency: row.get(12)?,
        premium: row.get(13)?,
        premium_currency: row.get(14)?,
        exchange_rate: row.get(15)?,
        exchange_rate_basis: row.get(16)?,
        capado: row.get::<_, i32>(17)? != 0,
        package_indicator: row.get::<_, i32>(18)? != 0,
        source_file: row.get(19)?,
        source_line: row.get(20)?,
        raw,
        notional_2: row.get(22)?,
        notional_currency_2: row.get(23)?,
    })
}

pub fn get_group(conn: &Connection, id: &str) -> Result<Grupo> {
    // Busca o cabeçalho e todas as pernas de um grupo pelo identificador estável.
    let row = conn.query_row(
        "SELECT id,timestamp,platform_identifier,par,numeros,regra_codigo,regra_codigo_ref FROM grupos WHERE id=?",
        [id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        },
    ).optional()?.ok_or_else(|| anyhow::anyhow!("grupo não encontrado: {id}"))?;
    let (id, timestamp, platform_identifier, par, numeros, regra_codigo, regra_codigo_ref) = row;
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUNAS_PERNA} FROM pernas WHERE grupo_id=? ORDER BY id"
    ))?;
    let pernas = stmt
        .query_map([&id], leg_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Grupo {
        id,
        timestamp: parse_timestamp(&timestamp)?,
        platform_identifier,
        par,
        pernas,
        features: serde_json::from_str(&numeros)?,
        regra_codigo,
        regra_codigo_ref,
    })
}

pub fn latest_classification(conn: &Connection, group_id: &str) -> Result<Option<Value>> {
    // Lê somente a classificação mais recente de um grupo.
    let value: Option<String> = conn
        .query_row(
            "SELECT resposta FROM classificacoes WHERE grupo_id=? ORDER BY id DESC LIMIT 1",
            [group_id],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|v| serde_json::from_str(&v).context("lendo classificação gravada"))
        .transpose()
}

fn mesma_estrutura(a: &str, b: &str) -> bool {
    // Compara nomes ignorando caixa, inclusive em letras acentuadas ("OPÇÃO ÚNICA").
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Lista grupos com a última classificação da IA. `par` filtra por trecho do par;
/// `estrutura` casa com a resposta da IA ou com a regra do código.
pub fn list_groups(
    conn: &Connection,
    par: Option<&str>,
    estrutura: Option<&str>,
    limite: usize,
) -> Result<Value> {
    let mut stmt = conn.prepare("SELECT id,timestamp,platform_identifier,par,numeros,regra_codigo,regra_codigo_ref FROM grupos ORDER BY timestamp,id")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut result = Vec::new();
    for (id, timestamp, platform, pair, numeros, regra, regra_ref) in rows {
        if par.is_some_and(|filter| !pair.to_uppercase().contains(&filter.trim().to_uppercase())) {
            continue;
        }
        let classification = latest_classification(conn, &id)?;
        let estrutura_ia = classification
            .as_ref()
            .and_then(|c| c.get("estrutura"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        if let Some(filter) = estrutura
            && !mesma_estrutura(&regra, filter)
            && !estrutura_ia
                .as_deref()
                .is_some_and(|ia| mesma_estrutura(ia, filter))
        {
            continue;
        }
        let features: Features = serde_json::from_str(&numeros)?;
        result.push(json!({
            "id": id, "timestamp": timestamp, "platform_identifier": platform, "par": pair,
            "n_opt": features.n_opt, "n_hedge": features.n_hedge,
            "regra_codigo": regra, "regra_codigo_ref": regra_ref,
            "classificacao": classification,
            "concorda": estrutura_ia.map(|ia| ia == regra),
        }));
        if result.len() >= limite {
            break;
        }
    }
    Ok(Value::Array(result))
}

pub fn history(conn: &Connection, group_id: &str) -> Result<Value> {
    // Retorna todas as tentativas de classificação e seus metadados de provider.
    let mut stmt = conn.prepare("SELECT id,modelo,brain_hash,config_hash,input_hash,resposta,criado_em,provider,model_version,request_id,latency_ms,status FROM classificacoes WHERE grupo_id=? ORDER BY id")?;
    let items = stmt.query_map([group_id], |row| {
        let response: String = row.get(5)?;
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "modelo": row.get::<_, String>(1)?,
            "brain_hash": row.get::<_, String>(2)?,
            "config_hash": row.get::<_, String>(3)?,
            "input_hash": row.get::<_, String>(4)?,
            "resposta": serde_json::from_str::<Value>(&response).unwrap_or(Value::String(response)),
            "criado_em": row.get::<_, String>(6)?,
            "provider": row.get::<_, String>(7)?,
            "model_version": row.get::<_, Option<String>>(8)?,
            "request_id": row.get::<_, Option<String>>(9)?,
            "latency_ms": row.get::<_, i64>(10)?,
            "status": row.get::<_, String>(11)?
        }))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Value::Array(items))
}

pub fn store_classification(
    conn: &Connection,
    group_id: &str,
    brain_hash: &str,
    config_hash: &str,
    input_hash: &str,
    response: &Classificacao,
    metadata: &ProviderMetadata,
) -> Result<()> {
    // Persiste resposta, hashes, metadados brutos e um evento correspondente no journal.
    let response = serde_json::to_string(response)?;
    let raw_response = serde_json::to_string(&metadata.raw_response)?;
    conn.execute(
        "INSERT INTO classificacoes(grupo_id,modelo,brain_hash,config_hash,input_hash,resposta,criado_em,provider,model_version,request_id,latency_ms,raw_response,status) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            group_id,
            metadata.model,
            brain_hash,
            config_hash,
            input_hash,
            response,
            Utc::now().to_rfc3339(),
            metadata.provider,
            metadata.model_version,
            metadata.request_id,
            metadata.latency_ms as i64,
            raw_response,
            metadata.status,
        ],
    )?;
    conn.execute(
        "INSERT INTO journal(tipo,objeto_id,payload,criado_em) VALUES(?,?,?,?)",
        params![
            "classification",
            group_id,
            response,
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

/// Grupos sem classificação para este brain, provider e modelo: o que `classificar` processa.
pub fn pending_groups(
    conn: &Connection,
    brain_hash: &str,
    provider: &str,
    model: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT g.id FROM grupos g WHERE NOT EXISTS (
             SELECT 1 FROM classificacoes c
             WHERE c.grupo_id = g.id AND c.brain_hash = ? AND c.provider = ? AND c.modelo = ?)
         ORDER BY g.timestamp, g.id",
    )?;
    let ids = stmt
        .query_map(params![brain_hash, provider, model], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids)
}

pub fn all_group_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM grupos ORDER BY timestamp, id")?;
    let ids = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids)
}

/// Contagem por estrutura (IA e código) e concordância IA × regra do código.
pub fn summary(conn: &Connection) -> Result<Value> {
    let groups = list_groups(conn, None, None, usize::MAX)?;
    let items = groups.as_array().map(Vec::as_slice).unwrap_or_default();
    let mut por_ia: BTreeMap<String, usize> = BTreeMap::new();
    let mut por_codigo: BTreeMap<String, usize> = BTreeMap::new();
    let mut discordancias = Vec::new();
    let (mut classificados, mut concordam) = (0usize, 0usize);
    for item in items {
        let code = item["regra_codigo"].as_str().unwrap_or_default();
        *por_codigo.entry(code.to_string()).or_default() += 1;
        if let Some(ia) = item["classificacao"]["estrutura"].as_str() {
            classificados += 1;
            *por_ia.entry(ia.to_string()).or_default() += 1;
            if item["concorda"] == Value::Bool(true) {
                concordam += 1;
            } else {
                discordancias.push(json!({"id": item["id"], "ia": ia, "codigo": code}));
            }
        }
    }
    let (inicio, fim): (Option<String>, Option<String>) = conn.query_row(
        "SELECT MIN(timestamp), MAX(timestamp) FROM grupos",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(json!({
        "total_grupos": items.len(),
        "amostra": {"inicio": inicio, "fim": fim},
        "por_estrutura_ia": por_ia,
        "por_regra_codigo": por_codigo,
        "classificados": classificados,
        "concordam_com_regra_codigo": concordam,
        "discordancias": discordancias,
    }))
}

pub fn require_group(conn: &Connection, id: &str) -> Result<Grupo> {
    // Faz a leitura explícita de um grupo obrigatório.
    get_group(conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calculos::tests::{grupo, opcao};

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            provider: "anthropic".into(),
            model: "test".into(),
            model_version: None,
            request_id: None,
            latency_ms: 0,
            raw_response: Value::Null,
            status: "200".into(),
        }
    }

    #[test]
    fn rebuild_e_leitura_preservam_grupo_pernas_e_classificacao() {
        let path =
            std::env::temp_dir().join(format!("iatcc-db-test-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut perna = opcao("leg-1", "GBP", 1.3, 10.0, 0.0);
        perna.grupo_id = "g".into();
        let mut group = grupo(vec![perna]);
        group.regra_codigo = "Opção única".into();
        group.regra_codigo_ref = "02/Opção única".into();
        rebuild(&path, &[group.clone()], false).unwrap();
        let conn = open_existente(&path).unwrap();
        let loaded = get_group(&conn, "g").unwrap();
        assert_eq!(loaded.pernas.len(), 1);
        assert_eq!(loaded.pernas[0].notional_currency_2.as_deref(), Some("USD"));
        assert_eq!(loaded.regra_codigo_ref, "02/Opção única");
        let filtro = |e: &str| list_groups(&conn, Some("gbp"), Some(e), 10).unwrap();
        assert_eq!(filtro("OPÇÃO ÚNICA").as_array().unwrap().len(), 1);
        assert_eq!(filtro("Straddle").as_array().unwrap().len(), 0);
        assert_eq!(
            pending_groups(&conn, "b", "anthropic", "test").unwrap(),
            ["g"]
        );
        let class = Classificacao {
            estrutura: "Opção única".into(),
            alternativas: vec![],
            regra: "02/Opção única".into(),
            confianca: "alta".into(),
            justificativa: "uma opção".into(),
        };
        store_classification(&conn, "g", "b", "c", "i", &class, &metadata()).unwrap();
        assert!(
            pending_groups(&conn, "b", "anthropic", "test")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            pending_groups(&conn, "outro-brain", "anthropic", "test").unwrap(),
            ["g"]
        );
        let resumo = summary(&conn).unwrap();
        assert_eq!(resumo["concordam_com_regra_codigo"], 1);
        // Reconstruir por cima de classificações exige --forcar.
        drop(conn);
        assert!(rebuild(&path, &[group.clone()], false).is_err());
        assert_eq!(rebuild(&path, &[group], true).unwrap(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn open_existente_nao_cria_banco() {
        let path = std::env::temp_dir().join("iatcc-nao-existe.sqlite");
        let _ = std::fs::remove_file(&path);
        assert!(open_existente(&path).is_err());
        assert!(!path.exists());
    }
}
