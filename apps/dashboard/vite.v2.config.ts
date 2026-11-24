import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: { strictPort: true },
  preview: {
    strictPort: true,
    headers: {
      "Content-Security-Policy":
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'",
      "X-Content-Type-Options": "nosniff",
    },
  },
  build: {
    outDir: "../../.factory/v2/p02/dist",
    emptyOutDir: true,
    rollupOptions: { input: "v2.html" },
  },
});
