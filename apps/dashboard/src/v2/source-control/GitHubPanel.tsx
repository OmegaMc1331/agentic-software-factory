import { useState } from "react";
import { Button, Input, Textarea } from "../design-system/primitives/Controls";
import { InlineAlert } from "../design-system/components/Application";
import { sourceRequest } from "./client";

interface Pull {
  updatedAt: string;
  number: number;
  title: string;
  url: string;
  state: string;
  draft: boolean;
  merged: boolean | null;
  base: string;
  head: string;
  baseRepository: string;
  headRepository: string;
  headSha: string;
  mergeable: boolean | null;
}
interface Remote {
  repository: {
    id: number;
    fullName: string;
    url: string;
    defaultBranch: string;
    permissions: { push?: boolean };
  };
  pullRequests: { items: Pull[]; page: number; mayHaveMore: boolean };
  observedAt: string;
}
interface Detail {
  page: number;
  pullRequest: Pull;
  body: string;
  files: { filename: string; status: string; additions: number; deletions: number }[];
  reviews: { id: number; state: string; body: string; user: { login: string } }[];
  comments: { id: number; path: string; line: number | null; body: string }[];
  checks: {
    check_runs: { id: number; name: string; status: string; conclusion: string | null }[];
  } | null;
  checksError: string | null;
  statuses: { id: number; context: string; state: string }[] | null;
  statusesError: string | null;
  complete: boolean;
  pagination: string;
}
interface Preview {
  repository: Remote["repository"];
  preview: {
    repository: string;
    base: string;
    head: string;
    title: string;
    body: string;
    draft: boolean;
    eligible: boolean;
    blockers: string[];
  };
  delivery: {
    integrationHead: string | null;
    state: string;
    pullRequest?: { url: string; number: number } | null;
  };
  changedFiles: string[];
  changedFilesComplete: boolean;
  comparisonBase: string | null;
  baseSource: string;
  authorizationBlocker: string | null;
  externalActions: string[];
  publishedByPreview: false;
}

