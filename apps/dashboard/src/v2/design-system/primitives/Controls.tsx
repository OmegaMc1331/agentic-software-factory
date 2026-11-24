import {
  forwardRef,
  useId,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
  type KeyboardEventHandler,
  type RefObject,
} from "react";
import { Icon, type IconName } from "../icons/Icon";

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  tone?: "primary" | "neutral" | "danger" | "quiet";
  loading?: boolean;
  icon?: IconName;
};
export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    tone = "neutral",
    loading,
    icon,
    className = "",
    children,
    disabled,
    type = "button",
    ...props
  },
  ref
) {
  return (
    <button
      {...props}
      ref={ref}
      type={type}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      className={`ds-button ds-button--${tone} ${className}`}
    >
      {loading ? (
        <span className="ds-busy-mark" aria-hidden="true" />
      ) : (
        icon && <Icon name={icon} />
      )}
      {children}
    </button>
  );
});

export const IconButton = forwardRef<
  HTMLButtonElement,
  Omit<ButtonProps, "children" | "icon"> & { label: string; icon: IconName }
>(function IconButton({ label, icon, className = "", ...props }, ref) {
  return (
    <Button {...props} ref={ref} className={`ds-icon-button ${className}`} aria-label={label}>
      <Icon name={icon} />
    </Button>
  );
});

type FieldProps = { label: string; error?: string; hint?: string; hideLabel?: boolean };
export const Input = forwardRef<
  HTMLInputElement,
  InputHTMLAttributes<HTMLInputElement> & FieldProps
>(function Input({ label, error, hint, hideLabel, id: suppliedId, className = "", ...props }, ref) {
  const generated = useId();
  const id = suppliedId ?? generated;
  return (
    <label className={`ds-field ${className}`} htmlFor={id}>
      <span id={`${id}-label`} className={hideLabel ? "ds-sr-only" : "ds-field-label"}>
        {label}
      </span>
      <input
        {...props}
        id={id}
        ref={ref}
        className="ds-input"
        aria-labelledby={`${id}-label`}
        aria-invalid={!!error}
        aria-describedby={error || hint ? `${id}-message` : undefined}
      />
      {(error || hint) && (
        <span id={`${id}-message`} className={error ? "ds-field-error" : "ds-field-hint"}>
          {error ?? hint}
        </span>
      )}
    </label>
  );
});

export const Textarea = forwardRef<
  HTMLTextAreaElement,
  TextareaHTMLAttributes<HTMLTextAreaElement> & FieldProps
>(function Textarea(
  { label, error, hint, hideLabel, id: suppliedId, className = "", ...props },
  ref
) {
  const generated = useId();
  const id = suppliedId ?? generated;
  return (
    <label className={`ds-field ${className}`} htmlFor={id}>
      <span id={`${id}-label`} className={hideLabel ? "ds-sr-only" : "ds-field-label"}>
        {label}
      </span>
      <textarea
        ref={ref}
        {...props}
        id={id}
        className="ds-input ds-textarea"
        aria-labelledby={`${id}-label`}
        aria-invalid={!!error}
        aria-describedby={error || hint ? `${id}-message` : undefined}
      />
      {(error || hint) && (
        <span id={`${id}-message`} className={error ? "ds-field-error" : "ds-field-hint"}>
          {error ?? hint}
        </span>
      )}
    </label>
  );
});

export function Select({
  label,
  hideLabel,
  error,
  children,
  className = "",
  ...props
}: SelectHTMLAttributes<HTMLSelectElement> & Omit<FieldProps, "hint">) {
  const id = useId();
  return (
    <label className={`ds-field ${className}`}>
      <span id={`${id}-label`} className={hideLabel ? "ds-sr-only" : "ds-field-label"}>
        {label}
      </span>
      <select
        {...props}
        className="ds-input ds-select"
        aria-labelledby={`${id}-label`}
        aria-invalid={!!error}
        aria-describedby={error ? id : undefined}
      >
        {children}
      </select>
      {error && (
        <span id={id} className="ds-field-error">
          {error}
        </span>
      )}
    </label>
  );
}

