import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig(({ mode }) => ({
  root: path.resolve(import.meta.dirname),
  base: `/${mode}/`,
  plugins: [react()],
  resolve: { alias: [
    { find: /^monaco-editor\/esm\/vs\//, replacement: path.resolve(import.meta.dirname, "../node_modules/monaco-editor/esm/vs/") + "/" },
    { find: "@candidate", replacement: path.resolve(import.meta.dirname, `../${mode}/adapter.ts`) }
  ] },
  define: { __CANDIDATE__: JSON.stringify(mode) },
  build: { outDir: path.resolve(import.meta.dirname, `../../../.factory/v2/p01/dist/${mode}`), emptyOutDir: true },
  worker: { format: "es" }
}));
