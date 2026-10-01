# Exemplos

1. GBP/USD, TSEF: put 1.3260 e call 1.3615, mesmo vencimento, hedge Forward de 15M GBP.
   res_mesma=0.248, res_opostas=0.000, res_cheio=0.250, sep=0.248, sep_cheio=0.250.
   → Risk reversal (03/R4), confiança alta: o hedge bate exatamente com a soma dos deltas.

2. HKD/USD, TPSE: strikes 7.72 e 7.81, hedge Forward de 6M USD.
   res_mesma=0.035, res_opostas=0.094, sep=0.128.
   → Strangle (03/R5), confiança média: res_mesma perto do limiar 0.05. Os deltas são
   assimétricos (0.18 e −0.13), por isso o strangle tem um delta líquido a cobrir.

3. MXN/USD, TPSE: strikes 18.04 e 19.243, hedge Forward de 9.16M USD.
   res_mesma=0.420, res_opostas=0.187, res_cheio=0.062, sep=0.233, sep_cheio=0.249.
   → Ambiguous: hedge não explicado (03/R8). Alternativas: strangle, risk reversal, collar.
   O hedge está perto do notional cheio, mas fora da tolerância.

4. Mesmas opções do exemplo 1 com hedge de 30M GBP:
   res_mesma=0.498, res_opostas=0.250, res_cheio=0.001, sep_cheio=0.250.
   → Collar (03/R7).

5. AUD/NZD, TSEF: duas pernas com notional capado.
   teste_hedge ausente, motivo "notional capado".
   → Ambiguous (03/R1). Alternativas: strangle, risk reversal, collar.

6. SGD/USD, BILT: put e call, strike 1.2583 nas duas, mesmo vencimento, sem hedge.
   → Straddle (02/Straddle), confiança alta.

7. JPY/USD, XXXX: 3 opções, strike 146.8 e mesmo vencimento; 2 com call USD, 1 com call JPY.
   → Strip/Strap (02/Strip-Strap), confiança alta.
