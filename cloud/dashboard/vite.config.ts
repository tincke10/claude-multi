import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The Worker is bundled by wrangler; vite only builds the SPA into dist/client
// (the Workers Static Assets directory).
export default defineConfig({
  plugins: [react()],
  build: { outDir: "dist/client", emptyOutDir: true },
});
