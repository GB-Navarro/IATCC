# Como separar strangle, risk reversal e collar

Você recebe `teste_hedge` com resíduos já calculados:
- res_mesma: quanto o hedge erra a previsão de strangle (0 = bate perfeito)
- res_opostas: quanto erra a previsão de risk reversal
- res_cheio: quanto erra a previsão de collar
- sep, sep_cheio: se as previsões estão longe o bastante umas das outras
- tau_res, tau_sep: os limiares

Aplique na ordem; vale a primeira regra verdadeira:
R1. teste_hedge ausente → Ambiguous (diga o motivo recebido).
R2. sep < tau_sep → Ambiguous: deltas pequenos, as hipóteses preveem quase o mesmo hedge.
R3. res_opostas ≤ tau_res e res_cheio ≤ tau_res e sep_cheio < tau_sep
    → Ambiguous entre risk reversal e collar.
R4. res_opostas ≤ tau_res e res_mesma > tau_res → Risk reversal.
R5. res_mesma ≤ tau_res e res_opostas > tau_res → Strangle.
R6. res_mesma ≤ tau_res e res_opostas ≤ tau_res → Ambiguous: deltas pequenos.
R7. res_cheio ≤ tau_res e sep_cheio ≥ tau_sep → Collar.
R8. nenhuma das anteriores → Ambiguous: hedge não explicado.

Nunca conclua "strangle" só porque o hedge não bate com risk reversal: um hedge parcial,
em spot ou de outra operação produz o mesmo efeito. Cada classe só vale quando o hedge
bate com a previsão dela.

Para um straddle (mesmo strike), dê o nome pela forma. O teste do hedge ainda não é
confiável para straddles; mencione o resultado só como informação.
