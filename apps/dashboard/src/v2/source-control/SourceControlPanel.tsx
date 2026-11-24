import { lazy, Suspense, useCallback, useEffect, useState } from "react";
import { Button, Input, Select, Textarea } from "../design-system/primitives/Controls";
import { InlineAlert, LoadingIndicator } from "../design-system/components/Application";
import type { DockSession, WorkspaceModel } from "../workspace/production/controller";
import type { WorkspaceView } from "../workspace/production/model";
import { ComparisonView } from "../files/ComparisonView";
import type { RootInfo } from "../files/types";
import { sourceRequest, useSourceData } from "./client";
import type { Comparison, GitBranch, GitChange, GitCommit, GitDiff, Worktree } from "./types";
import "./source-control.css";

const ReviewPanel = lazy(() => import("./ReviewPanel").then((m) => ({ default: m.ReviewPanel })));
const GitHubPanel = lazy(() => import("./GitHubPanel").then((m) => ({ default: m.GitHubPanel })));

export function SourceControlPanel({
  view,
  model,
  dock,
}: {
  view: WorkspaceView;
  model: WorkspaceModel;
  dock: DockSession;
}) {
  const [version, setVersion] = useState(0);
  const [tick, setTick] = useState(0);
  const [reviewsOpen, setReviewsOpen] = useState(false),
    [githubOpen, setGithubOpen] = useState(false);
  const rootId = view.state.rootId;
  const observed = useSourceData<Worktree>(
    view.projectId,
    rootId ? `repository?rootId=${encodeURIComponent(rootId)}` : "repository",
    version + tick
  );
  const root = observed.value;
  const roots = useSourceData<RootInfo[]>(
    view.projectId,
    "roots",
    version,
    !!root || !observed.loading
  );
  const choices = roots.value ?? (root ? [root.root] : []);
  const refresh = useCallback(() => setVersion((value) => value + 1), []);
  useEffect(() => {
    const timer = window.setInterval(() => setTick((value) => value + 1), 10_000);
    return () => clearInterval(timer);
  }, []);
  return (
    <div className="asf-source">
      <header className="sc-toolbar">
        <h2>Source Control</h2>
        <Button onClick={refresh}>Refresh repository</Button>
      </header>
      {observed.error ? (
        <InlineAlert tone="danger" title="Repository unavailable">
          {observed.error}
        </InlineAlert>
      ) : null}
      {roots.error ? (
        <InlineAlert tone="warning" title="Worktree list unavailable">
          {roots.error}
        </InlineAlert>
      ) : null}
      {(choices.length > 0 || roots.value) && (
        <Select
          label="Repository / worktree"
          value={root?.root.id ?? ""}
          onChange={(event) =>
            model.updateView(dock.sessionId, view.id, {
              rootId: event.target.value,
              selectedPath: undefined,
            })
          }
        >
          {!root && <option value="">Choose an available worktree</option>}
          {choices.map((item) => (
            <option key={item.id} value={item.id}>
              {item.label}
              {item.read_only ? " · read only" : " · user working tree"}
            </option>
          ))}
        </Select>
      )}
      {!root && rootId && observed.error ? (
        <InlineAlert tone="warning" title="Selected worktree unavailable">
          Choose an available authorized worktree. No mutation target was substituted.
        </InlineAlert>
      ) : root ? (
        <RepositoryView
          key={`${view.projectId}:${root.root.id}`}
          view={view}
          model={model}
          dock={dock}
          root={root}
          loading={observed.loading}
          observationFailed={!!observed.error}
          version={version}
          refresh={refresh}
        />
      ) : observed.loading ? (
        <LoadingIndicator label="Inspecting Git repository" />
      ) : null}
      <details onToggle={(event) => setReviewsOpen(event.currentTarget.open)}>
        <summary>Agent reviews and integration</summary>
        <Suspense fallback={<LoadingIndicator label="Loading review workspace" />}>
          {reviewsOpen && <ReviewPanel view={view} dock={dock} model={model} />}
        </Suspense>
      </details>
      <details onToggle={(event) => setGithubOpen(event.currentTarget.open)}>
        <summary>GitHub and authorized delivery</summary>
        <Suspense fallback={<LoadingIndicator label="Loading GitHub inspection" />}>
          {githubOpen && (
            <GitHubPanel key={view.projectId} projectId={view.projectId} runId={view.resourceId} />
          )}
        </Suspense>
      </details>
    </div>
  );
}

