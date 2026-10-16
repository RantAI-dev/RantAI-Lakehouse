# `SRC-7` result: how long a failure alert takes to arrive

Measured on pull request #124, commit `2e59283`, CI run 38358715137, job
"Rust · SRC-17 alert delivery", on 2026-10-13. One run, on a GitHub-hosted
`ubuntu-latest` runner. The step is `ops/g6/src17_alert_delivery_test.py`.

The stack is the product's own compose services (API, orchestrator, a MySQL
source, ClickHouse). The receiver is a listener the gate starts on its own
network. The time is from the end of the run, as the orchestrator records
it, to the arrival of the webhook at the receiver.

| Event | Channel | Seconds from the run's end | Target (feature-page decision 1) |
| --- | --- | --- | --- |
| A connector run fails (wrong password on the MySQL source) | Webhook | 6.0 | 300 |
| An upload fails to load (a file with a header and no row) | Webhook | 18.8 | 300 |
| Either of them | E-mail | *not measured* | 300 |

Also proven by the same step:

- The webhook's body names the connector (or the file and the upload) and
  the run, and carries no text from the source or its driver.
- A webhook target that answers 500 leaves an alert listed as not
  delivered, with the reason "the webhook server answered with an error" and
  none of the target's own answer. A target that answers 200 for the same
  run is listed as delivered.

The gate fails when an arrival takes more than 300 seconds, so the target is
checked on every pull request that touches the API, the orchestrator, the
gates or compose.

## What the time is made of

The orchestrator's run sensors look for finished runs about every 30
seconds and report each to the API, which delivers at once. The two
measurements are inside one sensor interval. A report that fails is tried
three times, three seconds apart.

## Not measured

- **E-mail.** The mail sender connects only with verified TLS, so it cannot
  reach a plain SMTP receiver in the gate, and no switch to turn the
  verification off was added for a test. E-mail goes out from the same
  place in the code, at the same moment, as the webhook; how long the mail
  server then takes is outside the product.
- More than one run. One measurement of each event, not a distribution.
- A loaded installation: many connectors finishing in the same interval,
  or many rules matching one event.
- The alert kinds "3 failures in a row", success and schema change: they
  are delivered by the same call; only their route tests cover them.
- An alert-rule store that is unreachable for longer than the three
  retries: the run is counted and its alert is not sent.
