import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const output = process.env.ASF_CI_OUTPUT ?? path.join(root, ".factory/v2/p08/ci-inspection.json");
try { await fs.copyFile(output, path.join(path.dirname(output), `ci-inspection-${Date.now()}.json`)); } catch (error) { if (error.code !== "ENOENT") throw error; }
const branches = ["p00-baseline", "p01-ide-evaluation", "p02-design-system", "p03-multi-project", "p04-production-workspace", "p05-production-editor", "p06-advanced-ide", "p07-orchestrator-conversation", "p07-r1-coding-acceptance", "p08-dynamic-orchestration"];
if (process.env.ASF_CI_EXTRA_PHASE) branches.push(process.env.ASF_CI_EXTRA_PHASE);
const results = [];
for (const suffix of branches) {
  const branch = `codex/v2-${suffix}`;
  const local = spawnSync("git", ["rev-parse", branch], { cwd: root, encoding: "utf8", windowsHide: true });
  if (local.status !== 0) { results.push({ branch, status: "LOCAL_REFERENCE_UNAVAILABLE" }); continue; }
  const sha = local.stdout.trim();
  try {
    const response = await fetch(`https://api.github.com/repos/OmegaMc1331/agentic-software-factory/actions/runs?head_sha=${sha}&per_page=100`, { headers: { Accept: "application/vnd.github+json", "User-Agent": "ASF-P08-read-only-qualification" }, signal: AbortSignal.timeout(20000) });
    const data = await response.json();
    results.push({ branch, sha, httpStatus: response.status, totalCount: data.total_count ?? null, runs: (data.workflow_runs ?? []).map(run => ({ id: run.id, headSha: run.head_sha, status: run.status, conclusion: run.conclusion, url: run.html_url })), qualification: response.ok && data.total_count === 0 ? "UNVERIFIED_NO_REMOTE_RUN" : response.ok ? "INSPECT_RECORDED_RESULTS" : "UNVERIFIED" });
  } catch (error) {
    const fallback = spawnSync("gh", ["api", `repos/OmegaMc1331/agentic-software-factory/actions/runs?head_sha=${sha}&per_page=100`], { cwd: root, encoding: "utf8", windowsHide: true, timeout: 30000 });
    if (fallback.status === 0) {
      const data = JSON.parse(fallback.stdout);
      results.push({ branch, sha, transport: "gh api read-only fallback", directFetchError: String(error), totalCount: data.total_count, runs: (data.workflow_runs ?? []).map(run => ({ id: run.id, headSha: run.head_sha, status: run.status, conclusion: run.conclusion, url: run.html_url })), qualification: data.total_count === 0 ? "UNVERIFIED_NO_REMOTE_RUN" : "INSPECT_RECORDED_RESULTS" });
    } else results.push({ branch, sha, qualification: "UNVERIFIED", error: String(error), fallbackExit: fallback.status });
  }
}
await fs.writeFile(output, JSON.stringify({ schema: 1, checked_at: new Date().toISOString(), scope: "Exact local branch heads, all GitHub Actions trigger types, read only; no run triggered", branches: results }, null, 2));
console.log(results.map(({ branch, httpStatus, totalCount, qualification }) => ({ branch, httpStatus, totalCount, qualification })));
