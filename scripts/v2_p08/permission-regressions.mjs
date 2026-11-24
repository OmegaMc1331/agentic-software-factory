import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import net from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = process.env.ASF_PERMISSION_OUTPUT ?? path.join(root, ".factory/v2/p08/permissions");
const binary =
  process.env.ASF_P07_R1_BINARY ?? path.join(root, ".factory/v2/p08/run/factory-p08.exe");
const fixture = await fs.mkdtemp(
  path.join(os.tmpdir(), "asf-r1-permission-regression-"),
);
const helper = path.join(fixture, "controlled_agent.py");
const results = {
  schema: 1,
  kind: "controlled_process_policy_regressions_not_model_qualification",
  started: new Date().toISOString(),
  fixture,
  binarySha256: createHash("sha256")
    .update(await fs.readFile(binary))
    .digest("hex"),
  tests: [],
};
try {
  await fs.copyFile(
    path.join(output, "permission-regressions.json"),
    path.join(output, `permission-regressions-attempt-${Date.now()}.json`),
  );
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
await fs.writeFile(
  helper,
  String.raw`import json, os, subprocess, sys
from pathlib import Path
prompt = sys.stdin.read()
role = sys.argv[1]
if role == "worker":
    source = "def increment(value):" + chr(10) + "    return " + ("0" if os.environ["R1_CASE"] == "failed-tests" else "value + 1") + chr(10)
    Path("src/calculator.py").write_text(source, encoding="utf8")
    test = subprocess.run(["python", "-m", "unittest", "discover", "-s", "tests", "-v"], capture_output=True, text=True)
    sys.stderr.write(test.stdout + test.stderr)
    print(json.dumps({"summary":"Original controlled regression worker, not a model", "commands":["python -m unittest discover -s tests -v"], "results":["Actual test exit " + str(test.returncode)]}))
elif role == "reviewer":
    test = subprocess.run(["python", "-m", "unittest", "discover", "-s", "tests", "-v"], capture_output=True, text=True)
    sys.stderr.write(test.stdout + test.stderr)
    print(json.dumps({"decision":"approve" if test.returncode == 0 else "request_changes", "reason":"Independent actual unittest exit " + str(test.returncode), "feedback":[] if test.returncode == 0 else ["The implementation changes do not pass the real regression test"]}))
else:
    raise SystemExit("Authored plan must not invoke Planner in this regression")
`,
);
function run(program, args, cwd) {
  const r = spawnSync(program, args, {
    cwd,
    windowsHide: true,
    encoding: "utf8",
    timeout: 60000,
  });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout.trim();
}
const entries = [];
for (const name of ["passing", "read-only", "failed-tests"]) {
  const folder = path.join(fixture, name);
  await fs.mkdir(path.join(folder, "src"), { recursive: true });
  await fs.mkdir(path.join(folder, "tests"));
  await fs.writeFile(
    path.join(folder, ".gitignore"),
    ".factory/\n__pycache__/\n*.pyc\n",
  );
  await fs.writeFile(path.join(folder, "src/__init__.py"), "");
  await fs.writeFile(
    path.join(folder, "src/calculator.py"),
    "def increment(value):\n    return -1\n",
  );
  await fs.writeFile(
    path.join(folder, "tests/test_calculator.py"),
    "import unittest\nfrom src.calculator import increment\nclass Regression(unittest.TestCase):\n    def test_increment(self):\n        self.assertEqual(increment(2), 3)\n",
  );
  for (const args of [
    ["init", "-q", "-b", "main"],
    ["config", "user.name", "ASF R1 regression"],
    ["config", "user.email", "r1@example.invalid"],
    ["add", "."],
    ["commit", "-qm", "controlled failing regression"],
  ])
    run("git", args, folder);
  run(binary, ["init"], folder);
  const config =
    ["planner", "worker", "reviewer"]
      .map(
        (role) =>
          `[agents.${role}]\ncommand="python"\nargs=${JSON.stringify([helper, role])}\n[agents.${role}.env]\nPYTHONDONTWRITEBYTECODE="1"\nR1_CASE=${JSON.stringify(name)}\n[[role_assignments]]\nrole=${JSON.stringify(role)}\nagent=${JSON.stringify(role)}\npreferred=true\n`,
      )
      .join("\n") +
    `\n[policies.roles.worker.filesystem]\nread=["**"]\nwrite=${name === "read-only" ? "[]" : '["src/**", "tests/**"]'}\n[policies.roles.reviewer.filesystem]\nread=["**"]\nwrite=[]\n`;
  await fs.writeFile(path.join(folder, ".factory/config.toml"), config);
  entries.push({ name, folder });
}
const port = await new Promise((resolve) => {
  const s = net.createServer();
  s.listen(0, "127.0.0.1", () => {
    const p = s.address().port;
    s.close(() => resolve(p));
  });
});
const base = `http://127.0.0.1:${port}`;
const log = await fs.open(
  path.join(output, "permission-regressions-server.log"),
  "w",
);
const server = spawn(
  binary,
  [
    "start",
    "--no-browser",
    "--port",
    String(port),
    "--registry",
    path.join(fixture, "registry.sqlite3"),
    "--allow-project-root",
    fixture,
  ],
  {
    cwd: entries[0].folder,
    windowsHide: true,
    stdio: ["ignore", log.fd, log.fd],
  },
);
await log.close();
async function until(fn) {
  for (let n = 0; n < 1200; n++) {
    const value = await fn();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("Controlled regression did not finish within 120s");
}
try {
  await until(async () => {
    try {
      return (await fetch(base + "/api/health")).ok;
    } catch {
      return false;
    }
  });
  const auth = await fetch(base + "/api/auth/session", {
    method: "POST",
    headers: { Origin: base, "Content-Type": "application/json" },
    body: "{}",
  });
  assert.equal(auth.status, 200);
  const cookie = auth.headers
    .getSetCookie()
    .map((value) => value.split(";")[0])
    .join("; ");
  const csrf = (await auth.json()).csrfToken;
  async function call(route, body) {
    const r = await fetch(base + route, {
      method: body === undefined ? "GET" : "POST",
      headers: {
        Origin: base,
        Cookie: cookie,
        "X-Factory-CSRF": csrf,
        "Content-Type": "application/json",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!r.ok) throw new Error(await r.text());
    return r.json();
  }
  const registry = await call("/api/projects");
  for (const entry of entries) {
    entry.project =
      entry.name === "passing"
        ? registry[0].project_id
        : (
            await call("/api/projects/register", {
              path: entry.folder,
              trust_project: true,
            })
          ).project_id;
    if (entry.name !== "passing")
      await call(`/api/projects/${entry.project}/open`, {});
    const conversation = await call(
      `/api/projects/${entry.project}/conversations`,
      { title: "Controlled permission regression" },
    );
    const objective =
      "Make increment return value + 1 and pass the real unittest regression";
    await call(
      `/api/projects/${entry.project}/conversations/${conversation.id}/messages`,
      {
        key: randomUUID(),
        workspace_session_id: "s_" + randomUUID().replaceAll("-", ""),
        content: objective,
        context: [],
        mission: {
          objective,
          plan: {
            objective,
            tasks: [
              {
                id: "T1",
                title: "Increment regression",
                objective,
                dependencies: [],
                acceptanceCriteria: ["Actual unittest passes"],
                role: "worker",
                operation: "implement",
              },
            ],
          },
        },
      },
    );
    const mission = await until(async () => {
      const value = (
        await call(
          `/api/projects/${entry.project}/conversations/${conversation.id}/missions`,
        )
      )[0];
      return value?.terminal ? value : false;
    });
    const detail = await call(
      `/api/projects/${entry.project}/runs/${mission.run_id}`,
    );
    entry.mission = mission;
    entry.detail = detail;
    if (entry.name === "passing") {
      assert.equal(mission.status, "completed");
      assert.ok(mission.integration_sha);
    } else if (entry.name === "read-only") {
      assert.equal(mission.status, "blocked");
      assert.equal(
        detail.sessions.filter(
          (session) => session.role === "worker" || session.role === "reviewer",
        ).length,
        0,
      );
      assert.equal(mission.integration_sha, null);
    } else {
      assert.equal(mission.status, "failed");
      assert.equal(mission.integration_sha, null);
      assert.ok(
        detail.attempts.every(
          (attempt) =>
            attempt.review?.decision === "request_changes" &&
            attempt.evidence?.diffPatch?.includes("return 0"),
        ),
      );
      assert.ok(
        detail.sessions.some(
          (session) =>
            session.role === "reviewer" &&
            session.stderr.includes("FAILED (failures=1)"),
        ),
      );
    }
    results.tests.push({
      name: entry.name,
      status: "passed",
      project: entry.project,
      mission: mission.run_id,
    });
    console.log(`PASS controlled permission regression: ${entry.name}`);
  }
  const passing = await call(`/api/projects/${entries[0].project}/runs/1`);
  assert.equal(passing.run.status, "completed");
  assert.equal(
    await fs.readFile(
      path.join(entries[1].folder, "src/calculator.py"),
      "utf8",
    ),
    "def increment(value):\n    return -1\n",
  );
  results.entries = entries;
} catch (error) {
  results.failure = String(error);
  throw error;
} finally {
  results.finished = new Date().toISOString();
  await fs.writeFile(
    path.join(output, "permission-regressions.json"),
    JSON.stringify(results, null, 2),
  );
  if (server.exitCode === null)
    await new Promise((resolve) => {
      server.once("exit", resolve);
      server.kill();
    });
}
