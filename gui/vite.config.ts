import { defineConfig } from "vite";

// Tauri expects a fixed port, and the window shows Rust's own errors.
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: { target: "es2021", outDir: "dist" },
});
