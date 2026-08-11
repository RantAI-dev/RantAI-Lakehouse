# RantAI Lake — New Frontend Repository Implementation Plan

## 1. Purpose

Build a new frontend repository for **RantAI Lake** that:

1. preserves the established RantAI design system, visual language, component behavior, spacing, typography, and dark interface style from the previous repository;
2. keeps the same frontend technology stack;
3. captures the complete RantAI Lake product scope, not only the existing prototype pages;
4. presents complex data-platform functions using intuitive, consistent, and familiar enterprise interaction patterns;
5. is structured as a contract-first frontend that can run against mock services now and real backend APIs later without redesigning the pages;
6. clearly separates product UI, domain state, API contracts, mock adapters, and reusable design-system components;
7. avoids treating simulated frontend interactions as completed backend capabilities.

The new repository is a fresh implementation. The previous repository may be used only as a visual, interaction, and feature reference. Do not copy large page files directly without decomposing them into reusable modules.

---

## 2. Product Positioning

RantAI Lake is an enterprise lakehouse console for managing and operating:

- analytical data across hot, warm, cold, and AI storage tiers;
- batch, incremental, streaming, document, and vector pipelines;
- SQL, natural-language, federated, real-time, and retrieval workloads;
- data catalog, schema, freshness, ownership, lineage, and quality;
- access policies, masking, row filters, retention, residency, and audit;
- users, tenants, quotas, service identities, and delegated agent access;
- AI knowledge, embeddings, hybrid search, agent tools, approval workflows, and digital employees;
- platform health, query performance, ingestion lag, storage usage, and service status.

The console must expose these capabilities as one coherent product rather than a collection of unrelated admin pages.

---

## 3. Non-Negotiable Constraints

### 3.1 Frontend stack

Keep the same frontend stack:

| Category | Required choice |
|---|---|
| Framework | Next.js 16, App Router, React 19 |
| Language | TypeScript 5 with strict mode |
| Styling | Tailwind CSS v4 and existing RantAI design tokens |
| Component primitives | shadcn/ui and `@base-ui/react` |
| Icons | `lucide-react` |
| Theme | `next-themes`, dark interface as the product default |
| SQL editor | `@uiw/react-codemirror`, SQL language extension, dark editor theme |
| Utilities | `clsx`, `tailwind-merge`, shared `cn()` helper |
| Variants | `class-variance-authority` |
| Fonts | Geist and Geist Mono from the existing design system |
| Routing | Next.js file-based App Router |
| State | Local `useState` / `useReducer`, URL state, and limited feature context only when genuinely cross-page |
| Lint | ESLint 9 and `eslint-config-next` |

Do not introduce a new UI framework, global state library, charting library, data-grid product, or CSS-in-JS solution unless explicitly approved.

### 3.2 Visual consistency

Use the existing RantAI design system as the single visual source of truth:

- reuse the existing color tokens, typography, radii, shadows, spacing, elevation, and interaction states;
- reuse or rebuild components from the existing design-system package rather than creating page-specific substitutes;
- use one consistent page header, toolbar, filter bar, table, status badge, empty state, loading state, error state, detail drawer, confirmation dialog, and form pattern;
- use Geist for product UI and Geist Mono for code, identifiers, query text, schema names, and technical metrics;
- maintain high information density without reducing readability;
- prefer progressive disclosure: overview first, detailed configuration in tabs, drawers, sheets, or step-based flows;
- do not expose raw infrastructure complexity unless it helps users make a decision.

### 3.3 Repository content rules

The repository and its AI context must use only the RantAI Lake product identity and terminology.

- Use product-neutral names for internal gateway, routing, policy, and orchestration concepts.
- Do not leave old asset names, storage keys, import aliases, CSS variables, comments, screenshots, placeholder content, or metadata from any earlier product identity.
- Do not describe removed branding or historical cleanup in README files, source comments, fixtures, route labels, or AI context.

---

## 4. Design and Interaction Principles

### 4.1 Common page anatomy

Every primary page should follow the same structure:

```text
Page title + concise purpose
Primary action + contextual secondary actions
Health or summary strip when relevant
Search / filter / saved-view toolbar
Main content: table, cards, graph, editor, or split panel
Detail drawer or detail route
Empty, loading, error, permission, and no-result states
```

### 4.2 Common CRUD behavior

