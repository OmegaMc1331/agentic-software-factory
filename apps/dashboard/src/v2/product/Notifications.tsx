import { useEffect, useRef, useState } from "react";
import { request } from "../../api";
import { projectPath } from "../projects/projectApi";
import { Button, Checkbox } from "../design-system/primitives/Controls";
import { EmptyState, InlineAlert, LoadingIndicator } from "../design-system/components/Application";
import { refreshData, useData } from "../workspace/production/data";
import type { WorkspaceModel } from "../workspace/production/controller";
import { usePreferences } from "./preferences";
import "./product.css";

export interface ProductNotification {
  id: number;
  revision: number;
  source_event_id: string;
  event_type: string;
  category: string;
  severity: "info" | "success" | "warning" | "danger";
  title: string;
  message: string;
  run_id: number | null;
  task_id: number | null;
  target: "mission" | "review" | "settings";
  created_at: string;
  read_at: string | null;
  dismissed_at: string | null;
}
export interface NotificationPage {
  project_id: string;
  items: ProductNotification[];
  unread: number;
  next_cursor: number | null;
  retention_limit: number;
}
export const notificationPath = "notifications?limit=50";
export function NotificationSignals({
  projectId,
  onUnread,
  onArrival,
}: {
  projectId: string;
  onUnread: (project: string, count: number | undefined) => void;
  onArrival: (project: string, value: ProductNotification) => void;
}) {
  const feed = useData<NotificationPage>(projectId, notificationPath);
  const last = useRef<number>();
  const preferences = usePreferences();
  const callbacks = useRef({ onUnread, onArrival, preferences: preferences.value?.preferences });
  callbacks.current = { onUnread, onArrival, preferences: preferences.value?.preferences };
  useEffect(() => {
    const page = feed.value;
    if (feed.error || !page || page.project_id !== projectId) {
      callbacks.current.onUnread(projectId, undefined);
      return;
    }
    callbacks.current.onUnread(projectId, page.unread);
    if (last.current !== undefined) {
      const fresh = page.items.filter((n) => n.id > last.current! && !n.dismissed_at && !n.read_at);
      const newest = fresh[0];
      if (newest) {
        if (callbacks.current.preferences?.notification_toasts)
          callbacks.current.onArrival(projectId, newest);
        if (
          callbacks.current.preferences?.browser_notifications &&
          typeof Notification !== "undefined" &&
          Notification.permission === "granted"
        ) {
          try {
            new Notification("ASF project event", {
              body: "Open ASF to inspect the recorded event and any required action.",
              tag: `asf:${projectId}:${newest.id}`,
            });
          } catch {
            last.current = Math.max(last.current ?? 0, newest.id);
          }
        }
      }
    }
    last.current = Math.max(last.current ?? 0, ...page.items.map((n) => n.id));
  }, [feed.value, feed.error, projectId]);
  return null;
}
export function NotificationCenter({
  projectId,
  model,
}: {
  projectId: string;
  model: WorkspaceModel;
}) {
  const [archived, setArchived] = useState(false),
    [before, setBefore] = useState<number>(),
    [busy, setBusy] = useState<number>(),
    [error, setError] = useState("");
  const path =
    notificationPath + (archived ? "&archived=true" : "") + (before ? `&before=${before}` : "");
  const page = useData<NotificationPage>(projectId, path);
  const confirmed = page.value?.project_id === projectId ? page.value : undefined;
  const items = confirmed?.items ?? [];
  const update = async (n: ProductNotification, read: boolean, dismissed: boolean) => {
    setBusy(n.id);
    setError("");
    try {
      await request(projectPath(projectId, `notifications/${n.id}`), {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ revision: n.revision, read, dismissed }),
      });
      await Promise.all([refreshData(projectId, path), refreshData(projectId, notificationPath)]);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(undefined);
    }
  };
  const open = async (n: ProductNotification) => {
    setError("");
    try {
      if (!model.navigateResource)
        throw new Error("Workspace navigation is unavailable. Reopen this view.");
      await model.navigateResource({
        projectId,
        kind: n.target === "review" ? "review" : n.run_id ? "workflow" : "settings",
        resourceId: n.run_id ?? undefined,
      });
      if (!n.read_at) await update(n, true, !!n.dismissed_at);
    } catch (e) {
      setError(String(e));
    }
  };
  return (
    <section className="product-notifications" aria-label="Notification center">
      <header>
        <h2>Notifications</h2>
        <p>
          Project <code>{projectId}</code> · {confirmed?.unread ?? "Unknown"} unread
        </p>
        <div className="product-actions">
          <Button
            onClick={() => {
              setBefore(undefined);
              void refreshData(projectId, path);
            }}
          >
            Refresh notifications
          </Button>
          <Checkbox
            label="Include dismissed notifications"
            checked={archived}
            onChange={(e) => {
              setArchived(e.target.checked);
              setBefore(undefined);
            }}
          />
        </div>
      </header>
      {page.value && !confirmed ? (
        <InlineAlert tone="danger" title="Notification scope mismatch">
          This response belongs to another project. No entries or counts are displayed.
        </InlineAlert>
      ) : null}
      {error || page.error ? (
        <InlineAlert tone="danger" title="Notifications need attention">
          {error || page.error} Cached entries are last confirmed data.
        </InlineAlert>
      ) : null}
      {!page.value && !page.error ? (
        <LoadingIndicator label="Loading recorded notifications" />
      ) : null}
      {confirmed && !items.length ? (
        <EmptyState
          title="No notifications on this page"
          description="Important runtime events appear here after they are recorded. Navigation and restoration do not start missions or fabricate events."
        />
      ) : null}
      <ol className="product-notification-list">
        {items.map((n) => (
          <li key={n.id} data-severity={n.severity} data-unread={!n.read_at}>
            <div>
              <strong>{n.title}</strong>
              <span>
                {n.category} · {n.severity} · {n.read_at ? "Read" : "Unread"}
                {n.dismissed_at ? " · Dismissed" : ""}
              </span>
              <time dateTime={n.created_at}>{new Date(n.created_at).toLocaleString()}</time>
            </div>
            <p>{n.message}</p>
            <p className="product-muted">
              {n.run_id ? `Mission #${n.run_id}` : "Project service"}
              {n.task_id ? ` · Task #${n.task_id}` : ""}
            </p>
            <div className="product-actions">
              <Button disabled={busy !== undefined} onClick={() => void open(n)}>
                Open related {n.run_id ? "mission" : "settings"}
              </Button>
              <Button
                disabled={busy !== undefined}
                onClick={() => void update(n, !n.read_at, !!n.dismissed_at)}
              >
                {n.read_at ? "Mark unread" : "Mark read"}
              </Button>
              <Button
                disabled={busy !== undefined}
                onClick={() => void update(n, !!n.read_at, !n.dismissed_at)}
              >
                {n.dismissed_at ? "Restore notification" : "Dismiss notification"}
              </Button>
            </div>
            <details>
              <summary>Event provenance</summary>
              <p>
                {n.source_event_id} · {n.event_type} · revision {n.revision}
              </p>
            </details>
          </li>
        ))}
      </ol>
      <div className="product-actions">
        <Button disabled={!before} onClick={() => setBefore(undefined)}>
          Latest notifications
        </Button>
        <Button
          disabled={!confirmed?.next_cursor}
          onClick={() => setBefore(confirmed?.next_cursor ?? undefined)}
        >
          Earlier notifications
        </Button>
      </div>
      <p className="product-muted">
        The latest {confirmed?.retention_limit ?? 1000} notifications are retained per project.
        Dismissal preserves the record until retention expires; original execution evidence has its
        own lifecycle.
      </p>
    </section>
  );
}