function RepositoryView({
  view,
  model,
  dock,
  root,
  loading,
  observationFailed,
  version,
  refresh,
}: {
  view: WorkspaceView;
  model: WorkspaceModel;
  dock: DockSession;
  root: Worktree;
  loading: boolean;
  observationFailed: boolean;
  version: number;
  refresh: () => void;
}) {
  const status = root.status;
  const [comparison, setComparison] = useState<Comparison>("working_index");
  const [path, setPath] = useState(view.state.selectedPath ?? "");
  const [commitVersion, setCommitVersion] = useState<{ base: string; target: string }>();
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [message, setMessage] = useState("");
  const [trusted, setTrusted] = useState(false);
  const blocked = busy || loading || observationFailed;
  const choose = (change: GitChange, staged: boolean) => {
    setPath(change.path);
    setCommitVersion(undefined);
    setComparison(staged ? "index_head" : "working_index");
    model.updateView(dock.sessionId, view.id, { selectedPath: change.path });
  };
  const mutate = async (operation: string, body: unknown) => {
    if (root.root.read_only || blocked) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const result = await sourceRequest<{ commit?: string }>(view.projectId, operation, body);
      setNotice(result.commit ? `Committed ${result.commit}` : "Git operation confirmed");
      refresh();
      return true;
    } catch (cause) {
      setError(String(cause));
      return false;
    } finally {
      setBusy(false);
    }
  };
  const stage = async (
    changes: GitChange[],
    unstage: boolean,
    hunk?: { id: string; version: string; generation: string }
  ) => {
    const paths = [
      ...new Set(
        changes.flatMap((change) =>
          change.originalPath ? [change.originalPath, change.path] : [change.path]
        )
      ),
    ];
    await mutate("index", {
      rootId: root.root.id,
      key: crypto.randomUUID(),
      request: {
        expectedGeneration: hunk?.generation ?? status.generation,
        paths,
        unstage,
        hunk: hunk?.id ?? null,
        diffVersion: hunk?.version ?? null,
        trustPrograms: trusted,
      },
    });
  };
  const staged = status.changes.filter((change) => change.index !== "." && change.index !== "?");
  const unstaged = status.changes.filter((change) => change.working !== ".");
  return (
    <>
      <div className="sc-status" aria-busy={loading}>
        <strong>{status.branch ?? (status.head ? "Detached HEAD" : "Unborn repository")}</strong>
        <code>{status.head?.slice(0, 12) ?? "No commit"}</code>
        {status.upstream && (
          <span>
            {status.upstream} · +{status.ahead ?? "?"} / −{status.behind ?? "?"}
          </span>
        )}
      </div>
      {(loading || observationFailed) && (
        <p role="status">
          {loading
            ? "Updating Git status. Showing the last observation."
            : "Showing the last Git observation."}
        </p>
      )}
      {root.root.read_only && (
        <InlineAlert tone="info" title="Managed worktree · inspection only">
          The agent or integration coordinator owns this worktree. Ordinary staging, commits and
          checkout are unavailable.
        </InlineAlert>
      )}
      {status.operation && (
        <InlineAlert tone="warning" title="Git recovery required">
          {status.operation}. Inspect the owning mission and preserved integration evidence.
        </InlineAlert>
      )}
      {error && (
        <InlineAlert tone="danger" title="Source Control action unavailable">
          {error}
        </InlineAlert>
      )}
      <p role="status" className="sc-notice">
        {notice}
      </p>
      <div className="sc-columns">
        <div className="sc-changes">
          <ChangeList
            title="Staged Changes"
            items={staged}
            staged
            busy={blocked}
            readOnly={root.root.read_only}
            choose={choose}
            stage={stage}
          />
          <ChangeList
            title="Changes"
            items={unstaged}
            staged={false}
            busy={blocked}
            readOnly={root.root.read_only}
            choose={choose}
            stage={stage}
          />
          {!status.changes.length && <p>Working tree clean.</p>}
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void mutate("commit", {
                rootId: root.root.id,
                key: crypto.randomUUID(),
                expectedGeneration: status.generation,
                message,
                trustPrograms: trusted,
              }).then((ok) => {
                if (ok) setMessage("");
              });
            }}
          >
            <Textarea
              label="Commit message"
              value={message}
              maxLength={8192}
              onChange={(event) => setMessage(event.target.value)}
              rows={3}
            />
            <p>{staged.length} staged paths · only index contents will be committed</p>
            <label className="sc-trust">
              <input
                type="checkbox"
                checked={trusted}
                onChange={(event) => setTrusted(event.target.checked)}
                disabled={root.root.read_only}
              />
              Authorize configured Git hooks and signing for this operation. They execute local
              programs; filters remain restricted.
            </label>
            <Button
              type="submit"
              loading={busy}
              disabled={root.root.read_only || blocked || !message.trim() || !staged.length}
            >
              Commit staged changes
            </Button>
          </form>
          <BranchHistory
            projectId={view.projectId}
            root={root}
            version={version}
            busy={blocked}
            onBranch={async (name, create) => {
              const dirty = model.documents
                .all()
                .some((document) => document.file.project_id === view.projectId && document.dirty);
              if (dirty) {
                setNotice("");
                setError(
                  "Unsaved editor buffers are preserved. Save or explicitly reconcile them before branch switching."
                );
                return;
              }
              const ok = await mutate("branches", {
                rootId: root.root.id,
                key: crypto.randomUUID(),
                expectedGeneration: status.generation,
                name,
                create,
                trustPrograms: trusted,
                dirtyBuffers: dirty,
              });
              if (ok)
                await Promise.all(
                  model.documents
                    .all()
                    .filter((document) => document.file.project_id === view.projectId)
                    .map((document) => model.documents.refresh(document.id))
                );
            }}
            onCommit={(base, target) => {
              setCommitVersion({ base, target });
              setComparison("commits");
            }}
            onPath={setPath}
          />
        </div>
        <div className="sc-detail">
          {path ? (
            <>
              <div className="sc-toolbar">
                <strong>{path}</strong>
                <Button
                  onClick={() =>
                    void dock.openFile({ project_id: view.projectId, root_id: root.root.id, path })
                  }
                >
                  Open source
                </Button>
              </div>
              {!commitVersion && (
                <Select
                  label="Git comparison"
                  value={comparison}
                  onChange={(event) => setComparison(event.target.value as Comparison)}
                >
                  <option value="working_index">Working tree vs index</option>
                  <option value="index_head">Index vs HEAD</option>
                  <option value="working_head">Working tree vs HEAD</option>
                </Select>
              )}
              <SourceDiff
                projectId={view.projectId}
                rootId={root.root.id}
                path={path}
                comparison={comparison}
                base={commitVersion?.base}
                target={commitVersion?.target}
                version={version}
                generation={status.generation}
                readOnly={root.root.read_only || blocked}
                stageHunk={async (diff, id) => {
                  await stage(
                    [
                      {
                        path,
                        originalPath: null,
                        index: ".",
                        working: "M",
                        supported: true,
                        conflict: false,
                      },
                    ],
                    comparison === "index_head",
                    { id, version: diff.version, generation: diff.generation }
                  );
                }}
              />
              {status.changes.some((change) => change.path === path && change.conflict) && (
                <ConflictInspector
                  projectId={view.projectId}
                  rootId={root.root.id}
                  path={path}
                  version={version}
                />
              )}
            </>
          ) : (
            <p>Select a changed file to inspect its actual Git versions.</p>
          )}
        </div>
      </div>
      <small>{status.observation}</small>
    </>
  );
}

