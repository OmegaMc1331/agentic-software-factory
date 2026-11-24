import { memo, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  createDockview,
  type IContentRenderer,
  type ITabRenderer,
  type TabPartInitParameters,
  type IGroupHeaderProps,
} from "dockview";
import "dockview/dist/styles/dockview.css";
import { LiveView } from "./Views";
import type { DockSession, WorkspaceModel } from "./controller";
type Portal = { id: string; element: HTMLElement };
export const DockSurface = memo(function DockSurface({
  model,
  dock,
  contextMenu,
}: {
  model: WorkspaceModel;
  dock: DockSession;
  contextMenu: (event: MouseEvent, id: string) => void;
}) {
  const host = useRef<HTMLDivElement>(null),
    menu = useRef(contextMenu);
  menu.current = contextMenu;
  const [portals, setPortals] = useState<Portal[]>([]);
  useEffect(() => {
    const container = host.current!;
    let disposed = false;
    const elements = new Map<string, HTMLElement>();
    class Content implements IContentRenderer {
      element: HTMLElement;
      constructor(readonly id: string) {
        let element = elements.get(id);
        if (!element) {
          element = document.createElement("div");
          element.className = "ws-panel-content";
          element.tabIndex = -1;
          element.dataset.workspacePanel = id;
          elements.set(id, element);
        }
        this.element = element;
      }
      init() {
        if (!disposed)
          setPortals((items) =>
            items.some((item) => item.id === this.id)
              ? items
              : [...items, { id: this.id, element: this.element }]
          );
      }
      dispose() {}
    }
    class Tab implements ITabRenderer {
      element = document.createElement("span");
      subscription?: { dispose(): void };
      private id = "";
      private context = (event: MouseEvent) => {
        event.preventDefault();
        menu.current(event, this.id);
      };
      init(parameters: TabPartInitParameters) {
        this.id = parameters.api.id;
        this.element.dataset.workspaceTab = this.id;
        this.element.className = "ws-tab-title";
        this.element.textContent = parameters.title;
        this.element.title = parameters.title;
        this.subscription = parameters.api.onDidTitleChange(({ title }) => {
          this.element.textContent = title;
          this.element.title = title;
        });
        this.element.addEventListener("contextmenu", this.context);
      }
      dispose() {
        this.subscription?.dispose();
        this.element.removeEventListener("contextmenu", this.context);
      }
    }
    class Header {
      element = document.createElement("div");
      params?: IGroupHeaderProps;
      private close = document.createElement("button");
      private action = () => dock.requestClose(this.params?.group.activePanel?.id);
      init(params: IGroupHeaderProps) {
        this.params = params;
        this.close.type = "button";
        this.close.className = "ds-button ds-button--quiet ws-group-close";
        this.close.setAttribute("aria-label", "Close active view in group");
        this.close.textContent = "×";
        this.close.addEventListener("click", this.action);
        this.element.append(this.close);
      }
      dispose() {
        this.close.removeEventListener("click", this.action);
      }
    }
    const api = createDockview(container, {
      theme: { name: "asf", className: "dockview-theme-asf" },
      defaultTabComponent: "asf",
      disableFloatingGroups: true,
      scrollbars: "native",
      createComponent: (options) => new Content(options.id),
      createTabComponent: () => new Tab(),
      createRightHeaderActionComponent: () => new Header(),
    });
    api.layout(container.clientWidth, container.clientHeight);
    dock.attach(api);
    api.layout(container.clientWidth, container.clientHeight);
    dock.capture();
    const events = [
      api.onDidLayoutChange(() => dock.capture()),
      api.onDidActivePanelChange(() => dock.capture()),
      api.onDidRemovePanel((panel) => dock.removed(panel)),
    ];
    const observer = new ResizeObserver(() =>
      api.layout(container.clientWidth, container.clientHeight)
    );
    observer.observe(container);
    const unsubscribe = model.subscribe(() => {
      for (const panel of api.panels) {
        const view = dock.session.views.find((v) => v.id === panel.id);
        const title = view ? `${view.state.dirty ? "• " : ""}${view.title}` : panel.title;
        if (title && panel.title !== title) panel.api.setTitle(title);
      }
    });
    const focus = (event: FocusEvent) => {
      if (event.target instanceof HTMLElement) {
        const view = event.target.closest<HTMLElement>("[data-workspace-panel]");
        if (view?.dataset.workspacePanel)
          api.getPanel(view.dataset.workspacePanel)?.api.setActive();
      }
    };
    container.addEventListener("focusin", focus);
    requestAnimationFrame(() => {
      if (!disposed) performance.mark("asf-p04-workspace-ready");
    });
    return () => {
      disposed = true;
      unsubscribe();
      container.removeEventListener("focusin", focus);
      observer.disconnect();
      for (const event of events) event.dispose();
      dock.api = undefined;
      api.dispose();
    };
  }, [model, dock]);
  return (
    <div className="ws-dock" ref={host} aria-label="Workspace view groups" role="region">
      {portals.map((portal) =>
        createPortal(
          <LiveView model={model} dock={dock} id={portal.id} />,
          portal.element,
          portal.id
        )
      )}
    </div>
  );
});
