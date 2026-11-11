import { expect, test } from "@playwright/test";

test.skip(({ baseURL }) => !baseURL, "Run with playwright.mcp.config.ts for the production Config UI.");

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const state = {
      settings: { repoRoot: "", firstRunCompleted: true, mcpEnabled: false, mcpBindAddress: "127.0.0.1", mcpPort: 46739, mcpAuthToken: "test-secret-token", mcpSensitiveActionPolicy: "require-ui-approval", mcpAllowedToolCategories: ["read-only", "runtime-control"] },
      runtime: { enabled: false, active: false, transport: "http", bindAddress: "127.0.0.1", port: 46739, implementationStage: "http-streamable-slice5", lastError: null as string | null },
      calls: [] as { command: string; args: any }[], copied: "", failSave: false,
    };
    Object.assign(window, { mcpTest: state });
    Object.defineProperty(navigator, "clipboard", { value: { writeText: async (text: string) => { state.copied = text; } } });
    Object.assign(window, { __TAURI_INTERNALS__: {
      transformCallback: () => 1,
      invoke: async (command: string, args: any) => {
        state.calls.push({ command, args });
        if (command === "is_first_run") return false;
        if (command === "get_sync_settings") return { ...state.settings };
        if (command === "save_sync_settings") {
          if (state.failSave) throw new Error("secret diagnostic test-secret-token");
          Object.assign(state.settings, args.settings);
          Object.assign(state.runtime, { enabled: state.settings.mcpEnabled, active: state.settings.mcpEnabled, bindAddress: state.settings.mcpBindAddress, port: state.settings.mcpPort });
        }
        if (command === "mcp_get_runtime_status" || command === "mcp_sync_runtime") return { ...state.runtime };
        if (command === "get_app_config") return { app: { defaultTerminalCols: 80, defaultTerminalRows: 24 }, openvpn: {}, boundary: {}, caddy: {} };
        if (command === "list_function_statuses" || command === "list_servers" || command === "list_scp_transfers") return [];
        if (command === "get_config_paths") return { appConfigPath: "/fixture/config.json" };
        if (command === "plugin:app|version") return "test";
        return {};
      },
    } });
  });
  await page.goto("/");
  await page.locator('#nav-config').click();
  await expect(page.locator("#cfg-mcp-auth-token")).toHaveValue("test-secret-token");
});

test("full endpoint and tested Inspector example keep credentials out of previews", async ({ page }) => {
  await expect(page.locator("#cfg-mcp-endpoint")).toHaveValue("http://127.0.0.1:46739/mcp");
  await expect(page.locator("#cfg-mcp-auth-token")).toHaveAttribute("type", "password");
  await expect(page.locator("#cfg-mcp-snippet")).toContainText("@modelcontextprotocol/inspector@2.14.0");
  await expect(page.locator("#cfg-mcp-snippet")).not.toContainText("test-secret-token");
  await page.locator("#cfg-mcp-copy-endpoint").click();
  expect(await page.evaluate(() => (window as any).mcpTest.copied)).toBe("http://127.0.0.1:46739/mcp");
  await page.locator("#cfg-mcp-copy-token").click();
  expect(await page.evaluate(() => (window as any).mcpTest.copied)).toBe("test-secret-token");
  await page.locator("#cfg-mcp-example-era").selectOption("legacy");
  await expect(page.locator("#cfg-mcp-snippet")).toContainText("--protocol-era legacy");
  await page.locator("#cfg-mcp-copy-example").click();
  expect(await page.evaluate(() => (window as any).mcpTest.copied)).toContain("--protocol-era legacy");
  await page.locator("#cfg-mcp-bind-address").fill("::1");
  await page.locator("#cfg-mcp-bind-address").blur();
  await expect(page.locator("#cfg-mcp-endpoint")).toHaveValue("http://[::1]:46739/mcp");
});

