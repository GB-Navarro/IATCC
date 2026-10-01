/*
   Este módulo captura e congela os slices públicos do DTCC. O manifesto guarda
   URLs, intervalos, contagem e hashes; preparar.rs usa os ZIPs registrados (e
   confere os hashes) para alimentar o parser e tornar a amostra reproduzível.
*/

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const SLICE_INDEX_URL: &str =
    "https://kgc0418-tdw-data-0.s3.amazonaws.com/dashboard/Slice.json";
pub const CUMULATIVE_INDEX_URL: &str =
    "https://kgc0418-tdw-data-0.s3.amazonaws.com/dashboard/Cumulative.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceManifest {
    pub slice_id: i64,
    pub url: String,
    pub start_ts: Option<String>,
    pub end_ts: Option<String>,
    pub row_count: Option<u64>,
    pub file: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub slices: Vec<SliceManifest>,
}

#[derive(Debug, Clone)]
struct SliceMeta {
    id: i64,
    url: String,
    file_name: Option<String>,
    start_ts: Option<String>,
    end_ts: Option<String>,
    row_count: Option<u64>,
}

fn field<'a>(value: &'a Value, names: &[&str]) -> Option<&'a Value> {
    // Procura um campo JSON aceitando os nomes alternativos do índice DTCC.
    let object = value.as_object()?;
    names.iter().find_map(|name| {
        object
            .get(*name)
            .or_else(|| object.get(&name.to_ascii_lowercase()))
    })
}

fn string_field(value: &Value, names: &[&str]) -> Option<String> {
    // Converte campo textual ou numérico em String.
    field(value, names).and_then(|v| {
        v.as_str()
            .map(ToOwned::to_owned)
            .or_else(|| Some(v.to_string()))
    })
}

fn parse_entries(root: &Value) -> Vec<SliceMeta> {
    // Transforma CFTC_FX em metadados uniformes de slices.
    let Some(raw) = field(root, &["CFTC_FX", "cftc_fx"]) else {
        return Vec::new();
    };
    let values: Vec<&Value> = match raw {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map.values().collect(),
        _ => Vec::new(),
    };
    values
        .into_iter()
        .filter_map(|entry| {
            let id = field(entry, &["sliceId", "slice_id", "id"])
                .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))?;
            let url = string_field(entry, &["fullFilePath", "full_file_path", "url"])?;
            Some(SliceMeta {
                id,
                url,
                file_name: string_field(entry, &["fileName", "file_name"]),
                start_ts: string_field(entry, &["startTs", "start_ts"]),
                end_ts: string_field(entry, &["endTs", "end_ts"]),
                row_count: field(entry, &["rowCount", "row_count"])
                    .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok())),
            })
        })
        .collect()
}