Use the same interaction model across modules:

- **Create**: dedicated page or step dialog for complex entities; modal only for small forms.
- **View**: list to detail route for durable entities; drawer for quick inspection.
- **Edit**: explicit edit mode with dirty-state warning and save/cancel actions.
- **Delete**: confirmation dialog showing impact and dependent objects.
- **Enable/disable**: immediate toggle only for reversible, low-risk actions; otherwise require confirmation.
- **Run/execute**: pre-run validation, visible progress, cancellable execution, result summary, and run history.
- **Retry**: preserve previous configuration and show the reason for failure.
- **Bulk action**: visible selection count, scoped action bar, and final confirmation.

### 4.3 Status language

Use a shared status taxonomy wherever possible:

- Draft
- Validating
- Ready
- Scheduled
- Running
- Paused
- Degraded
- Failed
- Completed
- Cancelled
- Archived

Each status must have:

- a shared badge style;
- a human-readable description;
- clear allowed actions;
- consistent filtering behavior.

### 4.4 Flow visualization

For pipelines, lineage, query plans, routing, and agent workflows:

- use left-to-right flow for short operational flows;
- use top-to-bottom flow for long lifecycle flows;
- support zoom, pan, fit-to-view, selection, and node detail;
- use consistent node categories and connection semantics;
- show current state, warnings, and failed nodes without relying only on color;
- provide a table or list alternative for accessibility and high-density inspection.

---

## 5. Target Information Architecture

## 5.1 Navigation groups

### Overview

- Overview
- Activity
- Alerts

### Data

- Data Explorer
- Catalog
- Storage Lifecycle
- Connectors

### Build

- Pipelines
- Streaming Jobs
- Vector & Knowledge Jobs
- Query Studio
- Saved Queries
- Collaboration

### Intelligence

- Knowledge
- Semantic Search
- Agent Workflows
- Digital Employees
- Tool Registry

### Governance

- Policies
- Classification & Masking
- Data Quality
- Lineage
- Audit
- Residency

### Operations

- Workloads
- Observability
- Services
- Usage & Budgets

### Administration

- Users
- Teams & Roles
- Tenants
- Service Identities
- Settings

---

## 5.2 Proposed route map

```text
/
/activity
/alerts

/data
/data/assets/[assetId]
/catalog
/catalog/namespaces/[namespaceId]
/storage
/storage/policies/[policyId]
/connectors
/connectors/new
/connectors/[connectorId]

/pipelines
/pipelines/new
/pipelines/[pipelineId]
/pipelines/[pipelineId]/runs/[runId]
/streaming
/streaming/new
/streaming/[jobId]
/vector-jobs
/vector-jobs/new
/vector-jobs/[jobId]
/query-studio
/query-studio/saved
/query-studio/history/[queryId]
/query-studio/collaboration
/query-studio/collaboration/[projectId]

/knowledge
/knowledge/sources/[sourceId]
/semantic-search
/agents/workflows
/agents/workflows/new
/agents/workflows/[workflowId]
/agents/employees
/agents/employees/[employeeId]
/agents/tools
/agents/tools/[toolId]

/governance/policies
/governance/classification
/governance/data-quality
/lineage
/audit
/residency

/workloads
/observability
/observability/explorer
/services
/usage

/admin/users
/admin/roles
/admin/tenants
/admin/service-identities
/settings
```

Redirect routes may be provided for compatibility, but the new route structure is the canonical product architecture.

---

## 6. Complete Feature Scope

## 6.1 Overview and activity

### Overview dashboard

Provide one executive-operational overview containing:

- total catalog assets by type and environment;
- data freshness and stale-asset count;
- active, failed, and delayed pipelines;
- streaming lag and unhealthy jobs;
- query volume, latency, failure rate, cache assistance, and scanned data;
- storage distribution across hot, warm, cold, and AI tiers;
- policy violations and pending approvals;
- active agent runs and budget utilization;
- service-health summary;
- recent incidents and important activity.

Users must be able to filter by:

- tenant;
- environment;
- time range;
- domain;
- data classification.

### Activity feed

Unify recent actions from:

- pipeline runs;
- query execution;
- schema changes;
- policy changes;
- connector events;
- agent actions;
- approvals;
- incident events.

Each activity item should link to its source object and associated audit detail.

### Alerts

Support:

- severity;
- source service;
- affected object;
- status;
- assignee;
- acknowledgement;
- resolution note;
- deep link to the related metric, run, query, or policy event.

---

## 6.2 Data Explorer and asset detail

### Data Explorer

Replace the previous zone-only dashboard with a complete data exploration experience while retaining logical layer filters.

Capabilities:

- browse assets by namespace, database, schema, owner, domain, layer, storage tier, classification, and freshness;
- search tables, columns, views, streams, vector datasets, materialized views, and knowledge sources;
- saved filters and shareable URLs;
- list, grid, and compact views;
- bulk tag, owner, classification, and retention actions when permitted.

### Asset detail

Each asset detail must include:

- overview and business description;
- technical identifiers;
- schema and column metadata;
- sample data with masked values where required;
- freshness and last update;
- storage tier and lifecycle policy;
- upstream and downstream lineage;
- data quality checks;
- access policy summary;
- usage statistics;
- recent queries;
- dependent pipelines and agents;
- change history and audit events.

Asset types must include:

- ClickHouse table or view;
- Iceberg table;
- streaming materialized view;
- vector or multimodal dataset;
- external/federated source;
- knowledge source.

---

## 6.3 Catalog

Provide:

- namespace hierarchy;
- table and view registry;
- schema versions;
- snapshots;
- ownership and stewardship;
- tags and business glossary links;
- classifications;
- lifecycle tier;
- freshness watermark;
- location and residency metadata;
- source engine;
- supported operations;
- change history.

Required flows:

- create namespace;
- register an external asset;
- inspect schema evolution;
- compare schema versions;
- propose a schema change;
- review impact before applying a schema change.

---

## 6.4 Storage lifecycle

Create a dedicated Storage Lifecycle module.

Capabilities:

- overview of hot, warm, cold, and AI storage usage;
- lifecycle policies by asset or asset group;
- retention and tier-movement rules;
- export status and snapshot history;
- compaction status;
- estimated storage savings;
- failed tiering operations;
- restore or rehydrate requests;
- time-travel and snapshot selection where supported;
- lifecycle impact preview before policy activation.

Use a timeline or tier-lane visualization for each asset:

```text
Hot -> Warm -> Cold
             \-> AI derivative dataset
```

---

## 6.5 Connectors

### Connector catalog

Support source categories:

- databases and CDC;
- streaming and messaging;
- object storage and files;
- SaaS APIs;
- observability sources;
- federated query sources.

Each connector card/list row should show:

- type;
- direction: source, sink, or bidirectional;
- status;
- environment;
- tenant;
- last successful test;
- last ingestion/query activity;
- supported capabilities;
- owner.

### Connector create/edit flow

Use a consistent wizard:

1. Select connector type.
2. Configure connection.
3. Select or create a secret reference.
4. Test network and authentication.
5. Discover schemas or topics.
6. Set capability and access scope.
7. Configure residency and classification.
8. Review and activate.

Do not place secrets directly in client fixtures or persistent browser storage.

### Connector detail

Include:

- health;
- configuration summary;
- capability matrix;
- discovered assets;
- ingestion checkpoints;
- query pushdown support;
- recent errors;
- dependent pipelines;
- audit history.

---

## 6.6 Pipelines

### Pipeline list

Provide separate but consistent views for:

- batch and incremental pipelines;
- document and knowledge pipelines;
- vector and embedding jobs.

Common list capabilities:

- search;
- filter by type, status, schedule, source, target, owner, and freshness;
- saved views;
- bulk pause/resume/archive;
- health summary;
- last run and next run;
- SLA indicator;
- row and card views.

### Pipeline builder

Use a step-based builder with a visual graph preview:

1. Basic information.
2. Source selection.
3. Schema discovery and sample.
4. Transformation design.
5. Target selection.
6. Schedule or event trigger.
7. Data quality and failure handling.
8. Governance, lineage, and ownership.
9. Validation.
10. Review and deploy.

Transformation types:

- select, filter, rename, cast;
- clean and standardize;
- join and enrich;
- aggregate;
- deduplicate;
- parse documents;
- split/chunk content;
- extract structured fields;
- generate embeddings;
- custom SQL;
- custom governed function.

Failure handling:

- retry policy;
- dead-letter target;
- partial-failure behavior;
- checkpoint and resume;
- timeout;
- alert routing.

