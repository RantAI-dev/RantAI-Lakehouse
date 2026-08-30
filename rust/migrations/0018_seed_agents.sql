-- Phase 2, Task 2.9: demo/seed digital employees, tools, workflows, runs,
-- and approvals — taken from `src/services/mock/agents.ts`'s fixtures.
-- Fixed ids + `ON CONFLICT DO NOTHING` make this idempotent.
INSERT INTO agent_employee (id, name, purpose, owner, autonomy, status, budget_limit, budget_spent, budget_reserved, allowed_tools, data_scope, approval_rate, success_rate, recent_runs, created_at)
VALUES
    ('emp-inventory', 'inventory-copilot', 'Prioritize dunning actions and draft customer outreach.', 'Collections Ops', 'L3', 'ready', 1000, 820, 40, ARRAY['query_sql','retrieve','lineage','whatif_branch'], 'finance + collections (confidential)', 0.72, 0.94, 128, now()),
    ('emp-risk', 'ops-sentinel', 'Triage fraud anomalies and explain signal provenance.', 'Risk', 'L2', 'running', 500, 210, 15, ARRAY['retrieve','query_sql','freshness'], 'risk signals + knowledge', 1, 0.97, 64, now())
ON CONFLICT DO NOTHING;

INSERT INTO agent_workflow (id, name, status, owner, trigger, steps, last_run_at, approval_required, created_at)
VALUES
    ('wf-dunning', 'Dunning priority workflow', 'ready', 'Collections Ops', 'Streaming event: sla_breach_score', 6, now() - interval '8 minutes', true, now()),
    ('wf-lag-triage', 'Streaming lag triage', 'ready', 'Data Platform', 'Alert: streaming lag', 4, now() - interval '200 minutes', false, now())
ON CONFLICT DO NOTHING;

INSERT INTO agent_tool (id, name, version, publisher, permission, health, approval_status, deprecated, rate_limit, usage_30d, created_at)
VALUES
    ('tool-query-sql', 'query_sql', '1.4.0', 'Rantai Lake', 'query:read', 'healthy', 'approved', false, '60/min', 18420, now()),
    ('tool-retrieve', 'retrieve', '1.2.1', 'Rantai Lake', 'knowledge:read', 'healthy', 'approved', false, '120/min', 9021, now()),
    ('tool-whatif', 'whatif_branch', '0.9.0', 'Rantai Lake', 'catalog:branch', 'degraded', 'pending', false, '10/min', 312, now())
ON CONFLICT DO NOTHING;

INSERT INTO agent_run (id, employee_id, workflow_id, status, trigger, actor, delegated_user, started_at, ended_at, budget_consumed, steps, audit_event_id, created_at)
VALUES
    ('run-col-01', 'emp-inventory', 'wf-dunning', 'running', 'Streaming threshold: sla_breach_score > 0.8', 'inventory-copilot', 'Rina Wijaya', now() - interval '8 minutes', NULL, 12.4,
     '[{"id":"s1","label":"Retrieve policy context","status":"completed","detail":"2 chunks from logistics-sop-2026"},{"id":"s2","label":"Query overdue accounts","status":"completed","detail":"hot-analytics · 0.02 cu"},{"id":"s3","label":"Propose priority updates","status":"running","detail":"Awaiting approval gate"}]'::jsonb,
     'aud-agent-run-col-01', now()),
    ('run-risk-01', 'emp-risk', NULL, 'completed', 'Manual investigation', 'ops-sentinel', 'Dewi Anggraini', now() - interval '120 minutes', now() - interval '110 minutes', 4.1,
     '[{"id":"s1","label":"Retrieve similar cases","status":"completed","detail":"hybrid search"},{"id":"s2","label":"Explain features","status":"completed","detail":"L2 propose only"}]'::jsonb,
     'aud-agent-run-risk-01', now())
ON CONFLICT DO NOTHING;

INSERT INTO approval_item (id, employee_id, employee_name, run_id, workflow_id, action, resource, reason, impact, evidence, policy, cost_estimate, expires_at, requested_at, status, risk, decided_at, comment, audit_event_id, created_at)
VALUES
    ('ap-01', 'emp-inventory', 'inventory-copilot', 'run-col-01', 'wf-dunning', 'Update dunning_priority proposals (24 accounts)', 'core.collections.dunning_priority', 'Delinquency score crossed approval threshold', 'Writes 24 proposal rows to a non-critical branch; no customer notifications yet.', ARRAY['Query qh-1 scanned orders_enriched (cache hit)','Retrieved 2 chunks from logistics-sop-2026 v12'], 'pol-1 · Tenant row isolation', 0.04, now() + interval '120 minutes', now() - interval '6 minutes', 'pending', 'Writes to non-critical proposal branch', NULL, NULL, 'aud-approval-ap-01', now()),
    ('ap-02', 'emp-inventory', 'inventory-copilot', 'run-col-01', 'wf-dunning', 'Notify customers in segment B', 'notification.email', 'Playbook step after priority update', 'External email to ~180 customers in segment B.', ARRAY['Policy section 4.2 escalation thresholds'], 'External notification gate', 0.12, NULL, now() - interval '80 minutes', 'approved', 'External notification', now() - interval '70 minutes', 'Approved for segment B only.', 'aud-approval-ap-02', now()),
    ('ap-03', 'emp-risk', 'ops-sentinel', NULL, NULL, 'Escalate fraud case FR-2291', 'risk.cases', 'Similarity search matched prior confirmed fraud', 'Creates investigator task; no automated account freeze.', ARRAY['Hybrid search hit score 0.91 on supplier-policy corpus'], 'L2 propose-only autonomy', 0.02, NULL, now() - interval '200 minutes', 'pending', 'Creates investigator workload', NULL, NULL, 'aud-approval-ap-03', now())
ON CONFLICT DO NOTHING;