function ChangeList({
  title,
  items,
  staged,
  busy,
  readOnly,
  choose,
  stage,
}: {
  title: string;
  items: GitChange[];
  staged: boolean;
  busy: boolean;
  readOnly: boolean;
  choose: (change: GitChange, staged: boolean) => void;
  stage: (changes: GitChange[], unstage: boolean) => Promise<void>;
}) {
  const [selected, setSelected] = useState<string[]>([]),
    [page, setPage] = useState(0);
  const allowed = items.filter((change) => change.supported && !change.conflict);
  const chosen = allowed.filter((change) => selected.includes(change.path));
  const currentPage = Math.min(page, Math.max(0, Math.ceil(items.length / 100) - 1));
  return (
    <details open>
      <summary>
        {title} · {items.length}
      </summary>
      <Button
        disabled={busy || readOnly || !allowed.length || allowed.length > 128}
        onClick={() => void stage(allowed, staged)}
      >
        {staged ? "Unstage" : "Stage"} all permitted
      </Button>
      <Button
        disabled={busy || readOnly || !chosen.length || chosen.length > 128}
        onClick={() => void stage(chosen, staged)}
      >
        {staged ? "Unstage" : "Stage"} selected · {chosen.length}
      </Button>
      <ul className="sc-file-list">
        {items.slice(currentPage * 100, (currentPage + 1) * 100).map((change) => (
          <li key={change.path}>
            <input
              type="checkbox"
              aria-label={`Select ${change.path} in ${title}`}
              checked={selected.includes(change.path)}
              disabled={readOnly || !change.supported || change.conflict}
              onChange={(event) =>
                setSelected((current) =>
                  event.target.checked
                    ? [...current, change.path]
                    : current.filter((path) => path !== change.path)
                )
              }
            />
            <button
              type="button"
              onClick={() => choose(change, staged)}
              disabled={!change.supported}
              title={change.originalPath ? `${change.originalPath} → ${change.path}` : change.path}
            >
              <code>{staged ? change.index : change.working}</code>
              <span>{change.path}</span>
              {change.conflict && <span>conflict</span>}
            </button>
            <Button
              tone="quiet"
              disabled={busy || readOnly || !change.supported || change.conflict}
              onClick={() => void stage([change], staged)}
              aria-label={`${staged ? "Unstage" : "Stage"} ${change.path}`}
            >
              {staged ? "−" : "+"}
            </Button>
            {!change.supported && (
              <small>Protected, linked or unsupported path; no manual mutation</small>
            )}
          </li>
        ))}
      </ul>
      {items.length > 100 && (
        <div className="sc-toolbar">
          <Button disabled={currentPage === 0} onClick={() => setPage(() => currentPage - 1)}>
            Previous files
          </Button>
          <span>
            Files {currentPage * 100 + 1}–{Math.min((currentPage + 1) * 100, items.length)} of{" "}
            {items.length}
          </span>
          <Button
            disabled={(currentPage + 1) * 100 >= items.length}
            onClick={() => setPage(() => currentPage + 1)}
          >
            Next files
          </Button>
        </div>
      )}
    </details>
  );
}