### Natural-language pipeline assistant

The assistant must:

- turn user intent into a draft, not deploy automatically;
- show assumptions;
- ask for unresolved source, target, schedule, key, and policy information;
- produce a readable graph and configuration diff;
- run validation;
- require explicit user review before saving or deploying.

### Pipeline detail

Tabs:

- Overview
- Graph
- Runs
- Configuration
- Schema
- Lineage
- Quality
- Logs
- Audit

### Run detail

Show:

- status timeline;
- stage or node status;
- processed, accepted, rejected, and retried records;
- source checkpoint;
- target watermark;
- duration;
- resource and cost metrics;
- errors and sample failed records;
- retry, resume, cancel, and rerun actions;
- resulting lineage and output assets.

---

## 6.7 Streaming jobs

Create a distinct real-time module rather than hiding streaming inside generic pipelines.

Capabilities:

- streaming SQL editor;
- source topics and CDC streams;
- materialized-view definition;
- streaming join;
- window aggregation;
- primary-key and upsert configuration;
- watermark and barrier interval;
- sink configuration;
- live lag, throughput, and state-size metrics;
- pause, resume, restart, and backfill;
- event-trigger outputs for agent workflows;
- change-log preview.

### Streaming job detail

Tabs:

- Overview
- SQL / Definition
- Sources & Sinks
- Materialized Views
- Live Metrics
- State & Checkpoints
- Triggers
- Logs
- Audit

---

## 6.8 Query Studio

The Query Studio remains a core product experience but must be rebuilt into smaller reusable modules.

### Workspace layout

Use a configurable split workspace:

- left: catalog, saved queries, history, and knowledge;
- center: natural-language conversation or SQL editor;
- bottom or right: results, query plan, metrics, and explanation.

### Natural-language mode

Capabilities:

- ask a data question;
- select agent/model policy where permitted;
- mention tables, columns, metrics, and knowledge sources;
- display assumptions and ambiguity warnings;
- generate editable SQL;
- show a plain-language explanation;
- show source assets and freshness;
- require confirmation for high-cost queries;
- preserve the conversation and generated artifacts.

### SQL mode

Capabilities:

- syntax highlighting and autocomplete;
- schema browser;
- multiple query tabs;
- execute selected statement or entire editor;
- cancel running query;
- parameter support;
- format SQL;
- explain plan;
- pre-run cost and policy check;
- result pagination or streaming;
- export permitted results;
- save and share query;
- query history and rerun.

### Query execution transparency

Show:

- selected workload class;
- execution engine category;
- source systems;
- pushdown operations;
- data freshness;
- policy obligations;
- scanned data;
- execution time;
- cache assistance;
- estimated and actual cost;
- warnings and failures.

Do not expose infrastructure labels as unexplained jargon. Provide tooltips and a simplified default view with an advanced detail panel.

### Federated query support

The UI must support queries across:

- hot analytical tables;
- open cold tables;
- streaming views;
- vector datasets;
- external data sources;
- observability data.

The query plan view must clearly show which parts execute at each source and which operations execute in the federated compute layer.

### Collaboration

Retain and improve:

- projects;
- members;
- roles;
- shared queries;
- comments;
- activity;
- version history;
- environment and tenant scope.

---

## 6.9 Knowledge and vector processing

### Knowledge sources

Support:

- uploaded files;
- object-storage folders;
- websites or web context;
- database tables;
- query results;
- code repositories where allowed;
- manual context.

Each source must show:

- sync status;
- version;
- last refresh;
- parsing status;
- chunk count;
- embedding model and version;
- index status;
- freshness;
- classification;
- owner;
- dependent agents.

### Vector jobs

Provide:

- source selection;
- content field selection;
- parsing and chunking configuration;
- metadata fields;
- embedding model;
- index type and configuration;
- lexical index option;
- refresh behavior;
- validation sample;
- run history;
- re-index and re-embed actions.

### Semantic search

Support:

- semantic search;
- lexical search;
- hybrid search;
- similarity exploration;
- metadata filters;
- result explanations;
- source citations;
- freshness and version information;
- retrieval audit;
- side-by-side comparison of search strategies.

---

## 6.10 Agent operations

### Agent workflow builder

Provide a visual workflow builder with:

- trigger;
- condition;
- model step;
- data query;
- retrieval;
- tool action;
- governed function;
- human approval;
- escalation;
- notification;
- output sink;
- failure and rollback path.

### Digital employees

List and detail should include:

- purpose;
- owner;
- autonomy level;
- allowed tools;
- allowed data scope;
- budget;
- schedule and event triggers;
- current status;
- recent runs;
- approval rate;
- success and failure rate;
- policy violations;
- suspension and revoke actions.

### Agent run detail

Show:

- trigger event;
- actor and delegated user context;
- step-by-step timeline;
- tool calls;
- queries and retrievals;
- inputs and outputs;
- consumed budget;
- approval checkpoints;
- policy decisions;
- failures and recovery;
- final outcome;
- audit and lineage correlation.

### Tool registry

Provide:

- tool inventory;
- version;
- publisher;
- input/output schema;
- required permissions;
- supported environments;
- health;
- rate limit;
- usage statistics;
- approval status;
- version pinning and deprecation status.

---

## 6.11 Governance

### Policy management

Support policies for:

- tenant and role access;
- attribute-based access;
- row filters;
- column masks;
- action permissions;
- agent tool scope;
- autonomy level;
- data residency;
- retention;
- budget and quota.

Policy flow:

1. Create draft.
2. Select subjects, resources, actions, and conditions.
3. Preview impacted users and assets.
4. Test with sample access requests.
5. Review policy conflicts.
6. Submit for approval when required.
7. Activate and version.
8. Monitor decisions and exceptions.

### Classification and masking

Provide:

- classification taxonomy;
- automatic and manual classification result;
- confidence and review status;
- column-level masking rules;
- preview by user role;
- bulk review;
- policy inheritance.

### Data quality

Provide:

- quality rules;
- dimensions: completeness, validity, uniqueness, freshness, consistency, accuracy;
- thresholds and severity;
- schedule or event trigger;
- result history;
- failed-record sample;
- downstream impact;
- alert and remediation workflow.

### Residency

Provide:

- approved sites and regions;
- classification-to-location rules;
- affected assets;
- policy simulation;
- blocked query events;
- permitted aggregation or boundary-crossing rules;
- violation history.

---

## 6.12 Lineage and impact analysis

Capabilities:

- dataset, column, pipeline, query, and agent-action lineage;
- upstream and downstream navigation;
- time-aware lineage version;
- column mapping;
- transformation detail;
- impact analysis before schema or policy changes;
- filter by environment, tenant, object type, and time;
- graph and table views;
- link to run, query, audit, and asset detail.

---

## 6.13 Audit

Audit list and detail must cover:

- actor;
- delegated actor context;
- tenant;
- action;
- resource;
- outcome;
- policy decision;
- masks and filters applied;
- execution category;
- source locations;
- estimated and actual cost;
- approval identity;
- timestamp;
- error;
- related query, run, workflow, and lineage event.

Support:

- saved filters;
- export where permitted;
- immutable-event detail;
- correlation across related events;
- clear distinction between user action, service action, and agent action.

---

## 6.14 Workload management and usage

### Workloads

Provide:

- active and queued requests;
- workload class;
- tenant and principal;
- elapsed time;
- estimated cost;
- engine category;
- queue reason;
- cancellation;
- historical workload patterns;
- quota and fairness indicators.

### Usage and budgets

Provide:

- tenant compute use;
- query credits or internal cost units;
- scanned and returned data;
- storage usage by tier;
- pipeline usage;
- agent token and compute budgets;
- budget alerts;
- reservation and settlement history;
- forecast and anomaly indicators.

---

## 6.15 Observability and service health

### Observability overview

Provide:

- metrics, logs, and traces summary;
- platform SLOs;
- query latency and errors;
- ingestion freshness and lag;
- pipeline failures;
- streaming lag;
- storage and compaction health;
- cache behavior;
- policy decision latency;
- agent performance;
- active incidents.

### Explorer

Support:

- metric query;
- log search;
- trace search;
- time range;
- service and tenant filters;
- correlation to query, pipeline, or agent run;
- saved explorations.

### Services

Show each platform service with:

- status;
- version;
- environment/site;
- replicas;
- resource use;
- error rate;
- latency;
- dependencies;
- recent deployment or configuration changes;
- incident history.

---

## 6.16 Identity and administration

### Users

