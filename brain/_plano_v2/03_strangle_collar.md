# Strangle e Collar pelo teste do NDF

Para duas opções com strikes distintos e mesmo vencimento, o código fornece:

- `res_strangle`: erro da hipótese de mesmo lado, com previsão `|Δc + Δp|`;
- `res_collar`: erro da hipótese de lados opostos, com previsão `Δc + |Δp|`;
- `separacao`: distância entre as duas previsões;
- `tau_res` e `tau_sep`: limiares numéricos da configuração.

Aplique as regras nesta ordem:

- R1: sem `teste_hedge` → `Ambiguous`, usando o motivo recebido.
- R2: `separacao < tau_sep` → `Ambiguous`, porque os deltas não distinguem as hipóteses.
- R3: `res_strangle <= tau_res` e `res_collar > tau_res` → `Strangle`.
- R4: `res_collar <= tau_res` e `res_strangle > tau_res` → `Collar`.
- R5: ambos os resíduos dentro do limiar → `Ambiguous`.
- R6: nenhum resíduo dentro do limiar → `Ambiguous`; o hedge não foi explicado.

A ausência de NDF não prova strangle. Hedge parcial, hedge fora do tape ou
associação incorreta produzem a mesma ausência. Collar aqui significa apenas a
geometria de lados opostos; a intenção de hedge não é observável.
