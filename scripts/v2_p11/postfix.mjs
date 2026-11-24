import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import net from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const original = JSON.parse(
  await fs.readFile(
    path.join(root, ".factory/v2/p11/native/results.json"),
    "utf8",
  ),
);
const project = original.projects.B;
const continuation = process.argv.includes("--continue");
const previous = continuation
  ? JSON.parse(
      await fs.readFile(
        path.join(root, ".factory/v2/p11/postfix/results.json"),
        "utf8",
      ),
    )
  : undefined;
const binary = process.env.ASF_P11_BINARY;
assert.ok(binary);
const output =
  process.env.ASF_P11_POSTFIX ??
  path.join(
    root,
    `.factory/v2/p11/${continuation ? "postfix-continuation" : "postfix"}`,
  );
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
const destination = path.join(output, "results.json");
try {
  await fs.access(destination);
  throw Error("Preserve existing evidence and select a new output directory");
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
const hash = (data) => createHash("sha256").update(data).digest("hex");
const result = {
  schema: 1,
  kind: continuation
    ? "Model-free completion and runtime reconciliation of the retained final extractor proof; no request is replayed"
    : "Bounded real-extractor requalification after the role/profile source-read guard; original coding acceptance remains separate",
  sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  }).stdout.trim(),
  binarySha256: hash(await fs.readFile(binary)),
  originalEvidenceSha256: hash(
    await fs.readFile(path.join(root, ".factory/v2/p11/native/results.json")),
  ),
  started: new Date().toISOString(),
  projectId: project.projectId,
  modelInvocationsAttempted: 0,
  providerHttpRequestCount: null,
  usageNote:
    "Counts ASF CLI invocations attempted, not provider HTTP requests, tokens or monetary cost.",
  timeline: [],
  screenshots: [],
};
const persist = () =>
  fs.writeFile(destination, JSON.stringify(result, null, 2) + "\n");
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium, request } = require("playwright");
const port = await new Promise((resolve) => {
  const listener = net.createServer();
  listener.listen(0, "127.0.0.1", () => {
    const port = listener.address().port;
    listener.close(() => resolve(port));
  });
});
const base = `http://127.0.0.1:${port}`;
const log = await fs.open(path.join(output, "server.log"), "w");
const server = spawn(
  binary,
  [
    "start",
    "--no-browser",
    "--port",
    String(port),
    "--registry",
    path.join(original.fixture, "registry.sqlite3"),
    "--allow-project-root",
    original.fixture,
  ],
  {
    cwd: project.root,
    windowsHide: true,
    stdio: ["ignore", log.fd, log.fd],
  },
);
await log.close();
let inspector, browser, csrf, saved, edited, originalSettings, currentSettings;
const allowedPath = "P11_FINAL_PUBLIC.md";
const deniedPath = "P11_FINAL_DENIED.md";
async function until(action, label, milliseconds = 60000) {
  const end = Date.now() + milliseconds;
  while (Date.now() < end) {
    const value = await action();
    if (value) return value;
    await new Promise((r) => setTimeout(r, 200));
  }
  throw Error(`Timed out: ${label}`);
}
async function api(url, method = "GET", data, status) {
  const response = await inspector.fetch(base + url, {
    method,
    headers: { Origin: base, "X-Factory-CSRF": csrf },
    ...(data === undefined ? {} : { data }),
    timeout: 240000,
  });
  const value = await response.json();
  if (value.code === "usage_limit_exceeded") {
    result.provider = "BLOCKED_PROVIDER_USAGE_LIMIT";
    throw Error("Provider usage limit; stop all further model experiments");
  }
  if (status === undefined)
    assert.ok(response.ok(), `${url}: ${JSON.stringify(value)}`);
  else
    assert.equal(response.status(), status, `${url}: ${JSON.stringify(value)}`);
  return value;
}
const p = (suffix, method, data, status) =>
  api(`/api/projects/${project.projectId}/${suffix}`, method, data, status);
const configuration = (value) =>
  Object.fromEntries(
    [
      "name",
      "description",
      "providerAdapterId",
      "modelId",
      "reasoningEffort",
      "roles",
      "instructions",
      "permissions",
      "maxConcurrency",
      "timeoutSecs",
      "enabled",
      "automaticDelegation",
      "preferred",
    ].map((key) => [key, value[key]]),
  );
