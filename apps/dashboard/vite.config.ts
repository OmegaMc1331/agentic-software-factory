/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  build: {
    rollupOptions: {
      input: { dashboard: "index.html", projects: "projects.html", workspace: "workspace.html" },
    },
  },
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:4321",
        changeOrigin: true,
        ws: true,
      },
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    maxWorkers: 2,
  },
});
