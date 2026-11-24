import {
  Component,
  useRef,
  useLayoutEffect,
  type ButtonHTMLAttributes,
  type CSSProperties,
  type ReactNode,
} from "react";
import { Button, IconButton } from "../primitives/Controls";
import { Tooltip } from "../primitives/Overlays";
import { Icon, type IconName } from "../icons/Icon";

export type Status =
  "idle" | "running" | "completed" | "failed" | "suspended" | "attention" | "cancelled";
const statusIcons: Record<Status, IconName> = {
  idle: "clock",
  running: "play",
  completed: "check",
  failed: "error",
  suspended: "pause",
  attention: "warning",
  cancelled: "stop",
};
const statusLabels: Record<Status, string> = {
  idle: "Ready",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  suspended: "Suspended",
  attention: "Attention required",
  cancelled: "Cancelled",
};

export function CompactStatus({ status, label }: { status: Status; label?: string }) {
  return (
    <span className={`ds-status ds-status--${status}`}>
      <Icon name={statusIcons[status]} size={13} />
      {label ?? statusLabels[status]}
    </span>
  );
}

export function ActivityBarItem({
  icon,
  label,
  selected,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  icon: IconName;
  label: string;
  selected?: boolean;
}) {
  return (
    <Tooltip content={label}>
      <IconButton
        {...props}
        icon={icon}
        label={label}
        tone="quiet"
        aria-pressed={!!selected}
        className="ds-activity-item"
      />
    </Tooltip>
  );
}

export function WorkspaceTab({
  id,
  label,
  selected,
  dirty,
  onSelect,
  onClose,
  closeLabel = "close",
  onMove,
}: {
  id: string;
  label: string;
  selected: boolean;
  dirty?: boolean;
  onSelect(): void;
  onClose?(): void;
  closeLabel?: string;
  onMove?(direction: "left" | "right"): void;
}) {
  return (
    <div className="ds-workspace-tab" data-selected={selected}>
      <button
        type="button"
        role="tab"
        id={id}
        aria-selected={selected}
        tabIndex={selected ? 0 : -1}
        title={onClose ? `${label} · Delete to ${closeLabel}` : label}
        onClick={onSelect}
        onKeyDown={(event) => {
          if (event.key === "Delete" && onClose) {
            event.preventDefault();
            onClose();
          }
          if (event.altKey && (event.key === "ArrowLeft" || event.key === "ArrowRight") && onMove) {
            event.preventDefault();
            onMove(event.key === "ArrowLeft" ? "left" : "right");
          }
        }}
      >
        <span>{label}</span>
        {dirty && <span className="ds-dirty" aria-label="Unsaved changes" />}
      </button>
    </div>
  );
}

export function PanelToolbar({
  label = "Panel actions",
  children,
}: {
  label?: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    ref.current
      ?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")
      .forEach((button, index) => {
        button.tabIndex = index === 0 ? 0 : -1;
      });
  }, [children]);
  return (
    <div
      ref={ref}
      className="ds-panel-toolbar"
      role="toolbar"
      aria-label={label}
      onFocusCapture={() => {
        ref.current
          ?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")
          .forEach((button) => {
            button.tabIndex = button === document.activeElement ? 0 : -1;
          });
      }}
      onKeyDown={(event) => {
        if (!["ArrowRight", "ArrowLeft", "Home", "End"].includes(event.key)) return;
        const buttons = [
          ...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []),
        ];
        if (!buttons.length) return;
        event.preventDefault();
        const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next =
          event.key === "Home"
            ? 0
            : event.key === "End"
              ? buttons.length - 1
              : (current + (event.key === "ArrowRight" ? 1 : buttons.length - 1)) % buttons.length;
        buttons[next].focus();
      }}
    >
      {children}
    </div>
  );
}

export function PanelHeader({
  title,
  detail,
  children,
}: {
  title: string;
  detail?: string;
  children?: ReactNode;
}) {
  return (
    <header className="ds-panel-header">
      <div>
        <h2>{title}</h2>
        {detail && <span>{detail}</span>}
      </div>
      {children && <PanelToolbar label={`${title} actions`}>{children}</PanelToolbar>}
    </header>
  );
}

