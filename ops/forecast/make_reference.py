#!/usr/bin/env python3
"""Reference numbers for `src/lib/forecast.ts` (BI-17 part B, plan task T2).

The console fits four additive exponential-smoothing models (ANN, AAN, ANA,
AAA) in TypeScript. This script produces the numbers the unit tests compare
against, from `statsmodels` `ETSModel` (additive error, no damping), so the
TypeScript is checked against an independent implementation and not against
itself. It is run by hand; the tests read the committed JSON and need
neither Python nor `statsmodels`.

    python3 -I ops/forecast/make_reference.py <dir with numpy+statsmodels> \\
        src/lib/forecast-reference.json

Two public series: the monthly airline passengers (`AirPassengers`, 144
points, Box and Jenkins) on a log scale, and the US quarterly real GDP
(`macrodata`, first 80 quarters, bundled with `statsmodels`), which has a
trend and no season.

Choices that make the comparison fair, all also made by the TypeScript:

* initial states by `initialization_method="heuristic"` (classical
  decomposition of the first seasons, regression on the first 10 points),
  so the TypeScript does not have to reproduce `statsmodels`' estimation of
  the initial states;
* AICc counted the usual way for these models (Hyndman et al. 2008): the
  smoothing parameters, the initial states and the variance are all
  parameters. `statsmodels` leaves the heuristic initial states out of its
  own `aicc`, so it is recomputed here from the log-likelihood;
* the 95 % band is the analytical one (`pred_int` of the results), a normal
  quantile of the model's forecast variance.
"""
from __future__ import annotations

import json
import sys

M = 12


def aicc(llf: float, k: int, n: int) -> float:
    return -2 * llf + 2 * k + 2 * k * (k + 1) / (n - k - 1)


def run(y, season: int, horizon: int) -> dict:
    import numpy as np
    import pandas as pd
    from statsmodels.tsa.exponential_smoothing.ets import ETSModel

    y = pd.Series(np.asarray(y, dtype=float))
    n = len(y)
    out = {}
    for name, (trend, seasonal) in {
        "ANN": (None, None), "AAN": ("add", None), "ANA": (None, "add"), "AAA": ("add", "add"),
    }.items():
        if seasonal and n < max(2 * season, 10 + 2 * (season // 2)):
            continue
        model = ETSModel(y, error="add", trend=trend, seasonal=seasonal,
                         seasonal_periods=season if seasonal else None, initialization_method="heuristic")
        res = model.fit(disp=False, maxiter=2000)
        smoothing = 1 + (1 if trend else 0) + (1 if seasonal else 0)
        states = 1 + (1 if trend else 0) + (season if seasonal else 0)
        k = smoothing + states + 1  # + the variance
        frame = res.get_prediction(start=n, end=n + horizon - 1).summary_frame(alpha=0.05)
        out[name] = {
            "params": [float(p) for p in res.params[:smoothing]],
            "sse": float(np.sum(res.resid ** 2)),
            "aicc": aicc(float(res.llf), k, n),
            "mean": [float(v) for v in frame["mean"]],
            "lower": [float(v) for v in frame["pi_lower"]],
            "upper": [float(v) for v in frame["pi_upper"]],
        }
    best = min(out, key=lambda name: out[name]["aicc"])
    return {"season": season, "horizon": horizon, "selected": best, "models": out}


def main(argv: list[str]) -> int:
    sys.path.insert(0, argv[1])
    import numpy as np
    import statsmodels
    import statsmodels.api as sm

    air = [float(v) for v in sm.datasets.get_rdataset("AirPassengers").data["value"]]
    gdp = [float(v) for v in sm.datasets.macrodata.load_pandas().data["realgdp"][:80]]
    result = {
        "statsmodels": statsmodels.__version__,
        "numpy": np.__version__,
        "airline": {"values": air, "scale": "log", **run(np.log(np.array(air)), 12, 12)},
        "gdp": {"values": gdp, "scale": "log", **run(np.log(np.array(gdp)), 4, 8)},
    }
    with open(argv[2], "w", encoding="utf-8") as f:
        json.dump(result, f, indent=1)
        f.write("\n")
    for key in ("airline", "gdp"):
        print(key, result[key]["selected"], {k: round(v["aicc"], 3) for k, v in result[key]["models"].items()})
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