function ConflictInspector({
  projectId,
  rootId,
  path,
  version,
}: {
  projectId: string;
  rootId: string;
  path: string;
  version: number;
}) {
  const result = useSourceData<{
    base: string | null;
    ours: string | null;
    theirs: string | null;
    indexRevision: string;
    resolution: string;
  }>(projectId, `conflicts?rootId=${rootId}&path=${encodeURIComponent(path)}`, version);
  if (result.error)
    return (
      <InlineAlert tone="danger" title="Conflict evidence unavailable">
        {result.error}
      </InlineAlert>
    );
  const value = result.value;
  if (!value) return <LoadingIndicator label="Reading unmerged index stages" />;
  return (
    <section aria-label="Conflict index stages">
      <h3>Actual unmerged index stages</h3>
      <p>{value.resolution}</p>
      <code>{value.indexRevision}</code>
      <p>Merge base → stage 2 (ours)</p>
      {value.base !== null && value.ours !== null ? (
        <ComparisonView
          before={value.base}
          after={value.ours}
          leftLabel="Merge base"
          rightLabel="Ours index stage 2"
        />
      ) : (
        <p>Binary or absent index stage; complete text is unavailable.</p>
      )}
      <p>Merge base → stage 3 (theirs)</p>
      {value.base !== null && value.theirs !== null ? (
        <ComparisonView
          before={value.base}
          after={value.theirs}
          leftLabel="Merge base"
          rightLabel="Theirs index stage 3"
        />
      ) : (
        <p>Binary or absent index stage; complete text is unavailable.</p>
      )}
    </section>
  );
}

