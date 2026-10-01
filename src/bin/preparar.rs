/*
   Este binário transforma a amostra congelada em grupos persistidos: carrega
   thresholds, confere os hashes do manifesto, lê ZIPs, parseia pernas, forma
   grupos e reconstrói o SQLite.
*/

use anyhow::{Context, Result, bail};
use clap::Parser;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "preparar", about = "Transforma a amostra congelada em SQLite")]
struct Args {
    // Caminhos da amostra, banco e thresholds que formam o pipeline offline.
    #[arg(long, default_value = "data/amostra")]
    amostra: PathBuf,
    #[arg(long, default_value = "mvp.db")]
    banco: PathBuf,
    #[arg(long, default_value = "config/thresholds.toml")]
    config: PathBuf,
    /// Apaga classificações já gravadas no banco, se houver.
    #[arg(long)]
    forcar: bool,
}

fn main() -> Result<()> {
    // Executa o pipeline parsear -> agrupar -> persistir.
    let args = Args::parse();
    let thresholds = IATCC::calculos::load_thresholds(&args.config)?;
    let paths = IATCC::dtcc::sample_zip_paths(&args.amostra)?;
    if paths.is_empty() {
        bail!("nenhum slice no manifesto de {}", args.amostra.display());
    }
    let mut legs = Vec::new();
    for path in paths {
        legs.extend(
            IATCC::parse::parse_zip(&path)
                .with_context(|| format!("processando {}", path.display()))?,
        );
    }
    let lidas = legs.len();
    let groups = IATCC::grupos::formar_grupos(legs, &thresholds);
    let apagadas = IATCC::db::rebuild(&args.banco, &groups, args.forcar)?;
    let com_teste = groups
        .iter()
        .filter(|g| g.features.teste_hedge.is_some())
        .count();
    let mut por_regra: BTreeMap<&str, usize> = BTreeMap::new();
    for g in &groups {
        *por_regra.entry(g.regra_codigo.as_str()).or_default() += 1;
    }
    println!(
        "{lidas} pernas NEWT lidas -> {} pernas úteis em {} grupos ({com_teste} com teste do hedge) em {}",
        groups.iter().map(|g| g.pernas.len()).sum::<usize>(),
        groups.len(),
        args.banco.display()
    );
    for (regra, n) in por_regra {
        println!("  {n:>6}  {regra}");
    }
    if apagadas > 0 {
        println!("{apagadas} classificação(ões) anteriores apagadas (--forcar)");
    }
    Ok(())
}
