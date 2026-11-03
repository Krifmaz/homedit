/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

/**
 * Vite configuration for the homedit web UI.
 *
 * The production build lands in `web/dist`, which the Rust binary embeds with
 * rust-embed. During development, `/api` and `/icons` are proxied to a local
 * backend on port 3000 unless `VITE_MOCK=1` is set, in which case an in-browser
 * mock API answers instead.
 */
export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
    target: "es2022",
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:3000",
      "/icons": "http://127.0.0.1:3000",
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    css: false,
  },
});
