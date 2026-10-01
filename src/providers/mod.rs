/*
   Este módulo reúne os quatro adaptadores concretos de classificação. Cada
   provider implementa o trait Classifier e converte seu protocolo externo para
   ProviderOutcome, mantendo o restante do MVP independente da API escolhida.
*/

pub mod anthropic;
pub mod devin;
pub mod jev;
pub mod openai;

#[cfg(test)]
mod tests {
    /* Os testes abaixo usam transporte fake para validar contratos sem credenciais ou rede. */
    use super::{anthropic, devin, jev, openai};
    use crate::classifier::{ClassificationRequest, Classifier};
    use crate::provider_http::{HttpResponse, HttpTransport};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    type Pedido = (String, String, Vec<(String, String)>, Option<Value>);

    #[derive(Clone, Default)]
    struct StubTransport {
        responses: Arc<Mutex<Vec<HttpResponse>>>,
        pedidos: Arc<Mutex<Vec<Pedido>>>,
    }

    impl StubTransport {
        fn new(responses: Vec<HttpResponse>) -> Arc<Self> {
            Arc::new(Self {
                responses: Arc::new(Mutex::new(responses)),
                ..Default::default()
            })
        }
    }

    impl HttpTransport for StubTransport {
        fn request_json(
            &self,
            method: &str,
            url: &str,
            headers: &[(String, String)],
            body: Option<&Value>,
        ) -> anyhow::Result<HttpResponse> {
            self.pedidos.lock().unwrap().push((
                method.into(),
                url.into(),
                headers.to_vec(),
                body.cloned(),
            ));
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(anyhow::anyhow!("stub sem resposta"));
            }
            Ok(responses.remove(0))
        }
    }

    fn request() -> ClassificationRequest {
        ClassificationRequest {
            brain: "Use as regras do brain.".into(),
            grupo: json!({"numeros":{"n_opt":2,"n_strikes":2,"n_vencimentos":1,"teste_hedge":{"res_mesma":0.01,"res_opostas":0.4,"sep":0.39}}}),
        }
    }

    fn response(value: Value) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: value,
        }
    }

    const STRANGLE: &str = r#"{"estrutura":"Strangle","alternativas":[],"regra":"03/R5","confianca":"alta","justificativa":"ok"}"#;

    #[test]
    fn anthropic_envia_brain_em_cache_schema_e_fallback() {
        let body = json!({"id":"msg_1","model":"claude-opus-5-5","stop_reason":"end_turn",
            "content":[{"type":"thinking","thinking":""},{"type":"text","text":STRANGLE}]});
        let stub = StubTransport::new(vec![response(body)]);
        let provider =
            anthropic::AnthropicClassifier::with_transport("key", "claude-opus-5-5", stub.clone());
        let result = provider.classify(&request()).unwrap();
        assert_eq!(result.classificacao.unwrap().estrutura, "Strangle");
        let pedidos = stub.pedidos.lock().unwrap();
        let (_, url, headers, body) = &pedidos[0];
        let body = body.as_ref().unwrap();
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert_eq!(body["max_tokens"], 8000);
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["fallbacks"], "default");
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "anthropic-beta" && v == "server-side-fallback-2026-07-01")
        );
        // A IA não pode ver a resposta do código (seção 8 do MVP).
        assert!(!body.to_string().contains("regra_codigo"));
    }

    #[test]
    fn anthropic_trata_recusa_e_max_tokens() {
        let recusa = json!({"id":"msg_2","stop_reason":"refusal","content":[]});
        let corte = json!({"id":"msg_3","stop_reason":"max_tokens","usage":{"output_tokens":8000},"content":[]});
        let provider = anthropic::AnthropicClassifier::with_transport(
            "key",
            "claude-haiku-4-5",
            StubTransport::new(vec![response(recusa), response(corte)]),
        );
        assert!(
            provider
                .classify(&request())
                .unwrap()
                .classificacao
                .is_none()
        );
        assert!(
            provider
                .classify(&request())
                .unwrap_err()
                .to_string()
                .contains("max_tokens")
        );
    }

    #[test]
    fn openai_parseia_resposta_structured() {
        let body = json!({"id":"resp_1","model":"test-model","output":[{"type":"message","content":[{"type":"output_text","text":STRANGLE}]}]});
        let provider = openai::OpenAiClassifier::with_transport(
            "key",
            "test-model",
            "https://example.test/v1",
            StubTransport::new(vec![response(body)]),
        );
        let result = provider.classify(&request()).unwrap();
        assert_eq!(result.classificacao.unwrap().estrutura, "Strangle");
    }

    #[test]
    fn jev_envia_brain_no_estado_e_mapeia_choice() {
        let body = json!({"id":"dec_1","model":"jev-1.13.0","answers":{"structure":{"choice":"Ambiguous","probabilities":{"Strangle":0.3,"Risk reversal":0.25,"Collar":0.05,"Ambiguous":0.4},"confidence":0.4}}});
        let profile = jev::JevProfile {
            instructions: "Escolha uma estrutura.".into(),
            criteria: [
                ("Strangle".into(), "mesma direção".into()),
                ("Risk reversal".into(), "direções opostas".into()),
                ("Collar".into(), "hedge cheio".into()),
                ("Ambiguous".into(), "incerto".into()),
            ]
            .into_iter()
            .collect(),
        };
        let stub = StubTransport::new(vec![response(body)]);
        let provider = jev::JevClassifier::with_transport(
            "key",
            "jev-1.13.0",
            "https://example.test",
            profile,
            stub.clone(),
        )
        .unwrap();
        let result = provider.classify(&request()).unwrap();
        let classificacao = result.classificacao.unwrap();
        assert_eq!(classificacao.estrutura, "Ambiguous");
        assert_eq!(classificacao.alternativas, ["Strangle", "Risk reversal"]);
        assert_eq!(classificacao.confianca, "baixa");
        let pedidos = stub.pedidos.lock().unwrap();
        assert_eq!(pedidos[0].1, "https://example.test/v1/systemone");
        assert_eq!(
            pedidos[0].3.as_ref().unwrap()["state"]["brain"],
            "Use as regras do brain."
        );
    }

    #[test]
    fn devin_cria_e_consulta_sessao() {
        let structured = json!({"estrutura":"Ambiguous","alternativas":["Strangle","Collar"],"regra":"03/R1","confianca":"baixa","justificativa":"sem hedge"});
        let provider = devin::DevinClassifier::with_transport(
            "key",
            "org",
            "https://example.test",
            StubTransport::new(vec![
                response(json!({"session_id":"session_1","status":"running"})),
                response(
                    json!({"session_id":"session_1","status":"completed","structured_output":structured}),
                ),
            ]),
            0,
            1,
        );
        let result = provider.classify(&request()).unwrap();
        assert_eq!(result.classificacao.unwrap().estrutura, "Ambiguous");
    }
}
