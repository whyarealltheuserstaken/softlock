import { defineConfig, devices } from "@playwright/test";
import devServer from "./dev-server.json" with { type: "json" };

const origin = `http://${devServer.host}:${devServer.port}`;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: "line",
  use: {
    baseURL: origin,
    ...devices["Desktop Chrome"],
  },
  webServer: {
    command: "npm run dev",
    url: origin,
    reuseExistingServer: !process.env.CI,
    env: {
      VITE_IPC_BRIDGE: "fake",
    },
  },
});