export function SourceDiff({
  projectId,
  rootId,
  path,
  comparison,
  base,
  target,
  version = 0,
  generation,
  readOnly = true,
  stageHunk,
}: {
  projectId: string;
  rootId: string;
  path: string;
  comparison: Comparison;
  base?: string;
  target?: string;
  version?: number;
  generation?: string;
  readOnly?: boolean;
  stageHunk?: (diff: GitDiff, id: string) => Promise<void>;
}) {
  const key = JSON.stringify([projectId, rootId, path, comparison, base, target, version]);
  const [state, setState] = useState<{ key: string; diff?: GitDiff; error?: string }>({ key });
  const [line, setLine] = useState<number>();
  useEffect(() => {
    const controller = new AbortController();
    void sourceRequest<GitDiff>(
      projectId,
      "diff",
      { rootId, diff: { path, comparison, base: base ?? null, target: target ?? null } },
      controller.signal
    ).then(
      (diff) => {
        if (!controller.signal.aborted) setState({ key, diff });
      },
      (error) => {
        if (!controller.signal.aborted) setState({ key, error: String(error) });
      }
    );
    return () => controller.abort();
  }, [projectId, rootId, path, comparison, base, target, key]);
  if (state.key !== key || !state.diff)
    return state.key === key && state.error ? (
      <InlineAlert tone="danger" title="Diff unavailable">
        {state.error}
      </InlineAlert>
    ) : (
      <LoadingIndicator label="Loading actual Git versions" />
    );
  const diff = state.diff,
    stale = generation !== undefined && generation !== diff.generation;
  return (
    <div className="sc-diff">
      <div className="sc-version">
        <span>Base: {diff.base}</span>
        <span>Target: {diff.target}</span>
      </div>
      {stale && <p role="status">This diff is outdated. Refresh before staging.</p>}
      {diff.limited ? (
        <InlineAlert tone="warning" title="Text comparison unavailable">
          {diff.reason}
        </InlineAlert>
      ) : (
        <ComparisonView
          before={diff.before ?? ""}
          after={diff.after ?? ""}
          line={line}
          leftLabel="Git base version"
          rightLabel="Git target version"
        />
      )}
      <div className="sc-hunks">
        {diff.hunks.map((hunk, index) => (
          <div key={hunk.id}>
            <Button onClick={() => setLine(Number(/\+(\d+)/.exec(hunk.header)?.[1] ?? 1))}>
              Hunk {index + 1}: {hunk.header}
            </Button>
            {stageHunk && (comparison === "working_index" || comparison === "index_head") && (
              <Button disabled={readOnly || stale} onClick={() => void stageHunk(diff, hunk.id)}>
                {comparison === "index_head" ? "Unstage" : "Stage"} hunk {index + 1}
              </Button>
            )}
          </div>
        ))}
      </div>
      {!diff.limited && !diff.hunks.length && (
        <p>
          Hunk staging is unavailable for empty, renamed, conflicted or unsupported text boundaries.
          Whole-file staging remains separate.
        </p>
      )}
    </div>
  );
}

