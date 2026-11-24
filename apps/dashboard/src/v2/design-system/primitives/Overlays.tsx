import {
  cloneElement,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { Button, IconButton } from "./Controls";
import { Icon, type IconName } from "../icons/Icon";

type Anchor = { x: number; y: number; width?: number; height?: number };
function overlayContainer(source?: Element | null) {
  return source?.closest("dialog[open]") ?? document.body;
}
export type MenuEntry = {
  id: string;
  label: string;
  icon?: IconName;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  action(): void;
};

function usePosition(ref: RefObject<HTMLElement>, anchor: Anchor) {
  const [position, setPosition] = useState({
    left: anchor.x,
    top: anchor.y + (anchor.height ?? 0) + 6,
  });
  useLayoutEffect(() => {
    const box = ref.current?.getBoundingClientRect();
    if (box)
      setPosition({
        left: Math.max(8, Math.min(anchor.x, window.innerWidth - box.width - 8)),
        top: Math.max(
          8,
          Math.min(anchor.y + (anchor.height ?? 0) + 6, window.innerHeight - box.height - 8)
        ),
      });
  }, [anchor.x, anchor.y, anchor.height, ref]);
  return position;
}

export function Menu({
  label,
  entries,
  anchor,
  onClose,
  returnFocus,
}: {
  label: string;
  entries: MenuEntry[];
  anchor: Anchor;
  onClose(): void;
  returnFocus?: HTMLElement | null;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const position = usePosition(ref, anchor);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    const menu = ref.current;
    menu?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    const outside = (event: PointerEvent) => {
      if (!menu?.contains(event.target as Node)) closeRef.current();
    };
    const resize = () => closeRef.current();
    document.addEventListener("pointerdown", outside);
    window.addEventListener("resize", resize);
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("resize", resize);
      if (returnFocus?.isConnected) returnFocus.focus();
    };
  }, [returnFocus]);
  return createPortal(
    <div
      ref={ref}
      className="ds-menu ds-overlay-root"
      role="menu"
      aria-label={label}
      style={position}
      onKeyDown={(event) => {
        const buttons = [
          ...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []),
        ];
        const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next =
          event.key === "ArrowDown"
            ? (index + 1) % buttons.length
            : event.key === "ArrowUp"
              ? (index + buttons.length - 1) % buttons.length
              : event.key === "Home"
                ? 0
                : event.key === "End"
                  ? buttons.length - 1
                  : -1;
        if (next >= 0) {
          event.preventDefault();
          buttons[next]?.focus();
        } else if (event.key === "Escape" || event.key === "Tab") {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        } else if (event.key.length === 1 && !event.ctrlKey && !event.metaKey)
          buttons
            .find((button) => button.textContent?.toLowerCase().startsWith(event.key.toLowerCase()))
            ?.focus();
      }}
    >
      {entries.map((entry) => (
        <button
          key={entry.id}
          type="button"
          role="menuitem"
          aria-label={entry.label}
          aria-keyshortcuts={entry.shortcut}
          disabled={entry.disabled}
          data-danger={entry.danger || undefined}
          onClick={() => {
            onClose();
            entry.action();
          }}
        >
          {entry.icon && <Icon name={entry.icon} />}
          <span>{entry.label}</span>
          {entry.shortcut && <kbd>{entry.shortcut}</kbd>}
        </button>
      ))}
    </div>,
    overlayContainer(returnFocus)
  );
}

export function Dropdown({
  label,
  entries,
  icon,
  tone = "quiet",
}: {
  label: string;
  entries: MenuEntry[];
  icon?: IconName;
  tone?: "quiet" | "neutral";
}) {
  const [anchor, setAnchor] = useState<Anchor>();
  const trigger = useRef<HTMLButtonElement>(null);
  return (
    <>
      <Button
        ref={trigger}
        tone={tone}
        icon={icon}
        aria-haspopup="menu"
        aria-expanded={!!anchor}
        onClick={() => {
          const box = trigger.current!.getBoundingClientRect();
          setAnchor(anchor ? undefined : box);
        }}
      >
        {label}
        <Icon name="down" size={12} />
      </Button>
      {anchor && (
        <Menu
          label={label}
          entries={entries}
          anchor={anchor}
          onClose={() => setAnchor(undefined)}
          returnFocus={trigger.current}
        />
      )}
    </>
  );
}

export function ContextMenu({
  label,
  entries,
  children,
}: {
  label: string;
  entries: MenuEntry[];
  children: ReactNode;
}) {
  const [anchor, setAnchor] = useState<Anchor>();
  const source = useRef<HTMLElement | null>(null);
  return (
    <div
      className="ds-context-target"
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
        source.current =
          event.target instanceof HTMLElement
            ? event.target.closest<HTMLElement>("button,[tabindex]")
            : null;
        setAnchor({ x: event.clientX, y: event.clientY });
      }}
      onKeyDown={(event) => {
        if (event.shiftKey && event.key === "F10") {
          event.preventDefault();
          source.current = document.activeElement as HTMLElement;
          const rect = event.currentTarget.getBoundingClientRect();
          setAnchor(rect);
        }
      }}
    >
      {children}
      {anchor && (
        <Menu
          label={label}
          entries={entries}
          anchor={anchor}
          onClose={() => setAnchor(undefined)}
          returnFocus={source.current}
        />
      )}
    </div>
  );
}