export function Checkbox({
  label,
  error,
  ...props
}: Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & { label: string; error?: string }) {
  const id = useId();
  return (
    <label className="ds-choice">
      <input
        {...props}
        type="checkbox"
        aria-invalid={!!error}
        aria-labelledby={`${id}-label`}
        aria-describedby={error ? `${id}-error` : undefined}
      />
      <span>
        <span id={`${id}-label`}>{label}</span>
        {error && (
          <small id={`${id}-error`} className="ds-field-error">
            {error}
          </small>
        )}
      </span>
    </label>
  );
}

export function RadioControl({
  label,
  ...props
}: Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & { label: string }) {
  return (
    <label className="ds-choice">
      <input {...props} type="radio" />
      <span>{label}</span>
    </label>
  );
}

export function Toggle({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  checked: boolean;
  onChange(value: boolean): void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      className="ds-toggle"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span className="ds-toggle-track" aria-hidden="true">
        <span />
      </span>
      {label}
    </button>
  );
}

export function SearchField({
  label = "Search",
  value,
  onChange,
  placeholder = "Find files or commands…",
  inputRef,
  onKeyDown,
  controls,
  activeDescendant,
}: {
  label?: string;
  value: string;
  onChange(value: string): void;
  placeholder?: string;
  inputRef?: RefObject<HTMLInputElement>;
  onKeyDown?: KeyboardEventHandler<HTMLInputElement>;
  controls?: string;
  activeDescendant?: string;
}) {
  return (
    <div className="ds-search">
      <Icon name="search" />
      <input
        ref={inputRef}
        type="search"
        role={controls ? "combobox" : undefined}
        aria-label={label}
        aria-controls={controls}
        aria-expanded={controls ? true : undefined}
        aria-activedescendant={activeDescendant}
        aria-autocomplete={controls ? "list" : undefined}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          onKeyDown?.(event);
          if (!event.defaultPrevented && event.key === "Escape" && value) {
            event.preventDefault();
            event.stopPropagation();
            onChange("");
          }
        }}
      />
      {value && (
        <IconButton
          icon="close"
          label={`Clear ${label.toLowerCase()}`}
          tone="quiet"
          onClick={() => onChange("")}
        />
      )}
    </div>
  );
}

export type TabOption = {
  id: string;
  label: string;
  disabled?: boolean;
  controls?: string;
  badge?: ReactNode;
};
export function Tabs({
  label,
  items,
  value,
  onChange,
}: {
  label: string;
  items: TabOption[];
  value: string;
  onChange(id: string): void;
}) {
  return (
    <div className="ds-tabs" role="tablist" aria-label={label}>
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          id={`${label.replaceAll(" ", "-")}-${item.id}`}
          role="tab"
          aria-selected={item.id === value}
          aria-controls={item.controls}
          disabled={item.disabled}
          tabIndex={item.id === value ? 0 : -1}
          onClick={() => onChange(item.id)}
          onKeyDown={(event) => {
            const enabled = items.filter((option) => !option.disabled);
            const index = enabled.findIndex((option) => option.id === value);
            const next =
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? enabled.length - 1
                  : event.key === "ArrowRight"
                    ? (index + 1) % enabled.length
                    : event.key === "ArrowLeft"
                      ? (index + enabled.length - 1) % enabled.length
                      : -1;
            if (next >= 0) {
              event.preventDefault();
              onChange(enabled[next].id);
              const target = event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>(
                '[role="tab"]:not(:disabled)'
              )[next];
              target?.focus();
              target?.scrollIntoView?.({ inline: "nearest", block: "nearest" });
            }
          }}
        >
          {item.label}
          {item.badge}
        </button>
      ))}
    </div>
  );
}