export function Breadcrumb({
  items,
  label = "File path",
}: {
  items: { label: string; onClick?: () => void }[];
  label?: string;
}) {
  return (
    <nav aria-label={label} className="ds-breadcrumb">
      <ol>
        {items.map((item, index) => (
          <li key={`${index}-${item.label}`}>
            {index > 0 && <Icon name="chevron" size={11} />}
            {item.onClick ? (
              <button type="button" onClick={item.onClick}>
                {item.label}
              </button>
            ) : (
              <span title={item.label}>{item.label}</span>
            )}
          </li>
        ))}
      </ol>
    </nav>
  );
}

export function FileTreeItem({
  path,
  label,
  depth = 0,
  directory,
  expanded,
  selected,
  focused,
  onSelect,
  id,
  position,
  total,
}: {
  path: string;
  label: string;
  depth?: number;
  directory?: boolean;
  expanded?: boolean;
  selected?: boolean;
  focused?: boolean;
  onSelect(): void;
  id?: string;
  position?: number;
  total?: number;
}) {
  return (
    <button
      type="button"
      id={id}
      role="treeitem"
      className="ds-tree-item"
      style={{ "--tree-depth": depth } as CSSProperties}
      tabIndex={-1}
      aria-label={path}
      aria-level={depth + 1}
      aria-selected={!!selected}
      data-focused={focused || undefined}
      aria-expanded={directory ? !!expanded : undefined}
      aria-posinset={position}
      aria-setsize={total}
      title={path}
      onClick={onSelect}
    >
      {directory ? (
        <Icon name={expanded ? "down" : "chevron"} size={12} />
      ) : (
        <span className="ds-tree-spacer" />
      )}
      <Icon name={directory ? "folder" : "file"} size={14} />
      <span>{label}</span>
      {path.endsWith(".rs") && <small>rs</small>}
    </button>
  );
}

export function ResizableDivider({
  label,
  value,
  min,
  max,
  onChange,
  controls,
  orientation = "vertical",
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  onChange(value: number): void;
  controls: string;
  orientation?: "vertical" | "horizontal";
}) {
  const drag = useRef<{ position: number; value: number }>();
  const clamp = (next: number) => Math.max(min, Math.min(max, next));
  return (
    <div
      role="separator"
      tabIndex={0}
      className={`ds-divider ds-divider--${orientation}`}
      aria-label={label}
      aria-orientation={orientation}
      aria-controls={controls}
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      onKeyDown={(event) => {
        const positive = orientation === "vertical" ? "ArrowRight" : "ArrowDown";
        const negative = orientation === "vertical" ? "ArrowLeft" : "ArrowUp";
        if ([positive, negative, "Home", "End"].includes(event.key)) {
          event.preventDefault();
          onChange(
            event.key === "Home"
              ? min
              : event.key === "End"
                ? max
                : clamp(value + (event.key === positive ? 1 : -1) * (event.shiftKey ? 32 : 8))
          );
        }
      }}
      onPointerDown={(event) => {
        drag.current = {
          position: orientation === "vertical" ? event.clientX : event.clientY,
          value,
        };
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (drag.current)
          onChange(
            clamp(
              drag.current.value +
                (orientation === "vertical" ? event.clientX : event.clientY) -
                drag.current.position
            )
          );
      }}
      onPointerUp={() => {
        drag.current = undefined;
      }}
      onPointerCancel={() => {
        drag.current = undefined;
      }}
    />
  );
}

export function StatusBarItem({
  icon,
  children,
  onClick,
}: {
  icon?: IconName;
  children: ReactNode;
  onClick?: () => void;
}) {
  const content = (
    <>
      {icon && <Icon name={icon} size={13} />}
      {children}
    </>
  );
  return onClick ? (
    <button type="button" className="ds-statusbar-item" onClick={onClick}>
      {content}
    </button>
  ) : (
    <span className="ds-statusbar-item">{content}</span>
  );
}