- user list and detail;
- active/inactive status;
- groups and roles;
- tenant membership;
- last activity;
- data access summary;
- agent delegation summary;
- audit events.

### Roles and teams

- role templates;
- permissions summary;
- inherited policy visibility;
- member management;
- impact preview before permission changes.

### Tenants

- tenant identity;
- environments;
- storage usage;
- quotas;
- budgets;
- residency configuration;
- active users and agents;
- service health;
- audit summary.

### Service identities

- client or service identity;
- scopes;
- environment;
- expiration;
- rotation status;
- recent use;
- revoke action.

---

## 7. Frontend Architecture

## 7.1 Recommended repository structure

```text
rantai-lake/
├── design-system/
│   ├── components/
│   ├── fonts/
│   ├── tokens/
│   └── index.ts
├── public/
├── src/
│   ├── app/
│   │   ├── (overview)/
│   │   ├── (data)/
│   │   ├── (build)/
│   │   ├── (intelligence)/
│   │   ├── (governance)/
│   │   ├── (operations)/
│   │   └── (admin)/
│   ├── components/
│   │   ├── app-shell/
│   │   ├── data-display/
│   │   ├── feedback/
│   │   ├── filters/
│   │   ├── forms/
│   │   ├── graphs/
│   │   ├── editors/
│   │   └── ui/
│   ├── features/
│   │   ├── overview/
│   │   ├── catalog/
│   │   ├── storage/
│   │   ├── connectors/
│   │   ├── pipelines/
│   │   ├── streaming/
│   │   ├── queries/
│   │   ├── knowledge/
│   │   ├── search/
│   │   ├── agents/
│   │   ├── governance/
│   │   ├── lineage/
│   │   ├── audit/
│   │   ├── workloads/
│   │   ├── observability/
│   │   └── administration/
│   ├── services/
│   │   ├── contracts/
│   │   ├── clients/
│   │   ├── mock/
│   │   └── errors/
│   ├── lib/
│   ├── hooks/
│   └── types/
├── AI_PROJECT_INSIGHTS.md
├── FEATURE_COVERAGE.md
├── README.md
└── package.json
```

### Feature folder pattern

```text
features/pipelines/
├── components/
├── hooks/
├── schemas/
├── services/
├── fixtures/
├── types/
├── utils/
└── index.ts
```

Page files must remain thin. They should compose feature components and must not contain hundreds of lines of fixtures, helper functions, dialogs, and business logic.

---

## 7.2 Contract-first service layer

Define typed service interfaces before page implementation.

Example categories:

```text
CatalogService
AssetService
ConnectorService
PipelineService
StreamingService
QueryService
KnowledgeService
SearchService
AgentService
PolicyService
LineageService
AuditService
WorkloadService
ObservabilityService
IdentityService
TenantService
```

Each service must have:

- TypeScript request and response types;
- explicit loading, empty, success, partial, and failure behavior;
- pagination and filter contracts;
- mock adapter;
- future HTTP/Flight adapter boundary;
- normalized error type;
- abort/cancellation support for long operations.

Pages must not import fixture arrays directly. Pages call a service. During the prototype stage, the service uses a mock adapter.

---

## 7.3 Mock strategy

Mocks should model realistic system states, not only successful sample data.

Every major entity requires examples of:

- healthy;
- empty;
- validating;
- running;
- delayed;
- degraded;
- failed;
- unauthorized;
- partially available;
- archived.

Mock asynchronous operations must use a shared mock transport with:

- deterministic delay;
- cancellable request;
- configurable error rate for development;
- simulated progress events;
- no page-level `setTimeout` duplication.

Use browser persistence only for local prototype convenience. The service contract must remain compatible with a future backend.

---

## 7.4 Shared component inventory

Create and reuse:

- `PageHeader`
- `PageToolbar`
- `FilterBar`
- `SavedViewMenu`
- `SummaryCard`
- `MetricCard`
- `StatusBadge`
- `HealthIndicator`
- `DataTable`
- `Pagination`
- `EmptyState`
- `ErrorState`
- `PermissionState`
- `LoadingSkeleton`
- `DetailDrawer`
- `EntityHeader`
- `MetadataList`
- `CodeBlock`
- `JsonViewer`
- `SqlEditor`
- `ResultGrid`
- `QueryMetrics`
- `Timeline`
- `RunStatusTimeline`
- `FlowCanvas`
- `LineageGraph`
- `ImpactSummary`
- `PolicyDecisionPanel`
- `CostSummary`
- `FreshnessIndicator`
- `ApprovalPanel`
- `AuditEventDetail`
- `ConfirmActionDialog`
- `FormStepLayout`
- `FormReviewSummary`

