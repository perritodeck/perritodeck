import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed port and doesn't want the screen cleared. The dev server doesn't
// watch src-tauri: Tauri rebuilds the Rust side itself, and on Windows watching its target
// folder fails with EBUSY on files the build has open.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: { target: "es2021", sourcemap: false },
});