fn sha256_bytes(bytes: &[u8]) -> String {
    // Calcula hash do arquivo para preservar integridade da amostra.
    let mut hash = Sha256::new();
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

fn get_bytes(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    // Baixa os bytes de um slice remoto.
    let response = agent
        .get(url)
        .call()
        .with_context(|| format!("baixando {url}"))?;
    let mut bytes = Vec::new();
    response.into_reader().read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_manifest(path: &Path) -> Result<Manifest> {
    // Lê manifesto existente ou começa com um manifesto vazio.
    if !path.exists() {
        return Ok(Manifest::default());
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

fn write_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    // Atualiza o manifesto por rename atômico de arquivo temporário.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(manifest)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .build()
}

fn fetch_index(agent: &ureq::Agent, url: &str) -> Result<Vec<SliceMeta>> {
    let index: Value = agent
        .get(url)
        .call()
        .with_context(|| format!("baixando {url}"))?
        .into_json()?;
    let mut entries = parse_entries(&index);
    entries.sort_by_key(|entry| entry.id);
    Ok(entries)
}

fn download_into(
    agent: &ureq::Agent,
    output_dir: &Path,
    manifest_path: &Path,
    manifest: &mut Manifest,
    slice: SliceMeta,
    file: String,
) -> Result<()> {
    // Baixa pela fullFilePath do índice (montar a URL à mão dá 403) e grava o manifesto em seguida.
    let bytes = get_bytes(agent, &slice.url)?;
    std::fs::write(output_dir.join(&file), &bytes)
        .with_context(|| format!("salvando slice {}", slice.id))?;
    manifest.slices.push(SliceManifest {
        slice_id: slice.id,
        url: slice.url,
        start_ts: slice.start_ts,
        end_ts: slice.end_ts,
        row_count: slice.row_count,
        file,
        sha256: sha256_bytes(&bytes),
    });
    manifest.slices.sort_by_key(|s| s.slice_id);
    write_manifest(manifest_path, manifest)
}

fn parse_ts(value: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|v| v.with_timezone(&Utc))
}

/// Captura os slices CFTC_FX publicados durante `minutes` minutos (seção 4 do MVP).
///
/// O Slice.json lista as últimas ~24 h; por isso só entram slices com `sliceId`
/// maior que o último listado na partida, mais os que começaram nos `recentes`
/// minutos anteriores. Uma falha num slice não interrompe os demais: ele fica fora
/// do manifesto e é tentado de novo na próxima volta.
pub fn capture_for(minutes: u64, recentes: u64, output_dir: impl AsRef<Path>) -> Result<()> {
    if minutes == 0 && recentes == 0 {
        bail!("informe --minutos ou --recentes maior que zero");
    }
    let output_dir = output_dir.as_ref();
    std::fs::create_dir_all(output_dir)?;
    let manifest_path = output_dir.join("manifest.json");
    let mut manifest = read_manifest(&manifest_path)?;
    let agent = agent();
    let started = Instant::now();
    let inicial = fetch_index(&agent, SLICE_INDEX_URL)?;
    let ultimo = inicial.iter().map(|e| e.id).max().unwrap_or(0);
    // A janela de `recentes` conta a partir do slice mais novo do índice, e não do relógio
    // local, que pode estar minutos fora do horário do DTCC.
    let mais_novo = inicial
        .iter()
        .filter_map(|e| parse_ts(e.start_ts.as_deref()))
        .max();
    let recentes_ids: BTreeSet<i64> = match mais_novo {
        Some(mais_novo) if recentes > 0 => {
            let corte = mais_novo - chrono::Duration::minutes(recentes as i64);
            inicial
                .iter()
                .filter(|e| parse_ts(e.start_ts.as_deref()).is_some_and(|ts| ts > corte))
                .map(|e| e.id)
                .collect()
        }
        _ => BTreeSet::new(),
    };
    let duration = Duration::from_secs(minutes * 60);
    let mut entries = inicial;
    loop {
        let mut novos = 0;
        for slice in entries {
            let conhecido = manifest.slices.iter().any(|s| s.slice_id == slice.id);
            if conhecido || (slice.id <= ultimo && !recentes_ids.contains(&slice.id)) {
                continue;
            }
            let id = slice.id;
            let file = format!("slice-{id}.zip");
            match download_into(
                &agent,
                output_dir,
                &manifest_path,
                &mut manifest,
                slice,
                file,
            ) {
                Ok(()) => novos += 1,
                Err(error) => eprintln!("captura: slice {id}: {error:#}"),
            }
        }
        eprintln!(
            "captura: {novos} novo(s) slice(s), {} no manifesto",
            manifest.slices.len()
        );
        if started.elapsed() >= duration {
            break;
        }
        let remaining = duration.saturating_sub(started.elapsed());
        std::thread::sleep(remaining.min(Duration::from_secs(15)));
        entries = fetch_index(&agent, SLICE_INDEX_URL).unwrap_or_else(|error| {
            eprintln!("captura: {error:#}");
            Vec::new()
        });
    }
    write_manifest(&manifest_path, &manifest)
}

/// Alternativa sem esperar uma hora: o arquivo cumulativo de um dia (Cumulative.json).
/// `dia` no formato AAAA-MM-DD; o DTCC publica o dia D depois da meia-noite UTC de D+1.
pub fn capture_day(dia: &str, output_dir: impl AsRef<Path>) -> Result<()> {
    let data = chrono::NaiveDate::parse_from_str(dia, "%Y-%m-%d")
        .with_context(|| format!("--dia deve ser AAAA-MM-DD: {dia}"))?;
    let marca = data.format("%Y_%m_%d").to_string();
    let output_dir = output_dir.as_ref();
    std::fs::create_dir_all(output_dir)?;
    let manifest_path = output_dir.join("manifest.json");
    let mut manifest = read_manifest(&manifest_path)?;
    let agent = agent();
    let entries = fetch_index(&agent, CUMULATIVE_INDEX_URL)?;
    let Some(entry) = entries
        .into_iter()
        .find(|e| e.file_name.as_deref().unwrap_or(&e.url).contains(&marca))
    else {
        bail!("Cumulative.json não lista o dia {dia} para CFTC_FX");
    };
    if manifest.slices.iter().any(|s| s.slice_id == entry.id) {
        eprintln!("captura: o dia {dia} já está no manifesto");
        return Ok(());
    }
    let file = format!("cumulativo-{dia}.zip");
    download_into(
        &agent,
        output_dir,
        &manifest_path,
        &mut manifest,
        entry,
        file,
    )?;
    eprintln!("captura: dia {dia} salvo em {}", output_dir.display());
    Ok(())
}

/// Caminhos dos zips do manifesto, conferindo o SHA-256 de cada um: a amostra é fixa.
pub fn sample_zip_paths(output_dir: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let output_dir = output_dir.as_ref();
    let manifest_path = output_dir.join("manifest.json");
    if !manifest_path.exists() {
        bail!(
            "{} não existe; rode antes `cargo run --release --bin capturar`",
            manifest_path.display()
        );
    }
    let manifest = read_manifest(&manifest_path)?;
    let mut paths = Vec::new();
    for slice in &manifest.slices {
        let path = output_dir.join(&slice.file);
        let bytes = std::fs::read(&path).with_context(|| format!("lendo {}", path.display()))?;
        if sha256_bytes(&bytes) != slice.sha256 {
            bail!(
                "{} mudou desde a captura: o SHA-256 não bate com o manifesto",
                path.display()
            );
        }
        paths.push(path);
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn aceita_indice_com_array_ou_objeto() {
        let root = json!({"CFTC_FX": [{"sliceId": "7", "fullFilePath": "x.zip", "fileName": "CFTC_CUMULATIVE_FOREX_2026_09_30.zip"}]});
        let entries = parse_entries(&root);
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0]
                .file_name
                .as_deref()
                .unwrap()
                .contains("2026_09_30")
        );
        let root = json!({"CFTC_FX": {"7": {"sliceId": 7, "fullFilePath": "x.zip"}}});
        assert_eq!(parse_entries(&root)[0].id, 7);
    }

    #[test]
    fn amostra_alterada_e_detectada() {
        let dir = std::env::temp_dir().join(format!("iatcc-amostra-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.zip"), b"conteudo").unwrap();
        let manifest = Manifest {
            slices: vec![SliceManifest {
                slice_id: 1,
                url: "x".into(),
                start_ts: None,
                end_ts: None,
                row_count: None,
                file: "a.zip".into(),
                sha256: sha256_bytes(b"conteudo"),
            }],
        };
        write_manifest(&dir.join("manifest.json"), &manifest).unwrap();
        assert_eq!(sample_zip_paths(&dir).unwrap().len(), 1);
        std::fs::write(dir.join("a.zip"), b"outro").unwrap();
        assert!(sample_zip_paths(&dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
