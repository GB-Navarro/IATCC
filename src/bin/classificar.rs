/*
   Este binário classifica grupos usando o provider selecionado por
   --provider/CLASSIFIER_PROVIDER e grava a resposta auditável no SQLite.
   Com --golden, roda a IA sobre os exemplos do brain e compara com a resposta
   esperada, sem gravar nada.
*/

use IATCC::classifier::{ClassificationRequest, Classifier};
use anyhow::{Context, Result, bail};
use clap::Parser;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "classificar",
    about = "Classifica grupos usando o brain e o provider configurado"
)]
struct Args {
    /// Classifica só este grupo (mesmo que já tenha classificação).
    #[arg(long)]
    grupo: Option<String>,
    /// Reclassifica todos os grupos, e não só os pendentes.
    #[arg(long)]
    todos: bool,
    /// Máximo de grupos nesta execução (controle de custo).
    #[arg(long)]
    limite: Option<usize>,
    /// Roda a IA sobre os golden cases (exemplos do brain) e compara, sem gravar.
    #[arg(long, num_args = 0..=1, default_missing_value = "golden/casos.jsonl")]
    golden: Option<PathBuf>,
    #[arg(long, default_value = "mvp.db")]
    banco: PathBuf,
    #[arg(long, default_value = "brain")]
    brain: PathBuf,
    #[arg(long, default_value = "config/thresholds.toml")]
    config: PathBuf,
    /// anthropic (padrão), openai, jev ou devin; também via CLASSIFIER_PROVIDER.
    #[arg(long)]
    provider: Option<String>,
    #[arg(long, default_value = "config/jev_criteria.json")]
    jev_criteria: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let classifier = IATCC::ia::classifier_from_env(args.provider.as_deref(), &args.jev_criteria)?;
    eprintln!(
        "provider {} · modelo {}",
        classifier.provider(),
        classifier.model()
    );
    if let Some(path) = &args.golden {
        return golden(path, &args.brain, classifier.as_ref());
    }
    let conn = IATCC::db::open_existente(&args.banco)?;
    let ids = if let Some(id) = args.grupo.clone() {
        vec![id]
    } else if args.todos {
        IATCC::db::all_group_ids(&conn)?
    } else {
        let brain = IATCC::ia::carregar_brain(&args.brain)?;
        IATCC::db::pending_groups(
            &conn,
            &IATCC::ia::sha256(&brain),
            classifier.provider(),
            classifier.model(),
        )?
    };
    let ids: Vec<String> = ids
        .into_iter()
        .take(args.limite.unwrap_or(usize::MAX))
        .collect();
    if ids.is_empty() {
        println!("nenhum grupo pendente para este brain e modelo (use --todos para reclassificar)");
        return Ok(());
    }
    eprintln!("{} grupo(s) a classificar", ids.len());
    let (mut ok, mut falhas) = (0usize, 0usize);
    for id in ids {
        match IATCC::ia::classificar_e_gravar(
            &conn,
            &id,
            &args.brain,
            &args.config,
            classifier.as_ref(),
        ) {
            Ok(outcome) => {
                let uso = IATCC::ia::uso_tokens(&outcome)
                    .map(|u| format!(" · {u}"))
                    .unwrap_or_default();
                match outcome.classificacao {
                    Some(c) => {
                        ok += 1;
                        println!("{id}: {}{uso}", IATCC::ia::resumo_classificacao(&c));
                    }
                    None => eprintln!("{id}: a IA recusou a classificação{uso}"),
                }
            }
            Err(error) => {
                falhas += 1;
                eprintln!("{id}: {error:#}");
            }
        }
    }
    println!("{ok} classificado(s), {falhas} falha(s)");
    Ok(())
}

fn golden(path: &Path, brain_dir: &Path, classifier: &dyn Classifier) -> Result<()> {
    // Consistência, não acerto no mundo real: a IA viu esses exemplos no brain (seção 12).
    let brain = IATCC::ia::carregar_brain(brain_dir)?;
    let casos = IATCC::golden::carregar(path)?;
    let mut acertos = 0;
    for caso in &casos {
        let outcome = classifier
            .classify(&ClassificationRequest {
                brain: brain.clone(),
                grupo: caso.entrada.clone(),
            })
            .with_context(|| format!("caso {}", caso.nome))?;
        let Some(c) = outcome.classificacao else {
            println!("RECUSA  {}", caso.nome);
            continue;
        };
        let bate = c.estrutura == caso.esperado.estrutura;
        acertos += bate as usize;
        println!(
            "{}  {}: IA {} · esperado {} ({})",
            if bate { "OK    " } else { "DIFERE" },
            caso.nome,
            IATCC::ia::resumo_classificacao(&c),
            caso.esperado.estrutura,
            caso.esperado.regra
        );
    }
    println!("{acertos}/{} golden cases reproduzidos", casos.len());
    if acertos < casos.len() {
        bail!("a IA não reproduziu todos os golden cases");
    }
    Ok(())
}
