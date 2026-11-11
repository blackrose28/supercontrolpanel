import { expect, test } from "@playwright/test";

test.skip(({ baseURL }) => !baseURL, "Run with playwright.backup.config.ts for the local UI fixture.");

// Use the real Config markup and UI controller with a simulated Tauri boundary.
test.beforeEach(async ({ page }) => {
  await page.route("**/src/main.ts", route => route.fulfill({ body: "", contentType: "application/javascript" }));
  await page.goto("/");
  await page.evaluate(async () => {
    document.querySelector("#view-config")!.classList.add("active");
    const calls: { command: string; args?: Record<string, unknown> }[] = [];
    Object.assign(window, { backupCalls: calls });
    const { mountSettingsBackup } = await import("/src/settingsBackupUi.ts");
    mountSettingsBackup({
      invoke: async (command: string, args?: Record<string, unknown>) => {
        calls.push({ command, args });
        if (command === "preview_settings_backup") {
          if (args?.passphrase === "wrong password") throw new Error("Wrong backup passphrase");
          if ((window as any).slowBackup) return new Promise(() => {});
          return { id: "preview-1", servers: 2, credentialsSet: 3, credentialsDeleted: 1, credentialsKept: 2, sections: [{ name: "config.json", state: "included", changed: true }], sourcePlatform: "linux", sourceVersion: "test", warnings: ["MCP and CC stay disabled"], externalFiles: [{ path: "/missing/client.ovpn", existsHere: false }], executableConfiguration: { sourcePaths: { repoRoot: "/foreign" } } };
        }
        return { complete: true, servers: 2, credentialsPresent: 3, credentialsAbsent: 4 };
      },
      selectExport: async () => "/tmp/settings.scpbak",
      selectImport: async () => "/tmp/settings.scpbak",
      confirmOverwrite: async () => true,
      afterImport: async () => { document.body.dataset.refreshed = "true"; },
    });
  });
});

test("export requires passphrase confirmation and clears fields", async ({ page }) => {
  await page.getByRole("button", { name: "Export settings", exact: true }).click();
  await page.locator("#backup-passphrase").fill("my backup passphrase");
  await page.locator("#backup-confirmation").fill("different");
  await page.getByRole("button", { name: "Write encrypted backup" }).click();
  await expect(page.locator("#backup-status")).toContainText("match");
  expect(await page.evaluate(() => (window as any).backupCalls.some((c: any) => c.command === "export_settings_backup"))).toBe(false);
  await page.locator("#backup-confirmation").fill("my backup passphrase");
  await page.getByRole("button", { name: "Write encrypted backup" }).click();
  await expect(page.locator("#backup-status")).toContainText("complete");
  await expect(page.locator("#backup-passphrase")).toHaveValue("");
  await expect(page.locator("#backup-confirmation")).toHaveValue("");
});

test("import previews before explicit replacement and refreshes the app", async ({ page }) => {
  await page.getByRole("button", { name: "Import settings", exact: true }).click();
  await page.locator("#backup-passphrase").fill("my backup passphrase");
  await page.getByRole("button", { name: "Preview backup" }).click();
  await expect(page.locator("#backup-preview")).toContainText("MCP and CC stay disabled");
  await expect(page.locator("#backup-preview")).toContainText("/missing/client.ovpn");
  await expect(page.locator("#backup-passphrase")).toHaveValue("");
  await expect(page.getByRole("button", { name: "Replace selected settings" })).toBeDisabled();
  await page.locator("#backup-replace-confirm").check();
  await page.getByRole("button", { name: "Replace selected settings" }).click();
  await expect(page.locator("body")).toHaveAttribute("data-refreshed", "true");
  const calls = await page.evaluate(() => (window as any).backupCalls);
  const commit = calls.find((call: any) => call.command === "import_settings_backup");
  expect(commit.args).toEqual({ previewId: "preview-1", confirmed: true });
});

test("wrong passphrase never imports; changing a selection invalidates preview", async ({ page }) => {
  await page.getByRole("button", { name: "Import settings", exact: true }).click();
  await page.locator("#backup-passphrase").fill("wrong password");
  await page.getByRole("button", { name: "Preview backup" }).click();
  await expect(page.locator("#backup-status")).toContainText("Wrong backup passphrase");
  await page.locator("#backup-passphrase").fill("my backup passphrase");
  await page.getByRole("button", { name: "Preview backup" }).click();
  await page.locator("#backup-local").uncheck();
  await expect(page.locator("#backup-preview")).toHaveText("");
  await expect(page.getByRole("button", { name: "Replace selected settings" })).toBeDisabled();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  const calls = await page.evaluate(() => (window as any).backupCalls);
  expect(calls.some((c: any) => c.command === "import_settings_backup")).toBe(false);
  expect(calls.some((c: any) => c.command === "discard_settings_backup_preview")).toBe(true);
});

test("path remapping is sent to preview", async ({ page }) => {
  await page.getByRole("button", { name: "Import settings", exact: true }).click();
  await page.locator("#backup-path-openvpnOvpnPath").fill("/remapped/client.ovpn");
  await page.locator("#backup-passphrase").fill("my backup passphrase");
  await page.getByRole("button", { name: "Preview backup" }).click();
  const calls = await page.evaluate(() => (window as any).backupCalls);
  expect(calls.find((c: any) => c.command === "preview_settings_backup").args.options.pathOverrides).toEqual({ openvpnOvpnPath: "/remapped/client.ovpn" });
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
});

test("a slow preview can be cancelled without applying settings", async ({ page }) => {
  await page.evaluate(() => { (window as any).slowBackup = true; });
  await page.getByRole("button", { name: "Import settings", exact: true }).click();
  await page.locator("#backup-passphrase").fill("my backup passphrase");
  await page.getByRole("button", { name: "Preview backup" }).click();
  await expect(page.locator("#backup-status")).toContainText("Authenticating");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.locator("#settings-backup-dialog")).not.toBeVisible();
  await expect(page.locator("#backup-passphrase")).toHaveValue("");
  const calls = await page.evaluate(() => (window as any).backupCalls);
  expect(calls.some((c: any) => c.command === "cancel_settings_backup_work" && c.args.requestId)).toBe(true);
  expect(calls.some((c: any) => c.command === "import_settings_backup")).toBe(false);
});
