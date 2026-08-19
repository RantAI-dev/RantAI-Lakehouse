/**
 * Klien Dagster GraphQL (server-side) — orkestrasi NYATA lakehouse kita.
 * Job `refresh_lakehouse` (7 aset bronze→silver→gold) + jadwal harian.
 */

const DAGSTER_URL = process.env.DAGSTER_URL ?? "http://localhost:13030/graphql";
const REPO = process.env.DAGSTER_REPO ?? "__repository__";
const LOCATION = process.env.DAGSTER_LOCATION ?? "dispar_orchestrate.definitions";

async function dg<T = unknown>(query: string, variables?: Record<string, unknown>, signal?: AbortSignal): Promise<T> {
  const res = await fetch(DAGSTER_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ query, variables }),
    signal,
    cache: "no-store",
  });
  const json = await res.json();
  if (json.errors) throw new Error(JSON.stringify(json.errors).slice(0, 300));
  return json.data as T;
}

export type DgJob = { name: string; schedules: { cronSchedule: string; scheduleState: { status: string } }[] };
export type DgRun = {
  runId: string; jobName: string; status: string; startTime: number | null; endTime: number | null;
};

type RepoNode = {
  jobs: { name: string }[];
  schedules: { cronSchedule: string; scheduleState: { status: string }; jobName?: string }[];
};

export async function listJobs(signal?: AbortSignal): Promise<DgJob[]> {
  const d = await dg<{ repositoriesOrError: { nodes?: RepoNode[] } }>(
    `{ repositoriesOrError { __typename ... on RepositoryConnection { nodes {
        jobs { name }
        schedules { cronSchedule scheduleState { status } jobName: pipelineName }
      } } } }`,
    undefined,
    signal,
  );
  const node = d.repositoriesOrError.nodes?.[0];
  if (!node) return [];
  return node.jobs
    .filter((j) => !j.name.startsWith("__"))
    .map((j) => ({
      name: j.name,
      schedules: node.schedules
        .filter((s) => s.jobName === j.name)
        .map((s) => ({ cronSchedule: s.cronSchedule, scheduleState: s.scheduleState })),
    }));
}

export async function listRuns(jobName?: string, limit = 25, signal?: AbortSignal): Promise<DgRun[]> {
  const filter = jobName ? `(filter: { pipelineName: "${jobName}" }, limit: ${limit})` : `(limit: ${limit})`;
  const d = await dg<{ runsOrError: { results?: DgRun[] } }>(
    `{ runsOrError${filter} { __typename ... on Runs { results {
        runId jobName status startTime endTime
      } } } }`,
    undefined,
    signal,
  );
  return d.runsOrError.results ?? [];
}

export async function launchRun(jobName: string, signal?: AbortSignal): Promise<{ runId?: string; error?: string }> {
  const d = await dg<{ launchRun: { __typename: string; run?: { runId: string }; message?: string; errors?: { message: string }[] } }>(
    `mutation($sel: JobOrPipelineSelector!) {
       launchRun(executionParams: { selector: $sel, mode: "default" }) {
         __typename
         ... on LaunchRunSuccess { run { runId } }
         ... on PythonError { message }
         ... on RunConfigValidationInvalid { errors { message } }
       }
     }`,
    { sel: { repositoryName: REPO, repositoryLocationName: LOCATION, pipelineName: jobName } },
    signal,
  );
  const r = d.launchRun;
  if (r.__typename === "LaunchRunSuccess" && r.run) return { runId: r.run.runId };
  return { error: r.message ?? r.errors?.map((e) => e.message).join("; ") ?? r.__typename };
}

/** Dagster run status → EntityStatus konsol. */
export function mapRunStatus(s: string): string {
  switch (s) {
    case "SUCCESS": return "completed";
    case "FAILURE": return "failed";
    case "CANCELED": case "CANCELING": return "cancelled";
    case "QUEUED": case "NOT_STARTED": return "queued";
    case "STARTED": case "STARTING": case "MANAGED": return "running";
    default: return "unknown";
  }
}
