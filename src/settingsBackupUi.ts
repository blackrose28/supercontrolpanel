type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
type Dependencies = {
  invoke: Invoke;
  selectExport: () => Promise<string | null>;
  selectImport: () => Promise<string | null>;
  confirmOverwrite: () => Promise<boolean>;
  afterImport: () => Promise<void>;
};
type Preview = {
  id: string; servers: number; credentialsSet: number; credentialsDeleted: number;
  credentialsKept: number; sourcePlatform: string; sourceVersion: string;
  sections: { name: string; state: string; changed: boolean }[];
  warnings: string[]; externalFiles: { path: string; existsHere: boolean }[];
  executableConfiguration: unknown;
  credentialActions?: { account: string; action: string }[];
};

export function mountSettingsBackup(deps: Dependencies) {
  const element = <T extends HTMLElement>(id: string) => {
    const node = document.getElementById(id);
    if (!node) throw new Error(`Missing settings backup control: ${id}`);
    return node as T;
  };
  const dialog = element<HTMLDialogElement>("settings-backup-dialog");
  const title = element("backup-title");
  const passphrase = element<HTMLInputElement>("backup-passphrase");
  const confirmation = element<HTMLInputElement>("backup-confirmation");
  const status = element("backup-status");
  const preview = element("backup-preview");
  const action = element<HTMLButtonElement>("backup-action");
  const replace = element<HTMLButtonElement>("backup-replace");
  const confirmed = element<HTMLInputElement>("backup-replace-confirm");
  const options = element<HTMLFieldSetElement>("backup-options");
  const importOnly = dialog.querySelectorAll<HTMLElement>(".backup-import-only");
  const exportOnly = dialog.querySelectorAll<HTMLElement>(".backup-export-only");
  let mode: "export" | "import" = "export";
  let path = "";
  let previewId = "";
  let busy = false;
  let applying = false;
  let runningRequest = "";
  let invalidating = Promise.resolve();

  function clearSecrets() { passphrase.value = ""; confirmation.value = ""; }
  function invalidate() {
    previewId = "";
    confirmed.checked = false;
    replace.disabled = true;
    preview.textContent = "";
    invalidating = invalidating.then(() => deps.invoke("discard_settings_backup_preview")).then(() => {}, error => {
      status.textContent = `Could not discard the previous preview: ${String(error)}`;
    });
    return invalidating;
  }
  function setBusy(value: boolean) {
    busy = value;
    action.disabled = value;
    options.disabled = value;
    passphrase.disabled = value;
    confirmation.disabled = value;
    element<HTMLButtonElement>("backup-cancel").disabled = applying;
    confirmed.disabled = value;
    replace.disabled = value || !previewId || !confirmed.checked;
  }
  function readOptions() {
    const checked = (id: string) => element<HTMLInputElement>(id).checked;
    return { shared: checked("backup-shared"), servers: checked("backup-servers"), local: checked("backup-local"), secrets: checked("backup-secrets"), preservePaths: checked("backup-preserve-paths"), recoverIdentity: checked("backup-recover-identity"), pathOverrides: Object.fromEntries(
      Array.from(options.querySelectorAll<HTMLInputElement>("[data-backup-path]")).filter(input => input.value.trim()).map(input => [input.dataset.backupPath!, input.value.trim()]),
    ) };
  }
  async function open(nextMode: "export" | "import") {
    if (busy) return;
    mode = nextMode;
    options.querySelectorAll<HTMLInputElement>("input[type=checkbox]").forEach(input => { input.checked = input.id !== "backup-recover-identity"; });
    options.querySelectorAll<HTMLInputElement>("[data-backup-path]").forEach(input => { input.value = ""; });
    clearSecrets();
    invalidate();
    status.textContent = "";
    path = (await (mode === "export" ? deps.selectExport() : deps.selectImport())) ?? "";
    if (!path) return;
    title.textContent = mode === "export" ? "Export settings" : "Import settings";
    action.textContent = mode === "export" ? "Write encrypted backup" : "Preview backup";
    importOnly.forEach(node => { node.hidden = mode !== "import"; });
    exportOnly.forEach(node => { node.hidden = mode !== "export"; });
    element("backup-path").textContent = path;
    dialog.showModal();
    passphrase.focus();
  }
  function close() {
    if (applying) return;
    if (runningRequest) {
      void deps.invoke("cancel_settings_backup_work", { requestId: runningRequest }).catch(error => { status.textContent = String(error); });
      runningRequest = "";
    }
    clearSecrets();
    invalidate();
    dialog.close();
  }
  async function run() {
    if (busy) return;
    await invalidate();
    if (mode === "export" && (Array.from(passphrase.value).length < 12 || passphrase.value !== confirmation.value)) {
      status.textContent = "Use at least 12 characters and make both passphrases match.";
      return;
    }
    if (!passphrase.value) { status.textContent = "Enter the backup passphrase."; return; }
    const requestId = crypto.randomUUID();
    runningRequest = requestId;
    setBusy(true);
    status.textContent = mode === "export" ? "Reading the OS credential store and encrypting…" : "Authenticating backup and preparing a read-only preview…";
    try {
      if (mode === "export") {
        // Native save dialogs normally confirm overwrite. Explicit confirmation also covers clients that don't.
        const overwrite = await deps.confirmOverwrite();
        if (!overwrite) { status.textContent = "Export cancelled."; return; }
        const response = deps.invoke("export_settings_backup", { path, passphrase: passphrase.value, confirmation: confirmation.value, overwrite, requestId });
        clearSecrets();
        const result = await response as { complete: boolean; servers: number; credentialsPresent: number; credentialsAbsent: number };
        if (runningRequest !== requestId) return;
        status.textContent = `Encrypted backup complete: ${result.servers} servers, ${result.credentialsPresent} credentials present, ${result.credentialsAbsent} absent.`;
      } else {
        const response = deps.invoke("preview_settings_backup", { path, passphrase: passphrase.value, options: readOptions(), requestId });
        clearSecrets();
        const result = await response as Preview;
        if (runningRequest !== requestId) return;
        previewId = result.id;
        preview.textContent = [
          `Source: ${result.sourcePlatform}, app ${result.sourceVersion}. ${result.servers} servers.`,
          ...result.sections.map(section => `${section.name}: ${section.state}${section.changed ? " (will change)" : ""}`),
          `Credentials: ${result.credentialsSet} set, ${result.credentialsDeleted} delete, ${result.credentialsKept} keep.`,
          ...(result.credentialActions ?? []).map(item => `${item.account}: ${item.action}`),
          ...result.warnings,
          "Executable configuration to review:", JSON.stringify(result.executableConfiguration, null, 2),
          "External references (files are not included):",
          ...result.externalFiles.map(file => `${file.existsHere ? "Found" : "Missing"}: ${file.path}`),
        ].join("\n\n");
        status.textContent = "Review the preview, then confirm replacement. Preview expires after 10 minutes.";
      }
    } catch (error) {
      status.textContent = String(error);
    } finally {
      runningRequest = "";
      clearSecrets();
      setBusy(false);
    }
  }
  async function commit() {
    if (busy || !previewId || !confirmed.checked) return;
    applying = true;
    setBusy(true);
    status.textContent = "Applying settings with recovery protection…";
    try {
      await deps.invoke("import_settings_backup", { previewId, confirmed: true });
      invalidate();
      await deps.afterImport();
      status.textContent = "Settings imported. MCP and CC remain disabled. Review paths and enroll CC before connecting.";
    } catch (error) {
      invalidate();
      status.textContent = String(error);
    } finally { applying = false; setBusy(false); }
  }
  element("cfg-export-settings").addEventListener("click", () => { void open("export").catch(error => { status.textContent = String(error); }); });
  element("cfg-import-settings").addEventListener("click", () => { void open("import").catch(error => { status.textContent = String(error); }); });
  element("backup-cancel").addEventListener("click", close);
  dialog.addEventListener("cancel", event => { event.preventDefault(); close(); });
  options.addEventListener("input", () => { void invalidate(); });
  confirmed.addEventListener("change", () => { replace.disabled = busy || !previewId || !confirmed.checked; });
  action.addEventListener("click", () => { void run(); });
  replace.addEventListener("click", () => { void commit(); });
}