Do not implement separate visual variants of the same pattern in different modules.

---

## 8. Delivery Phases for Cursor Agent

## Phase 0 — Repository foundation

### Tasks

- initialize a clean Next.js repository with the required stack;
- bring in the existing RantAI design system and assets;
- configure TypeScript strict mode and aliases;
- configure Tailwind v4 and shared tokens;
- implement fonts and default dark theme;
- implement root layout, navigation, page shell, responsive sidebar, top bar, and command/search entry;
- define route groups;
- define lint and build scripts;
- create `AI_PROJECT_INSIGHTS.md` and `FEATURE_COVERAGE.md` from the new repository facts only.

### Exit criteria

- application builds;
- all top-level routes have a consistent placeholder shell;
- no stale identity, asset, import, storage-key, or token references;
- design tokens and typography match the existing RantAI system;
- responsive navigation works.

---

## Phase 1 — Shared product primitives

### Tasks

- implement shared list/detail, table, filter, drawer, form-step, status, empty, loading, error, and confirmation patterns;
- implement common entity and run-status models;
- implement service contracts and mock transport;
- implement common pagination, sorting, filtering, and URL-state utilities;
- implement consistent date, duration, byte, rate, and cost formatting;
- implement accessibility behavior for forms, dialogs, tables, and graphs.

### Exit criteria

- every later module can be assembled from shared primitives;
- no major page needs to invent a unique table, status, toolbar, or form shell;
- mocks can simulate success, loading, failure, empty, and permission states.

---

## Phase 2 — Existing product workflows rebuilt cleanly

### Modules

- Overview dashboard;
- Data Explorer and asset detail;
- Pipelines list, create, detail, and runs;
- Query Studio natural-language and SQL modes;
- Collaboration;
- Knowledge sources;
- Catalog;
- Governance basics;
- Lineage;
- Connectors;
- Audit;
- Semantic Search;
- Users and Tenants.

### Implementation rules

- rebuild, do not copy monolithic page files;
- use new service contracts;
- move all fixtures behind mock services;
- preserve useful previous interactions while making terminology and status consistent;
- add complete empty/loading/error/permission states.

### Exit criteria

- all previous functional scope is available in the new information architecture;
- each module has list, detail, and primary action flows where relevant;
- routes are shareable and filters persist in URL state where useful;
- no page contains large inline fixtures or duplicated helpers.

---

## Phase 3 — Complete lakehouse capabilities

### Modules

- Storage Lifecycle;
- Streaming Jobs;
- Federated-query transparency;
- Workloads;
- Usage and Budgets;
- Observability;
- Service Health;
- Residency;
- Service Identities;
- advanced catalog metadata and schema evolution;
- advanced data quality and impact analysis.

### Exit criteria

- every major system capability has a clear product surface;
- operational users can see freshness, lag, query routing category, storage tier, and service health;
- governance users can inspect residency and policy impact;
- administrators can inspect quota and usage behavior.

---

## Phase 4 — Complete agent operations

### Modules

- Agent Workflow Builder;
- Digital Employees;
- Agent Run Detail;
- Approval Queue;
- Tool Registry;
- delegated actor context;
- autonomy, budget, policy, and audit views;
- event-trigger configuration.

### Exit criteria

- agent actions are never represented as ungoverned chatbot behavior;
- every run shows trigger, data access, tool calls, budget, approval, outcome, and audit trail;
- approval and escalation flows are intuitive and consistent with other high-risk actions.

---

## Phase 5 — Integration readiness and hardening

### Tasks

- finalize API contract documentation;
- ensure every mock service can be swapped for a real adapter;
- add route-level error boundaries and loading states;
- add unit tests for utilities and reducers;
- add component tests for shared flows;
- add Playwright smoke tests for critical journeys;
- add accessibility checks;
- run build, type check, lint, and responsive review;
- review all copy, status, permission, and destructive-action patterns;
- update AI context and feature coverage.

### Critical journeys

