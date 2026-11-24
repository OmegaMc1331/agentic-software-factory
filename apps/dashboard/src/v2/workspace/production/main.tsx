import { createRoot } from "react-dom/client";
import { ThemeProvider } from "../../design-system/themes/ThemeProvider";
import { ViewErrorBoundary } from "../../design-system/components/Application";
import { WorkspaceModel } from "./controller";
import { WorkspaceShell } from "./WorkspaceShell";
import "./workspace.css";
import "../../files/files.css";
const model = new WorkspaceModel();
createRoot(document.getElementById("root")!).render(
  <ThemeProvider>
    <ViewErrorBoundary onRetry={() => location.reload()}>
      <WorkspaceShell model={model} />
    </ViewErrorBoundary>
  </ThemeProvider>
);
import "../../ide/ide.css";
