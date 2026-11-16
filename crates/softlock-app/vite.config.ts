import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";
import devServer from "./dev-server.json" with { type: "json" };

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: devServer.port,
    strictPort: true,
    host: devServer.host,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
});
