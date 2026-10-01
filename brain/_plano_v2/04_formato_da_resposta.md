# Formato da resposta

Responda somente JSON com estes campos:

```json
{
  "estrutura": "Straddle | Strangle | Collar | Ambiguous",
  "probabilidades": {
    "Straddle": 0.0,
    "Strangle": 0.0,
    "Collar": 0.0,
    "Ambiguous": 0.0
  },
  "alternativas": [],
  "regra": "03/R3",
  "confianca": "alta | media | baixa",
  "justificativa": "Duas ou três frases em português."
}
```

`alternativas` deve listar somente classes compatíveis quando a estrutura for
`Ambiguous`. Não repita a classe escolhida como alternativa.