try {
  await until(async () => {
    try {
      return (await fetch(base + "/api/health")).ok;
    } catch {
      return false;
    }
  }, "owned final server");
  inspector = await request.newContext();
  const authentication = await inspector.post(base + "/api/auth/session", {
    headers: { Origin: base },
    data: {},
  });
  assert.ok(authentication.ok());
  csrf = (await authentication.json()).csrfToken;
  await api(`/api/projects/${project.projectId}/open`, "POST");
  const workspaces = await api("/api/workspaces");
  let workspace;
  for (const entry of workspaces) {
    const current = (await api(`/api/workspaces/${entry.id}`)).document;
    if (
      current.sessions.some(
        (session) => session.projectId === project.projectId,
      )
    ) {
      workspace = current;
      break;
    }
  }
  assert.ok(workspace);
  browser = await chromium.launch({
    channel: "msedge",
    headless: true,
    args: [
      "--disable-gpu",
      "--disable-extensions",
      "--disable-background-networking",
    ],
  });
  const context = await browser.newContext({
    viewport: { width: 1500, height: 1050 },
    reducedMotion: "reduce",
  });
  await context.addInitScript(
    (id) => localStorage.setItem("asf.v2.workspace.last", id),
    workspace.id,
  );
  const page = await context.newPage();
  let lease;
  page.on("response", async (response) => {
    if (
      new URL(response.url()).pathname.includes("/browser-clients/") &&
      response.ok()
    ) {
      try {
        const value = await response.json();
        if (value.leaseId) lease = value;
      } catch {}
    }
  });
  await page.goto(base + "/workspace.html");
  await page
    .getByLabel("Open registered project")
    .selectOption(project.projectId);
  const binding = await until(
    () =>
      lease?.bindings.find(
        (binding) => binding.projectId === project.projectId,
      ),
    "authorized owning browser lease",
  );
  result.workspaceSessionId = binding.sessionId;
  if (continuation) {
    assert.equal(previous.allowed.session.status, "success");
    assert.equal(previous.denied.session.status, "failed");
    assert.ok(!previous.denied.session.outputMeta?.processId);
    const changes = spawnSync(
      "git",
      [
        "diff",
        "--name-only",
        previous.sourceCommit,
        "--",
        "crates",
        "Cargo.toml",
        "Cargo.lock",
      ],
      { cwd: root, encoding: "utf8", windowsHide: true },
    );
    assert.equal(changes.status, 0);
    const productionChanges = changes.stdout
      .trim()
      .split(/\r?\n/)
      .filter((file) => file && !file.includes("/tests/"));
    assert.deepEqual(productionChanges, []);
    result.runtimeEquivalence = {
      referenceCommit: previous.sourceCommit,
      changedProductionRustFiles: productionChanges,
      previousBinarySha256: previous.binarySha256,
      currentBinarySha256: result.binarySha256,
      note: "UI/test rebuild; original extractor runtime source is identical",
    };
    result.retainedSuccessfulExtractor = previous.allowed;
    result.retainedDeniedSource = previous.denied;
    const current = await p(`agent-profiles/${project.profiles.planner.id}`);
    assert.equal(current.profile.revision, previous.restoredProfileRevision);
    const sessions = await p(`agents/${project.profiles.planner.id}/sessions`);
    const interrupted = sessions.find(
      (session) => session.id > previous.allowed.session.id,
    );
    assert.equal(interrupted.status, "interrupted");
    const interruptedDetails = await p(`sessions/${interrupted.id}`);
    const pid = interruptedDetails.outputMeta.processId;
    assert.ok(Number.isSafeInteger(pid) && pid > 0);
    const probe = spawnSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        `Get-CimInstance Win32_Process -Filter 'ProcessId = ${pid}' | ForEach-Object { @{processId=$_.ProcessId;startUnixMs=([DateTimeOffset]$_.CreationDate).ToUnixTimeMilliseconds()} | ConvertTo-Json -Compress }`,
      ],
      { encoding: "utf8", windowsHide: true },
    );
    assert.equal(probe.status, 0);
    const observed = probe.stdout.trim() ? JSON.parse(probe.stdout) : null;
    assert.ok(
      !observed ||
        Math.abs(
          observed.startUnixMs -
            interruptedDetails.outputMeta.processStartedUnixMs,
        ) > 2000,
      "The original owned process remains alive; no replay or termination is attempted",
    );
    const conversationId = previous.error.match(
      /\/conversations\/(c_[0-9a-f]+)\/messages/,
    )[1];
    const messages = await p(
      `conversations/${conversationId}/messages?limit=100`,
    );
    assert.ok(
      messages.some(
        (message) =>
          message.role === "orchestrator" && message.status === "interrupted",
      ),
    );
    result.supplementaryQuery = {
      status: "interrupted_by_harness_teardown_after_normal_202",
      agentSessionId: interrupted.id,
      processId: pid,
      processObservedAbsentAt: new Date().toISOString(),
      reusedPidObservation: observed,
      messages,
      usage: await p(`memory/usage?sessionId=${interrupted.id}`),
      newInvocations: 0,
    };
    result.agentSessionsAfterRecovery = sessions.map((session) => ({
      id: session.id,
      role: session.role,
      status: session.status,
    }));
    assert.equal(sessions.length, 4);
    result.currentRetrieval = await p("memory/retrieve", "POST", {
      query: "immutable inventory balance snapshots",
    });
    assert.ok(
      result.currentRetrieval.records.some(
        (memory) => memory.id === project.memory.id,
      ),
    );
  } else {
    const profiles = await p("agent-profiles");
    saved = profiles.find(
      (profile) => profile.id === project.profiles.planner.id,
    );
    assert.ok(saved?.enabled && saved.preferred);
    originalSettings = await p("memory/settings");
    currentSettings = await p("memory/settings", "PUT", {
      ...originalSettings,
      extractionMode: "review",
    });
    await fs.writeFile(
      path.join(project.root, allowedPath),
      "Inventory balances are represented by immutable snapshots. Updates return a new snapshot without changing the caller's input.\n",
    );
    await fs.writeFile(
      path.join(project.root, deniedPath),
      "Denied controlled fixture evidence. This source must not enter the selected Planner process.\n",
    );
    const primary = (await p("files/roots")).roots.find(
      (item) => item.kind === "repository",
    );
    const source = (file) => ({
      projectId: project.projectId,
      reference: {
        kind: "repository_file",
        rootId: primary.id,
        path: file,
        revision: null,
      },
    });
    const before = await p(`agents/${saved.id}/sessions`);
    edited = await p(`agent-profiles/${saved.id}`, "PATCH", {
      expectedRevision: saved.revision,
      configuration: {
        ...configuration(saved),
        permissions: {
          ...saved.permissions,
          filesystem: { read: [allowedPath], write: [], deny_write: [] },
        },
      },
    });
    result.limitedProfile = {
      id: edited.id,
      revision: edited.revision,
      permissions: edited.permissions,
    };
    const denied = await p(
      "memory/extract",
      "POST",
      { sources: [source(deniedPath)] },
      403,
    );
    assert.equal(denied.code, "memory_policy_denied");
    const failed = (await p("memory/extractions")).find((item) =>
      item.sourceReferences.some(
        (reference) => reference.source.reference.path === deniedPath,
      ),
    );
    assert.equal(failed.state, "failed");
    assert.ok(failed.agentSessionId);
    const deniedSession = await p(`sessions/${failed.agentSessionId}`);
    assert.equal(deniedSession.status, "failed");
    assert.ok(!deniedSession.outputMeta?.processId);
    assert.equal(deniedSession.stdout ?? "", "");
    result.denied = {
      response: denied,
      extraction: failed,
      session: {
        id: deniedSession.id,
        status: deniedSession.status,
        outputMeta: deniedSession.outputMeta,
      },
    };
    result.timeline.push({
      event: "source.policy_denied_before_process",
      at: new Date().toISOString(),
    });
    await persist();
    result.modelInvocationsAttempted++;
    const extracted = await p("memory/extract", "POST", {
      sources: [source(allowedPath)],
    });
    assert.equal(extracted.extraction.state, "completed");
    assert.ok(extracted.candidates.length);
    const allowedSession = await p(
      `sessions/${extracted.extraction.agentSessionId}`,
    );
    assert.equal(allowedSession.status, "success");
    assert.ok(allowedSession.outputMeta?.processId);
    result.allowed = {
      extraction: extracted.extraction,
      candidates: extracted.candidates,
      session: {
        id: allowedSession.id,
        agent: allowedSession.agent,
        status: allowedSession.status,
        policyAudit: allowedSession.policyAudit,
        outputMeta: allowedSession.outputMeta,
        stdoutSha256: hash(allowedSession.stdout ?? ""),
        stderrSha256: hash(allowedSession.stderr ?? ""),
        nativeHeader: (allowedSession.stderr ?? "")
          .split(/\r?\n/)
          .slice(0, 30)
          .filter((line) =>
            /^(OpenAI Codex|model:|approval:|sandbox:|workdir:)/.test(line),
          ),
      },
      snapshot: await p(
        `agent-sessions/${allowedSession.id}/execution-snapshot`,
      ),
    };
    assert.ok(
      result.allowed.session.nativeHeader.includes(`model: ${saved.modelId}`),
    );
    result.timeline.push({
      event: "allowed_source.real_extractor_completed",
      at: new Date().toISOString(),
    });
    edited = await p(`agent-profiles/${saved.id}`, "PATCH", {
      expectedRevision: edited.revision,
      configuration: configuration(saved),
    });
    const restoredRevision = edited.revision;
    edited = undefined;
    result.restoredProfileRevision = restoredRevision;
    const conversation = await p("conversations", "POST", {
      title: "Final bounded memory context requalification",
    });
    result.modelInvocationsAttempted++;
    const command = await p(
      `conversations/${conversation.id}/messages`,
      "POST",
      {
        key: `p11-final-${Date.now()}`,
        workspace_session_id: binding.sessionId,
        content:
          "From saved project memory, what convention applies to immutable inventory balance snapshots? Cite the saved memory if available; do not create a mission.",
        context: [],
      },
    );
    const message = await until(
      async () => {
        const messages = await p(
          `conversations/${conversation.id}/messages?limit=100`,
        );
        const response = messages.find(
          (message) => message.id === command.response_id,
        );
        if (response?.status === "failed")
          throw Error(
            "Final conversation failed; stop model requests and inspect durable session",
          );
        return response?.status === "finished" && response;
      },
      "final genuine conversation",
      240000,
    );
    const usage = await p(
      `memory/usage?sessionId=${message.provenance.agent_session_id}`,
    );
    assert.ok(
      usage.some((item) =>
        item.memories.some((memory) => memory.memoryId === project.memory.id),
      ),
    );
    assert.ok(
      usage.every((item) =>
        item.memories.every(
          (memory) => memory.memoryId !== original.projects.A.memory.id,
        ),
      ),
    );
    result.conversation = { message, usage };
    result.newSessions = (await p(`agents/${saved.id}/sessions`))
      .filter((session) => !before.some((old) => old.id === session.id))
      .map((session) => ({
        id: session.id,
        status: session.status,
        role: session.role,
        outputMeta: session.outputMeta,
      }));
    assert.equal(result.newSessions.length, 3);
  }
  await page.getByRole("button", { name: "Memory", exact: true }).click();
  await page
    .locator(".asf-memory:visible")
    .last()
    .getByRole("button", { name: "Refresh", exact: true })
    .click();
  const screenshot = "screenshots/final-current-project-memory.png";
  await until(
    async () =>
      (await page.locator(".ws-status").innerText()).includes(
        "Workspace saved",
      ),
    "persisted Memory view layout",
  );
  await page.screenshot({ path: path.join(output, screenshot) });
  result.screenshots.push({
    file: screenshot,
    sha256: hash(await fs.readFile(path.join(output, screenshot))),
  });
  const a = original.projects.A;
  await api(`/api/projects/${a.projectId}/open`, "POST");
  const finalIntegrations = (
    await api(`/api/projects/${a.projectId}/runs/${a.runId}/scheduler`)
  ).integrations;
  assert.deepEqual(finalIntegrations, a.integrations);
  result.integrationHistoryUnchanged = true;
  await api(
    `/api/projects/${a.projectId}/memory/${a.memory.id}`,
    "GET",
    undefined,
    404,
  );
  const historical = await api(
    `/api/projects/${a.projectId}/memory/usage?sessionId=${a.secondWorker.id}`,
  );
  assert.ok(
    historical.some((entry) =>
      entry.memories.some(
        (memory) => memory.memoryId === a.memory.id && memory.revision === 5,
      ),
    ),
  );
  result.historicalAuditAfterDeletion = historical;
  await page.getByLabel("Open registered project").selectOption(a.projectId);
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page.getByRole("menuitem", { name: "Progress", exact: true }).click();
  await page
    .getByRole("button", { name: "Open workflow and review", exact: true })
    .first()
    .click();
  await page
    .getByRole("button", { name: new RegExp(`#${a.secondWorker.id}.*success`) })
    .first()
    .click();
  const audit = page.locator(".asf-memory-usage:visible").last();
  await audit
    .getByText("Project memory supplied to this invocation", { exact: true })
    .click();
  await audit.getByText(a.memory.id, { exact: false }).waitFor();
  await page
    .getByRole("button", { name: "Layout actions", exact: true })
    .click();
  await page
    .getByRole("menuitem", { name: "Toggle maximization", exact: true })
    .click();
  await audit.scrollIntoViewIfNeeded();
  await until(
    async () =>
      (
        await page
          .locator(".ws-output:visible .ws-output-toolbar")
          .last()
          .innerText()
      ).includes("success"),
    "persisted terminal session snapshot",
  );
  assert.equal(
    await page
      .locator(".ws-output:visible")
      .getByText(
        "Session stream disconnected. Reconnect after checking the project runtime.",
        { exact: true },
      )
      .count(),
    0,
  );
  const auditScreenshot =
    "screenshots/final-maximized-historical-memory-audit.png";
  await until(
    async () =>
      (await page.locator(".ws-status").innerText()).includes(
        "Workspace saved",
      ),
    "persisted historical audit layout",
  );
  result.terminalSessionView = {
    agentSessionId: a.secondWorker.id,
    status: "success",
    disconnectBanner: false,
    layoutSaved: true,
  };
  await page.screenshot({ path: path.join(output, auditScreenshot) });
  result.screenshots.push({
    file: auditScreenshot,
    sha256: hash(await fs.readFile(path.join(output, auditScreenshot))),
  });
  result.status = "PASS";
} catch (error) {
  result.status = "FAILED";
  result.error = String(error.stack ?? error)
    .replaceAll(csrf ?? "__unset_csrf__", "[REDACTED]")
    .slice(0, 4000);
  process.exitCode = 1;
} finally {
  if (edited && saved && inspector) {
    try {
      await p(`agent-profiles/${saved.id}`, "PATCH", {
        expectedRevision: edited.revision,
        configuration: configuration(saved),
      });
      result.profileRestoredAfterFailure = true;
    } catch (error) {
      result.profileRestoreError = String(error).slice(0, 1000);
      result.status = "FAILED";
      process.exitCode = 1;
    }
  }
  if (currentSettings && originalSettings && inspector) {
    try {
      await p("memory/settings", "PUT", {
        ...originalSettings,
        revision: currentSettings.revision,
      });
      result.memorySettingsRestored = true;
    } catch (error) {
      result.settingsRestoreError = String(error).slice(0, 1000);
      result.status = "FAILED";
      process.exitCode = 1;
    }
  }
  await browser?.close();
  await inspector?.dispose();
  if (server.exitCode === null) {
    const exit = new Promise((resolve) => server.once("exit", resolve));
    server.kill();
    await exit;
  }
  result.finished = new Date().toISOString();
  await persist();
  process.stdout.write(
    JSON.stringify({
      status: result.status,
      modelInvocationsAttempted: result.modelInvocationsAttempted,
      error: result.error,
    }) + "\n",
  );
}