export type NotificationData = {
  id: string;
  title: string;
  detail: string;
  severity: "info" | "warning" | "success" | "danger";
  createdAt: string;
};
export function NotificationItem({
  item,
  onDismiss,
}: {
  item: NotificationData;
  onDismiss(id: string): void;
}) {
  return (
    <article className={`ds-notification ds-tone--${item.severity}`}>
      <Icon
        name={
          item.severity === "success"
            ? "check"
            : item.severity === "warning"
              ? "warning"
              : item.severity === "danger"
                ? "error"
                : "info"
        }
      />
      <div>
        <h3>{item.title}</h3>
        <p>{item.detail}</p>
        <time>{item.createdAt}</time>
      </div>
      <IconButton
        label={`Dismiss ${item.title}`}
        tone="quiet"
        icon="close"
        onClick={() => onDismiss(item.id)}
      />
    </article>
  );
}

export function EmptyState({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="ds-empty">
      <h3>{title}</h3>
      <p>{description}</p>
      {action}
    </div>
  );
}

export function LoadingIndicator({ label = "Loading" }: { label?: string }) {
  return (
    <span className="ds-loading" role="status">
      <span className="ds-busy-mark" aria-hidden="true" />
      {label}
    </span>
  );
}

export function ErrorBoundaryView({
  title = "This view could not be displayed",
  message,
  onRetry,
}: {
  title?: string;
  message: string;
  onRetry(): void;
}) {
  return (
    <div className="ds-error-view" role="alert">
      <Icon name="error" size={20} />
      <h3>{title}</h3>
      <p>{message}</p>
      <Button onClick={onRetry}>Reload view</Button>
    </div>
  );
}

export class ViewErrorBoundary extends Component<
  { children: ReactNode; onRetry?(): void },
  { error: string }
> {
  state = { error: "" };
  static getDerivedStateFromError(error: Error) {
    return { error: error.message };
  }
  render() {
    return this.state.error ? (
      <ErrorBoundaryView
        message={this.state.error}
        onRetry={() => {
          this.props.onRetry?.();
          this.setState({ error: "" });
        }}
      />
    ) : (
      this.props.children
    );
  }
}

export function InlineAlert({
  tone = "info",
  title,
  children,
  onDismiss,
}: {
  tone?: "info" | "warning" | "danger" | "success";
  title: string;
  children?: ReactNode;
  onDismiss?(): void;
}) {
  return (
    <div
      className={`ds-inline-alert ds-tone--${tone}`}
      role={tone === "danger" ? "alert" : "status"}
    >
      <Icon
        name={
          tone === "danger"
            ? "error"
            : tone === "warning"
              ? "warning"
              : tone === "success"
                ? "check"
                : "info"
        }
      />
      <div>
        <strong>{title}</strong>
        {children && <p>{children}</p>}
      </div>
      {onDismiss && (
        <IconButton label={`Dismiss ${title}`} icon="close" tone="quiet" onClick={onDismiss} />
      )}
    </div>
  );
}

export function ConfirmationPrompt({
  title,
  description,
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  title: string;
  description: string;
  confirmLabel: string;
  onConfirm(): void;
  onCancel(): void;
}) {
  return (
    <section className="ds-confirmation">
      <h3>{title}</h3>
      <p>{description}</p>
      <div className="ds-inline-actions">
        <Button tone="primary" onClick={onConfirm}>
          {confirmLabel}
        </Button>
        <Button tone="quiet" onClick={onCancel}>
          Keep current state
        </Button>
      </div>
    </section>
  );
}

export function ProgressRow({
  title,
  state,
  detail,
}: {
  title: string;
  state: Status;
  detail?: string;
}) {
  return (
    <li className="ds-progress-row" aria-current={state === "running" ? "step" : undefined}>
      <span className={`ds-status ds-status--${state}`}>
        <Icon name={statusIcons[state]} size={14} />
      </span>
      <div>
        <span>{title}</span>
        {detail && <small>{detail}</small>}
      </div>
      <span className="ds-sr-only">{statusLabels[state]}</span>
    </li>
  );
}
