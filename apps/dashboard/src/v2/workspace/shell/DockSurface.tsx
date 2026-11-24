import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  createDockview,
  type IContentRenderer,
  type GroupPanelPartInitParameters,
  type ITabRenderer,
  type TabPartInitParameters,
  type IGroupHeaderProps,
} from "dockview";
import "dockview/dist/styles/dockview.css";
import { useWorkspace } from "./context";
import { PanelContent } from "../panels/Views";
import type { ViewName } from "../layouts/presets";

type PortalView = {
  id: string;
  name: ViewName;
  params: Record<string, unknown>;
  element: HTMLElement;
};

export function DockSurface() {
  const model = useWorkspace();
  const host = useRef<HTMLDivElement>(null);
  const [portals, setPortals] = useState<PortalView[]>([]);
  useEffect(() => {
    const container = host.current!;
    let disposed = false;
    class Panel implements IContentRenderer {
      element = document.createElement("div");
      constructor(
        private id: string,
        private name: ViewName
      ) {
        this.element.className = "v2-panel-content";
        this.element.dataset.panel = id;
      }
      init(parameters: GroupPanelPartInitParameters) {
        if (!disposed)
          setPortals((previous) => [
            ...previous.filter((view) => view.id !== this.id),
            { id: this.id, name: this.name, params: parameters.params, element: this.element },
          ]);
      }
      dispose() {
        if (!disposed) setPortals((previous) => previous.filter((view) => view.id !== this.id));
      }
    }
    class Tab implements ITabRenderer {
      element = document.createElement("span");
      private subscription?: { dispose(): void };
      init(parameters: TabPartInitParameters) {
        this.element.className = "v2-dock-tab-title";
        const title = (value: string) => {
          this.element.textContent = value;
          this.element.title = `${value} · Delete to close`;
        };
        title(parameters.title);
        this.subscription = parameters.api.onDidTitleChange((event) => title(event.title));
      }
      dispose() {
        this.subscription?.dispose();
      }
    }
    class HeaderActions {
      element = document.createElement("div");
      private close = document.createElement("button");
      private params?: IGroupHeaderProps;
      private click = () => this.params?.group.activePanel?.api.close();
      init(params: IGroupHeaderProps) {
        this.params = params;
        this.close.type = "button";
        this.close.className = "ds-button ds-button--quiet ds-icon-button v2-close-view";
        this.close.setAttribute("aria-label", "Close active view in group");
        this.close.title = "Close active view";
        const glyph = document.createElement("span");
        glyph.setAttribute("aria-hidden", "true");
        glyph.textContent = "×";
        this.close.append(glyph);
        this.element.append(this.close);
        this.close.addEventListener("click", this.click);
      }
      dispose() {
        this.close.removeEventListener("click", this.click);
      }
    }
    const api = createDockview(host.current!, {
      theme: { name: "asf", className: "dockview-theme-asf" },
      defaultTabComponent: "asf",
      createComponent: (options) => new Panel(options.id, options.name as ViewName),
      createTabComponent: () => new Tab(),
      createRightHeaderActionComponent: () => new HeaderActions(),
    });
    api.layout(host.current!.clientWidth, host.current!.clientHeight);
    model.attach(api);
    const events = [
      api.onDidLayoutChange(() => model.layoutChanged()),
      api.onDidMutateLayout(() => model.selectionChanged()),
      api.onDidActivePanelChange(() => model.selectionChanged()),
    ];
    const observer = new ResizeObserver(() => {
      if (host.current) {
        api.layout(host.current.clientWidth, host.current.clientHeight);
        model.adaptWidth(window.innerWidth);
      }
    });
    observer.observe(host.current!);
    const pagehide = () => model.persist();
    window.addEventListener("pagehide", pagehide);
    const closeTab = (event: KeyboardEvent) => {
      if (event.key !== "Delete") return;
      const target =
        event.target instanceof HTMLElement ? event.target.closest<HTMLElement>(".dv-tab") : null;
      if (target?.dataset.tabPanelId) {
        event.preventDefault();
        api.getPanel(target.dataset.tabPanelId)?.api.close();
      }
    };
    host.current!.addEventListener("keydown", closeTab);
    requestAnimationFrame(() => {
      if (!disposed) performance.mark("asf-p02-shell-ready");
    });
    return () => {
      disposed = true;
      window.removeEventListener("pagehide", pagehide);
      container.removeEventListener("keydown", closeTab);
      observer.disconnect();
      for (const event of events) event.dispose();
      model.detach();
      api.dispose();
    };
  }, [model]);
  return (
    <div className="v2-dock-surface" ref={host} role="region" aria-label="Workspace panels">
      {portals.map((view) =>
        createPortal(
          <PanelContent name={view.name} params={view.params} panelId={view.id} />,
          view.element,
          view.id
        )
      )}
    </div>
  );
}