export function GitHubPanel({ projectId, runId }: { projectId: string; runId?: number }) {
  const [remote, setRemote] = useState<Remote>(),
    [detail, setDetail] = useState<Detail>(),
    [page, setPage] = useState(1);
  const [preview, setPreview] = useState<Preview>(),
    [run, setRun] = useState(runId?.toString() ?? "");
  const [title, setTitle] = useState(""),
    [body, setBody] = useState(""),
    [draft, setDraft] = useState(false),
    [authorize, setAuthorize] = useState(false);
  const [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false);
  const perform = async (action: () => Promise<void>) => {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };
  const load = (next = page) =>
    perform(async () => {
      setRemote(await sourceRequest<Remote>(projectId, `github?page=${next}`));
      setPage(next);
    });
  const loadPreview = () =>
    perform(async () => {
      const next = await sourceRequest<Preview>(projectId, `delivery/${Number(run)}`);
      setPreview(next);
      setTitle(next.preview.title);
      setBody(next.preview.body);
      setDraft(next.preview.draft);
      setAuthorize(false);
    });
  const publish = () =>
    perform(async () => {
      if (!preview || !authorize || !preview.delivery.integrationHead) return;
      if (preview.delivery.pullRequest) {
        if (detail?.pullRequest.number !== preview.delivery.pullRequest.number)
          throw new Error("Read the recorded published PR before authorizing its update.");
        await sourceRequest(projectId, `delivery/${Number(run)}/update`, {
          key: crypto.randomUUID(),
          authorize,
          repositoryId: preview.repository.id,
          expectedIntegration: preview.delivery.integrationHead,
          expectedRemoteUpdatedAt: detail.pullRequest.updatedAt,
          title,
          body,
          draft,
        });
        setAuthorize(false);
        setNotice("Remote Pull Request update verified. No branch history was rewritten.");
        return;
      }
      const result = await sourceRequest<{
        started?: boolean;
        delivery?: { state: string; pull_request?: { url: string } };
      }>(projectId, `delivery/${Number(run)}`, {
        key: crypto.randomUUID(),
        authorize,
        repository: preview.repository.fullName,
        repositoryId: preview.repository.id,
        expectedIntegration: preview.delivery.integrationHead,
        base: preview.preview.base,
        head: preview.preview.head,
        title,
        body,
        draft,
      });
      setAuthorize(false);
      setNotice(
        `Delivery request recorded: ${result.delivery?.state ?? "inspect durable delivery status"}. Publication is confirmed only by the remote result.`
      );
    });
  return (
    <section className="sc-remote" aria-label="GitHub delivery">
      <div className="sc-toolbar">
        <h3>GitHub</h3>
        <Button loading={busy} onClick={() => void load()}>
          Read authenticated remote
        </Button>
      </div>
      <p>
        Remote reads are explicit and bounded. Local review is separate from GitHub review. Opening
        this panel does not publish.
      </p>
      {error && (
        <InlineAlert tone="danger" title="GitHub operation unavailable">
          {error}
        </InlineAlert>
      )}
      <p role="status">{notice}</p>
      {remote && (
        <>
          <p>
            <a href={remote.repository.url} target="_blank" rel="noreferrer">
              {remote.repository.fullName}
            </a>{" "}
            · repository ID {remote.repository.id} · default {remote.repository.defaultBranch}
          </p>
          <small>Observed {remote.observedAt}</small>
          <ul className="sc-file-list">
            {remote.pullRequests.items.map((pull) => (
              <li key={pull.number}>
                <Button
                  onClick={() =>
                    void perform(async () =>
                      setDetail(
                        await sourceRequest<Detail>(projectId, `github/pulls/${pull.number}`)
                      )
                    )
                  }
                >
                  #{pull.number} {pull.title} · {pull.merged ? "merged" : pull.state}
                  {pull.draft ? " · draft" : ""}
                </Button>
              </li>
            ))}
          </ul>
          <Button disabled={busy || page === 1} onClick={() => void load(page - 1)}>
            Previous PR page
          </Button>
          <Button
            disabled={busy || !remote.pullRequests.mayHaveMore}
            onClick={() => void load(page + 1)}
          >
            Next PR page
          </Button>
        </>
      )}
      {detail && (
        <article>
          <h3>Pull Request #{detail.pullRequest.number}</h3>
          <p>{detail.pullRequest.title}</p>
          <a href={detail.pullRequest.url} target="_blank" rel="noreferrer">
            Open remote Pull Request
          </a>
          <p>
            {detail.pullRequest.baseRepository}:{detail.pullRequest.base} ←{" "}
            {detail.pullRequest.headRepository}:{detail.pullRequest.head}
          </p>
          <code>{detail.pullRequest.headSha}</code>
          <p>
            Mergeable:{" "}
            {detail.pullRequest.mergeable === null
              ? "unknown"
              : String(detail.pullRequest.mergeable)}
          </p>
          <pre>{detail.body}</pre>
          <h3>Changed files</h3>
          <ul>
            {detail.files.map((file) => (
              <li key={file.filename}>
                {file.filename} · {file.status} · +{file.additions} / −{file.deletions}
              </li>
            ))}
          </ul>
          <h3>Remote checks</h3>
          {detail.checksError && <p role="status">Checks unavailable: {detail.checksError}</p>}
          <ul>
            {detail.checks?.check_runs.map((check) => (
              <li key={check.id}>
                {check.name} · {check.status} · {check.conclusion ?? "pending / unknown"}
              </li>
            ))}
          </ul>
          {detail.statusesError && <p>Statuses unavailable: {detail.statusesError}</p>}
          <ul>
            {detail.statuses?.map((status) => (
              <li key={status.id}>
                {status.context} · {status.state}
              </li>
            ))}
          </ul>
          <h3>Remote reviews</h3>
          {detail.reviews.map((review) => (
            <div key={review.id}>
              <strong>
                {review.user.login} · {review.state}
              </strong>
              <p>{review.body}</p>
            </div>
          ))}
          {detail.comments.map((comment) => (
            <div key={comment.id}>
              <strong>
                {comment.path}:{comment.line ?? "outdated"}
              </strong>
              <p>{comment.body}</p>
            </div>
          ))}
          <p>
            <small>
              Evidence page {detail.page ?? 1}. {detail.pagination}
            </small>
          </p>
          <Button
            disabled={busy || (detail.page ?? 1) === 1}
            onClick={() =>
              void perform(async () =>
                setDetail(
                  await sourceRequest<Detail>(
                    projectId,
                    `github/pulls/${detail.pullRequest.number}?page=${(detail.page ?? 1) - 1}`
                  )
                )
              )
            }
          >
            Previous evidence page
          </Button>
          <Button
            disabled={busy || (detail.page ?? 1) >= 1000}
            onClick={() =>
              void perform(async () =>
                setDetail(
                  await sourceRequest<Detail>(
                    projectId,
                    `github/pulls/${detail.pullRequest.number}?page=${(detail.page ?? 1) + 1}`
                  )
                )
              )
            }
          >
            Next evidence page
          </Button>
        </article>
      )}
      <h3>Delivery preview</h3>
      <Input
        label="Mission ID for delivery"
        type="number"
        min={1}
        value={run}
        onChange={(event) => {
          setRun(event.target.value);
          setPreview(undefined);
          setAuthorize(false);
        }}
      />
      <Button disabled={busy || !Number(run)} onClick={() => void loadPreview()}>
        Preview delivery · no publication
      </Button>
      {preview && (
        <>
          <p>
            {preview.repository.fullName} · {preview.preview.base} ← {preview.preview.head}
          </p>
          <code>{preview.delivery.integrationHead ?? "No integration commit"}</code>
          {preview.delivery.pullRequest && (
            <Button
              onClick={() =>
                void perform(async () => {
                  const current = await sourceRequest<Detail>(
                    projectId,
                    `github/pulls/${preview.delivery.pullRequest!.number}`
                  );
                  setDetail(current);
                  setTitle(current.pullRequest.title);
                  setBody(current.body);
                  setDraft(current.pullRequest.draft);
                  setAuthorize(false);
                })
              }
            >
              Read recorded PR for update
            </Button>
          )}
          <p>Recorded delivery: {preview.delivery.state}</p>
          <p>
            Comparison base: {preview.comparisonBase ?? "Unavailable"} · {preview.baseSource}
          </p>
          {!preview.changedFilesComplete && (
            <InlineAlert tone="warning" title="Change list unavailable">
              The recorded integration has no verified comparison base.
            </InlineAlert>
          )}
          <ul>
            {preview.changedFiles.map((path) => (
              <li key={path}>{path}</li>
            ))}
          </ul>
          {preview.authorizationBlocker && (
            <InlineAlert tone="warning" title="Publication blocked">
              {preview.authorizationBlocker}
            </InlineAlert>
          )}
          <ul>
            {preview.preview.blockers.map((blocker) => (
              <li key={blocker}>{blocker}</li>
            ))}
          </ul>
          <Input
            label="Pull Request title"
            value={title}
            maxLength={256}
            onChange={(event) => setTitle(event.target.value)}
          />
          <Textarea
            label="Pull Request description"
            value={body}
            maxLength={48_000}
            rows={6}
            onChange={(event) => setBody(event.target.value)}
          />
          <label className="sc-trust">
            <input
              type="checkbox"
              checked={draft}
              onChange={(event) => setDraft(event.target.checked)}
            />
            Create as draft
          </label>
          <ul>
            {preview.externalActions.map((action) => (
              <li key={action}>{action}</li>
            ))}
          </ul>
          <label className="sc-trust">
            <input
              type="checkbox"
              checked={authorize}
              onChange={(event) => setAuthorize(event.target.checked)}
              disabled={!!preview.authorizationBlocker || !preview.preview.eligible}
            />
            I authorize these external actions against {preview.repository.fullName}, branch{" "}
            {preview.preview.head}, commit {preview.delivery.integrationHead?.slice(0, 12)}.
          </label>
          <Button
            disabled={
              busy || !authorize || !preview.preview.eligible || !!preview.authorizationBlocker
            }
            onClick={() => void publish()}
          >
            {preview.delivery.pullRequest
              ? "Update authorized Pull Request"
              : "Publish authorized delivery"}
          </Button>
        </>
      )}
    </section>
  );
}
