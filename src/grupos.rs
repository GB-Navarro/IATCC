/*
   Este arquivo agrupa pernas em candidatos OTC e injeta as features calculadas.
   O resultado é o Grupo persistido no SQLite e posteriormente entregue aos
   classificadores determinístico ou de IA.
*/

use crate::calculos::{Thresholds, calcula_teste_hedge, features_basicas, regra_codigo};
use crate::tipos::{Grupo, Perna};
use chrono::{DateTime, SecondsFormat, Utc};
use std::collections::{BTreeMap, BTreeSet};

pub fn id_grupo(timestamp: DateTime<Utc>, platform: &str, par: &str) -> String {
    // Gera identificador determinístico por segundo, plataforma e par.
    format!(
        "{}|{}|{}",
        timestamp.to_rfc3339_opts(SecondsFormat::Secs, true),
        platform,
        par
    )
}

pub fn formar_grupos(pernas: Vec<Perna>, thresholds: &Thresholds) -> Vec<Grupo> {
    // Deduplica por Dissemination Identifier e agrupa por segundo + plataforma + par (seção 5.3).
    // Opções entram sempre; hedges só com Package indicator; grupos sem opção são descartados.
    let mut por_chave: BTreeMap<(DateTime<Utc>, String, String), Vec<Perna>> = BTreeMap::new();
    let mut vistos = BTreeSet::new();
    for perna in pernas {
        if !vistos.insert(perna.id.clone()) {
            continue;
        }
        let entra = perna.is_option() || (perna.is_hedge() && perna.package_indicator);
        if !entra {
            continue;
        }
        let chave = (
            perna.execution_timestamp,
            perna.platform_identifier.clone(),
            perna.par.clone(),
        );
        por_chave.entry(chave).or_default().push(perna);
    }
    let mut grupos = Vec::new();
    for ((timestamp, platform, par), mut pernas) in por_chave {
        pernas.sort_by(|a, b| a.id.cmp(&b.id));
        let id = id_grupo(timestamp, &platform, &par);
        for perna in &mut pernas {
            perna.grupo_id = id.clone();
        }
        if !pernas.iter().any(Perna::is_option) {
            continue;
        }
        let mut grupo = Grupo {
            id,
            timestamp,
            platform_identifier: platform,
            par,
            features: features_basicas(&pernas),
            pernas,
            regra_codigo: String::new(),
            regra_codigo_ref: String::new(),
        };
        match calcula_teste_hedge(&grupo, thresholds) {
            Ok(teste) => grupo.features.teste_hedge = Some(teste),
            Err(motivo) => grupo.features.motivo_teste_hedge = Some(motivo),
        }
        (grupo.regra_codigo, grupo.regra_codigo_ref) = regra_codigo(&grupo);
        grupos.push(grupo);
    }
    grupos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calculos::tests::{hedge, opcao};

    #[test]
    fn agrupa_por_segundo_plataforma_e_par() {
        let mut exotica = opcao("e", "GBP", 1.4, 1.0, 0.0);
        exotica.familia = "Opção exótica".into();
        let mut sem_pacote = hedge(1.0);
        sem_pacote.id = "h2".into();
        sem_pacote.package_indicator = false;
        let mut outro_segundo = opcao("o", "GBP", 1.3, 1.0, 0.0);
        outro_segundo.execution_timestamp += chrono::Duration::seconds(1);
        let pernas = vec![
            opcao("c", "GBP", 1.3, 1.0, 0.0),
            opcao("p", "USD", 1.2, 1.0, 0.0),
            exotica,
            hedge(1.0),
            sem_pacote,
            outro_segundo,
        ];
        let grupos = formar_grupos(pernas, &Thresholds::default());
        assert_eq!(grupos.len(), 2);
        let g = &grupos[0];
        assert_eq!(g.id, "2025-10-02T17:47:10Z|TSEF|GBP-USD");
        // Exótica entra como opção; hedge sem Package indicator fica de fora.
        assert_eq!((g.features.n_opt, g.features.n_hedge), (3, 1));
        assert!(g.pernas.iter().all(|p| p.grupo_id == g.id));
        assert_eq!(grupos[1].regra_codigo, "Opção única");
    }
}
