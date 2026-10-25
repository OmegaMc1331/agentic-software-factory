import { useCallback, useEffect, useRef } from "react";

export const UNSAVED_CHANGES_MESSAGE = "You have unsaved changes. Discard them?";
export const UNSAVED_AGENT_MESSAGE = "You have unsaved agent changes. Discard them?";
export const UNSAVED_ROLE_MESSAGE = "You have unsaved role changes. Discard them?";
export const UNSAVED_RECIPE_MESSAGE = "You have unsaved recipe changes. Discard them?";

const dirtyEditors = new Set<symbol>();
let beforeUnloadAttached = false;

function beforeUnload(event: BeforeUnloadEvent) {
  event.preventDefault();
  event.returnValue = "";
}

function syncBeforeUnload() {
  if (typeof window === "undefined") return;
  if (dirtyEditors.size > 0 && !beforeUnloadAttached) {
    window.addEventListener("beforeunload", beforeUnload);
    beforeUnloadAttached = true;
  } else if (dirtyEditors.size === 0 && beforeUnloadAttached) {
    window.removeEventListener("beforeunload", beforeUnload);
    beforeUnloadAttached = false;
  }
}

function register(id: symbol) {
  if (dirtyEditors.has(id)) return;
  dirtyEditors.add(id);
  syncBeforeUnload();
}

function unregister(id: symbol) {
  if (!dirtyEditors.delete(id)) return;
  syncBeforeUnload();
}

export function confirmDiscardChanges(
  isDirty: boolean,
  message = UNSAVED_CHANGES_MESSAGE
): boolean {
  if (!isDirty || typeof window === "undefined") return true;
  return window.confirm(message);
}

export function useUnsavedChanges(
  isDirty: boolean,
  options: {
    message?: string;
    onDirtyChange?: (dirty: boolean) => void;
  } = {}
): { isDirty: boolean; requestClose: (close: () => void) => boolean } {
  const idRef = useRef<symbol | null>(null);
  if (idRef.current === null) idRef.current = Symbol("editor");
  const dirtyRef = useRef(isDirty);
  dirtyRef.current = isDirty;
  const onDirtyChangeRef = useRef(options.onDirtyChange);
  onDirtyChangeRef.current = options.onDirtyChange;
  const message = options.message ?? UNSAVED_CHANGES_MESSAGE;

  useEffect(() => {
    const id = idRef.current;
    if (id === null) return;
    if (isDirty) register(id);
    else unregister(id);
    return () => unregister(id);
  }, [isDirty]);

  useEffect(() => {
    onDirtyChangeRef.current?.(isDirty);
  }, [isDirty]);

  useEffect(
    () => () => {
      onDirtyChangeRef.current?.(false);
    },
    []
  );

  const requestClose = useCallback(
    (close: () => void) => {
      if (!confirmDiscardChanges(dirtyRef.current, message)) return false;
      close();
      return true;
    },
    [message]
  );

  return { isDirty, requestClose };
}

export function useUnsavedChangesGuard(): {
  hasUnsavedChanges: () => boolean;
  confirmDiscard: (message?: string) => boolean;
} {
  const hasUnsavedChanges = useCallback(() => dirtyEditors.size > 0, []);
  const confirmDiscard = useCallback(
    (message = UNSAVED_CHANGES_MESSAGE) => confirmDiscardChanges(dirtyEditors.size > 0, message),
    []
  );
  return { hasUnsavedChanges, confirmDiscard };
}