test("enable, policy, categories and token rotation persist partial patches", async ({ page }) => {
  await expect(page.locator("#cfg-mcp-policy-help")).toContainText("blocked");
  await page.locator("#cfg-mcp-enabled").check();
  await expect(page.locator("#cfg-mcp-status")).toContainText("Running");
  await page.locator("#cfg-mcp-sensitive-policy").selectOption("allow-with-token");
  await expect(page.locator("#cfg-mcp-policy-help")).toContainText("allowed categories");
  await page.locator("#cfg-mcp-allowed-file-transfer").check();
  await page.locator("#cfg-mcp-token-regenerate").click();
  await expect(page.locator("#cfg-mcp-auth-token")).not.toHaveValue("test-secret-token");
  await page.locator("#cfg-mcp-enabled").uncheck();
  await expect(page.locator("#cfg-mcp-status")).toContainText("Disabled");
  const patches = await page.evaluate(() => (window as any).mcpTest.calls.filter((c: any) => c.command === "save_sync_settings").map((c: any) => c.args.settings));
  expect(patches).toContainEqual({ mcpEnabled: true });
  expect(patches).toContainEqual({ mcpEnabled: false });
  expect(patches).toContainEqual({ mcpSensitiveActionPolicy: "allow-with-token" });
  expect(patches).toContainEqual({ mcpAllowedToolCategories: ["read-only", "runtime-control", "file-transfer"] });
  expect(patches.every((patch: any) => Object.keys(patch).length === 1)).toBe(true);
});

test("runtime refresh, restart and save errors are visible without diagnostics or tokens", async ({ page }) => {
  await page.evaluate(() => { Object.assign((window as any).mcpTest.runtime, { enabled: true, active: false }); });
  await page.locator("#cfg-mcp-refresh").click();
  await expect(page.locator("#cfg-mcp-status")).toContainText("Enabled but not active");
  await page.evaluate(() => { (window as any).mcpTest.runtime.lastError = "test-secret-token"; });
  await page.locator("#cfg-mcp-refresh").click();
  await expect(page.locator("#cfg-mcp-status")).toContainText("Listener error");
  await expect(page.locator("#cfg-mcp-status")).not.toContainText("test-secret-token");
  await page.evaluate(() => { Object.assign((window as any).mcpTest.runtime, { active: true, lastError: null }); });
  await page.locator("#cfg-mcp-restart").click();
  await expect(page.locator("#cfg-mcp-status")).toContainText("Running");
  expect(await page.evaluate(() => (window as any).mcpTest.calls.some((c: any) => c.command === "mcp_sync_runtime"))).toBe(true);
  await page.evaluate(() => { (window as any).mcpTest.failSave = true; });
  await page.locator("#cfg-mcp-port").fill("46740");
  await page.locator("#cfg-mcp-port").blur();
  await expect(page.locator("#cfg-mcp-action-status")).toContainText("Save failed");
  await expect(page.locator("#cfg-mcp-action-status")).not.toContainText("test-secret-token");
});

test("invalid edits never save and clipboard errors do not expose values", async ({ page }) => {
  await page.locator("#cfg-mcp-port").fill("65536");
  await page.locator("#cfg-mcp-port").blur();
  await expect(page.locator("#cfg-status")).toContainText("MCP port must be 1-65535");
  await page.locator("#cfg-mcp-auth-token").fill("");
  await page.locator("#cfg-mcp-auth-token").blur();
  await expect(page.locator("#cfg-status")).toContainText("MCP auth token cannot be empty");
  expect(await page.evaluate(() => (window as any).mcpTest.calls.some((c: any) => c.command === "save_sync_settings"))).toBe(false);
  await page.evaluate(() => { navigator.clipboard.writeText = async () => { throw new Error("test-secret-token"); }; });
  await page.locator("#cfg-mcp-copy-token").click();
  await expect(page.locator("#cfg-mcp-action-status")).toContainText("Copy failed");
  await expect(page.locator("#cfg-mcp-action-status")).not.toContainText("test-secret-token");
});
