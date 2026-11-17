-- Demo data for the Gantt chart QA (BI-16 C1): a small project plan with two
-- date columns, which no other serving table has.
--
-- Run it by hand against a DEV ClickHouse, never against a shared or
-- production one:
--
--   docker exec -i lakehouse-clickhouse-1 clickhouse-client --multiquery < ops/demo/mart_demo_projects.sql
--
-- It creates `serving.mart_demo_projects` (15 tasks in three teams, dates
-- around the second half of 2026, progress 0 to 100, two rows that end before
-- they start so the "left out" note shows) and is safe to run twice: the table
-- is dropped and recreated. Then, in the console: Dashboards > new chart >
-- Gantt, data source `mart_demo_projects`, task `task`, group `team`, start
-- `start_date`, end `end_date`, progress `progress`.

DROP TABLE IF EXISTS serving.mart_demo_projects;

CREATE TABLE serving.mart_demo_projects
(
    task        String,
    team        String,
    start_date  Date,
    end_date    Date,
    progress    Float64
)
ENGINE = MergeTree
ORDER BY (team, start_date);

INSERT INTO serving.mart_demo_projects (task, team, start_date, end_date, progress) VALUES
    ('Requirements',        'Product',     '2026-08-03', '2026-08-28', 100),
    ('Wireframes',          'Product',     '2026-08-24', '2026-09-18', 100),
    ('Pricing review',      'Product',     '2026-09-21', '2026-10-09', 80),
    ('Pilot feedback',      'Product',     '2026-10-12', '2026-11-13', 30),
    ('Data model',          'Engineering', '2026-08-17', '2026-09-11', 100),
    ('Ingest pipeline',     'Engineering', '2026-09-07', '2026-10-16', 90),
    ('Dashboards',          'Engineering', '2026-10-05', '2026-11-20', 55),
    ('Access control',      'Engineering', '2026-10-19', '2026-12-04', 15),
    ('Load tests',          'Engineering', '2026-11-16', '2026-12-18', 0),
    ('Pilot sites',         'Operations',  '2026-09-14', '2026-10-23', 70),
    ('Training material',   'Operations',  '2026-10-12', '2026-11-27', 40),
    ('Go-live checklist',   'Operations',  '2026-11-23', '2026-12-11', 0),
    ('Support runbook',     'Operations',  '2026-11-02', '2026-12-04', 10),
    ('Backfilled estimate', 'Product',     '2026-10-20', '2026-10-06', 0),
    ('Typo in the dates',   'Operations',  '2026-12-15', '2026-12-01', 0);
