import { createRoot } from "react-dom/client";
import { ThemeProvider } from "../design-system/themes/ThemeProvider";
import { ViewErrorBoundary } from "../design-system/components/Application";
import { ProjectsView } from "./ProjectsView";
import "./projects.css";
createRoot(document.getElementById("root")!).render(
  <ThemeProvider>
    <ViewErrorBoundary onRetry={() => location.reload()}>
      <ProjectsView />
    </ViewErrorBoundary>
  </ThemeProvider>
);