function BranchHistory({
  projectId,
  root,
  version,
  busy,
  onBranch,
  onCommit,
  onPath,
}: {
  projectId: string;
  root: Worktree;
  version: number;
  busy: boolean;
  onBranch: (name: string, create: boolean) => Promise<void>;
  onCommit: (base: string, target: string) => void;
  onPath: (path: string) => void;
}) {
  const [branchesOpen, setBranchesOpen] = useState(false),
    [historyOpen, setHistoryOpen] = useState(false);
  const branches = useSourceData<GitBranch[]>(
    projectId,
    `branches?rootId=${root.root.id}`,
    version,
    branchesOpen
  );
  const [name, setName] = useState(""),
    [skip, setSkip] = useState(0),
    [selected, setSelected] = useState<GitCommit>();
  const history = useSourceData<GitCommit[]>(
    projectId,
    `history?rootId=${root.root.id}&skip=${skip}&limit=25`,
    version,
    historyOpen
  );
  const paths = useSourceData<string[]>(
    projectId,
    `commit-files?rootId=${root.root.id}&base=${selected?.parents[0] ?? ""}&target=${selected?.sha ?? ""}`,
    version,
    !!selected?.parents[0]
  );
  return (
    <>
      <details onToggle={(event) => setBranchesOpen(event.currentTarget.open)}>
        <summary>Branches</summary>
        {branches.error && <p role="alert">{branches.error}</p>}
        <ul className="sc-file-list">
          {branches.value?.map((branch) => (
            <li key={branch.name}>
              <Button
                disabled={
                  root.root.read_only ||
                  busy ||
                  branch.protected ||
                  branch.current ||
                  !!branch.worktree
                }
                onClick={() => void onBranch(branch.name, false)}
              >
                {branch.current ? "● " : ""}
                {branch.name}
              </Button>
              <small>
                {branch.upstream} {branch.tracking}
                {branch.worktree && " · checked out"}
                {branch.protected && " · coordinator owned"}
              </small>
            </li>
          ))}
        </ul>
        <Input
          label="New local branch"
          value={name}
          maxLength={200}
          onChange={(event) => setName(event.target.value)}
        />
        <Button
          disabled={busy || root.root.read_only || !name.trim()}
          onClick={() => void onBranch(name, true)}
        >
          Create and switch branch
        </Button>
      </details>
      <details onToggle={(event) => setHistoryOpen(event.currentTarget.open)}>
        <summary>Commit history</summary>
        {history.error && <p role="alert">{history.error}</p>}
        <ol className="sc-file-list">
          {history.value?.map((commit) => (
            <li key={commit.sha}>
              <Button
                onClick={() => {
                  setSelected(commit);
                  if (commit.parents[0]) onCommit(commit.parents[0], commit.sha);
                }}
              >
                <code>{commit.sha.slice(0, 9)}</code> {commit.subject}
              </Button>
              <small>
                {commit.author} · {commit.timestamp}
              </small>
            </li>
          ))}
        </ol>
        <div className="sc-toolbar">
          <Button disabled={skip === 0} onClick={() => setSkip((value) => Math.max(0, value - 25))}>
            Newer
          </Button>
          <Button
            disabled={history.value?.length !== 25}
            onClick={() => setSkip((value) => value + 25)}
          >
            Older
          </Button>
        </div>
        {selected && (
          <>
            <code>{selected.sha}</code>
            {!selected.parents.length && <p>Initial commit has no parent comparison.</p>}
            <ul>
              {paths.value?.map((path) => (
                <li key={path}>
                  <Button onClick={() => onPath(path)}>{path}</Button>
                </li>
              ))}
            </ul>
          </>
        )}
      </details>
    </>
  );
}