1. Discover an asset and inspect lineage, policy, quality, and freshness.
2. Create, validate, deploy, monitor, and retry a data pipeline.
3. Create and monitor a streaming materialized view.
4. Ask a natural-language question, inspect generated SQL, run it, and inspect cost and sources.
5. Run a federated SQL query and inspect its execution plan.
6. Add a knowledge source, run embedding, and test hybrid search.
7. Create a policy, simulate it, activate it, and inspect audit events.
8. Create an agent workflow with an approval gate and inspect the resulting run.
9. Inspect a platform incident from alert to metric/log/trace evidence.
10. Review tenant usage, budget, quota, and residency posture.

### Exit criteria

- `npm run build` passes;
- `npx tsc --noEmit` passes;
- `npm run lint` adds no errors;
- critical Playwright journeys pass;
- no placeholder function is represented as production behavior;
- all modules use shared design and interaction patterns.

---

## 9. Cursor Agent Working Protocol

Cursor Agent must follow this sequence for each feature:

1. Read `AI_PROJECT_INSIGHTS.md`, `FEATURE_COVERAGE.md`, and the relevant feature contract.
2. Inspect existing shared components before creating a new component.
3. Identify the route, user goal, required states, data contract, and acceptance criteria.
4. Implement the service contract or mock method before the page UI.
5. Build the smallest complete vertical slice: list/detail/action, not disconnected visuals.
6. Add loading, empty, error, permission, and destructive-action handling.
7. Use existing design tokens and shared components.
8. Run type check and lint for the changed scope.
9. Verify responsive behavior and keyboard navigation.
10. Update `FEATURE_COVERAGE.md` and `AI_PROJECT_INSIGHTS.md` only with facts proven by the repository.

### Cursor must not

- copy entire old page files into the new repository;
- add a global state library;
- add large fixture arrays inside page components;
- claim backend execution exists when only a mock adapter exists;
- create unique visual patterns when a shared pattern already exists;
- store secrets or credentials in frontend source;
- deploy a generated pipeline, query, policy, or agent action without explicit user confirmation;
- use color alone to communicate status;
- create files larger than necessary when clear feature decomposition is possible.

---

## 10. Definition of Done per Feature

A feature is complete only when:

- its route exists and is reachable from navigation;
- its user goal is clear from page title and primary action;
- list, detail, create/edit, and operational actions are implemented where relevant;
- service interfaces and mock adapters exist;
- loading, empty, error, permission, and no-result states exist;
- statuses and actions follow the shared taxonomy;
- filters and shareable URL state work where relevant;
- keyboard and screen-reader basics work;
- responsive layout works at desktop and tablet widths;
- destructive actions include impact-aware confirmation;
- a test or repeatable verification covers the main journey;
- feature coverage and AI context are updated accurately.

---

## 11. Required Repository Documentation

### README.md

Include only:

- product overview;
- stack;
- setup;
- scripts;
- folder structure;
- environment configuration;
- mock-versus-real adapter explanation;
- test commands.

### AI_PROJECT_INSIGHTS.md

Must contain:

- current repository status;
- implemented routes and features;
- service contracts;
- mock and real integration status;
- important components and hooks;
- known issues;
- verified build, type-check, lint, and test baseline;
- rules for future AI changes.

Do not describe planned features as implemented.

### FEATURE_COVERAGE.md

Maintain a matrix containing:

- domain;
- feature;
- route;
- previous-reference coverage;
- new target coverage;
- implementation status;
- mock status;
- real API status;
- acceptance-test status;
- notes.

---

## 12. Final Acceptance Criteria for the New Repository

The repository is accepted when:

1. The complete feature scope in this plan is represented in navigation and route architecture.
2. All useful functionality from the previous repository is retained or improved.
3. New modules for storage lifecycle, streaming, workloads, observability, residency, service health, usage, and agent operations are included.
4. The design system is applied consistently across every module.
5. The product uses common enterprise patterns for lists, details, configuration, execution, approvals, and audits.
6. Large pages are decomposed into feature components, hooks, services, schemas, and utilities.
7. Pages depend on typed service contracts rather than direct fixtures.
8. Mock behavior is clearly separated from real integration behavior.
9. No historical product naming, visual tokens, assets, identifiers, or comments remain.
10. Build, type check, lint, critical tests, responsive review, and accessibility review pass.
11. Repository documentation reflects only verified implementation state.

