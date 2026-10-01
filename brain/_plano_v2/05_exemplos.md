# Golden cases

1. Duas opções com strike 1.2583, mesmo vencimento e sem hedge: `Straddle`, regra
   `02/Straddle`.
2. Duas opções com strikes diferentes e `res_strangle=0.010`,
   `res_collar=0.400`, `separacao=0.390`, `tau_res=0.05`: `Strangle`, regra `03/R3`.
3. Duas opções com strikes diferentes e `res_strangle=0.400`,
   `res_collar=0.010`, `separacao=0.390`, `tau_res=0.05`: `Collar`, regra `03/R4`.
4. Duas opções com strikes diferentes, mas sem hedge observado: `Ambiguous`,
   alternativas `Strangle` e `Collar`, regra `03/R1`.
