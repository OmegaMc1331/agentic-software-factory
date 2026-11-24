import { createRoot } from "react-dom/client";
import { ThemeProvider } from "../design-system/themes/ThemeProvider";
import { ViewErrorBoundary } from "../design-system/components/Application";
import { WorkspacePrototype } from "./WorkspacePrototype";

createRoot(document.getElementById("root")!).render(
  <ThemeProvider>
    <ViewErrorBoundary onRetry={() => location.reload()}>
      <WorkspacePrototype />
    </ViewErrorBoundary>
  </ThemeProvider>
);