export function Tooltip({
  content,
  children,
}: {
  content: string;
  children: ReactElement<{ "aria-describedby"?: string }>;
}) {
  const id = useId();
  const [anchor, setAnchor] = useState<Anchor>();
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const host = useRef<HTMLSpanElement>(null);
  const open = () => {
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setAnchor(host.current?.getBoundingClientRect()), 300);
  };
  const close = () => {
    clearTimeout(timer.current);
    setAnchor(undefined);
  };
  useEffect(() => () => clearTimeout(timer.current), []);
  useEffect(() => {
    if (!anchor) return;
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setAnchor(undefined);
      }
    };
    document.addEventListener("keydown", key, true);
    return () => document.removeEventListener("keydown", key, true);
  }, [anchor]);
  return (
    <span
      ref={host}
      className="ds-tooltip-anchor"
      onMouseEnter={open}
      onMouseLeave={close}
      onFocus={open}
      onBlur={close}
      onKeyDown={(event) => {
        if (event.key === "Escape") close();
      }}
    >
      {cloneElement(children, { "aria-describedby": anchor ? id : undefined })}
      {anchor &&
        createPortal(
          <TooltipBody id={id} anchor={anchor}>
            {content}
          </TooltipBody>,
          overlayContainer(host.current)
        )}
    </span>
  );
}

function TooltipBody({
  id,
  anchor,
  children,
}: {
  id: string;
  anchor: Anchor;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const position = usePosition(ref, anchor);
  return (
    <div ref={ref} id={id} role="tooltip" className="ds-tooltip ds-overlay-root" style={position}>
      {children}
    </div>
  );
}

export function Dialog({
  open,
  onClose,
  title,
  description,
  children,
  footer,
  alert = false,
}: {
  open: boolean;
  onClose(): void;
  title: string;
  description?: string;
  children?: ReactNode;
  footer?: ReactNode;
  alert?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  useEffect(() => {
    if (!open) return;
    const element = ref.current!;
    const previous = document.activeElement as HTMLElement | null;
    element.showModal();
    return () => {
      element.close();
      if (previous?.isConnected) previous.focus();
    };
  }, [open]);
  return createPortal(
    <dialog
      ref={ref}
      className="ds-dialog ds-overlay-root"
      role={alert ? "alertdialog" : "dialog"}
      aria-modal="true"
      aria-labelledby={titleId}
      aria-describedby={description ? descriptionId : undefined}
      onKeyDown={(event) => {
        if (event.key !== "Tab") return;
        const controls = [
          ...event.currentTarget.querySelectorAll<HTMLElement>(
            'button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),a[href],[tabindex="0"]'
          ),
        ].filter(
          (control) =>
            control.tabIndex >= 0 &&
            control.getClientRects().length &&
            getComputedStyle(control).visibility !== "hidden"
        );
        const first = controls[0];
        const last = controls.at(-1);
        if (!first) {
          event.preventDefault();
          event.currentTarget.focus();
        } else if (
          event.shiftKey &&
          (document.activeElement === first || document.activeElement === event.currentTarget)
        ) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        if (
          event.target === event.currentTarget &&
          (event.clientX < rect.left ||
            event.clientX > rect.right ||
            event.clientY < rect.top ||
            event.clientY > rect.bottom)
        )
          onClose();
      }}
    >
      <header>
        <h2 id={titleId}>{title}</h2>
        <IconButton
          label={`Close ${title.toLowerCase()}`}
          icon="close"
          tone="quiet"
          onClick={onClose}
        />
      </header>
      {description && (
        <p id={descriptionId} className="ds-dialog-description">
          {description}
        </p>
      )}
      <div className="ds-dialog-content">{children}</div>
      {footer && <footer>{footer}</footer>}
    </dialog>,
    document.body
  );
}

export function ConfirmationDialog({
  open,
  onClose,
  onConfirm,
  title,
  description,
  confirmLabel = "Confirm",
  danger = false,
}: {
  open: boolean;
  onClose(): void;
  onConfirm(): void;
  title: string;
  description: string;
  confirmLabel?: string;
  danger?: boolean;
}) {
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={title}
      description={description}
      alert
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            tone={danger ? "danger" : "primary"}
            onClick={() => {
              onConfirm();
              onClose();
            }}
          >
            {confirmLabel}
          </Button>
        </>
      }
    />
  );
}

export function Popover({
  label,
  trigger,
  children,
}: {
  label: string;
  trigger: string;
  children: ReactNode;
}) {
  const [anchor, setAnchor] = useState<Anchor>();
  const button = useRef<HTMLButtonElement>(null);
  return (
    <>
      <Button
        ref={button}
        aria-expanded={!!anchor}
        aria-haspopup="dialog"
        onClick={() => setAnchor(anchor ? undefined : button.current!.getBoundingClientRect())}
      >
        {trigger}
      </Button>
      {anchor && (
        <PopoverBody
          label={label}
          anchor={anchor}
          onClose={() => setAnchor(undefined)}
          returnFocus={button.current}
        >
          {children}
        </PopoverBody>
      )}
    </>
  );
}

function PopoverBody({
  label,
  anchor,
  children,
  onClose,
  returnFocus,
}: {
  label: string;
  anchor: Anchor;
  children: ReactNode;
  onClose(): void;
  returnFocus: HTMLElement | null;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const position = usePosition(ref, anchor);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    ref.current?.focus();
    const outside = (event: PointerEvent) => {
      if (
        !ref.current?.contains(event.target as Node) &&
        !returnFocus?.contains(event.target as Node)
      )
        closeRef.current();
    };
    document.addEventListener("pointerdown", outside);
    return () => {
      document.removeEventListener("pointerdown", outside);
      returnFocus?.focus();
    };
  }, [returnFocus]);
  return createPortal(
    <div
      ref={ref}
      tabIndex={-1}
      role="dialog"
      aria-label={label}
      className="ds-popover ds-overlay-root"
      style={position}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        }
      }}
    >
      {children}
      <Button tone="quiet" onClick={onClose}>
        Close
      </Button>
    </div>,
    overlayContainer(returnFocus)
  );
}
