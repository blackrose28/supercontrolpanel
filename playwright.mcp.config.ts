import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests", testMatch: "mcpConfig.spec.ts", workers: 1,
  use: { browserName: "chromium", headless: true, baseURL: "http://127.0.0.1:5173" },
  webServer: { command: "npm run dev -- --host 127.0.0.1 --port 5173", url: "http://127.0.0.1:5173", reuseExistingServer: !process.env.CI },
});
